//! The geometry every shape is drawn with, shared by the export and the
//! editor's canvas so both draw the same line.

use kadr_core::color::Rgba;
use kadr_core::document::{Kind, MARKER_ALPHA, P, Shape, TextStyle, rect};

use crate::text::{metrics, text_width};

/// A shape's box in image pixels (left, top, right, bottom): what it covers
/// once drawn, text and cursors measured as they're drawn.
pub trait Bounds {
    fn bounds(&self) -> (f32, f32, f32, f32);
}

impl Bounds for Shape {
    fn bounds(&self) -> (f32, f32, f32, f32) {
        match &self.kind {
            Kind::Pen(points) | Kind::Marker(points) => {
                let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for p in points {
                    (l, t, r, b) = (l.min(p.0), t.min(p.1), r.max(p.0), b.max(p.1));
                }
                let w = self.size / 2.;
                (l - w, t - w, r + w, b + w)
            }
            Kind::Text(text) => {
                let m = metrics(self.size);
                let width = text.split('\n').map(|l| text_width(l, self.size)).fold(self.size * 0.5, f32::max);
                let lines = text.split('\n').count().max(1) as f32;
                let (px, py) = match self.text {
                    TextStyle::Plain => (0., 0.),
                    TextStyle::Plate => {
                        let (px, py, _) = TextStyle::plate(self.size);
                        (px, py)
                    }
                    TextStyle::Outline => (TextStyle::outline(self.size), TextStyle::outline(self.size)),
                };
                (self.a.0 - px, self.a.1 - py, self.a.0 + width + px, self.a.1 + lines * m.line + py)
            }
            Kind::Counter(_) => {
                let r = self.size / 2.;
                (self.a.0 - r, self.a.1 - r, self.a.0 + r, self.a.1 + r)
            }
            Kind::Cursor(name) => {
                let ((hx, hy), (w, h)) = crate::cursors::geometry(name, self.size);
                let (l, t) = (self.a.0 - hx, self.a.1 - hy);
                (l, t, l + w, t + h)
            }
            Kind::Line | Kind::Arrow => {
                let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for p in self.path() {
                    (l, t, r, b) = (l.min(p.0), t.min(p.1), r.max(p.0), b.max(p.1));
                }
                let w = self.size / 2.;
                (l - w, t - w, r + w, b + w)
            }
            Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight | Kind::Picture(_) => rect(self.a, self.b),
        }
    }
}

/// White or near-black text, whichever reads better on `c`.
pub fn on_color(c: Rgba) -> Rgba {
    if c.lightness() > 0.65 { Rgba::hsl(0., 0., 0.08, 1.) } else { Rgba::WHITE }
}

/// An arrow along `points`: the shaft, stopping where the head begins, and
/// the head pointing along the line's last stretch.
pub fn arrow_parts(points: &[P], w: f32) -> (Vec<P>, [P; 3]) {
    let (a, b) = (points[0], points[points.len() - 1]);
    let length: f32 = points.windows(2).map(|s| (s[1].0 - s[0].0).hypot(s[1].1 - s[0].1)).sum();
    // The direction the curve comes in at; a straight arrow is its own.
    let from = if points.len() > 2 {
        let reach = (w * 4.).max(w + 10.).min(length * 0.7);
        *points.iter().rev().find(|p| (p.0 - b.0).hypot(p.1 - b.1) >= reach).unwrap_or(&a)
    } else {
        a
    };
    let (dx, dy) = (b.0 - from.0, b.1 - from.1);
    let d = dx.hypot(dy).max(0.001);
    let (end, head) = arrow_geometry((b.0 - dx / d * length, b.1 - dy / d * length), b, w);
    let cut = (end.0 - b.0).hypot(end.1 - b.1);
    let mut shaft: Vec<P> = points.iter().copied().filter(|p| (p.0 - b.0).hypot(p.1 - b.1) > cut).collect();
    if shaft.is_empty() {
        shaft.push(a);
    }
    shaft.push(end);
    (shaft, head)
}

/// Where the shaft ends and the head triangle.
pub fn arrow_geometry(a: P, b: P, w: f32) -> (P, [P; 3]) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy).max(0.001);
    let (ux, uy) = (dx / len, dy / len);
    let head = (w * 4.).max(w + 10.).min(len * 0.7);
    let half = head * 0.55;
    let base = (b.0 - ux * head, b.1 - uy * head);
    let left = (base.0 - uy * half, base.1 + ux * half);
    let right = (base.0 + uy * half, base.1 - ux * half);
    // Run the shaft a little into the head so they join without a seam.
    ((base.0 + ux * w * 0.5, base.1 + uy * w * 0.5), [b, left, right])
}

/// Points around an ellipse inscribed in the a–b rectangle.
pub fn ellipse_points(a: P, b: P) -> Vec<P> {
    let (l, t, r, bt) = rect(a, b);
    let (cx, cy, rx, ry) = ((l + r) / 2., (t + bt) / 2., (r - l) / 2., (bt - t) / 2.);
    (0..=72)
        .map(|i| {
            let th = i as f32 / 72. * std::f32::consts::TAU;
            (cx + rx * th.cos(), cy + ry * th.sin())
        })
        .collect()
}

/// Smooth freehand points into a curve: quadratic segments between the
/// midpoints of consecutive points, with each point as the control.
pub fn smooth(raw: &[P]) -> Vec<P> {
    if raw.len() < 3 {
        return raw.to_vec();
    }
    // First average each point with its neighbours (ends stay put) to take out
    // the hand and sensor jitter, then curve through what's left.
    let r = 2usize;
    let points: Vec<P> = (0..raw.len())
        .map(|i| {
            if i == 0 || i + 1 == raw.len() {
                return raw[i];
            }
            let k = r.min(i).min(raw.len() - 1 - i);
            let window = &raw[i - k..=i + k];
            let n = window.len() as f32;
            (window.iter().map(|p| p.0).sum::<f32>() / n, window.iter().map(|p| p.1).sum::<f32>() / n)
        })
        .collect();
    let points = &points[..];
    let mid = |a: P, b: P| ((a.0 + b.0) / 2., (a.1 + b.1) / 2.);
    let mut out = vec![points[0]];
    for w in points.windows(3) {
        let (start, ctrl, end) = (mid(w[0], w[1]), w[1], mid(w[1], w[2]));
        for i in 1..=6 {
            let t = i as f32 / 6.;
            let u = 1. - t;
            out.push((u * u * start.0 + 2. * u * t * ctrl.0 + t * t * end.0, u * u * start.1 + 2. * u * t * ctrl.1 + t * t * end.1));
        }
    }
    out.push(*points.last().unwrap());
    out
}

pub fn rounded_rect(l: f32, t: f32, r: f32, b: f32, radius: f32) -> Option<tiny_skia::Path> {
    let radius = radius.min((r - l) / 2.).min((b - t) / 2.);
    let mut p = tiny_skia::PathBuilder::new();
    p.move_to(l + radius, t);
    p.line_to(r - radius, t);
    p.quad_to(r, t, r, t + radius);
    p.line_to(r, b - radius);
    p.quad_to(r, b, r - radius, b);
    p.line_to(l + radius, b);
    p.quad_to(l, b, l, b - radius);
    p.line_to(l, t + radius);
    p.quad_to(l, t, l + radius, t);
    p.close();
    p.finish()
}

pub fn paint_for(c: Rgba, alpha: f32) -> tiny_skia::Paint<'static> {
    let mut p = tiny_skia::Paint::default();
    p.set_color_rgba8((c.r * 255.) as u8, (c.g * 255.) as u8, (c.b * 255.) as u8, (alpha * 255.) as u8);
    p.anti_alias = true;
    p
}

pub fn stroke_for(width: f32) -> tiny_skia::Stroke {
    tiny_skia::Stroke { width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..tiny_skia::Stroke::default() }
}

pub fn polyline(points: &[P], closed: bool) -> Option<tiny_skia::Path> {
    let mut b = tiny_skia::PathBuilder::new();
    let (first, rest) = points.split_first()?;
    b.move_to(first.0, first.1);
    for p in rest {
        b.line_to(p.0, p.1);
    }
    if closed {
        b.close();
    }
    b.finish()
}

/// The shapes drawn as lines and fills (`Kind::is_vector`), mapped by `tf`.
/// False for any other kind, which is left alone.
pub fn draw_vector(pm: &mut tiny_skia::Pixmap, shape: &Shape, tf: tiny_skia::Transform) -> bool {
    use tiny_skia::{FillRule, PathBuilder, Rect};
    let (a, b, c, size) = (shape.a, shape.b, shape.color, shape.size);
    let solid = paint_for(c, 1.);
    match &shape.kind {
        Kind::Rect => {
            if let Some(path) = polyline(&[a, (b.0, a.1), b, (a.0, b.1)], true) {
                if shape.filled {
                    pm.fill_path(&path, &solid, FillRule::Winding, tf, None);
                } else {
                    pm.stroke_path(&path, &solid, &stroke_for(size), tf, None);
                }
            }
        }
        Kind::Ellipse => {
            let (l, t, r, bt) = rect(a, b);
            if let Some(path) = Rect::from_ltrb(l, t, r, bt).and_then(PathBuilder::from_oval) {
                if shape.filled {
                    pm.fill_path(&path, &solid, FillRule::Winding, tf, None);
                } else {
                    pm.stroke_path(&path, &solid, &stroke_for(size), tf, None);
                }
            }
        }
        Kind::Line => {
            if let Some(path) = polyline(&shape.path(), false) {
                pm.stroke_path(&path, &solid, &stroke_for(size), tf, None);
            }
        }
        Kind::Pen(points) => {
            if let Some(path) = polyline(&smooth(points), false) {
                pm.stroke_path(&path, &solid, &stroke_for(size), tf, None);
            }
        }
        Kind::Marker(points) => {
            if let Some(path) = polyline(&smooth(points), false) {
                pm.stroke_path(&path, &paint_for(c, MARKER_ALPHA), &stroke_for(size), tf, None);
            }
        }
        Kind::Arrow => {
            let (shaft, head) = arrow_parts(&shape.path(), size);
            if let Some(path) = polyline(&shaft, false) {
                pm.stroke_path(&path, &solid, &stroke_for(size), tf, None);
            }
            if let Some(path) = polyline(&head, true) {
                pm.fill_path(&path, &solid, FillRule::Winding, tf, None);
            }
        }
        _ => return false,
    }
    true
}
