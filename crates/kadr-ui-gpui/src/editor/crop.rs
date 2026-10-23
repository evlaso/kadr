//! The crop mode's geometry: aspect ratios, turning and
//! mirroring the picture with everything drawn on it, and snapping the
//! crop's edges to the picture's own edges.

use image::RgbaImage;

use kadr_raster::Bounds as _;

use super::{Doc, Kind, P, Shape, rect};
use crate::i18n::t;

/// A crop box in image pixels: left, top, right, bottom.
pub(super) type Rect = (f32, f32, f32, f32);

/// What the crop keeps to, as picked in the aspect menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Aspect {
    Free,
    /// The picture's own shape.
    Original,
    /// Width to height.
    Ratio(u32, u32),
}

/// The aspect menu, in order; the swap button turns any of them sideways.
pub(super) const ASPECTS: [Aspect; 9] =
    [Aspect::Free, Aspect::Original, Aspect::Ratio(1, 1), Aspect::Ratio(16, 9), Aspect::Ratio(9, 16), Aspect::Ratio(4, 3), Aspect::Ratio(3, 2), Aspect::Ratio(4, 5), Aspect::Ratio(21, 9)];

impl Aspect {
    /// Width over height for a picture of `size`, None when free.
    pub fn ratio(self, size: (u32, u32)) -> Option<f32> {
        match self {
            Aspect::Free => None,
            Aspect::Original => Some(size.0.max(1) as f32 / size.1.max(1) as f32),
            Aspect::Ratio(w, h) => Some(w.max(1) as f32 / h.max(1) as f32),
        }
    }

    /// The same shape on its side.
    pub fn swapped(self, size: (u32, u32)) -> Aspect {
        match self {
            Aspect::Free => Aspect::Free,
            Aspect::Original => {
                let g = gcd(size.0.max(1), size.1.max(1));
                Aspect::Ratio(size.1.max(1) / g, size.0.max(1) / g)
            }
            Aspect::Ratio(w, h) => Aspect::Ratio(h, w),
        }
    }

    pub fn label(self) -> String {
        match self {
            Aspect::Free => t("editor.freeform").into(),
            Aspect::Original => t("editor.original").into(),
            Aspect::Ratio(w, h) => format!("{w}:{h}"),
        }
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

pub(super) use kadr_core::document::Orientation;

/// One step of turning the shown picture.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Turn {
    Clockwise,
    Mirror,
}

impl Turn {
    /// Where a point of a picture `(w, h)` ends up.
    fn point(self, p: P, (w, h): (f32, f32)) -> P {
        match self {
            Turn::Clockwise => (h - p.1, p.0),
            Turn::Mirror => (w - p.0, p.1),
        }
    }

    fn picture(self, picture: &RgbaImage) -> RgbaImage {
        match self {
            Turn::Clockwise => image::imageops::rotate90(picture),
            Turn::Mirror => image::imageops::flip_horizontal(picture),
        }
    }
}

/// Turn the drawing and the crop along with the picture, shown at `size`
/// before the turn. Text and step numbers stay upright, in the same place.
pub(super) fn turn_doc(doc: &mut Doc, turn: Turn, size: (u32, u32)) {
    let s = (size.0 as f32, size.1 as f32);
    for shape in &mut doc.shapes {
        turn_shape(shape, turn, s);
    }
    doc.crop = doc.crop.map(|(a, b)| (turn.point(a, s), turn.point(b, s)));
    doc.view = match turn {
        Turn::Clockwise => doc.view.turned(),
        Turn::Mirror => doc.view.mirrored(),
    };
}

fn turn_shape(shape: &mut Shape, turn: Turn, s: (f32, f32)) {
    let f = |p: P| turn.point(p, s);
    match &mut shape.kind {
        Kind::Text(_) => {
            let (l, t, r, b) = shape.bounds();
            let c = ((l + r) / 2., (t + b) / 2.);
            let to = f(c);
            shape.translate((to.0 - c.0, to.1 - c.1));
            return;
        }
        Kind::Pen(points) | Kind::Marker(points) => points.iter_mut().for_each(|p| *p = f(*p)),
        Kind::Picture(picture) => *picture = std::sync::Arc::new(turn.picture(picture)),
        _ => {}
    }
    shape.a = f(shape.a);
    shape.b = f(shape.b);
    shape.bend = shape.bend.map(f);
}

/// Back to the picture as taken, the drawing turned back with it.
pub(super) fn upright(doc: &mut Doc, mut size: (u32, u32)) {
    for _ in 0..(4 - doc.view.turns) % 4 {
        turn_doc(doc, Turn::Clockwise, size);
        size = (size.1, size.0);
    }
    if doc.view.mirrored {
        turn_doc(doc, Turn::Mirror, size);
    }
}

/// Dragging toward `p` from the corner `fixed` keeps to `ratio`: the
/// longer side of the move wins, the other follows.
pub(super) fn keep_ratio(fixed: P, p: P, ratio: f32) -> P {
    let (dx, dy) = (p.0 - fixed.0, p.1 - fixed.1);
    let (mut w, mut h) = (dx.abs(), dy.abs());
    if w / ratio > h {
        h = w / ratio;
    } else {
        w = h * ratio;
    }
    let sign = |v: f32| if v < 0. { -1. } else { 1. };
    (fixed.0 + w * sign(dx), fixed.1 + h * sign(dy))
}

/// The biggest box of `ratio` inside `r`, centred in it.
pub(super) fn fit_ratio(r: Rect, ratio: f32) -> Rect {
    let (w, h) = (r.2 - r.0, r.3 - r.1);
    let (nw, nh) = if w / h > ratio { (h * ratio, h) } else { (w, w / ratio) };
    let (cx, cy) = ((r.0 + r.2) / 2., (r.1 + r.3) / 2.);
    (cx - nw / 2., cy - nh / 2., cx + nw / 2., cy + nh / 2.)
}

/// The same box with its sides swapped, around its centre.
pub(super) fn swap_sides(r: Rect) -> Rect {
    let (w, h) = (r.2 - r.0, r.3 - r.1);
    let (cx, cy) = ((r.0 + r.2) / 2., (r.1 + r.3) / 2.);
    (cx - h / 2., cy - w / 2., cx + h / 2., cy + w / 2.)
}

/// `v` moved onto the nearest of `lines` within `reach`.
pub(super) fn snap(v: f32, lines: &[f32], reach: f32) -> f32 {
    lines.iter().copied().filter(|l| (l - v).abs() <= reach).min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs())).unwrap_or(v)
}

/// The shift that brings the nearer of `a` and `b` onto a line, if any is
/// within `reach`.
fn snap_shift(a: f32, b: f32, lines: &[f32], reach: f32) -> f32 {
    let (sa, sb) = (snap(a, lines, reach) - a, snap(b, lines, reach) - b);
    match (sa != 0., sb != 0.) {
        (true, true) if sb.abs() < sa.abs() => sb,
        (true, _) => sa,
        (_, true) => sb,
        _ => 0.,
    }
}

/// Lines the crop's edges snap to: across (x) and down (y).
#[derive(Clone, Default)]
pub(super) struct Lines {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
    pub reach: f32,
}

/// The crop `orig` edited by dragging knob `handle` (corners from the top
/// left clockwise, then the middles of the top, right, bottom and left
/// edges) to `p`, or moved whole (None) by `p - from`, kept inside `area`.
pub(super) fn edit(orig: Rect, handle: Option<usize>, p: P, from: P, ratio: Option<f32>, area: Rect, lines: &Lines) -> Rect {
    let (l, t, r, b) = orig;
    let corners = [(l, t), (r, t), (r, b), (l, b)];
    // Snapping fights a kept ratio, so only a free crop snaps its knobs.
    let snapped = |p: P| if ratio.is_some() { p } else { (snap(p.0, &lines.xs, lines.reach), snap(p.1, &lines.ys, lines.reach)) };
    match handle {
        None => {
            let dx = (p.0 - from.0).clamp(area.0 - l, area.2 - r);
            let dy = (p.1 - from.1).clamp(area.1 - t, area.3 - b);
            let (mut l, mut t, mut r, mut b) = (l + dx, t + dy, r + dx, b + dy);
            let sx = snap_shift(l, r, &lines.xs, lines.reach);
            let sy = snap_shift(t, b, &lines.ys, lines.reach);
            (l, r, t, b) = (l + sx, r + sx, t + sy, b + sy);
            (l, t, r, b)
        }
        Some(i @ 0..=3) => {
            let fixed = corners[(i + 2) % 4];
            let q = snapped(p);
            let q = ratio.map_or(q, |k| keep_ratio(fixed, q, k));
            rect(fixed, q)
        }
        Some(i) => {
            let p = snapped(p);
            let (mut l, mut t, mut r, mut b) = orig;
            match i {
                4 => t = p.1,
                5 => r = p.0,
                6 => b = p.1,
                _ => l = p.0,
            }
            if let Some(k) = ratio {
                // The other side follows, around the box's middle.
                if i == 4 || i == 6 {
                    let (w, cx) = ((b - t).abs() * k, (l + r) / 2.);
                    (l, r) = (cx - w / 2., cx + w / 2.);
                } else {
                    let (h, cy) = ((r - l).abs() / k, (t + b) / 2.);
                    (t, b) = (cy - h / 2., cy + h / 2.);
                }
            }
            rect((l, t), (r, b))
        }
    }
}

/// Where the picture has long straight edges (window borders, panels, the
/// lines between sidebars and content): the columns and rows where
/// brightness jumps along much of the picture's height or width.
pub(super) fn strong_edges(picture: &RgbaImage) -> (Vec<f32>, Vec<f32>) {
    let (w, h) = picture.dimensions();
    if w < 3 || h < 3 {
        return (Vec::new(), Vec::new());
    }
    let luma = |x: u32, y: u32| {
        let p = picture.get_pixel(x, y).0;
        (p[0] as i32 * 299 + p[1] as i32 * 587 + p[2] as i32 * 114) / 1000
    };
    // Every other row (or column) is plenty to find a long edge.
    let mut cols = vec![0u32; w as usize];
    let mut rows = vec![0u32; h as usize];
    for y in (0..h).step_by(2) {
        for x in 1..w {
            if (luma(x, y) - luma(x - 1, y)).abs() > 24 {
                cols[x as usize] += 1;
            }
        }
    }
    for x in (0..w).step_by(2) {
        for y in 1..h {
            if (luma(x, y) - luma(x, y - 1)).abs() > 24 {
                rows[y as usize] += 1;
            }
        }
    }
    let pick = |counts: &[u32], samples: u32| {
        let need = (samples as f32 * 0.3).max(4.) as u32;
        let mut found: Vec<(u32, usize)> = (1..counts.len())
            .filter(|&i| counts[i] >= need && counts[i] >= counts[i - 1] && counts.get(i + 1).is_none_or(|&n| counts[i] > n))
            .map(|i| (counts[i], i))
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0));
        let mut lines: Vec<f32> = found.into_iter().take(48).map(|(_, i)| i as f32).collect();
        lines.sort_by(f32::total_cmp);
        lines
    };
    (pick(&cols, h.div_ceil(2)), pick(&rows, w.div_ceil(2)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| image::Rgba([x as u8, y as u8, (x * 7 + y * 13) as u8, 255]))
    }

    #[test]
    fn turns_and_mirrors_add_up_like_the_steps() {
        let pic = numbered(5, 3);
        let mut view = Orientation::default();
        let mut stepwise = pic.clone();
        for step in [Turn::Clockwise, Turn::Mirror, Turn::Clockwise, Turn::Mirror, Turn::Mirror, Turn::Clockwise, Turn::Clockwise] {
            stepwise = step.picture(&stepwise);
            view = match step {
                Turn::Clockwise => view.turned(),
                Turn::Mirror => view.mirrored(),
            };
            assert_eq!(view.apply(&pic), stepwise, "after {step:?}");
            assert_eq!(view.size(pic.dimensions()), stepwise.dimensions());
        }
    }

    #[test]
    fn points_follow_their_pixels() {
        let pic = numbered(5, 3);
        for turn in [Turn::Clockwise, Turn::Mirror] {
            let out = turn.picture(&pic);
            for (x, y) in [(0, 0), (4, 0), (1, 2), (3, 1)] {
                let c = turn.point((x as f32 + 0.5, y as f32 + 0.5), (5., 3.));
                assert_eq!(out.get_pixel(c.0 as u32, c.1 as u32), pic.get_pixel(x, y), "{turn:?} ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_turned_drawing_comes_back_upright() {
        let red = kadr_core::color::Rgba::hsl(0., 1., 0.5, 1.);
        let line = Shape { kind: Kind::Arrow, a: (10., 20.), b: (70., 25.), color: red, size: 3., filled: false, text: super::super::TextStyle::Plain, bend: Some((40., 5.)) };
        let mut doc = Doc { shapes: vec![line], crop: Some(((5., 6.), (50., 40.))), ..Doc::default() };
        let mut size = (100u32, 60u32);
        for step in [Turn::Clockwise, Turn::Mirror, Turn::Clockwise] {
            turn_doc(&mut doc, step, size);
            if step == Turn::Clockwise {
                size = (size.1, size.0);
            }
        }
        assert!(!doc.view.is_upright());
        upright(&mut doc, size);
        assert!(doc.view.is_upright());
        let close = |p: P, q: P| (p.0 - q.0).abs() < 1e-3 && (p.1 - q.1).abs() < 1e-3;
        let s = &doc.shapes[0];
        assert!(close(s.a, (10., 20.)) && close(s.b, (70., 25.)) && close(s.bend.unwrap(), (40., 5.)));
        let (a, b) = doc.crop.unwrap();
        assert!(close(a, (5., 6.)) && close(b, (50., 40.)));
    }

    #[test]
    fn ratios_hold_while_dragging() {
        // The longer move wins.
        assert_eq!(keep_ratio((0., 0.), (160., 10.), 16. / 9.), (160., 90.));
        assert_eq!(keep_ratio((100., 100.), (90., 0.), 1.), (0., 0.));
        let none = Lines::default();
        let area = (-100., -100., 1000., 1000.);
        // A corner keeps the ratio from the opposite one.
        let r = edit((0., 0., 100., 100.), Some(2), (300., 120.), (0., 0.), Some(2.), area, &none);
        assert_eq!(r, (0., 0., 300., 150.));
        // An edge brings the other side along, around the middle.
        let r = edit((0., 0., 100., 50.), Some(6), (0., 100.), (0., 0.), Some(2.), area, &none);
        assert_eq!(r, (-50., 0., 150., 100.));
        assert_eq!(fit_ratio((0., 0., 200., 100.), 1.), (50., 0., 150., 100.));
        assert_eq!(swap_sides((0., 0., 40., 20.)), (10., -10., 30., 30.));
        assert_eq!(Aspect::Original.swapped((1920, 1080)), Aspect::Ratio(9, 16));
        assert_eq!(Aspect::Ratio(4, 5).swapped((1, 1)), Aspect::Ratio(5, 4));
    }

    #[test]
    fn a_free_crop_snaps_to_lines() {
        let lines = Lines { xs: vec![0., 200.], ys: vec![0., 120.], reach: 6. };
        let area = (-50., -50., 300., 200.);
        let r = edit((10., 10., 100., 100.), Some(2), (196., 123.), (0., 0.), None, area, &lines);
        assert_eq!(r, (10., 10., 200., 120.));
        // Moved whole, the nearer edge snaps.
        let r = edit((10., 10., 100., 100.), None, (5., 5.), (12., 0.), None, area, &lines);
        assert_eq!(r, (0., 15., 90., 105.));
    }

    #[test]
    fn finds_a_window_border() {
        let pic = RgbaImage::from_fn(120, 80, |x, y| if (30..90).contains(&x) && (20..60).contains(&y) { image::Rgba([240, 240, 240, 255]) } else { image::Rgba([20, 20, 30, 255]) });
        let (xs, ys) = strong_edges(&pic);
        assert_eq!(xs, vec![30., 90.]);
        assert_eq!(ys, vec![20., 60.]);
    }
}
