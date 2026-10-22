//! An annotated screenshot as data: the shapes drawn on it, the crop, how
//! the picture is turned, and the background it's put on. Geometry is in the
//! (turned) picture's pixels. Nothing here draws; `kadr-raster` does.

use std::path::PathBuf;
use std::sync::Arc;

use image::RgbaImage;

use crate::color::Rgba;

/// A point in image pixels.
pub type P = (f32, f32);

/// Opacity of the highlighter.
pub const MARKER_ALPHA: f32 = 0.4;
/// How dark a spotlight leaves everything outside it.
pub const SPOT_SHADE: f32 = 0.55;
/// Backdrop margin by default, as a fraction of the picture's longer side.
pub const FRAME_PAD: f32 = 0.06;
/// The backdrop's margin and corner radius at most, the same way.
pub const MAX_PAD: f32 = 0.2;
pub const MAX_ROUND: f32 = 0.05;
/// How far a crop can reach past the picture at most, as a share of its
/// longer side, against absurd sizes.
pub const MAX_REACH: f32 = 3.;
/// The backdrop's aspect ratios, by `Frame::aspect`; None fits the picture.
pub const FRAME_ASPECTS: [Option<f32>; 5] = [None, Some(16. / 9.), Some(4. / 3.), Some(1.), Some(9. / 16.)];

/// The cursor stamps' names, like CSS cursors; the first is the fallback.
pub const CURSORS: [&str; 13] = [
    "arrow",
    "pointer",
    "text",
    "grab",
    "grabbing",
    "crosshair",
    "move",
    "not-allowed",
    "wait",
    "ew-resize",
    "ns-resize",
    "nwse-resize",
    "nesw-resize",
];

/// The cursor with this name, or the arrow.
pub fn cursor(name: &str) -> &'static str {
    CURSORS.iter().find(|&&n| n == name).unwrap_or(&CURSORS[0])
}

/// How strongly a hidden area is blurred or pixelated, from its `size`
/// (the slider's position, 0..1): about the old fixed amount near the
/// slider's start, twice it at the end.
pub fn hide_strength(size: f32) -> f32 {
    if (0. ..=1.).contains(&size) { 0.6 + 1.4 * size } else { 0.9 }
}

/// The room around a picture of `size` to draw and crop in, in its pixels.
pub fn work_margin_at(size: (u32, u32), share: f32) -> f32 {
    (size.0.max(size.1) as f32 * share).round()
}

/// Left, top, right, bottom.
pub fn rect(a: P, b: P) -> (f32, f32, f32, f32) {
    (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
}

pub fn segment_distance(p: P, a: P, b: P) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0. { 0. } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0., 1.) };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// A straight segment, or a quadratic curve from `a` to `b` passing through
/// `bend` halfway, as points close enough to draw as a polyline.
pub fn curve(a: P, b: P, bend: Option<P>) -> Vec<P> {
    let Some(m) = bend else { return vec![a, b] };
    // The control point that puts the curve's midpoint on `m`.
    let c = (2. * m.0 - (a.0 + b.0) / 2., 2. * m.1 - (a.1 + b.1) / 2.);
    const STEPS: usize = 48;
    (0..=STEPS)
        .map(|i| {
            let t = i as f32 / STEPS as f32;
            let (u, v, w) = ((1. - t) * (1. - t), 2. * (1. - t) * t, t * t);
            (u * a.0 + v * c.0 + w * b.0, u * a.1 + v * c.1 + w * b.1)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hide {
    Pixelate,
    Blur,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Arrow,
    Rect,
    Ellipse,
    Line,
    Pen(Vec<P>),
    Marker(Vec<P>),
    Text(String),
    Counter(u32),
    /// A macOS-like pointer, by `CURSORS` name, its hotspot at `a` and its
    /// arrow `size` tall.
    Cursor(&'static str),
    /// Hidden by pixelating or blurring.
    Blur(Hide),
    /// Everything outside the a–b box is dimmed.
    Spotlight,
    /// Another picture dropped or pasted on top, filling the a–b box.
    Picture(Arc<RgbaImage>),
}

/// Pictures are the same when they're the same picture, not a copy.
impl PartialEq for Kind {
    fn eq(&self, other: &Kind) -> bool {
        match (self, other) {
            (Kind::Pen(a), Kind::Pen(b)) | (Kind::Marker(a), Kind::Marker(b)) => a == b,
            (Kind::Text(a), Kind::Text(b)) => a == b,
            (Kind::Counter(a), Kind::Counter(b)) => a == b,
            (Kind::Cursor(a), Kind::Cursor(b)) => a == b,
            (Kind::Blur(a), Kind::Blur(b)) => a == b,
            (Kind::Picture(a), Kind::Picture(b)) => Arc::ptr_eq(a, b),
            (Kind::Arrow, Kind::Arrow) | (Kind::Rect, Kind::Rect) | (Kind::Ellipse, Kind::Ellipse) | (Kind::Line, Kind::Line) | (Kind::Spotlight, Kind::Spotlight) => true,
            _ => false,
        }
    }
}

impl Kind {
    /// Drawn as lines and fills.
    pub fn is_vector(&self) -> bool {
        matches!(self, Kind::Arrow | Kind::Rect | Kind::Ellipse | Kind::Line | Kind::Pen(_) | Kind::Marker(_))
    }
}

/// How text stands out from the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    Plain,
    /// On a rounded plate of its colour, lettered in white or black.
    Plate,
    /// Outlined in a contrasting colour.
    Outline,
}

impl TextStyle {
    /// Plate margins and corner radius for a font size.
    pub fn plate(size: f32) -> (f32, f32, f32) {
        (size * 0.4, size * 0.18, size * 0.3)
    }

    /// Outline thickness for a font size.
    pub fn outline(size: f32) -> f32 {
        (size * 0.07).max(1.)
    }
}

/// Geometry is in image pixels. For text `a` is the top-left corner and
/// `size` the font size; for counters `a` is the center and `size` the
/// diameter; otherwise `size` is the stroke width.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub kind: Kind,
    pub a: P,
    pub b: P,
    pub color: Rgba,
    pub size: f32,
    /// Boxes and ellipses: solid instead of an outline.
    pub filled: bool,
    pub text: TextStyle,
    /// Lines and arrows: a point the line bends through, set by dragging
    /// the middle handle. None keeps it straight.
    pub bend: Option<P>,
}

impl Shape {
    pub fn translate(&mut self, d: P) {
        let shift = |p: &mut P| *p = (p.0 + d.0, p.1 + d.1);
        shift(&mut self.a);
        shift(&mut self.b);
        if let Some(bend) = &mut self.bend {
            shift(bend);
        }
        if let Kind::Pen(points) | Kind::Marker(points) = &mut self.kind {
            points.iter_mut().for_each(shift);
        }
    }

    /// Points along a line or arrow: its two ends, or a curve through the bend.
    pub fn path(&self) -> Vec<P> {
        curve(self.a, self.b, self.bend)
    }
}

/// How the shown picture comes from the one taken: mirrored left to right
/// first, then turned clockwise by quarter turns.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Orientation {
    pub turns: u8,
    pub mirrored: bool,
}

impl Orientation {
    /// A quarter turn clockwise more.
    pub fn turned(self) -> Orientation {
        Orientation { turns: (self.turns + 1) % 4, ..self }
    }

    /// Mirrored left to right as it's shown now. On its side, the picture's
    /// own left–right is the shown up–down, which is a mirror and a half turn.
    pub fn mirrored(self) -> Orientation {
        Orientation { turns: if self.turns % 2 == 1 { (self.turns + 2) % 4 } else { self.turns }, mirrored: !self.mirrored }
    }

    pub fn is_upright(self) -> bool {
        self == Orientation::default()
    }

    /// The shown size of a picture taken at `size`.
    pub fn size(self, size: (u32, u32)) -> (u32, u32) {
        if self.turns % 2 == 1 { (size.1, size.0) } else { size }
    }

    pub fn apply(self, picture: &RgbaImage) -> RgbaImage {
        let mut out = if self.mirrored { image::imageops::flip_horizontal(picture) } else { picture.clone() };
        for _ in 0..self.turns {
            out = image::imageops::rotate90(&out);
        }
        out
    }
}

/// Everything undo/redo restores.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub shapes: Vec<Shape>,
    pub crop: Option<(P, P)>,
    /// How the picture is turned and mirrored; the shapes and the crop are
    /// in the turned picture's pixels.
    pub view: Orientation,
    /// The result's size, as a fraction of the cropped picture's.
    pub scale: f32,
    /// What fills a crop past the picture's edges (RGBA); None leaves it
    /// see-through.
    pub fill: Option<[u8; 4]>,
}

impl Default for Doc {
    fn default() -> Self {
        Doc { shapes: Vec::new(), crop: None, view: Orientation::default(), scale: 1., fill: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Backdrop {
    None,
    /// Index into `kadr_raster::backdrops()`.
    Gradient(usize),
    Wallpaper(PathBuf),
    /// Nothing behind the picture: a PNG with transparency around it.
    Transparent,
    /// The screenshot itself, blurred, like CleanShot's.
    Blurred,
}

/// The background the picture is put on, like CleanShot's background tool.
#[derive(Clone, Debug)]
pub struct Frame {
    pub backdrop: Backdrop,
    /// Margin around the picture, as a fraction of its longer side.
    pub padding: f32,
    /// Corner radius, as a fraction of the longer side.
    pub radius: f32,
    /// 0..1.
    pub shadow: f32,
    /// Index into `FRAME_ASPECTS`.
    pub aspect: usize,
}

impl Default for Frame {
    fn default() -> Self {
        Frame { backdrop: Backdrop::None, padding: FRAME_PAD, radius: 0.012, shadow: 0.5, aspect: 0 }
    }
}

impl Frame {
    pub fn to_json(&self) -> serde_json::Value {
        let backdrop = match &self.backdrop {
            Backdrop::None => serde_json::json!("none"),
            Backdrop::Transparent => serde_json::json!("transparent"),
            Backdrop::Blurred => serde_json::json!("blurred"),
            Backdrop::Gradient(i) => serde_json::json!({ "gradient": i }),
            Backdrop::Wallpaper(path) => serde_json::json!({ "wallpaper": path.display().to_string() }),
        };
        serde_json::json!({ "backdrop": backdrop, "padding": self.padding, "radius": self.radius, "shadow": self.shadow, "aspect": self.aspect })
    }

    pub fn from_json(v: &serde_json::Value) -> Frame {
        let d = Frame::default();
        let b = &v["backdrop"];
        let backdrop = if let Some(i) = b["gradient"].as_u64() {
            Backdrop::Gradient(i as usize)
        } else if let Some(path) = b["wallpaper"].as_str().map(PathBuf::from).filter(|p| p.exists()) {
            Backdrop::Wallpaper(path)
        } else if b.as_str() == Some("transparent") {
            Backdrop::Transparent
        } else if b.as_str() == Some("blurred") {
            Backdrop::Blurred
        } else {
            Backdrop::None
        };
        let num = |k: &str, d: f32, max: f32| v[k].as_f64().map_or(d, |x| x as f32).clamp(0., max);
        Frame {
            backdrop,
            padding: num("padding", d.padding, MAX_PAD),
            radius: num("radius", d.radius, MAX_ROUND),
            shadow: num("shadow", d.shadow, 1.),
            aspect: v["aspect"].as_u64().map_or(d.aspect, |a| (a as usize).min(FRAME_ASPECTS.len() - 1)),
        }
    }

    pub fn on(&self) -> bool {
        self.backdrop != Backdrop::None
    }

    /// A background without padding: what's drawn stops at the picture's
    /// edge instead of growing the result past it.
    pub fn clips(&self) -> bool {
        self.on() && self.padding <= 0.0005
    }

    /// The whole output's size and the picture's offset in it, for a picture
    /// of `w`×`h` (any unit).
    pub fn layout(&self, w: f32, h: f32) -> ((f32, f32), (f32, f32)) {
        let pad = (w.max(h) * self.padding).round();
        let (mut ow, mut oh) = (w + 2. * pad, h + 2. * pad);
        if let Some(a) = FRAME_ASPECTS[self.aspect.min(FRAME_ASPECTS.len() - 1)] {
            if ow / oh < a {
                ow = (oh * a).round();
            } else {
                oh = (ow / a).round();
            }
        }
        ((ow, oh), (((ow - w) / 2.).round(), ((oh - h) / 2.).round()))
    }
}
