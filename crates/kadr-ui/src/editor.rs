//! The screenshot editor's state: the drawing, its undo history, and what
//! the editor does with a shape beyond drawing it.

use kadr_core::document::{Doc, Kind, P, Shape, segment_distance};
use kadr_raster::Bounds;

use crate::history::History;

/// Stroke widths, font sizes and counter sizes in screen pixels, from the
/// size slider's start to its end.
pub const WIDTHS: (f32, f32) = (2., 16.);
pub const FONT_SIZES: (f32, f32) = (12., 56.);
pub const COUNTER_SIZES: (f32, f32) = (18., 52.);
/// The cursor tool's arrow height, small to large, in screen pixels.
pub const CURSOR_SIZES: (f32, f32) = (18., 72.);
/// A size from a range at the slider's position (0..1).
pub fn sized((lo, hi): (f32, f32), weight: f32) -> f32 {
    lo + (hi - lo) * weight
}
/// The selection band's gap from a shape, in screen pixels.
pub const BAND_GAP: f32 = 5.;

/// What the editor does with a shape beyond drawing it: hit-testing, its
/// handles, and reshaping it by them.
pub trait Edit {
    fn hit(&self, p: P, tol: f32) -> bool;
    fn handles(&self) -> Vec<P>;
    fn band_pad(&self, scale: f32) -> f32;
    fn knobs(&self, scale: f32) -> Vec<P>;
    fn scales(&self) -> bool;
    fn scale_from(&mut self, orig: &Shape, handle: usize, p: P);
    fn set_handle(&mut self, handle: usize, p: P);
    fn set_weight(&mut self, weight: f32, s: f32);
}

impl Edit for Shape {
    fn hit(&self, p: P, tol: f32) -> bool {
        let reach = tol + self.size / 2.;
        match &self.kind {
            Kind::Line | Kind::Arrow(_) => self.path().windows(2).any(|w| segment_distance(p, w[0], w[1]) <= reach),
            Kind::Pen(points) | Kind::Marker(points) => points.windows(2).any(|w| segment_distance(p, w[0], w[1]) <= reach),
            _ => {
                let (l, t, r, b) = self.bounds();
                p.0 >= l - tol && p.0 <= r + tol && p.1 >= t - tol && p.1 <= b + tol
            }
        }
    }

    /// Resize handles: corners then edge middles for boxes, ends for lines.
    fn handles(&self) -> Vec<P> {
        let (a, b) = (self.a, self.b);
        let mid = ((a.0 + b.0) / 2., (a.1 + b.1) / 2.);
        match self.kind {
            Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight => vec![a, (b.0, a.1), b, (a.0, b.1), (mid.0, a.1), (b.0, mid.1), (mid.0, b.1), (a.0, mid.1)],
            // Ends, then the middle that bends it.
            Kind::Line | Kind::Arrow(_) => vec![a, b, self.bend.unwrap_or(mid)],
            // Text, counters and freehand scale from their box's corners.
            _ => {
                let (l, t, r, bt) = self.bounds();
                vec![(l, t), (r, t), (r, bt), (l, bt)]
            }
        }
    }

    /// How far the selection band sits outside a boxy shape, in image
    /// pixels at `scale`: clear of the stroke.
    fn band_pad(&self, scale: f32) -> f32 {
        let stroke = if matches!(self.kind, Kind::Rect | Kind::Ellipse) && !self.filled { self.size } else { 0. };
        stroke + BAND_GAP / scale
    }

    /// Where the handles are drawn and grabbed: on the band round boxy
    /// shapes, on the points themselves for lines.
    fn knobs(&self, scale: f32) -> Vec<P> {
        let hs = self.handles();
        if matches!(self.kind, Kind::Line | Kind::Arrow(_)) {
            return hs;
        }
        let (l, t, r, b) = self.bounds();
        let pad = self.band_pad(scale);
        let out = |v: f32, lo: f32, hi: f32| if v <= lo { v - pad } else if v >= hi { v + pad } else { v };
        hs.into_iter().map(|(x, y)| (out(x, l, r), out(y, t, b))).collect()
    }

    /// Shapes whose corner handles scale them rather than move a point.
    fn scales(&self) -> bool {
        matches!(self.kind, Kind::Text(_) | Kind::Counter(_) | Kind::Cursor(_) | Kind::Pen(_) | Kind::Marker(_) | Kind::Picture(_))
    }

    /// Scale `orig` by dragging corner `handle` to `p`, the opposite corner
    /// staying put: text, counters and pictures keep their proportions (the
    /// size grows with them), freehand stretches each way.
    fn scale_from(&mut self, orig: &Shape, handle: usize, p: P) {
        let corners = orig.handles();
        let (Some(&h), Some(&o)) = (corners.get(handle), corners.get((handle + 2) % 4)) else { return };
        let (dx, dy) = (h.0 - o.0, h.1 - o.1);
        let around = |q: P, fx: f32, fy: f32| (o.0 + (q.0 - o.0) * fx, o.1 + (q.1 - o.1) * fy);
        *self = orig.clone();
        match &mut self.kind {
            Kind::Pen(points) | Kind::Marker(points) => {
                let fx = if dx.abs() > 1. { ((p.0 - o.0) / dx).max(0.05) } else { 1. };
                let fy = if dy.abs() > 1. { ((p.1 - o.1) / dy).max(0.05) } else { 1. };
                points.iter_mut().for_each(|q| *q = around(*q, fx, fy));
                self.a = around(orig.a, fx, fy);
                self.b = around(orig.b, fx, fy);
            }
            _ => {
                // Project the pointer on the corner's diagonal.
                let len2 = (dx * dx + dy * dy).max(1.);
                let f = (((p.0 - o.0) * dx + (p.1 - o.1) * dy) / len2).max(0.2);
                self.size = orig.size * f;
                self.a = around(orig.a, f, f);
                self.b = around(orig.b, f, f);
            }
        }
    }

    fn set_handle(&mut self, handle: usize, p: P) {
        match (&self.kind, handle) {
            (Kind::Line | Kind::Arrow(_), 0) | (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 0) => self.a = p,
            (Kind::Line | Kind::Arrow(_), 2) => {
                // Back to straight when the middle is dragged onto the line.
                let mid = ((self.a.0 + self.b.0) / 2., (self.a.1 + self.b.1) / 2.);
                let len = (self.b.0 - self.a.0).hypot(self.b.1 - self.a.1);
                self.bend = (segment_distance(p, self.a, self.b) > len * 0.02 || (p.0 - mid.0).hypot(p.1 - mid.1) > len * 0.3).then_some(p);
            }
            (Kind::Line | Kind::Arrow(_), 1) | (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 2) => self.b = p,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 1) => (self.b.0, self.a.1) = p,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 3) => (self.a.0, self.b.1) = p,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 4) => self.a.1 = p.1,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 5) => self.b.0 = p.0,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 6) => self.b.1 = p.1,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 7) => self.a.0 = p.0,
            _ => {}
        }
    }

    /// Apply the toolbar's weight to an existing shape.
    fn set_weight(&mut self, weight: f32, s: f32) {
        self.size = match self.kind {
            Kind::Text(_) => sized(FONT_SIZES, weight),
            Kind::Counter(_) => sized(COUNTER_SIZES, weight),
            Kind::Cursor(_) => sized(CURSOR_SIZES, weight),
            Kind::Marker(_) => sized(WIDTHS, weight) * 3.,
            // A hidden area keeps the slider's position as its strength.
            Kind::Blur(..) => {
                self.size = weight;
                return;
            }
            Kind::Spotlight | Kind::Picture(_) => return,
            _ => sized(WIDTHS, weight),
        } / s;
    }
}

/// The editor's drawing and its history.
#[derive(Default)]
pub struct EditorVm {
    pub doc: Doc,
    pub history: History,
}

impl EditorVm {
    pub fn new(doc: Doc) -> EditorVm {
        EditorVm { doc, history: History::default() }
    }

    /// Before a change: what undo will bring back.
    pub fn checkpoint(&mut self) {
        self.history.checkpoint(&self.doc);
    }

    /// The last checkpoint changed nothing after all (a click, not a drag).
    pub fn drop_checkpoint(&mut self) {
        self.history.drop_last();
    }

    pub fn undo(&mut self) -> bool {
        self.history.undo(&mut self.doc)
    }

    pub fn redo(&mut self) -> bool {
        self.history.redo(&mut self.doc)
    }

    /// The topmost shape under `p`, `tol` image pixels round it.
    pub fn hit(&self, p: P, tol: f32) -> Option<usize> {
        self.doc.shapes.iter().rposition(|sh| sh.hit(p, tol))
    }

    /// Every shape in the drawing and its history, for caches to keep.
    pub fn all_shapes(&self) -> impl Iterator<Item = &Shape> {
        self.doc.shapes.iter().chain(self.history.shapes())
    }
}

#[cfg(test)]
mod tests {
    use kadr_core::color::Rgba;
    use kadr_core::document::TextStyle;

    use super::*;

    fn shape(kind: Kind, a: P, b: P) -> Shape {
        Shape { kind, a, b, color: Rgba::WHITE, size: 4., filled: false, text: TextStyle::Plain, bend: None }
    }

    #[test]
    fn the_topmost_shape_is_hit() {
        let mut vm = EditorVm::default();
        vm.doc.shapes.push(shape(Kind::Rect, (0., 0.), (100., 100.)));
        vm.doc.shapes.push(shape(Kind::Rect, (40., 40.), (60., 60.)));
        assert_eq!(vm.hit((50., 50.), 2.), Some(1));
        assert_eq!(vm.hit((10., 10.), 2.), Some(0));
        assert_eq!(vm.hit((300., 300.), 2.), None);
    }

    #[test]
    fn handles_reshape_a_box() {
        let mut sh = shape(Kind::Rect, (0., 0.), (10., 10.));
        sh.set_handle(2, (20., 30.));
        assert_eq!((sh.a, sh.b), ((0., 0.), (20., 30.)));
        sh.set_handle(4, (5., 2.));
        assert_eq!(sh.a, (0., 2.));
    }
}
