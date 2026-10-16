//! Area picker, like CleanShot's: the screen freezes, you drag a rectangle
//! over the frozen picture (or click a window), with crosshair guides, a
//! magnifier with the pixel's colour and the selection's size. Shared by
//! screenshots, recording, scrolling capture and text recognition.

use std::sync::Arc;
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

/// Rectangle in the monitor's logical coordinates: x, y, w, h.
type Rect = (f32, f32, f32, f32);

/// What was picked.
pub struct Picked {
    pub mon: Mon,
    /// Logical, on the whole desktop: x, y, w, h.
    pub area: (i32, i32, i32, i32),
    /// The whole monitor was picked.
    pub whole: bool,
    /// The monitor as it was frozen, in physical pixels.
    pub frozen: Arc<image::RgbaImage>,
}

impl Picked {
    /// The picked area cut from the frozen screen, in physical pixels.
    pub fn crop(&self) -> image::RgbaImage {
        let s = self.mon.scale;
        let (x, y) = (((self.area.0 - self.mon.x) as f64 * s).round() as u32, ((self.area.1 - self.mon.y) as f64 * s).round() as u32);
        let (w, h) = ((self.area.2 as f64 * s).round() as u32, (self.area.3 as f64 * s).round() as u32);
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
    let windows = if snap { windows_on(&mon) } else { Vec::new() };
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
    cx.update(|cx| {
        kit::open_layer(cx, spec, move |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                let picture = Arc::new(RenderImage::new([image::Frame::new(bgra(&shown))]));
                Picker { picture, lens: None, frozen: shown, scale, windows, snap, color, cursor: None, start: None, selection: None, moving: false, last: None, done: Some(tx), focus }
            })
        })
    });
    let rect = rx.await.ok().flatten()?;
    let whole = rect.2 >= mon.w as f32 - 1. && rect.3 >= mon.h as f32 - 1.;
    Some(Picked {
        area: (mon.x + rect.0.round() as i32, mon.y + rect.1.round() as i32, rect.2.round().max(1.) as i32, rect.3.round().max(1.) as i32),
        whole,
        mon,
        frozen,
    })
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

struct Picker {
    picture: Arc<RenderImage>,
    /// The magnifier's picture: whole screen pixels as squares with a grid,
    /// for the pointer position it was made at.
    lens: Option<((f32, f32), Arc<RenderImage>)>,
    frozen: Arc<image::RgbaImage>,
    scale: f32,
    windows: Vec<Rect>,
    snap: bool,
    /// A click on nothing copies the colour under it.
    color: bool,
    cursor: Option<(f32, f32)>,
    /// Where the drag began.
    start: Option<(f32, f32)>,
    selection: Option<Rect>,
    /// Space held while dragging moves the selection instead.
    moving: bool,
    last: Option<(f32, f32)>,
    done: Option<oneshot::Sender<Option<Rect>>>,
    focus: FocusHandle,
}

fn local(p: Point<Pixels>) -> (f32, f32) {
    (f32::from(p.x), f32::from(p.y))
}

impl Picker {
    fn finish(&mut self, rect: Option<Rect>, window: &mut Window) {
        if let Some(tx) = self.done.take() {
            let _ = tx.send(rect.filter(|r| r.2 >= 1. && r.3 >= 1.));
        }
        window.remove_window();
    }

    fn hovered_window(&self) -> Option<Rect> {
        let (x, y) = self.cursor?;
        self.windows.iter().copied().find(|r| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3)
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let p = local(e.position);
        if let (Some(start), true) = (self.start, e.pressed_button == Some(MouseButton::Left)) {
            if self.moving {
                // Space: drag the whole selection along.
                if let (Some(last), Some(sel)) = (self.last, self.selection.as_mut()) {
                    let (dx, dy) = (p.0 - last.0, p.1 - last.1);
                    sel.0 += dx;
                    sel.1 += dy;
                    self.start = Some((start.0 + dx, start.1 + dy));
                }
            } else {
                let (mut dx, mut dy) = (p.0 - start.0, p.1 - start.1);
                if e.modifiers.shift {
                    let m = dx.abs().max(dy.abs());
                    (dx, dy) = (m * dx.signum(), m * dy.signum());
                }
                let (x0, y0) = (start.0.min(start.0 + dx), start.1.min(start.1 + dy));
                self.selection = Some((x0, y0, dx.abs(), dy.abs()));
            }
        }
        self.cursor = Some(p);
        self.last = Some(p);
        cx.notify();
    }

    fn mouse_up(&mut self, e: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.button != MouseButton::Left || self.start.is_none() {
            return;
        }
        self.start = None;
        let picked = match self.selection {
            Some(sel) if sel.2 >= 4. && sel.3 >= 4. => Some(sel),
            // A click: the window under it, else the colour or the whole monitor.
            _ => match self.hovered_window().filter(|_| self.snap) {
                Some(win) => Some(win),
                None if self.color => {
                    if let Some(hex) = self.color_at(local(e.position)) {
                        crate::platform::copy_text(&hex);
                        crate::platform::notify(t("picker.color_copied"), &hex);
                    }
                    None
                }
                None => {
                    let screen = window.viewport_size();
                    Some((0., 0., f32::from(screen.width), f32::from(screen.height)))
                }
            },
        };
        self.finish(picked, window);
        cx.notify();
    }

    /// The picked colour under the pointer, as "#rrggbb".
    /// The magnifier around (x, y): each screen pixel a sharp CELL square,
    /// a light blue grid along their edges. Made by hand, since a scaled
    /// image would come out smoothed.
    fn lens_at(&self, (x, y): (f32, f32)) -> Arc<RenderImage> {
        let side = (LOUPE * self.scale).round() as u32;
        let cell = CELL * self.scale;
        let (cx, cy) = (x * self.scale, y * self.scale);
        let (fw, fh) = self.frozen.dimensions();
        let grid = [0x9C, 0xD0, 0xFF];
        let image = image::RgbaImage::from_fn(side, side, |u, v| {
            let sx = cx + (u as f32 + 0.5 - side as f32 / 2.) / cell;
            let sy = cy + (v as f32 + 0.5 - side as f32 / 2.) / cell;
            let mut p = if sx < 0. || sy < 0. || sx >= fw as f32 || sy >= fh as f32 { [0, 0, 0, 255] } else { self.frozen.get_pixel(sx as u32, sy as u32).0 };
            if sx.rem_euclid(1.) * cell < 1. || sy.rem_euclid(1.) * cell < 1. {
                for i in 0..3 {
                    p[i] = (p[i] as f32 * 0.65 + grid[i] as f32 * 0.35) as u8;
                }
            }
            // BGRA for GPUI.
            image::Rgba([p[2], p[1], p[0], 255])
        });
        Arc::new(RenderImage::new([image::Frame::new(image)]))
    }

    fn color_at(&self, (x, y): (f32, f32)) -> Option<String> {
        let (px_x, px_y) = ((x * self.scale) as u32, (y * self.scale) as u32);
        let p = self.frozen.get_pixel_checked(px_x, px_y)?.0;
        Some(format!("#{:02X}{:02X}{:02X}", p[0], p[1], p[2]))
    }
}

/// Size of the magnifier, and how big one screen pixel is in it.
const LOUPE: f32 = 132.;
const CELL: f32 = 11.;

impl Render for Picker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = window.viewport_size();
        let (sw, sh) = (f32::from(screen.width), f32::from(screen.height));
        let accent = cx.theme().colors().text_accent;
        let picture = self.picture.clone();
        let selection = self.selection;
        let dragging = self.start.is_some();
        let hovered = if selection.is_none() && self.snap { self.hovered_window() } else { None };
        let lit = selection.or(hovered);
        let cursor = self.cursor;

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
                if let (Some((cx0, cy0)), false) = (cursor, dragging) {
                    let guide = gpui::hsla(0., 0., 1., 0.35);
                    window.paint_quad(fill(at(0., cy0.floor(), sw, 1.), guide));
                    window.paint_quad(fill(at(cx0.floor(), 0., 1., sh), guide));
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        // The selection's size in real pixels, under its bottom-right corner.
        let label = selection.map(|(x, y, w, h)| {
            let (rw, rh) = ((w * self.scale).round(), (h * self.scale).round());
            let (lx, ly) = ((x + w - 120.).max(4.), if y + h + 34. < sh { y + h + 8. } else { (y - 30.).max(4.) });
            div()
                .absolute()
                .left(px(lx))
                .top(px(ly))
                .w(px(120.))
                .flex()
                .justify_end()
                .child(div().px_2().py_0p5().rounded_md().bg(gpui::hsla(0., 0., 0., 0.72)).font_family(kit::MONO_FONT).text_sm().text_color(gpui::white()).child(format!("{rw} × {rh}")))
        });

        // The magnifier follows the pointer, flipping away from the edges.
        if let Some(at) = cursor
            && self.lens.as_ref().is_none_or(|l| l.0 != at)
        {
            let lens = self.lens_at(at);
            if let Some((_, old)) = self.lens.replace((at, lens)) {
                let _ = window.drop_image(old);
            }
        }
        let loupe = cursor.map(|(x, y)| {
            let (lx, ly) = (if x + 24. + LOUPE < sw { x + 24. } else { x - 24. - LOUPE }, if y + 24. + LOUPE + 40. < sh { y + 24. } else { y - 24. - LOUPE - 40. });
            let picture = self.lens.as_ref().map(|l| l.1.clone());
            let scale = self.scale;
            let lens = canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let c = bounds.center();
                    if let Some(picture) = picture.clone() {
                        let _ = window.paint_image(bounds, bounds, px(12.).into(), 0., picture, 0, false);
                    }
                    // Where the screen's pixel grid falls in the lens.
                    let zoom = CELL * scale;
                    let (ox, oy) = (f32::from(c.x) - x * zoom, f32::from(c.y) - y * zoom);
                    // The pixel under the pointer, outlined.
                    let (ix, iy) = ((x * scale).floor(), (y * scale).floor());
                    let pixel = Bounds::new(point(px(ox + ix * CELL), px(oy + iy * CELL)), size(px(CELL + 1.), px(CELL + 1.)));
                    window.paint_quad(quad(pixel, px(1.), gpui::transparent_black(), px(1.5), gpui::white(), gpui::BorderStyle::Solid));
                },
            )
            .size(px(LOUPE));
            div()
                .absolute()
                .left(px(lx))
                .top(px(ly))
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .child(div().size(px(LOUPE)).rounded(px(12.)).border_2().border_color(gpui::white()).shadow(kit::shadow()).child(lens))
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_md()
                        .bg(gpui::hsla(0., 0., 0., 0.72))
                        .font_family(kit::MONO_FONT)
                        .text_xs()
                        .text_color(gpui::white())
                        .child(format!("{}, {}  {}", (x * self.scale).round(), (y * self.scale).round(), self.color_at((x, y)).unwrap_or_default())),
                )
        });

        kit::root(cx)
            .relative()
            .cursor(CursorStyle::Crosshair)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                match e.keystroke.key.as_str() {
                    "escape" => this.finish(None, window),
                    "enter" => {
                        let sel = this.selection;
                        if sel.is_some() {
                            this.finish(sel, window);
                        }
                    }
                    "space" => this.moving = true,
                    _ => {}
                }
                cx.notify();
            }))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, _| {
                if e.keystroke.key == "space" {
                    this.moving = false;
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
                let p = local(e.position);
                this.start = Some(p);
                this.last = Some(p);
                this.selection = None;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, _, window, _| this.finish(None, window)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, window, cx| this.mouse_up(e, window, cx)))
            .child(scene)
            .children(label)
            .children(loupe)
    }
}

/// The whole monitor under the pointer, frozen, without picking (a fullscreen shot).
pub async fn whole_monitor() -> Option<Picked> {
    let mon = util::focused_monitor()?;
    let frozen = Arc::new(crate::platform::grab_monitor(&mon)?);
    Some(Picked { area: (mon.x, mon.y, mon.w, mon.h), whole: true, mon, frozen })
}
