//! Picking an area, without a window: the selection, how the pointer and
//! the keys change it, and what it ends in. The shells (GPUI's picker now,
//! the overlay's own windows later) feed it events in the monitor's
//! logical pixels and draw what it says.

/// x, y, w, h in the monitor's logical pixels.
pub type Rect = (f32, f32, f32, f32);

pub const LEFT: u8 = 1;
pub const RIGHT: u8 = 2;
pub const TOP: u8 = 4;
pub const BOTTOM: u8 = 8;

/// How near a handle the pointer grabs it.
const HANDLE_REACH: f32 = 10.;

/// The aspect ratios A steps through: a label (an i18n key or the ratio)
/// and width over height.
pub const ASPECTS: [(&str, Option<f32>); 5] = [("picker.free", None), ("1:1", Some(1.)), ("4:3", Some(4. / 3.)), ("16:9", Some(16. / 9.)), ("9:16", Some(9. / 16.))];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    /// Drawing a new selection from this corner.
    New((f32, f32)),
    /// Moving the selection; the pointer's last position.
    Move((f32, f32)),
    /// Dragging a handle: the edges it moves, the selection and the pointer
    /// when it was grabbed.
    Resize { edges: u8, from: Rect, start: (f32, f32) },
}

/// What picking ended in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// An area (and whether it's a window's), or nothing: cancelled.
    Done(Option<(Rect, bool)>),
    /// A click on nothing with colour picking on: the shell copies the
    /// colour there and finishes.
    Color((f32, f32)),
}

/// The pointer's shape over the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Crosshair,
    PointingHand,
    OpenHand,
    ClosedHand,
    /// Over a handle, or dragging it: its edges.
    Resize(u8),
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    /// The monitor's logical size, and its physical pixels per logical one.
    pub screen: (f32, f32),
    pub scale: f32,
    /// Windows to snap to, on this monitor.
    pub windows: Vec<Rect>,
    pub snap: bool,
    /// Space toggled: a click takes the window under it, nothing is drawn.
    pub windows_only: bool,
    /// A click on nothing picks the colour under it.
    pub color: bool,
    /// The selection stays when the mouse is let go.
    pub adjust: bool,
    /// The last area picked on this monitor (P brings it back).
    pub previous: Option<Rect>,
    /// Index into `ASPECTS`.
    pub aspect: usize,
    /// A size being typed in physical pixels: width, height, and whether the
    /// height is being typed.
    pub typing: Option<(String, String, bool)>,
    pub cursor: Option<(f32, f32)>,
    pub drag: Option<Drag>,
    /// The selection before a new one was begun, back if it was only a click.
    pub before: Option<Rect>,
    pub selection: Option<Rect>,
    /// Space held while drawing moves the selection instead.
    pub moving: bool,
    pub last: Option<(f32, f32)>,
    /// Shift was held at the last pointer event.
    pub shift: bool,
    /// What a press picked, taken once the button is up: ending while it's
    /// held hands the rest of the click to the window below.
    pub pending: Option<Option<(Rect, bool)>>,
}

pub fn inside(r: Rect, (x, y): (f32, f32)) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// The eight handles: which edges each moves and where it sits.
pub fn handles(r: Rect) -> [(u8, (f32, f32)); 8] {
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

impl Model {
    pub fn new(screen: (f32, f32), scale: f32) -> Model {
        Model { screen, scale, ..Default::default() }
    }

    pub fn hovered_window(&self) -> Option<Rect> {
        let p = self.cursor?;
        self.windows.iter().copied().find(|r| inside(*r, p))
    }

    pub fn ratio(&self) -> Option<f32> {
        ASPECTS[self.aspect].1
    }

    /// The selection can be changed with its handles.
    pub fn editable(&self) -> bool {
        self.adjust && self.selection.is_some() && !matches!(self.drag, Some(Drag::New(_)))
    }

    pub fn handle_at(&self, p: (f32, f32)) -> Option<u8> {
        let sel = self.selection?;
        handles(sel).into_iter().find(|(_, (hx, hy))| (p.0 - hx).abs() <= HANDLE_REACH && (p.1 - hy).abs() <= HANDLE_REACH).map(|(e, _)| e)
    }

    /// Kept on the screen, at least a pixel big.
    pub fn clamp(&self, (x, y, w, h): Rect) -> Rect {
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

    /// The left button goes down at `p`; `clicks` counts a double click.
    pub fn press(&mut self, p: (f32, f32), clicks: usize) {
        // Still dragging: the button was let go past the monitor's edge,
        // where the release went to another surface. This press ends it,
        // as that release would have.
        if let Some(drag) = self.drag.take() {
            match self.selection {
                Some(sel) if matches!(drag, Drag::New(_)) && sel.2 >= 4. && sel.3 >= 4. => {
                    if self.adjust {
                        self.before = None;
                    } else {
                        self.pending = Some(Some((sel, false)));
                    }
                }
                _ if matches!(drag, Drag::New(_)) && self.before.is_some() => self.selection = self.before.take(),
                _ => {}
            }
            return;
        }
        self.last = Some(p);
        if self.adjust
            && let Some(sel) = self.selection
        {
            if clicks == 2 && inside(sel, p) {
                self.pending = Some(Some((sel, false)));
                return;
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
        if self.windows_only {
            if let Some(win) = self.hovered_window() {
                self.pending = Some(Some((win, true)));
            }
            return;
        }
        self.typing = None;
        self.before = self.selection.take().filter(|_| self.adjust);
        self.drag = Some(Drag::New(p));
    }

    /// The pointer moves to `p`, with Shift held or not.
    pub fn motion(&mut self, p: (f32, f32), shift: bool) {
        self.shift = shift;
        if self.drag.is_some() {
            self.drag_to(p);
        } else {
            self.last = Some(p);
        }
        self.cursor = Some(p);
    }

    /// A drag goes on to `p` (also from outside, when the pointer is held
    /// on the monitor while dragging past its edge).
    pub fn drag_to(&mut self, p: (f32, f32)) {
        // The pointer stops short of the right and bottom edges; within a
        // pixel of them counts as on them.
        let (sw, sh) = self.screen;
        let p = (if p.0 >= sw - 1. { sw } else { p.0.max(0.) }, if p.1 >= sh - 1. { sh } else { p.1.max(0.) });
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
                } else if self.shift {
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
        self.last = Some(p);
    }

    /// The left button comes up at `p`.
    pub fn release(&mut self) -> Option<Outcome> {
        if let Some(picked) = self.pending.take() {
            return Some(Outcome::Done(picked));
        }
        let drag = self.drag.take()?;
        if !matches!(drag, Drag::New(_)) {
            return None;
        }
        match self.selection {
            Some(sel) if sel.2 >= 4. && sel.3 >= 4. => {
                if self.adjust {
                    self.before = None;
                    None
                } else {
                    Some(Outcome::Done(Some((sel, false))))
                }
            }
            // Only a click beside an adjusted selection: it stays.
            _ if self.before.is_some() => {
                self.selection = self.before.take();
                None
            }
            // A click: the window under it, else the colour or the whole monitor.
            _ => Some(match self.hovered_window().filter(|_| self.snap) {
                Some(win) => Outcome::Done(Some((win, true))),
                None if self.color => Outcome::Color(self.last.unwrap_or_default()),
                None => Outcome::Done(Some(((0., 0., self.screen.0, self.screen.1), false))),
            }),
        }
    }

    /// A key, by GPUI's name ("escape", "a", "left"); `held` for a repeat.
    pub fn key(&mut self, key: &str, shift: bool, alt: bool, held: bool) -> Option<Outcome> {
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
            return None;
        }

        match key {
            "escape" => return Some(Outcome::Done(None)),
            "enter" => {
                let sel = self.selection.or(self.previous);
                if sel.is_some() {
                    return Some(Outcome::Done(sel.map(|s| (s, false))));
                }
            }
            // Held while drawing: moves the selection. Pressed otherwise:
            // windows only, or back to drawing an area.
            "space" if matches!(self.drag, Some(Drag::New(_))) => self.moving = true,
            "space" if !held && self.drag.is_none() && !self.editable() => self.windows_only = !self.windows_only,
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
                let (x, y, w, h) = self.selection?;
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
        None
    }

    /// Space let go: drawing goes on from where the selection was moved.
    pub fn key_up(&mut self, key: &str) {
        if key == "space" {
            self.moving = false;
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

    pub fn cursor_shape(&self) -> Cursor {
        match (self.drag, self.cursor) {
            (Some(Drag::Resize { edges, .. }), _) => Cursor::Resize(edges),
            (Some(Drag::Move(_)), _) => Cursor::ClosedHand,
            (None, _) if self.windows_only => Cursor::PointingHand,
            (None, Some(p)) if self.editable() => match self.handle_at(p) {
                Some(edges) => Cursor::Resize(edges),
                None if self.selection.is_some_and(|s| inside(s, p)) => Cursor::OpenHand,
                None => Cursor::Crosshair,
            },
            _ => Cursor::Crosshair,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        Model::new((1000., 800.), 2.)
    }

    #[test]
    fn a_drag_picks_its_area_on_release() {
        let mut m = model();
        m.press((100., 100.), 1);
        m.motion((300., 250.), false);
        assert_eq!(m.release(), Some(Outcome::Done(Some(((100., 100., 200., 150.), false)))));
    }

    #[test]
    fn shift_makes_a_square_and_a_ratio_holds_the_shape() {
        let mut m = model();
        m.press((100., 100.), 1);
        m.motion((300., 150.), true);
        assert_eq!(m.selection, Some((100., 100., 200., 200.)));
        let mut m = model();
        m.key("a", false, false, false);
        m.key("a", false, false, false);
        assert_eq!(m.ratio(), Some(4. / 3.));
        m.press((0., 0.), 1);
        m.motion((400., 10.), false);
        assert_eq!(m.selection, Some((0., 0., 400., 300.)));
    }

    #[test]
    fn a_click_takes_the_window_under_it_else_the_monitor() {
        let mut m = model();
        m.snap = true;
        m.windows = vec![(50., 50., 300., 200.)];
        m.motion((60., 60.), false);
        m.press((60., 60.), 1);
        assert_eq!(m.release(), Some(Outcome::Done(Some(((50., 50., 300., 200.), true)))));
        let mut m = model();
        m.press((900., 700.), 1);
        assert_eq!(m.release(), Some(Outcome::Done(Some(((0., 0., 1000., 800.), false)))));
        let mut m = model();
        m.color = true;
        m.press((10., 20.), 1);
        assert_eq!(m.release(), Some(Outcome::Color((10., 20.))));
    }

    #[test]
    fn the_keyboard_keeps_and_adjusts_the_selection() {
        let mut m = model();
        m.press((100., 100.), 1);
        m.motion((200., 200.), false);
        // Arrows before letting go: the selection stays for adjusting.
        m.key("right", true, false, false);
        assert!(m.adjust);
        assert_eq!(m.release(), None);
        assert_eq!(m.selection, Some((110., 100., 100., 100.)));
        m.key("down", false, true, false);
        assert_eq!(m.selection, Some((110., 100., 100., 101.)));
        // A typed size in physical pixels: 400 x 200 at scale 2.
        for k in ["4", "0", "0", "x", "2", "0", "0", "enter"] {
            m.key(k, false, false, false);
        }
        assert_eq!(m.selection, Some((110., 100., 200., 100.)));
        // Dragged by its corner handle, then taken with Enter.
        m.motion((310., 200.), false);
        assert_eq!(m.cursor_shape(), Cursor::Resize(RIGHT | BOTTOM));
        m.press((310., 200.), 1);
        m.motion((410., 300.), false);
        assert_eq!(m.release(), None);
        assert_eq!(m.selection, Some((110., 100., 300., 200.)));
        assert_eq!(m.key("enter", false, false, false), Some(Outcome::Done(Some(((110., 100., 300., 200.), false)))));
    }

    #[test]
    fn escape_cancels_and_selections_stay_on_screen() {
        let mut m = model();
        assert_eq!(m.key("escape", false, false, false), Some(Outcome::Done(None)));
        let mut m = model();
        m.adjust = true;
        m.selection = Some((900., 700., 200., 200.));
        m.key("right", false, false, false);
        assert_eq!(m.selection, Some((800., 600., 200., 200.)));
    }
}
