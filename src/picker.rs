//! Area picker, like CleanShot's: the screen freezes, you drag a rectangle
//! over the frozen picture (or click a window), with crosshair guides, a
//! magnifier with the pixel's colour and the selection's size. Shared by
//! screenshots, recording, scrolling capture and text recognition.
//!
//! A screenshot is taken when the mouse is let go, like CleanShot's. For a
//! recording (or once the keyboard has touched it) the selection stays: it
//! has handles, moves when dragged from inside, nudges with the arrows, keeps
//! an aspect ratio (A), takes a typed size (digits) and is taken with Enter.
//! P brings back the last area picked.

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
    /// A window was clicked: the area is that window's.
    pub window: bool,
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

/// How the picker behaves.
#[derive(Clone, Copy, Default)]
pub struct PickOpts {
    /// Hovering highlights windows and a plain click takes one.
    pub snap: bool,
    /// A click on nothing copies the colour under the pointer.
    pub color: bool,
    /// The selection stays after the mouse is let go, to be adjusted and
    /// taken with Enter (recording).
    pub adjust: bool,
}

/// Freeze the monitor under the pointer and let the user pick an area on it.
/// With `snap`, hovering highlights windows and a plain click takes one.
/// A click on nothing takes the whole monitor, or with `color` copies the
/// colour under the pointer instead. None when cancelled.
pub async fn pick(snap: bool, color: bool, cx: &mut AsyncApp) -> Option<Picked> {
    pick_with(PickOpts { snap, color, adjust: false }, cx).await
}

pub async fn pick_with(opts: PickOpts, cx: &mut AsyncApp) -> Option<Picked> {
    // One selection at a time: a second hotkey while picking does nothing.
    if PICKING.swap(true, Ordering::SeqCst) {
        return None;
    }
    let _picking = Picking;
    let mon = util::focused_monitor()?;
    let frozen = Arc::new(crate::platform::grab_monitor(&mon)?);
    let windows = if opts.snap { windows_on(&mon) } else { Vec::new() };
    // The last area, if it was on this monitor.
    let previous = previous_area()
        .filter(|(name, _)| *name == mon.name)
        .map(|(_, (x, y, w, h))| ((x - mon.x) as f32, (y - mon.y) as f32, w as f32, h as f32))
        .filter(|r| r.0 >= 0. && r.1 >= 0. && r.0 + r.2 <= mon.w as f32 + 0.5 && r.1 + r.3 <= mon.h as f32 + 0.5);
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
    cx.update(|cx| {
        kit::open_layer(cx, spec, move |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                let picture = Arc::new(RenderImage::new([image::Frame::new(bgra(&shown))]));
                Picker {
                    picture,
                    lens: None,
                    frozen: shown,
                    scale,
                    screen,
                    windows,
                    snap: opts.snap,
                    color: opts.color,
                    adjust: opts.adjust,
                    previous,
                    aspect: 0,
                    typing: None,
                    cursor: None,
                    drag: None,
                    before: None,
                    selection: None,
                    moving: false,
                    last: None,
                    done: Some(tx),
                    focus,
                }
            })
        })
    });
    let (rect, window) = rx.await.ok().flatten()?;
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

fn remember(monitor: &str, (x, y, w, h): (i32, i32, i32, i32)) {
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

/// Aspect ratios the selection can keep (A cycles them): width over height.
const ASPECTS: [(&str, Option<f32>); 5] = [("picker.free", None), ("1:1", Some(1.)), ("4:3", Some(4. / 3.)), ("16:9", Some(16. / 9.)), ("9:16", Some(9. / 16.))];

/// The selection's edges a handle moves.
const LEFT: u8 = 1;
const RIGHT: u8 = 2;
const TOP: u8 = 4;
const BOTTOM: u8 = 8;

/// How near a handle the pointer has to be to grab it, and its drawn size.
const HANDLE_REACH: f32 = 10.;
const HANDLE: f32 = 9.;

#[derive(Clone, Copy)]
enum Drag {
    /// Drawing a new selection from this corner.
    New((f32, f32)),
    /// Moving the selection; the pointer's last position.
    Move((f32, f32)),
    /// Dragging a handle: the edges it moves, the selection and the pointer
    /// when it was grabbed.
    Resize { edges: u8, from: Rect, start: (f32, f32) },
}

struct Picker {
    picture: Arc<RenderImage>,
    /// The magnifier's picture: whole screen pixels as squares with a grid,
    /// for the pointer position it was made at.
    lens: Option<((u32, u32), Arc<RenderImage>)>,
    frozen: Arc<image::RgbaImage>,
    scale: f32,
    /// The monitor's logical size.
    screen: (f32, f32),
    windows: Vec<Rect>,
    snap: bool,
    /// A click on nothing copies the colour under it.
    color: bool,
    /// The selection stays when the mouse is let go.
    adjust: bool,
    /// The last area picked on this monitor (P brings it back).
    previous: Option<Rect>,
    /// Index into `ASPECTS`.
    aspect: usize,
    /// A size being typed in physical pixels: width, height, and whether the
    /// height is being typed.
    typing: Option<(String, String, bool)>,
    cursor: Option<(f32, f32)>,
    drag: Option<Drag>,
    /// The selection before a new one was begun, back if it was only a click.
    before: Option<Rect>,
    selection: Option<Rect>,
    /// Space held while drawing moves the selection instead.
    moving: bool,
    last: Option<(f32, f32)>,
    done: Option<oneshot::Sender<Option<(Rect, bool)>>>,
    focus: FocusHandle,
}

fn local(p: Point<Pixels>) -> (f32, f32) {
    (f32::from(p.x), f32::from(p.y))
}

fn inside(r: Rect, (x, y): (f32, f32)) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// The eight handles: which edges each moves and where it sits.
fn handles(r: Rect) -> [(u8, (f32, f32)); 8] {
    let (x0, y0, x1, y1, mx, my) = (r.0, r.1, r.0 + r.2, r.1 + r.3, r.0 + r.2 / 2., r.1 + r.3 / 2.);
    [
        (LEFT | TOP, (x0, y0)),
        (TOP, (mx, y0)),
        (RIGHT | TOP, (x1, y0)),
        (RIGHT, (x1, my)),
        (RIGHT | BOTTOM, (x1, y1)),
        (BOTTOM, (mx, y1)),
        (LEFT | BOTTOM, (x0, y1)),
        (LEFT, (x0, my)),
    ]
}

fn resize_cursor(edges: u8) -> CursorStyle {
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

    fn hovered_window(&self) -> Option<Rect> {
        let p = self.cursor?;
        self.windows.iter().copied().find(|r| inside(*r, p))
    }

    fn ratio(&self) -> Option<f32> {
        ASPECTS[self.aspect].1
    }

    /// The selection can be changed with its handles.
    fn editable(&self) -> bool {
        self.adjust && self.selection.is_some() && !matches!(self.drag, Some(Drag::New(_)))
    }

    fn handle_at(&self, p: (f32, f32)) -> Option<u8> {
        let sel = self.selection?;
        handles(sel).into_iter().find(|(_, (hx, hy))| (p.0 - hx).abs() <= HANDLE_REACH && (p.1 - hy).abs() <= HANDLE_REACH).map(|(e, _)| e)
    }

    /// Kept on the screen, at least a pixel big.
    fn clamp(&self, (x, y, w, h): Rect) -> Rect {
        let (sw, sh) = self.screen;
        let (w, h) = (w.clamp(1., sw), h.clamp(1., sh));
        (x.clamp(0., sw - w), y.clamp(0., sh - h), w, h)
    }

    /// The selection with its height (or width, if that would run off the
    /// screen) fitted to the aspect ratio.
    fn fit_ratio(&self, (x, y, w, mut h): Rect) -> Rect {
        let Some(r) = self.ratio() else { return (x, y, w, h) };
        let mut w = w;
        h = w / r;
        if y + h > self.screen.1 {
            h = self.screen.1 - y;
            w = h * r;
        }
        self.clamp((x, y, w, h))
    }

    fn resized(&self, from: Rect, edges: u8, (dx, dy): (f32, f32)) -> Rect {
        let (sw, sh) = self.screen;
        let (mut x0, mut y0, mut x1, mut y1) = (from.0, from.1, from.0 + from.2, from.1 + from.3);
        if edges & LEFT != 0 {
            x0 = (x0 + dx).clamp(0., sw);
        }
        if edges & RIGHT != 0 {
            x1 = (x1 + dx).clamp(0., sw);
        }
        if edges & TOP != 0 {
            y0 = (y0 + dy).clamp(0., sh);
        }
        if edges & BOTTOM != 0 {
            y1 = (y1 + dy).clamp(0., sh);
        }
        let (mut x0, mut x1, mut y0, mut y1) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
        if let Some(r) = self.ratio() {
            if edges & (LEFT | RIGHT) != 0 {
                let h = (x1 - x0) / r;
                if edges & TOP != 0 { y0 = y1 - h } else { y1 = y0 + h }
            } else {
                let w = (y1 - y0) * r;
                if edges & LEFT != 0 { x0 = x1 - w } else { x1 = x0 + w }
            }
        }
        self.clamp((x0, y0, x1 - x0, y1 - y0))
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window) {
        let p = local(e.position);
        self.last = Some(p);
        if self.adjust
            && let Some(sel) = self.selection
        {
            if e.click_count == 2 && inside(sel, p) {
                return self.finish(Some(sel), false, window);
            }
            if let Some(edges) = self.handle_at(p) {
                self.drag = Some(Drag::Resize { edges, from: sel, start: p });
                return;
            }
            if inside(sel, p) {
                self.drag = Some(Drag::Move(p));
                return;
            }
        }
        self.typing = None;
        self.before = self.selection.take().filter(|_| self.adjust);
        self.drag = Some(Drag::New(p));
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let p = local(e.position);
        if e.pressed_button == Some(MouseButton::Left) {
            match self.drag {
                Some(Drag::New(start)) if self.moving => {
                    // Space: drag the whole selection along.
                    if let (Some(last), Some(sel)) = (self.last, self.selection.as_mut()) {
                        let (dx, dy) = (p.0 - last.0, p.1 - last.1);
                        sel.0 += dx;
                        sel.1 += dy;
                        self.drag = Some(Drag::New((start.0 + dx, start.1 + dy)));
                    }
                }
                Some(Drag::New(start)) => {
                    let (mut dx, mut dy) = (p.0 - start.0, p.1 - start.1);
                    if let Some(r) = self.ratio() {
                        dy = dx.abs() / r * if dy < 0. { -1. } else { 1. };
                    } else if e.modifiers.shift {
                        let m = dx.abs().max(dy.abs());
                        (dx, dy) = (m * dx.signum(), m * dy.signum());
                    }
                    let (x0, y0) = (start.0.min(start.0 + dx), start.1.min(start.1 + dy));
                    self.selection = Some((x0, y0, dx.abs(), dy.abs()));
                }
                Some(Drag::Move(last)) => {
                    if let Some(sel) = self.selection {
                        self.selection = Some(self.clamp((sel.0 + p.0 - last.0, sel.1 + p.1 - last.1, sel.2, sel.3)));
                    }
                    self.drag = Some(Drag::Move(p));
                }
                Some(Drag::Resize { edges, from, start }) => self.selection = Some(self.resized(from, edges, (p.0 - start.0, p.1 - start.1))),
                None => {}
            }
        }
        self.cursor = Some(p);
        self.last = Some(p);
        cx.notify();
    }

    fn mouse_up(&mut self, e: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if e.button != MouseButton::Left {
            return;
        }
        let Some(drag) = self.drag.take() else { return };
        if !matches!(drag, Drag::New(_)) {
            return cx.notify();
        }
        match self.selection {
            Some(sel) if sel.2 >= 4. && sel.3 >= 4. => {
                if self.adjust {
                    self.before = None;
                } else {
                    self.finish(Some(sel), false, window);
                }
            }
            // Only a click beside an adjusted selection: it stays.
            _ if self.before.is_some() => self.selection = self.before.take(),
            // A click: the window under it, else the colour or the whole monitor.
            _ => match self.hovered_window().filter(|_| self.snap) {
                Some(win) => self.finish(Some(win), true, window),
                None if self.color => {
                    if let Some(hex) = self.color_at(local(e.position)) {
                        crate::platform::copy_text(&hex);
                        crate::platform::notify(t("picker.color_copied"), &hex);
                    }
                    self.finish(None, false, window);
                }
                None => {
                    let (sw, sh) = self.screen;
                    self.finish(Some((0., 0., sw, sh)), false, window);
                }
            },
        }
        cx.notify();
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window) {
        let key = e.keystroke.key.as_str();
        let (shift, alt) = (e.keystroke.modifiers.shift, e.keystroke.modifiers.alt);

        // A size being typed: digits, then x (or Tab) and the height.
        if let Some((w, h, on_h)) = self.typing.as_mut() {
            let field = if *on_h { h } else { w };
            match key {
                "escape" => self.typing = None,
                "enter" => self.apply_typed(),
                "x" | "tab" | "space" => *on_h = true,
                "backspace" if field.is_empty() => *on_h = false,
                "backspace" => {
                    field.pop();
                }
                k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) && field.len() < 5 => field.push_str(k),
                _ => {}
            }
            return;
        }

        match key {
            "escape" => self.finish(None, false, window),
            "enter" => {
                let sel = self.selection.or(self.previous);
                if sel.is_some() {
                    self.finish(sel, false, window);
                }
            }
            "space" => self.moving = true,
            "p" => {
                if let Some(prev) = self.previous {
                    self.selection = Some(self.clamp(prev));
                    self.adjust = true;
                }
            }
            "a" => {
                self.aspect = (self.aspect + 1) % ASPECTS.len();
                if let Some(sel) = self.selection {
                    self.selection = Some(self.fit_ratio(sel));
                    self.adjust = true;
                }
            }
            "left" | "right" | "up" | "down" => {
                let Some((x, y, w, h)) = self.selection else { return };
                self.adjust = true;
                let step = if shift { 10. } else { 1. };
                let (dx, dy) = match key {
                    "left" => (-step, 0.),
                    "right" => (step, 0.),
                    "up" => (0., -step),
                    _ => (0., step),
                };
                let next = if alt {
                    // Alt: the right and bottom edges grow and shrink.
                    let (w, h) = ((w + dx).max(1.), (h + dy).max(1.));
                    match self.ratio() {
                        Some(r) if dx != 0. => (x, y, w, w / r),
                        Some(r) => (x, y, h * r, h),
                        None => (x, y, w, h),
                    }
                } else {
                    (x + dx, y + dy, w, h)
                };
                self.selection = Some(self.clamp(next));
            }
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) && self.selection.is_some() => {
                self.adjust = true;
                self.typing = Some((k.to_owned(), String::new(), false));
            }
            _ => {}
        }
    }

    /// The typed size, in physical pixels, from the selection's corner. A
    /// missing height follows the aspect ratio, or stays.
    fn apply_typed(&mut self) {
        let (Some((w, h, _)), Some(sel)) = (self.typing.take(), self.selection) else { return };
        let s = self.scale;
        let w = w.parse::<f32>().ok().filter(|v| *v > 0.).map(|v| v / s).unwrap_or(sel.2);
        let h = match (h.parse::<f32>().ok().filter(|v| *v > 0.), self.ratio()) {
            (Some(v), _) => v / s,
            (None, Some(r)) => w / r,
            (None, None) => sel.3,
        };
        self.selection = Some(self.clamp((sel.0, sel.1, w, h)));
    }

    /// One screen pixel's side in the magnifier, in physical pixels: whole,
    /// so every pixel there is an equal square.
    fn cell_px(&self) -> u32 {
        (CELL * self.scale).round().max(2.) as u32
    }

    /// The magnifier's side in logical pixels: a whole number of cells.
    fn loupe(&self) -> f32 {
        (CELLS * self.cell_px()) as f32 / self.scale
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
        let (px_x, px_y) = ((x * self.scale) as u32, (y * self.scale) as u32);
        let p = self.frozen.get_pixel_checked(px_x, px_y)?.0;
        Some(format!("#{:02X}{:02X}{:02X}", p[0], p[1], p[2]))
    }

    fn cursor_style(&self) -> CursorStyle {
        match (self.drag, self.cursor) {
            (Some(Drag::Resize { edges, .. }), _) => resize_cursor(edges),
            (Some(Drag::Move(_)), _) => CursorStyle::ClosedHand,
            (None, Some(p)) if self.editable() => match self.handle_at(p) {
                Some(edges) => resize_cursor(edges),
                None if self.selection.is_some_and(|s| inside(s, p)) => CursorStyle::OpenHand,
                None => CursorStyle::Crosshair,
            },
            _ => CursorStyle::Crosshair,
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
        let (sw, sh) = self.screen;
        let accent = cx.theme().colors().text_accent;
        let picture = self.picture.clone();
        let selection = self.selection;
        let editable = self.editable();
        let hovered = if selection.is_none() && self.snap && self.drag.is_none() { self.hovered_window() } else { None };
        let lit = selection.or(hovered);
        let ghost = self.previous.filter(|_| selection.is_none() && self.drag.is_none());
        // Guides and the magnifier while aiming, not while adjusting.
        let aiming = !editable || matches!(self.drag, Some(Drag::Resize { .. }));
        let cursor = self.cursor;
        let guides = cursor.filter(|_| self.drag.is_none() && !editable);

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
                // The last area, to bring back with P.
                if let Some((x, y, w, h)) = ghost {
                    window.paint_quad(outline(at(x, y, w, h), gpui::hsla(0., 0., 1., 0.55), gpui::BorderStyle::Dashed));
                }
                if let (Some(sel), true) = (selection, editable) {
                    for (_, (hx, hy)) in handles(sel) {
                        let b = at(hx - HANDLE / 2., hy - HANDLE / 2., HANDLE, HANDLE);
                        window.paint_quad(quad(b, px(HANDLE / 2.), gpui::white(), px(1.), gpui::hsla(0., 0., 0., 0.35), gpui::BorderStyle::Solid));
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
            let size_text = match &self.typing {
                Some((tw, th, on_h)) => format!("{}{} × {}{}", tw, if *on_h { "" } else { "|" }, th, if *on_h { "|" } else { "" }),
                None => format!("{} × {}", (w * self.scale).round(), (h * self.scale).round()),
            };
            let (aspect_key, ratio) = ASPECTS[self.aspect];
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
                                this.aspect = (this.aspect + 1) % ASPECTS.len();
                                if let Some(sel) = this.selection {
                                    this.selection = Some(this.fit_ratio(sel));
                                }
                                cx.notify();
                            })),
                    )
                })
        });

        // The magnifier follows the pointer, flipping away from the edges.
        // Made again only when the pointer reaches another pixel.
        if let Some(at) = cursor.map(|(x, y)| ((x * self.scale).max(0.) as u32, (y * self.scale).max(0.) as u32))
            && self.lens.as_ref().is_none_or(|l| l.0 != at)
        {
            let lens = self.lens_at(at);
            if let Some((_, old)) = self.lens.replace((at, lens)) {
                let _ = window.drop_image(old);
            }
        }
        let (scale, size_l, cell) = (self.scale, self.loupe(), self.cell_px() as f32 / self.scale);
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
                .child(plate().font_family(kit::MONO_FONT).text_xs().child(format!("{}, {}  {}", (x * self.scale).round(), (y * self.scale).round(), self.color_at((x, y)).unwrap_or_default())))
        });

        // What the keys do right now, at the bottom of the screen.
        let mut keys: Vec<(&str, &str)> = Vec::new();
        if self.typing.is_some() {
            keys.extend([("Enter", t("picker.set_size")), ("X", t("picker.height")), ("Esc", t("common.cancel"))]);
        } else if editable {
            keys.extend([("Enter", t("picker.take")), (t("picker.arrows"), t("picker.move")), (t("picker.alt_arrows"), t("picker.resize")), ("A", t("picker.aspect")), ("0–9", t("picker.size")), ("Esc", t("common.cancel"))]);
        } else if self.drag.is_none() {
            if self.previous.is_some() {
                keys.extend([("P", t("picker.previous_area")), ("Enter", t("picker.take_previous"))]);
            }
            keys.extend([("A", ASPECTS[self.aspect].1.map(|_| ASPECTS[self.aspect].0).unwrap_or(t("picker.aspect"))), ("Esc", t("common.cancel"))]);
        }
        let hints = (!keys.is_empty()).then(|| {
            div().absolute().bottom(px(28. * kit::SCALE)).left_0().right_0().flex().justify_center().child(
                plate().py_1().px_3().rounded_lg().flex().gap_3().text_xs().children(
                    keys.into_iter().map(|(key, what)| div().flex().gap_1().child(div().text_color(gpui::hsla(0., 0., 1., 0.6)).child(key)).child(what)),
                ),
            )
        });

        kit::root(cx)
            .relative()
            .cursor(self.cursor_style())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                this.key_down(e, window);
                cx.notify();
            }))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, _| {
                if e.keystroke.key == "space" {
                    this.moving = false;
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, cx| {
                this.mouse_down(e, window);
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, _, window, _| this.finish(None, false, window)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, e: &MouseUpEvent, window, cx| this.mouse_up(e, window, cx)))
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
