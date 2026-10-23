//! Area picker: the screen freezes, you drag a rectangle
//! over the frozen picture (or click a window), with crosshair guides, a
//! magnifier with the pixel's colour and the selection's size. Shared by
//! screenshots, recording, scrolling capture and text recognition.
//!
//! The area is taken when the mouse is let go (a
//! recording's is adjusted afterwards, on the recording bar). Once the
//! keyboard has touched it the selection stays: it has handles, moves when dragged from inside, nudges with the arrows, keeps
//! an aspect ratio (A), takes a typed size (digits) and is taken with Enter.
//! P brings back the last area picked.

use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::channel::oneshot;
use gpui::{
    AsyncApp, Bounds, Context, CursorStyle, FocusHandle, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, RenderImage, Window, canvas, div, point, px, size,
};

use crate::kit::{self, Anchor, KeyboardInteractivity, LayerSpec};
use crate::ui::{fill, outline, prelude::*, quad};
use crate::util::{self, Mon};
use crate::i18n::t;


/// What was picked.
pub struct Picked {
    pub mon: Mon,
    /// Logical, on the whole desktop: x, y, w, h.
    pub area: (i32, i32, i32, i32),
    /// The whole monitor was picked.
    pub whole: bool,
    /// A window was clicked: the area is that window's.
    pub window: bool,
    /// The monitor as it was frozen, in physical pixels.
    pub frozen: Arc<image::RgbaImage>,
}

impl Picked {
    /// The picked area cut from the frozen screen, in physical pixels.
    pub fn crop(&self) -> image::RgbaImage {
        // Edges rounded on their own, as the compositor draws windows: rounding
        // the size instead can take in a pixel of the border next to it.
        let s = self.mon.scale;
        let px = |v: i32| (v as f64 * s).round() as u32;
        let (lx, ly) = (self.area.0 - self.mon.x, self.area.1 - self.mon.y);
        let (x, y) = (px(lx), px(ly));
        let (w, h) = (px(lx + self.area.2) - x, px(ly + self.area.3) - y);
        let (fw, fh) = self.frozen.dimensions();
        image::imageops::crop_imm(self.frozen.as_ref(), x.min(fw - 1), y.min(fh - 1), w.min(fw - x.min(fw - 1)).max(1), h.min(fh - y.min(fh - 1)).max(1)).to_image()
    }

    /// "X,Y WxH" for grim and the recorder.
    pub fn geometry(&self) -> String {
        format!("{},{} {}x{}", self.area.0, self.area.1, self.area.2, self.area.3)
    }
}

/// Freeze the monitor under the pointer and let the user pick an area on it.
/// With `snap`, hovering highlights windows and a plain click takes one.
/// Space switches to picking windows only and back.
/// A click on nothing takes the whole monitor, or with `color` copies the
/// colour under the pointer instead. None when cancelled.
pub async fn pick(snap: bool, color: bool, cx: &mut AsyncApp) -> Option<Picked> {
    // One selection at a time: a second hotkey while picking does nothing.
    if PICKING.swap(true, Ordering::SeqCst) {
        return None;
    }
    let _picking = Picking;
    let mon = util::focused_monitor()?;
    let frozen = Arc::new(crate::platform::grab_monitor(&mon)?);
    let windows = windows_on(&mon);
    // The last area, if it was on this monitor.
    let previous = previous_area()
        .filter(|(name, _)| *name == mon.name)
        .map(|(_, (x, y, w, h))| ((x - mon.x) as f32, (y - mon.y) as f32, w as f32, h as f32))
        .filter(|r| r.0 >= 0. && r.1 >= 0. && r.0 + r.2 <= mon.w as f32 + 0.5 && r.1 + r.3 <= mon.h as f32 + 0.5);
    // On Wayland, the overlay's own window: drawn on the CPU, on a thread
    // of its own (SPEC 3.8). GPUI's picker below where it can't show itself,
    // or with KADR_PICKER=gpui.
    #[cfg(target_os = "linux")]
    if std::env::var("KADR_PICKER").as_deref() != Ok("gpui") {
        let accent = cx.update(|cx| crate::ui::to_rgba(cx.theme().colors().text_accent));
        let screen = kadr_overlay::Screen { output: mon.name.clone(), logical: (mon.w as u32, mon.h as u32), scale: mon.scale, frozen: frozen.clone(), windows: windows.clone(), previous, origin: (mon.x as f64, mon.y as f64) };
        let options = kadr_overlay::Options {
            snap,
            color,
            accent,
            free: t("picker.free").to_owned(),
            t: |k| crate::i18n::t(k).to_owned(),
            pointer: Some((crate::platform::cursor_pos, crate::platform::move_cursor)),
        };
        let (tx, rx) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(kadr_overlay::pick(&screen, &options));
        });
        match rx.await {
            Ok(Ok(outcome)) => return picked(outcome, mon, frozen),
            _ => {}
        }
    }
    let (tx, rx) = oneshot::channel();
    let shown = frozen.clone();
    let spec = LayerSpec {
        namespace: "kadr-picker",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mon.w as f32, mon.h as f32),
        keyboard: KeyboardInteractivity::Exclusive,
        ignore_zones: true,
        monitor: Some(mon.clone()),
    };
    let scale = mon.scale as f32;
    let screen = (mon.w as f32, mon.h as f32);
    let origin = (mon.x as f32, mon.y as f32);
    cx.update(|cx| {
        kit::open_layer(cx, spec, move |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                let picture = Arc::new(RenderImage::new([image::Frame::new(bgra(&shown))]));
                let mut m = Model::new(screen, scale);
                m.windows = windows;
                m.snap = snap;
                m.color = color;
                m.previous = previous;
                Picker { m, picture, lens: None, frozen: shown, origin, watching: false, done: Some(tx), focus }
            })
        })
    });
    let (rect, window) = rx.await.ok().flatten()?;
    to_picked(rect, window, mon, frozen)
}

/// What the overlay ended in, as what the app takes: an area, or the
/// colour under a plain click copied (and nothing to take).
#[cfg(target_os = "linux")]
fn picked(outcome: Option<kadr_overlay::Outcome>, mon: Mon, frozen: Arc<image::RgbaImage>) -> Option<Picked> {
    match outcome? {
        kadr_overlay::Outcome::Done(picked) => {
            let (rect, window) = picked?;
            to_picked(rect, window, mon, frozen)
        }
        kadr_overlay::Outcome::Color((x, y)) => {
            let s = mon.scale as f32;
            let p = frozen.get_pixel_checked((x * s) as u32, (y * s) as u32)?.0;
            let hex = format!("#{:02X}{:02X}{:02X}", p[0], p[1], p[2]);
            crate::platform::copy_text(&hex);
            crate::platform::notify(t("picker.color_copied"), &hex);
            None
        }
    }
}

fn to_picked(rect: Rect, window: bool, mon: Mon, frozen: Arc<image::RgbaImage>) -> Option<Picked> {
    let whole = rect.2 >= mon.w as f32 - 1. && rect.3 >= mon.h as f32 - 1.;
    let area = (mon.x + rect.0.round() as i32, mon.y + rect.1.round() as i32, rect.2.round().max(1.) as i32, rect.3.round().max(1.) as i32);
    if !whole {
        remember(&mon.name, area);
    }
    Some(Picked { area, whole, window, mon, frozen })
}

fn previous_file() -> std::path::PathBuf {
    util::config_dir().join("last-area.json")
}

/// The last area picked: its monitor's name and the area on the desktop.
pub fn previous_area() -> Option<(String, (i32, i32, i32, i32))> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(previous_file()).ok()?).ok()?;
    let n = |i: usize| v["area"][i].as_i64().map(|n| n as i32);
    let area = (n(0)?, n(1)?, n(2)?, n(3)?);
    (area.2 > 0 && area.3 > 0).then(|| (v["monitor"].as_str().unwrap_or_default().to_owned(), area))
}

pub fn remember(monitor: &str, (x, y, w, h): (i32, i32, i32, i32)) {
    let _ = std::fs::create_dir_all(util::config_dir());
    let _ = std::fs::write(previous_file(), serde_json::json!({ "monitor": monitor, "area": [x, y, w, h] }).to_string());
}

/// The last area again without picking, cut from a fresh grab of its
/// monitor. None (with a notice) when there isn't one yet.
pub fn previous() -> Option<Picked> {
    let Some((name, area)) = previous_area() else {
        util::notify("kadr", t("picker.no_previous_area"));
        return None;
    };
    let mons = util::monitors();
    let mon = mons.iter().find(|m| m.name == name).or_else(|| mons.iter().find(|m| m.contains(area.0 + area.2 / 2, area.1 + area.3 / 2)))?.clone();
    let frozen = Arc::new(crate::platform::grab_monitor(&mon)?);
    Some(Picked { area, whole: false, window: false, mon, frozen })
}

static PICKING: AtomicBool = AtomicBool::new(false);

/// Clears `PICKING` however `pick` ends.
struct Picking;

impl Drop for Picking {
    fn drop(&mut self) {
        PICKING.store(false, Ordering::SeqCst);
    }
}

/// GPUI keeps images as BGRA.
fn bgra(rgba: &image::RgbaImage) -> image::RgbaImage {
    let mut out = rgba.clone();
    for px in out.pixels_mut() {
        px.0.swap(0, 2);
    }
    out
}

/// Visible windows on the monitor's workspace, in its logical coordinates,
/// floating ones first (they're on top), then smaller before larger.
fn windows_on(mon: &Mon) -> Vec<Rect> {
    let monitors: serde_json::Value = util::hypr("j/monitors").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let workspace = monitors.as_array().into_iter().flatten().find(|m| m["name"] == mon.name.as_str()).and_then(|m| m["activeWorkspace"]["id"].as_i64());
    let clients: serde_json::Value = util::hypr("j/clients").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut found: Vec<(bool, Rect)> = clients
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["mapped"] == true && c["hidden"] != true && c["workspace"]["id"].as_i64() == workspace)
        .filter_map(|c| {
            let (x, y) = (c["at"][0].as_f64()?, c["at"][1].as_f64()?);
            let (w, h) = (c["size"][0].as_f64()?, c["size"][1].as_f64()?);
            Some((c["floating"] == true, ((x - mon.x as f64) as f32, (y - mon.y as f64) as f32, w as f32, h as f32)))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then((a.1.2 * a.1.3).total_cmp(&(b.1.2 * b.1.3))));
    found.into_iter().map(|(_, r)| r).collect()
}

pub(crate) use kadr_overlay::model::{BOTTOM, LEFT, RIGHT, Rect, TOP, handles};
use kadr_overlay::model::{ASPECTS, Cursor, Drag, Model, Outcome};

/// The GPUI side of picking: the frozen picture, the magnifier and the
/// window it's drawn in; the selection and what the pointer and keys do to
/// it are `kadr_overlay::model`.
struct Picker {
    m: Model,
    picture: Arc<RenderImage>,
    /// The magnifier's picture: whole screen pixels as squares with a grid,
    /// for the pointer position it was made at.
    lens: Option<((u32, u32), Arc<RenderImage>)>,
    frozen: Arc<image::RgbaImage>,
    /// The monitor's place in the layout.
    origin: (f32, f32),
    /// Keeping the pointer on this monitor while dragging.
    watching: bool,
    done: Option<oneshot::Sender<Option<(Rect, bool)>>>,
    focus: FocusHandle,
}

fn local(p: Point<Pixels>) -> (f32, f32) {
    (f32::from(p.x), f32::from(p.y))
}

pub(crate) fn resize_cursor(edges: u8) -> CursorStyle {
    match edges {
        LEFT | RIGHT => CursorStyle::ResizeLeftRight,
        TOP | BOTTOM => CursorStyle::ResizeUpDown,
        e if e == LEFT | TOP || e == RIGHT | BOTTOM => CursorStyle::ResizeUpLeftDownRight,
        _ => CursorStyle::ResizeUpRightDownLeft,
    }
}

impl Picker {
    fn finish(&mut self, rect: Option<Rect>, window_picked: bool, window: &mut Window) {
        if let Some(tx) = self.done.take() {
            let _ = tx.send(rect.filter(|r| r.2 >= 1. && r.3 >= 1.).map(|r| (r, window_picked)));
        }
        window.remove_window();
    }

    fn outcome(&mut self, outcome: Option<Outcome>, window: &mut Window) {
        match outcome {
            Some(Outcome::Done(picked)) => {
                let (rect, window_picked) = picked.map_or((None, false), |(r, w)| (Some(r), w));
                self.finish(rect, window_picked, window);
            }
            Some(Outcome::Color(at)) => {
                if let Some(hex) = self.color_at(at) {
                    crate::platform::copy_text(&hex);
                    crate::platform::notify(t("picker.color_copied"), &hex);
                }
                self.finish(None, false, window);
            }
            None => {}
        }
    }

    /// While dragging, keep the pointer on this monitor: the picker only
    /// gets pointer events over itself, so past the edge the selection would
    /// stop short, and a button let go on another monitor would never come.
    /// Pushed past an edge, the selection takes it in whole.
    fn watch_pointer(&mut self, cx: &mut Context<Self>) {
        if self.m.drag.is_none() || self.watching {
            return;
        }
        self.watching = true;
        let ((ox, oy), (sw, sh)) = (self.origin, self.m.screen);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(8)).await;
                let pos = crate::platform::cursor_pos();
                let going = this.update(cx, |picker, cx| {
                    if picker.m.drag.is_none() {
                        picker.watching = false;
                        return false;
                    }
                    if let Some((x, y)) = pos {
                        let (lx, ly) = (x as f32 - ox, y as f32 - oy);
                        if lx < 0. || ly < 0. || lx >= sw || ly >= sh {
                            let (cx0, cy0) = (lx.clamp(0., sw - 1.), ly.clamp(0., sh - 1.));
                            crate::platform::move_cursor((ox + cx0) as f64, (oy + cy0) as f64);
                            picker.m.drag_to((lx.clamp(0., sw), ly.clamp(0., sh)));
                            picker.m.cursor = Some((cx0, cy0));
                            cx.notify();
                        }
                    }
                    true
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// One screen pixel's side in the magnifier, in physical pixels: whole,
    /// so every pixel there is an equal square.
    fn cell_px(&self) -> u32 {
        (CELL * self.m.scale).round().max(2.) as u32
    }

    /// The magnifier's side in logical pixels: a whole number of cells.
    fn loupe(&self) -> f32 {
        (CELLS * self.cell_px()) as f32 / self.m.scale
    }

    /// The magnifier around the pixel (ix, iy), which sits whole in the
    /// middle: each screen pixel a sharp square, a light blue grid between
    /// them. Made by hand, since a scaled image would come out smoothed.
    fn lens_at(&self, (ix, iy): (u32, u32)) -> Arc<RenderImage> {
        let cell = self.cell_px();
        let side = CELLS * cell;
        let half = (CELLS / 2) as i64;
        let (fw, fh) = self.frozen.dimensions();
        let grid = [0x9C, 0xD0, 0xFF];
        let image = image::RgbaImage::from_fn(side, side, |u, v| {
            let sx = ix as i64 + (u / cell) as i64 - half;
            let sy = iy as i64 + (v / cell) as i64 - half;
            let mut p = if sx < 0 || sy < 0 || sx >= fw as i64 || sy >= fh as i64 { [0, 0, 0, 255] } else { self.frozen.get_pixel(sx as u32, sy as u32).0 };
            if (u % cell == 0 && u > 0) || (v % cell == 0 && v > 0) {
                for i in 0..3 {
                    p[i] = (p[i] as f32 * 0.65 + grid[i] as f32 * 0.35) as u8;
                }
            }
            // BGRA for GPUI.
            image::Rgba([p[2], p[1], p[0], 255])
        });
        Arc::new(RenderImage::new([image::Frame::new(image)]))
    }

    /// The colour under the pointer, as "#RRGGBB".
    fn color_at(&self, (x, y): (f32, f32)) -> Option<String> {
        let (px_x, px_y) = ((x * self.m.scale) as u32, (y * self.m.scale) as u32);
        let p = self.frozen.get_pixel_checked(px_x, px_y)?.0;
        Some(format!("#{:02X}{:02X}{:02X}", p[0], p[1], p[2]))
    }

    fn cursor_style(&self) -> CursorStyle {
        match self.m.cursor_shape() {
            Cursor::Resize(edges) => resize_cursor(edges),
            Cursor::ClosedHand => CursorStyle::ClosedHand,
            Cursor::OpenHand => CursorStyle::OpenHand,
            Cursor::PointingHand => CursorStyle::PointingHand,
            Cursor::Crosshair => CursorStyle::Crosshair,
        }
    }
}

/// How many screen pixels the magnifier shows across (odd, so one is in
/// the middle), and about how big each is, in logical pixels.
const CELLS: u32 = 13;
const CELL: f32 = 11.;

/// The dark plates the picker writes on.
fn plate() -> gpui::Div {
    div().px_2().py_0p5().rounded_md().bg(gpui::hsla(0., 0., 0., 0.72)).text_color(gpui::white())
}

impl Render for Picker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (sw, sh) = self.m.screen;
        let accent = cx.theme().colors().text_accent;
        let picture = self.picture.clone();
        let selection = self.m.selection;
        let editable = self.m.editable();
        let hovered = if selection.is_none() && (self.m.snap || self.m.windows_only) && self.m.drag.is_none() { self.m.hovered_window() } else { None };
        let lit = selection.or(hovered);
        // Guides and the magnifier while aiming, not while adjusting.
        let aiming = (!editable && !self.m.windows_only) || matches!(self.m.drag, Some(Drag::Resize { .. }));
        let cursor = self.m.cursor;
        let guides = cursor.filter(|_| self.m.drag.is_none() && !editable && !self.m.windows_only);

        // The frozen screen, dimmed outside what would be taken, with guides.
        let scene = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let _ = window.paint_image(bounds, bounds, px(0.).into(), 0., picture.clone(), 0, false);
                let o = bounds.origin;
                let at = |x: f32, y: f32, w: f32, h: f32| Bounds::new(point(o.x + px(x), o.y + px(y)), size(px(w.max(0.)), px(h.max(0.))));
                let shade = gpui::hsla(0., 0., 0., 0.42);
                match lit {
                    Some((x, y, w, h)) => {
                        for r in [at(0., 0., sw, y), at(0., y + h, sw, sh - y - h), at(0., y, x, h), at(x + w, y, sw - x - w, h)] {
                            window.paint_quad(fill(r, shade));
                        }
                        let color = if selection.is_some() { gpui::white() } else { accent };
                        window.paint_quad(outline(at(x - 1., y - 1., w + 2., h + 2.), color, gpui::BorderStyle::Solid));
                    }
                    None => window.paint_quad(fill(at(0., 0., sw, sh), shade)),
                }
                if let (Some(sel), true) = (selection, editable) {
                    for (_, (hx, hy)) in handles(sel) {
                        crate::ui::paint_knob(window, point(o.x + px(hx), o.y + px(hy)), accent);
                    }
                }
                if let Some((cx0, cy0)) = guides {
                    let guide = gpui::hsla(0., 0., 1., 0.35);
                    window.paint_quad(fill(at(0., cy0.floor(), sw, 1.), guide));
                    window.paint_quad(fill(at(cx0.floor(), 0., 1., sh), guide));
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        // The selection's size in real pixels under its bottom-right corner,
        // with the aspect ratio; a typed size shows as it's typed.
        let label = selection.map(|(x, y, w, h)| {
            let size_text = match &self.m.typing {
                Some((tw, th, on_h)) => format!("{}{} × {}{}", tw, if *on_h { "" } else { "|" }, th, if *on_h { "|" } else { "" }),
                None => format!("{} × {}", (w * self.m.scale).round(), (h * self.m.scale).round()),
            };
            let (aspect_key, ratio) = ASPECTS[self.m.aspect];
            let aspect = if ratio.is_some() { aspect_key } else { t(aspect_key) };
            let width = 240.;
            let (lx, ly) = ((x + w - width).max(4.), if y + h + 34. < sh { y + h + 8. } else { (y - 30.).max(4.) });
            div()
                .absolute()
                .left(px(lx))
                .top(px(ly))
                .w(px(width))
                .flex()
                .justify_end()
                .gap_1()
                .child(plate().font_family(kit::MONO_FONT).text_sm().child(size_text))
                .when(editable || ratio.is_some(), |row| {
                    row.child(
                        plate()
                            .text_sm()
                            .cursor_pointer()
                            .when(ratio.is_none(), |p| p.text_color(gpui::hsla(0., 0., 1., 0.7)))
                            .child(aspect)
                            // A click cycles the ratio, like A.
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.m.key("a", false, false, false);
                                cx.notify();
                            })),
                    )
                })
        });

        // The magnifier follows the pointer, flipping away from the edges.
        // Made again only when the pointer reaches another pixel.
        if let Some(at) = cursor.map(|(x, y)| ((x * self.m.scale).max(0.) as u32, (y * self.m.scale).max(0.) as u32))
            && self.lens.as_ref().is_none_or(|l| l.0 != at)
        {
            let lens = self.lens_at(at);
            if let Some((_, old)) = self.lens.replace((at, lens)) {
                let _ = window.drop_image(old);
            }
        }
        let (scale, size_l, cell) = (self.m.scale, self.loupe(), self.cell_px() as f32 / self.m.scale);
        // The white frame sits outside the lens, a whole number of physical
        // pixels wide, and the lens lands on whole pixels: nothing is cut.
        let edge = (2. * scale).round() / scale;
        let outer = size_l + 2. * edge;
        let loupe = cursor.filter(|_| aiming).map(|(x, y)| {
            let snap = |v: f32| (v * scale).round() / scale;
            let (lx, ly) = (snap(if x + 24. + outer < sw { x + 24. } else { x - 24. - outer }), snap(if y + 24. + outer + 40. < sh { y + 24. } else { y - 24. - outer - 40. }));
            let picture = self.lens.as_ref().map(|l| l.1.clone());
            let lens = canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    if let Some(picture) = picture.clone() {
                        let _ = window.paint_image(bounds, bounds, px(12.).into(), 0., picture, 0, false);
                    }
                    // The pixel under the pointer, outlined: the middle cell.
                    let mid = (CELLS / 2) as f32 * cell;
                    let pixel = Bounds::new(point(bounds.origin.x + px(mid), bounds.origin.y + px(mid)), size(px(cell + 1. / scale), px(cell + 1. / scale)));
                    window.paint_quad(quad(pixel, px(1.), gpui::transparent_black(), px(1.5), gpui::white(), gpui::BorderStyle::Solid));
                },
            )
            .size(px(size_l));
            div()
                .absolute()
                .left(px(lx))
                .top(px(ly))
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(div().size(px(outer)).p(px(edge)).rounded(px(12. + edge)).bg(gpui::white()).shadow(kit::shadow()).child(lens))
                .child(plate().font_family(kit::MONO_FONT).text_xs().child(format!("{}, {}  {}", (x * self.m.scale).round(), (y * self.m.scale).round(), self.color_at((x, y)).unwrap_or_default())))
        });

        // What the keys do right now, at the bottom of the screen.
        let mut keys: Vec<(&str, &str)> = Vec::new();
        if self.m.typing.is_some() {
            keys.extend([("Enter", t("picker.set_size")), ("X", t("picker.height")), ("Esc", t("common.cancel"))]);
        } else if editable {
            keys.extend([("Enter", t("picker.take")), (t("picker.arrows"), t("picker.move")), (t("picker.alt_arrows"), t("picker.resize")), ("A", t("picker.aspect")), ("0–9", t("picker.size")), ("Esc", t("common.cancel"))]);
        } else if self.m.windows_only {
            keys.extend([(t("picker.click"), t("picker.take_window")), (t("picker.space"), t("picker.area")), ("Esc", t("common.cancel"))]);
        } else if self.m.drag.is_none() {
            keys.push((t("picker.space"), t("common.window")));
            if self.m.previous.is_some() {
                keys.extend([("P", t("picker.previous_area")), ("Enter", t("picker.take_previous"))]);
            }
            keys.extend([("A", ASPECTS[self.m.aspect].1.map(|_| ASPECTS[self.m.aspect].0).unwrap_or(t("picker.aspect"))), ("Esc", t("common.cancel"))]);
        }
        let hints = (!keys.is_empty()).then(|| {
            div().absolute().bottom(px(28. * kit::SCALE)).left_0().right_0().flex().justify_center().child(
                // Styled like the tooltips.
                crate::ui::tip_plate().py(crate::ui::rems_from_px(4.)).gap_3().children(
                    keys.into_iter().map(|(key, what)| div().flex().gap_1().child(div().text_color(crate::ui::TIP_DIM).child(key)).child(what)),
                ),
            )
        });

        kit::root(cx)
            .relative()
            .cursor(self.cursor_style())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let k = &e.keystroke;
                let outcome = this.m.key(&k.key, k.modifiers.shift, k.modifiers.alt, e.is_held);
                this.outcome(outcome, window);
                cx.notify();
            }))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, _| this.m.key_up(&e.keystroke.key)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
                this.m.press(local(e.position), e.click_count);
                this.watch_pointer(cx);
                cx.notify();
            }))
            // On release, for the same reason as `pending`.
            .on_mouse_up(MouseButton::Right, cx.listener(|this, _, window, _| this.finish(None, false, window)))
            // Not only while the button reads as pressed: GPUI forgets it when the
            // pointer leaves the monitor, and the drag goes on once it's back.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                this.m.motion(local(e.position), e.modifiers.shift);
                cx.notify();
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, window, cx| {
                let outcome = this.m.release();
                this.outcome(outcome, window);
                cx.notify();
            }))
            .child(scene)
            .children(hints)
            .children(label)
            .children(loupe)
    }
}

/// The whole monitor under the pointer, frozen, without picking (a fullscreen shot).
pub async fn whole_monitor() -> Option<Picked> {
    let mon = util::focused_monitor()?;
    let frozen = Arc::new(crate::platform::grab_monitor(&mon)?);
    Some(Picked { area: (mon.x, mon.y, mon.w, mon.h), whole: true, window: false, mon, frozen })
}
