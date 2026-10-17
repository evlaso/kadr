//! Annotation editor in Zed's look with CleanShot's tools: draw on a
//! screenshot, select and move what you drew, crop, put it on a backdrop,
//! then copy or save. The view lives on a full-monitor layer (GPUI can't
//! move or resize layers, and a modal overlay suits it anyway). Saving
//! re-draws every shape at full resolution with tiny-skia, so the file
//! matches what was on screen without the screen's scaling.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use gpui::{
    FillOptions, PathStyle,
    App, BorderStyle, Bounds, BoxShadow, Context, PathPromptOptions, CursorStyle, FocusHandle, Hsla, Image, ImageFormat,
    KeyDownEvent, MouseButton, ScrollWheelEvent, relative, MouseDownEvent, MouseMoveEvent, ObjectFit, PathBuilder, Point,
    Window, canvas, div, img, linear_color_stop, linear_gradient, 
    point, px, rgba, size,
};
use image::RgbaImage;
use crate::ui::{ButtonLike, Divider, IconButtonShape, Tooltip, prelude::*};

use crate::kit::{self, Anchor, KeyboardInteractivity, LayerSpec};
use crate::ui::{fill, outline};
use crate::{cards, ocr, palette, studio_editor, util};
use crate::i18n::t;

mod crop;
mod project;
mod redact;
pub(crate) use project::prune as prune_projects;
pub(crate) use project::root as projects_dir;

/// A point in image pixels.
type P = (f32, f32);

/// Stroke widths, font sizes and counter sizes in screen pixels, from the
/// size slider's start to its end.
const WIDTHS: (f32, f32) = (2., 16.);
const FONT_SIZES: (f32, f32) = (12., 56.);
const COUNTER_SIZES: (f32, f32) = (18., 52.);
/// Where the size slider starts.
const WEIGHT: f32 = 0.22;

/// A size from a range at the slider's position (0..1).
fn sized((lo, hi): (f32, f32), weight: f32) -> f32 {
    lo + (hi - lo) * weight
}
/// Opacity of the highlighter.
const MARKER_ALPHA: f32 = 0.4;
/// Backdrop margin, as a fraction of the picture's longer side.
const FRAME_PAD: f32 = 0.06;
/// Room around the picture to draw and crop in, the same way: like
/// CleanShot, the result grows to take in what's drawn outside the picture.
const WORK_PAD: f32 = 0.1;
/// Points this close to the picture's edge (screen pixels) snap onto it.
const EDGE_SNAP: f32 = 6.;
/// Grab distance for shapes and handles, in screen pixels.
const GRAB: f32 = 7.;
/// The selection band's gap from a shape and its width, in screen pixels.
const BAND_GAP: f32 = 5.;
const BAND: f32 = 5.;
/// Toolbar and footer together, roughly: the first layout's guess at the
/// stage height, measured from then on.
const CHROME: f32 = 110. * kit::SCALE;
/// Narrower than this, the toolbar folds its style controls into a menu and
/// the footer drops its zoom.
const COMPACT_TOOLBAR: f32 = 1000.;
/// Wide enough for the "Hide sensitive" label next to its icon.
const WIDE_TOOLBAR: f32 = 1600.;
/// Width of the background panel.
const PANEL: f32 = 264.;
/// The background sidebar's tiles: a swatch, its gap and ring, and the
/// space between; five fit across. Plain colours are dots.
const TILE: f32 = 36.;
const TILE_RADIUS: f32 = 10.;
const TILE_GAP: f32 = 4.;
const DOT: f32 = 22.;
/// Gradients shown before "Show more".
const GRADIENT_ROWS: usize = 10;
/// Slider ranges, as fractions of the picture's longer side.
const MAX_PAD: f32 = 0.2;
const MAX_ROUND: f32 = 0.05;
/// Output shapes for the framed picture.
const ASPECTS: [(&str, Option<f32>); 5] = [("editor.auto", None), ("16:9", Some(16. / 9.)), ("4:3", Some(4. / 3.)), ("1:1", Some(1.)), ("9:16", Some(9. / 16.))];

#[derive(Clone, PartialEq)]
enum Backdrop {
    None,
    /// Index into `palette::backdrops()`.
    Gradient(usize),
    Wallpaper(PathBuf),
    /// Nothing behind the picture: a PNG with transparency around it.
    Transparent,
    /// The screenshot itself, blurred, like CleanShot's.
    Blurred,
}

/// The background the picture is put on, like CleanShot's background tool.
#[derive(Clone)]
struct Frame {
    backdrop: Backdrop,
    /// Margin around the picture, as a fraction of its longer side.
    padding: f32,
    /// Corner radius, as a fraction of the longer side.
    radius: f32,
    /// 0..1.
    shadow: f32,
    /// Index into `ASPECTS`.
    aspect: usize,
}

impl Default for Frame {
    fn default() -> Self {
        Frame { backdrop: Backdrop::None, padding: FRAME_PAD, radius: 0.012, shadow: 0.5, aspect: 0 }
    }
}

impl Frame {
    fn to_json(&self) -> serde_json::Value {
        let backdrop = match &self.backdrop {
            Backdrop::None => serde_json::json!("none"),
            Backdrop::Transparent => serde_json::json!("transparent"),
            Backdrop::Blurred => serde_json::json!("blurred"),
            Backdrop::Gradient(i) => serde_json::json!({ "gradient": i }),
            Backdrop::Wallpaper(path) => serde_json::json!({ "wallpaper": path.display().to_string() }),
        };
        serde_json::json!({ "backdrop": backdrop, "padding": self.padding, "radius": self.radius, "shadow": self.shadow, "aspect": self.aspect })
    }

    fn from_json(v: &serde_json::Value) -> Frame {
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
            aspect: v["aspect"].as_u64().map_or(d.aspect, |a| (a as usize).min(ASPECTS.len() - 1)),
        }
    }

    /// The background last used, for the next screenshot.
    fn last() -> Frame {
        let saved = std::fs::read_to_string(util::config_dir().join("editor-frame.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
        saved.map(|v| Frame::from_json(&v)).unwrap_or_default()
    }

    fn remember(&self) {
        let _ = std::fs::create_dir_all(util::config_dir());
        let _ = std::fs::write(util::config_dir().join("editor-frame.json"), self.to_json().to_string());
    }

    fn on(&self) -> bool {
        self.backdrop != Backdrop::None
    }

    /// The whole output's size and the picture's offset in it, for a picture
    /// of `w`×`h` (any unit).
    fn layout(&self, w: f32, h: f32) -> ((f32, f32), (f32, f32)) {
        let pad = (w.max(h) * self.padding).round();
        let (mut ow, mut oh) = (w + 2. * pad, h + 2. * pad);
        if let Some(a) = ASPECTS[self.aspect.min(ASPECTS.len() - 1)].1 {
            if ow / oh < a {
                ow = (oh * a).round();
            } else {
                oh = (ow / a).round();
            }
        }
        ((ow, oh), (((ow - w) / 2.).round(), ((oh - h) / 2.).round()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Knob {
    Padding,
    Radius,
    Shadow,
    /// The toolbar's size slider.
    Weight,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Select,
    Arrow,
    Rect,
    Ellipse,
    Line,
    Pen,
    Marker,
    Text,
    Counter,
    Blur,
    Spotlight,
    Crop,
}

impl Tool {
    const ALL: [Tool; 12] = [
        Tool::Select,
        Tool::Arrow,
        Tool::Line,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Pen,
        Tool::Marker,
        Tool::Text,
        Tool::Counter,
        Tool::Blur,
        Tool::Spotlight,
        Tool::Crop,
    ];

    /// The toolbar's groups, set apart with space: selecting, shapes,
    /// freehand, words, hiding and highlighting, cropping.
    const GROUPS: [&'static [Tool]; 6] = [
        &[Tool::Select],
        &[Tool::Arrow, Tool::Line, Tool::Rect, Tool::Ellipse],
        &[Tool::Pen, Tool::Marker],
        &[Tool::Text, Tool::Counter],
        &[Tool::Blur, Tool::Spotlight],
        &[Tool::Crop],
    ];

    fn icon(self) -> Icon {
        match self {
            Tool::Select => Icon::from_path("icons/kadr/pointer.svg"),
            Tool::Arrow => Icon::new(IconName::ArrowUpRight),
            Tool::Rect => Icon::from_path("icons/kadr/rect.svg"),
            Tool::Ellipse => Icon::from_path("icons/kadr/ellipse.svg"),
            Tool::Line => Icon::new(IconName::Slash),
            Tool::Pen => Icon::new(IconName::Pencil),
            Tool::Marker => Icon::from_path("icons/kadr/marker.svg"),
            Tool::Text => Icon::new(IconName::CaseSensitive),
            Tool::Counter => Icon::from_path("icons/kadr/counter.svg"),
            Tool::Blur => Icon::from_path("icons/kadr/pixelate.svg"),
            Tool::Spotlight => Icon::from_path("icons/kadr/spotlight.svg"),
            Tool::Crop => Icon::from_path("icons/kadr/crop.svg"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tool::Select => t("editor.select_move"),
            Tool::Arrow => t("common.arrow"),
            Tool::Rect => t("editor.rectangle"),
            Tool::Ellipse => t("editor.ellipse"),
            Tool::Line => t("editor.line"),
            Tool::Pen => t("editor.pencil"),
            Tool::Marker => t("editor.highlighter"),
            Tool::Text => t("common.text"),
            Tool::Counter => t("editor.step_numbers"),
            Tool::Blur => t("editor.hide_tool"),
            Tool::Spotlight => t("editor.spotlight_dim_everything_around"),
            Tool::Crop => t("common.trim"),
        }
    }

    /// Single-key shortcut, CleanShot-style.
    fn key(self) -> &'static str {
        match self {
            Tool::Select => "v",
            Tool::Arrow => "a",
            Tool::Rect => "r",
            Tool::Ellipse => "o",
            Tool::Line => "l",
            Tool::Pen => "p",
            Tool::Marker => "h",
            Tool::Text => "t",
            Tool::Counter => "n",
            Tool::Blur => "b",
            Tool::Spotlight => "s",
            Tool::Crop => "c",
        }
    }
}

#[derive(Clone)]
enum Kind {
    Arrow,
    Rect,
    Ellipse,
    Line,
    Pen(Vec<P>),
    Marker(Vec<P>),
    Text(String),
    Counter(u32),
    /// Hidden by pixelating or blurring; the patch, once computed for the
    /// current position.
    Blur(Hide, Option<Arc<Image>>),
    /// Everything outside the a–b box is dimmed.
    Spotlight,
    /// Another picture dropped or pasted on top, filling the a–b box.
    Picture(Picture),
}

/// How the hide tool covers what's under it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hide {
    Pixelate,
    Blur,
}

/// A picture laid over the screenshot: its pixels for the export, and the
/// same as a GPUI image for the canvas.
#[derive(Clone)]
struct Picture {
    pixels: Arc<RgbaImage>,
    shown: Arc<Image>,
}

impl Picture {
    fn decode(bytes: &[u8]) -> Option<Picture> {
        Picture::from_pixels(image::load_from_memory(bytes).ok()?.to_rgba8())
    }

    fn from_pixels(pixels: RgbaImage) -> Option<Picture> {
        let mut png = Vec::new();
        pixels.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(Picture { pixels: Arc::new(pixels), shown: Arc::new(Image::from_bytes(gpui::ImageFormat::Png, png)) })
    }
}

/// Geometry is in image pixels. For text `a` is the top-left corner and
/// `size` the font size; for counters `a` is the center and `size` the
/// diameter; otherwise `size` is the stroke width.
#[derive(Clone)]
struct Shape {
    kind: Kind,
    a: P,
    b: P,
    color: Hsla,
    size: f32,
    /// Boxes and ellipses: solid instead of an outline.
    filled: bool,
    text: TextStyle,
    /// Lines and arrows: a point the line bends through, set by dragging
    /// the middle handle. None keeps it straight.
    bend: Option<P>,
}

/// How text stands out from the picture.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextStyle {
    Plain,
    /// On a rounded plate of its colour, lettered in white or black.
    Plate,
    /// Outlined in a contrasting colour.
    Outline,
}

impl TextStyle {
    /// Plate margins and corner radius for a font size.
    fn plate(size: f32) -> (f32, f32, f32) {
        (size * 0.4, size * 0.18, size * 0.3)
    }

    /// Outline thickness for a font size.
    fn outline(size: f32) -> f32 {
        (size * 0.07).max(1.)
    }
}

impl Shape {
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

    fn hit(&self, p: P, tol: f32) -> bool {
        let reach = tol + self.size / 2.;
        match &self.kind {
            Kind::Line | Kind::Arrow => self.path().windows(2).any(|w| segment_distance(p, w[0], w[1]) <= reach),
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
            Kind::Line | Kind::Arrow => vec![a, b, self.bend.unwrap_or(mid)],
            // Text, counters and freehand scale from their box's corners.
            _ => {
                let (l, t, r, bt) = self.bounds();
                vec![(l, t), (r, t), (r, bt), (l, bt)]
            }
        }
    }

    /// How far the selection band sits outside a boxy shape, in image
    /// pixels at `scale`: clear of the stroke, like CleanShot's.
    fn band_pad(&self, scale: f32) -> f32 {
        let stroke = if matches!(self.kind, Kind::Rect | Kind::Ellipse) && !self.filled { self.size } else { 0. };
        stroke + BAND_GAP / scale
    }

    /// Where the handles are drawn and grabbed: on the band round boxy
    /// shapes, on the points themselves for lines.
    fn knobs(&self, scale: f32) -> Vec<P> {
        let hs = self.handles();
        if matches!(self.kind, Kind::Line | Kind::Arrow) {
            return hs;
        }
        let (l, t, r, b) = self.bounds();
        let pad = self.band_pad(scale);
        let out = |v: f32, lo: f32, hi: f32| if v <= lo { v - pad } else if v >= hi { v + pad } else { v };
        hs.into_iter().map(|(x, y)| (out(x, l, r), out(y, t, b))).collect()
    }

    /// Shapes whose corner handles scale them rather than move a point.
    fn scales(&self) -> bool {
        matches!(self.kind, Kind::Text(_) | Kind::Counter(_) | Kind::Pen(_) | Kind::Marker(_) | Kind::Picture(_))
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
            (Kind::Line | Kind::Arrow, 0) | (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 0) => self.a = p,
            (Kind::Line | Kind::Arrow, 2) => {
                // Back to straight when the middle is dragged onto the line.
                let mid = ((self.a.0 + self.b.0) / 2., (self.a.1 + self.b.1) / 2.);
                let len = (self.b.0 - self.a.0).hypot(self.b.1 - self.a.1);
                self.bend = (segment_distance(p, self.a, self.b) > len * 0.02 || (p.0 - mid.0).hypot(p.1 - mid.1) > len * 0.3).then_some(p);
            }
            (Kind::Line | Kind::Arrow, 1) | (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 2) => self.b = p,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 1) => (self.b.0, self.a.1) = p,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 3) => (self.a.0, self.b.1) = p,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 4) => self.a.1 = p.1,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 5) => self.b.0 = p.0,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 6) => self.b.1 = p.1,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight, 7) => self.a.0 = p.0,
            _ => {}
        }
        self.stale_patch();
    }

    fn translate(&mut self, d: P) {
        let shift = |p: &mut P| *p = (p.0 + d.0, p.1 + d.1);
        shift(&mut self.a);
        shift(&mut self.b);
        if let Some(bend) = &mut self.bend {
            shift(bend);
        }
        if let Kind::Pen(points) | Kind::Marker(points) = &mut self.kind {
            points.iter_mut().for_each(shift);
        }
        self.stale_patch();
    }

    /// Points along a line or arrow: its two ends, or a curve through the bend.
    fn path(&self) -> Vec<P> {
        curve(self.a, self.b, self.bend)
    }

    /// A moved hidden area shows its outline until the new patch is ready.
    fn stale_patch(&mut self) {
        if let Kind::Blur(_, patch) = &mut self.kind {
            *patch = None;
        }
    }

    /// Apply the toolbar's weight to an existing shape.
    fn set_weight(&mut self, weight: f32, s: f32) {
        self.size = match self.kind {
            Kind::Text(_) => sized(FONT_SIZES, weight),
            Kind::Counter(_) => sized(COUNTER_SIZES, weight),
            Kind::Marker(_) => sized(WIDTHS, weight) * 3.,
            Kind::Blur(..) | Kind::Spotlight | Kind::Picture(_) => return,
            _ => sized(WIDTHS, weight),
        } / s;
    }
}

/// Everything undo/redo restores.
#[derive(Clone)]
struct Doc {
    shapes: Vec<Shape>,
    crop: Option<(P, P)>,
    /// How the picture is turned and mirrored; the shapes and the crop are
    /// in the turned picture's pixels.
    view: crop::Orientation,
    /// The result's size, as a fraction of the cropped picture's.
    scale: f32,
}

impl Default for Doc {
    fn default() -> Self {
        Doc { shapes: Vec::new(), crop: None, view: crop::Orientation::default(), scale: 1. }
    }
}

enum Drag {
    Draw,
    Move { index: usize, last: P, moved: bool },
    Handle { index: usize, handle: usize },
    Scale { index: usize, handle: usize, orig: Shape },
    Crop { start: P },
    /// A crop's knob (corners, then edge middles) dragged, or the crop
    /// moved whole (`handle` None) from `from`.
    CropEdit { handle: Option<usize>, orig: (f32, f32, f32, f32), from: P },
    Slider(Knob),
}

enum Export {
    Copy,
    Save,
    /// Save to a file picked in the system's dialog.
    SaveAs(PathBuf),
}

/// What Cancel goes back to, and the tool to return to after cropping.
struct CropSession {
    entry: Doc,
    history: usize,
    tool: Tool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CropMenu {
    Aspect,
    Scale,
    Zoom,
    /// Save's other ways: as a new file, as a project.
    Save,
}

/// A dropdown choice: its text, whether it's the current one, and what
/// picking it does.
type MenuPick = Rc<dyn Fn(&mut Editor, &mut Window, &mut Context<Editor>)>;
type MenuItem = (String, bool, MenuPick);

/// Image sizes in the crop mode's menu.
const SCALES: [f32; 5] = [1., 0.75, 0.5, 1. / 3., 0.25];
/// Crop edges this close to a line (screen pixels) snap onto it.
const CROP_SNAP: f32 = 8.;

pub struct Editor {
    path: PathBuf,
    image: Arc<Image>,
    px: (u32, u32),
    /// Decoded picture as shown (turned as `shown` says), for pixelation and
    /// the final render.
    source: Option<Arc<RgbaImage>>,
    /// The picture as taken, decoded, and its size; what `image` and `source`
    /// are turned from.
    original: Option<Arc<RgbaImage>>,
    px_taken: (u32, u32),
    /// How `image` and `source` are turned; catches up with `doc.view`.
    shown: crop::Orientation,
    /// The crop mode, while it's on.
    cropping: Option<CropSession>,
    /// What the crop keeps to.
    aspect: crop::Aspect,
    /// Crop edges snap to the picture's edges (Ctrl flips it while held).
    snap: bool,
    /// Long straight edges in the shown picture, found when the crop mode
    /// opens: across, then down.
    edges: Option<Rc<(Vec<f32>, Vec<f32>)>>,
    /// Typing an exact crop size: the height (or the width) so far.
    size_field: Option<(bool, String)>,
    /// The crop mode's open dropdown.
    crop_menu: Option<CropMenu>,
    crop_menu_bounds: crate::ui::MenuBounds,
    /// On-screen size of the picture.
    disp: (f32, f32),
    doc: Doc,
    history: Vec<Doc>,
    future: Vec<Doc>,
    draft: Option<Shape>,
    drag: Option<Drag>,
    selected: Option<usize>,
    /// Index of the text shape being typed.
    editing: Option<usize>,
    tool: Tool,
    colors: Vec<Hsla>,
    color: usize,
    /// The size slider, 0..1.
    weight: f32,
    /// Boxes and ellipses are drawn solid.
    fill: bool,
    /// Pixelate or blur, for the hide tool.
    hide: Hide,
    /// "Hide sensitive" is reading the picture.
    redacting: bool,
    /// The picture as it was before any drawing (the original, or the one
    /// kept with a reopened project), kept with the drawing on export.
    base: Arc<Vec<u8>>,
    /// Opened with the drawing from an earlier save.
    restored: bool,
    /// The `.kadr` this session keeps the drawing in: the one it was opened
    /// from, or the one "Save project" wrote. Saving writes it again.
    bundle: Option<PathBuf>,
    /// Pointer shape over a handle or a shape, while no button is held.
    hover: Option<CursorStyle>,
    text_style: TextStyle,
    /// Canvas zoom on top of fitting the window; 1 fits.
    zoom: f32,
    /// The picture's real size on screen (100%).
    real: (f32, f32),
    frame: Frame,
    /// The background panel is open.
    panel: bool,
    /// The background as last written to disk.
    remembered: serde_json::Value,
    /// Saved backgrounds, by name.
    presets: crate::presets::Presets,
    /// The side panel's scroll, and its scrollbar's grab.
    side_scroll: gpui::ScrollHandle,
    side_grab: std::rc::Rc<std::cell::Cell<Option<f32>>>,
    walls: Vec<(PathBuf, Arc<Image>)>,
    /// The shown picture blurred, for the "Blurred" background's preview.
    blurred: Option<Arc<Image>>,
    /// All the gradients are shown, not just the first rows.
    more_gradients: bool,
    /// The picture's largest on-screen size; `disp` shrinks from it to make
    /// room for the background.
    natural: (f32, f32),
    /// `natural` and `real` for the picture as taken, before any turning.
    natural_taken: (f32, f32),
    real_taken: (f32, f32),
    /// Slider tracks (x, width) in window coordinates.
    sliders: Rc<RefCell<HashMap<Knob, (f32, f32)>>>,
    /// Picture origin in window coordinates, updated every frame.
    origin: Rc<Cell<(f32, f32)>>,
    /// Height the stage got in the last layout; the picture fits into it.
    stage_h: Rc<Cell<f32>>,
    /// An ordinary window, or a panel of `size` over everything (a layer).
    windowed: bool,
    /// The colour dropdown in the toolbar is open.
    colors_open: bool,
    colors_menu: crate::ui::MenuBounds,
    /// The "⋯" menu with the style controls, when the toolbar is too narrow.
    more_open: bool,
    size: (f32, f32),
    focus: FocusHandle,
}

/// Open a screenshot for annotation, or a `.kadr` project made here.
pub fn open(path: PathBuf, cx: &mut App) {
    // A project file: its drawing on its picture; saving writes the result
    // next to it and the project again.
    let (path, bytes, mut restored, bundle) = if crate::bundle::is_bundle(&path) {
        let unpacked = crate::bundle::unpack(&path).ok().filter(|u| u.kind == crate::bundle::Kind::Shot);
        let Some(restored) = unpacked.and_then(|u| project::load_unpacked(&u.dir)) else {
            return util::notify(t("bundle.couldnt_open"), &path.display().to_string());
        };
        (path.with_extension("png"), Vec::new(), Some(restored), Some(path))
    } else {
        let Ok(bytes) = std::fs::read(&path) else {
            return util::notify(t("common.couldnt_open_image"), &path.display().to_string());
        };
        // Saved from the editor before: the drawing comes back, on the
        // picture it was drawn on.
        let restored = project::load(&path, &bytes);
        (path, bytes, restored, None)
    };
    let bytes = restored.as_mut().map(|r| std::mem::take(&mut r.base)).unwrap_or(bytes);
    let base = Arc::new(bytes.clone());
    let Some((w, h)) = cards::png_size(&bytes) else {
        return util::notify(t("editor.editor_opens_png_only"), &path.display().to_string());
    };
    let Some(mon) = util::focused_monitor() else { return };
    let (mw, mh) = (mon.w as f32, mon.h as f32);

    // Real size on screen (screenshots are in physical pixels), shrunk to
    // leave room for the toolbars and a backdrop.
    let natural = (w as f32 / mon.scale as f32, h as f32 / mon.scale as f32);
    let fit = ((mw - 160.) / natural.0).min((mh - 260. * kit::SCALE) / natural.1).min(1.0);
    let disp = (natural.0 * fit, natural.1 * fit);

    let image = Arc::new(Image::from_bytes(ImageFormat::Png, bytes.clone()));
    // A window around the picture with its backdrop margin, the stage
    // padding and the bars; the picture refits whenever the window resizes.
    let pad = disp.0.max(disp.1) * WORK_PAD * 2.;
    // Never a letterbox: at least a comfortable height, whatever the picture.
    let size = ((disp.0 + pad + 48.).max(1180. * kit::SCALE).min(mw - 80.), (disp.1 + pad + 48. + CHROME).max(640. * kit::SCALE).min(mh - 80.));
    let windowed = !cfg!(target_os = "linux") || crate::settings::Settings::load().editor_window;
    let layer = LayerSpec {
        namespace: "kadr-editor",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mw, mh),
        keyboard: KeyboardInteractivity::Exclusive,
        ignore_zones: true,
        monitor: Some(mon.clone()),
    };
    let build = move |window: &mut Window, cx: &mut App| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move { image::load_from_memory(&bytes).ok().map(|i| Arc::new(i.to_rgba8())) })
                    .await;
                let _ = this.update(cx, |editor: &mut Editor, cx| {
                    editor.original = decoded.clone();
                    editor.source = decoded;
                    // A reopened drawing may be on a turned picture.
                    editor.sync_view(cx);
                    editor.refresh_blurred(cx);
                    // A reopened drawing's hidden areas.
                    for i in 0..editor.doc.shapes.len() {
                        editor.refresh_patch(i, cx);
                    }
                    cx.notify();
                });
                let walls = studio_editor::wallpaper_thumbs().await;
                let _ = this.update(cx, |editor: &mut Editor, cx| {
                    editor.walls = walls;
                    cx.notify();
                });
            })
            .detach();
            let (doc, frame) = match restored {
                Some(r) => (r.doc, Some(r.frame)),
                None => (Doc { shapes: Vec::new(), crop: None, ..Doc::default() }, None),
            };
            let reopened = frame.is_some();
            let frame = frame.unwrap_or_else(Frame::last);
            let mut editor = Editor {
                path,
                image,
                px: (w, h),
                source: None,
                original: None,
                px_taken: (w, h),
                shown: crop::Orientation::default(),
                cropping: None,
                aspect: crop::Aspect::Free,
                snap: true,
                edges: None,
                size_field: None,
                crop_menu: None,
                crop_menu_bounds: Default::default(),
                disp,
                doc,
                history: Vec::new(),
                future: Vec::new(),
                draft: None,
                drag: None,
                selected: None,
                editing: None,
                tool: Tool::Arrow,
                colors: palette::annotation_colors(),
                color: 0,
                weight: WEIGHT,
                fill: false,
                hide: Hide::Pixelate,
                redacting: false,
                base,
                restored: reopened,
                bundle,
                hover: None,
                text_style: TextStyle::Plain,
                zoom: 1.,
                real: natural,
                remembered: frame.to_json(),
                frame,
                presets: crate::presets::Presets::new(frame_presets(), save_frame_presets, cx),
                side_scroll: gpui::ScrollHandle::new(),
                side_grab: Default::default(),
                panel: false,
                walls: Vec::new(),
                blurred: None,
                more_gradients: false,
                natural: disp,
                natural_taken: disp,
                real_taken: natural,
                sliders: Rc::new(RefCell::new(HashMap::new())),
                origin: Rc::new(Cell::new((0., 0.))),
                stage_h: Rc::new(Cell::new(size.1 - CHROME)),
                windowed,
                colors_open: false,
                colors_menu: Default::default(),
                more_open: false,
                size,
                focus,
            };
            // The sizes of a reopened turned picture.
            editor.sync_geometry();
            editor
        })
    };
    if windowed {
        kit::open_window(cx, "kadr-editor", Some(&mon), size, (640., 420.), build);
    } else {
        kit::open_layer(cx, layer, build);
    }
}

impl Editor {
    /// Screen pixels per image pixel.
    fn scale(&self) -> f32 {
        self.disp.0 / self.px.0 as f32
    }

    /// The room around the picture, in image pixels.
    fn margin(&self) -> f32 {
        work_margin(self.px)
    }

    /// A point on screen in image pixels: anywhere in the work area around
    /// the picture, snapped onto the picture's edges when close.
    fn to_image(&self, pos: Point<gpui::Pixels>, clamp: bool) -> Option<P> {
        let (ox, oy) = self.origin.get();
        let (s, m) = (self.scale(), self.margin());
        let (x, y) = ((f32::from(pos.x) - ox) / s - m, (f32::from(pos.y) - oy) / s - m);
        let (w, h) = (self.px.0 as f32, self.px.1 as f32);
        let snap = |v: f32, edge: f32| if (v - edge).abs() < EDGE_SNAP / s { edge } else { v };
        let snapped = |x: f32, y: f32| (snap(snap(x, 0.), w), snap(snap(y, 0.), h));
        if clamp {
            Some(snapped(x.clamp(-m, w + m), y.clamp(-m, h + m)))
        } else {
            (x >= -m && y >= -m && x <= w + m && y <= h + m).then(|| snapped(x, y))
        }
    }

    /// The crop's knobs: corners from the top left clockwise, then the
    /// middles of the top, right, bottom and left edges.
    fn crop_handles(&self) -> Vec<P> {
        let Some((l, t, r, b)) = self.doc.crop.map(|(a, b)| rect(a, b)) else { return Vec::new() };
        let (mx, my) = ((l + r) / 2., (t + b) / 2.);
        vec![(l, t), (r, t), (r, b), (l, b), (mx, t), (r, my), (mx, b), (l, my)]
    }

    /// Sizes that follow the picture's turn: its pixels and how big it
    /// shows.
    fn sync_geometry(&mut self) {
        let view = self.doc.view;
        let side = |p: (f32, f32)| if view.turns % 2 == 1 { (p.1, p.0) } else { p };
        self.px = view.size(self.px_taken);
        self.natural = side(self.natural_taken);
        self.real = side(self.real_taken);
    }

    /// Bring the shown picture in line with `doc.view` after a turn, an undo
    /// or a revert, and redo what depends on its pixels.
    fn sync_view(&mut self, cx: &mut Context<Self>) {
        self.sync_geometry();
        let view = self.doc.view;
        if view == self.shown {
            return;
        }
        let Some(original) = self.original.clone() else { return };
        let turned = view.apply(&original);
        let mut png = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::NoFilter);
        if turned.write_with_encoder(encoder).is_err() {
            return;
        }
        self.image = Arc::new(Image::from_bytes(ImageFormat::Png, png));
        self.source = Some(Arc::new(turned));
        self.shown = view;
        self.refresh_blurred(cx);
        self.edges = None;
        if self.cropping.is_some() {
            self.find_edges(cx);
        }
        for i in 0..self.doc.shapes.len() {
            self.refresh_patch(i, cx);
        }
    }

    /// The "Blurred" background's preview, made off the UI thread.
    fn refresh_blurred(&mut self, cx: &mut Context<Self>) {
        let Some(src) = self.source.clone() else { return };
        cx.spawn(async move |this, cx| {
            let png = cx
                .background_executor()
                .spawn(async move {
                    let h = (320. * src.height() as f32 / src.width().max(1) as f32).round().max(1.) as u32;
                    let mut png = Vec::new();
                    blurred_pixels(&src, 320, h).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
                    Some(png)
                })
                .await;
            let _ = this.update(cx, |editor, cx| {
                editor.blurred = png.map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                cx.notify();
            });
        })
        .detach();
    }

    /// Pick a tool; the crop tool is a mode of its own, and leaving it for
    /// another tool keeps the crop.
    fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.finish_text();
        if tool == Tool::Crop {
            return self.enter_crop(cx);
        }
        if self.cropping.is_some() {
            self.apply_crop();
        }
        self.tool = tool;
    }

    fn enter_crop(&mut self, cx: &mut Context<Self>) {
        if self.cropping.is_some() {
            return;
        }
        self.cropping = Some(CropSession { entry: self.doc.clone(), history: self.history.len(), tool: self.tool });
        self.tool = Tool::Crop;
        self.selected = None;
        self.colors_open = false;
        self.more_open = false;
        self.panel = false;
        // Like CleanShot, the frame starts round the whole picture.
        if self.doc.crop.is_none() {
            self.doc.crop = Some(((0., 0.), (self.px.0 as f32, self.px.1 as f32)));
        }
        self.find_edges(cx);
    }

    /// Crop: keep what's set and go back to the tool from before.
    fn apply_crop(&mut self) {
        let Some(session) = self.cropping.take() else { return };
        self.size_field = None;
        self.crop_menu = None;
        // A frame left round the whole picture crops nothing.
        let whole = ((0., 0.), (self.px.0 as f32, self.px.1 as f32));
        if self.doc.crop.is_some_and(|(a, b)| rect(a, b) == (whole.0.0, whole.0.1, whole.1.0, whole.1.1)) {
            self.doc.crop = None;
        }
        self.tool = if session.tool == Tool::Crop { Tool::Select } else { session.tool };
    }

    /// Cancel: back to how it was when the crop mode opened.
    fn cancel_crop(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.cropping.take() else { return };
        self.size_field = None;
        self.crop_menu = None;
        self.doc = session.entry;
        self.history.truncate(session.history);
        self.future.clear();
        self.tool = if session.tool == Tool::Crop { Tool::Select } else { session.tool };
        self.sync_view(cx);
    }

    /// Look for the picture's long edges to snap the crop to.
    fn find_edges(&mut self, cx: &mut Context<Self>) {
        let Some(src) = self.source.clone().filter(|_| self.edges.is_none()) else { return };
        let view = self.shown;
        cx.spawn(async move |this, cx| {
            let edges = cx.background_executor().spawn(async move { crop::strong_edges(&src) }).await;
            let _ = this.update(cx, |editor, _| {
                if editor.shown == view {
                    editor.edges = Some(Rc::new(edges));
                }
            });
        })
        .detach();
    }

    fn crop_ratio(&self) -> Option<f32> {
        self.aspect.ratio(self.px)
    }

    /// The crop, or the whole picture when there's none.
    fn crop_or_picture(&self) -> crop::Rect {
        self.doc.crop.map(|(a, b)| rect(a, b)).unwrap_or((0., 0., self.px.0 as f32, self.px.1 as f32))
    }

    /// What the crop's edges snap to: the picture's edges and its long
    /// straight lines, and the edges of what's drawn. Nothing when snapping
    /// is off (Ctrl flips it).
    fn snap_lines(&self, ctrl: bool) -> crop::Lines {
        if self.snap == ctrl {
            return crop::Lines::default();
        }
        let (w, h) = (self.px.0 as f32, self.px.1 as f32);
        let mut lines = crop::Lines { xs: vec![0., w], ys: vec![0., h], reach: CROP_SNAP / self.scale() };
        if let Some(edges) = &self.edges {
            lines.xs.extend(&edges.0);
            lines.ys.extend(&edges.1);
        }
        for shape in &self.doc.shapes {
            let (l, t, r, b) = shape.bounds();
            lines.xs.extend([l, r]);
            lines.ys.extend([t, b]);
        }
        lines
    }

    /// A new crop dragged out, or an existing one edited, toward `p`.
    fn drag_crop(&mut self, p: P, ctrl: bool) {
        let ratio = self.crop_ratio();
        let lines = self.snap_lines(ctrl);
        let m = self.margin();
        let area = (-m, -m, self.px.0 as f32 + m, self.px.1 as f32 + m);
        let r = match self.drag {
            Some(Drag::Crop { start }) => {
                let p = if ratio.is_none() { (crop::snap(p.0, &lines.xs, lines.reach), crop::snap(p.1, &lines.ys, lines.reach)) } else { p };
                rect(start, ratio.map_or(p, |k| crop::keep_ratio(start, p, k)))
            }
            Some(Drag::CropEdit { handle, orig, from }) => crop::edit(orig, handle, p, from, ratio, area, &lines),
            _ => return,
        };
        self.doc.crop = Some(((r.0, r.1), (r.2, r.3)));
    }

    /// The aspect menu: a fixed shape fits the crop (or the picture) to it.
    fn pick_aspect(&mut self, aspect: crop::Aspect) {
        self.aspect = aspect;
        if let Some(k) = self.crop_ratio() {
            self.checkpoint();
            let r = crop::fit_ratio(self.crop_or_picture(), k);
            self.doc.crop = Some(((r.0, r.1), (r.2, r.3)));
        }
    }

    /// The ↔ button: the crop and its shape on their side.
    fn swap_crop_sides(&mut self) {
        self.aspect = self.aspect.swapped(self.px);
        if let Some((a, b)) = self.doc.crop {
            self.checkpoint();
            let r = crop::swap_sides(rect(a, b));
            self.doc.crop = Some(((r.0, r.1), (r.2, r.3)));
        }
    }

    /// An exact width or height typed in, the other side following a fixed
    /// shape.
    fn apply_size_field(&mut self) {
        let Some((height, text)) = self.size_field.take() else { return };
        let Ok(v) = text.parse::<f32>() else { return };
        let v = v.clamp(1., 100_000.);
        let (l, t, r, b) = self.crop_or_picture();
        let (mut w, mut h) = (r - l, b - t);
        if height {
            h = v;
        } else {
            w = v;
        }
        if let Some(k) = self.crop_ratio() {
            if height {
                w = h * k;
            } else {
                h = w / k;
            }
        }
        self.checkpoint();
        self.doc.crop = Some(((l, t), (l + w.round(), t + h.round())));
    }

    /// Turn the picture a quarter clockwise, or mirror it, with everything
    /// on it.
    fn turn(&mut self, turn: crop::Turn, cx: &mut Context<Self>) {
        self.size_field = None;
        self.checkpoint();
        crop::turn_doc(&mut self.doc, turn, self.px);
        self.sync_view(cx);
    }

    fn pick_scale(&mut self, scale: f32) {
        if (self.doc.scale - scale).abs() > 1e-4 {
            self.checkpoint();
            self.doc.scale = scale;
        }
    }

    /// Revert to Original: no crop, turn, mirror or resize; the drawing stays.
    fn revert_crop(&mut self, cx: &mut Context<Self>) {
        self.size_field = None;
        self.checkpoint();
        crop::upright(&mut self.doc, self.px);
        self.doc.crop = None;
        self.doc.scale = 1.;
        self.aspect = crop::Aspect::Free;
        self.sync_view(cx);
    }

    /// The canvas zoom for a share of the real size, as far as it goes.
    fn zoom_to_real(&mut self, share: f32) {
        let fit = self.disp.0 / self.zoom;
        self.zoom = (share * self.real.0 / fit.max(1.)).clamp(1., 8.);
    }

    fn crop_handle_at(&self, p: P) -> Option<usize> {
        let reach = GRAB * 2. / self.scale();
        self.crop_handles().iter().position(|h| (h.0 - p.0).hypot(h.1 - p.1) <= reach)
    }

    /// What Save produces, in image pixels: the crop, or the picture and
    /// everything drawn around it.
    fn extent(&self) -> (f32, f32, f32, f32) {
        extent(&self.doc, self.px)
    }

    fn current_color(&self) -> Hsla {
        self.colors[self.color]
    }

    /// Remember the current state for undo, before changing it.
    fn checkpoint(&mut self) {
        self.history.push(self.doc.clone());
        self.future.clear();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        self.finish_text();
        if let Some(doc) = self.history.pop() {
            self.future.push(std::mem::replace(&mut self.doc, doc));
            self.selected = None;
            self.sync_view(cx);
        }
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.future.pop() {
            self.history.push(std::mem::replace(&mut self.doc, doc));
            self.selected = None;
            self.sync_view(cx);
        }
    }

    fn finish_text(&mut self) {
        if let Some(i) = self.editing.take()
            && matches!(&self.doc.shapes[i].kind, Kind::Text(t) if t.trim().is_empty())
        {
            self.doc.shapes.remove(i);
            self.selected = None;
        }
    }

    fn delete_selected(&mut self) {
        if let Some(i) = self.selected.take() {
            self.checkpoint();
            self.doc.shapes.remove(i);
        }
    }

    /// Topmost shape under the pointer.
    fn shape_at(&self, p: P) -> Option<usize> {
        let tol = GRAB / self.scale();
        self.doc.shapes.iter().rposition(|s| s.hit(p, tol))
    }

    fn handle_at(&self, p: P) -> Option<(usize, usize)> {
        let i = self.selected?;
        let tol = (GRAB + 4.) / self.scale();
        let h = self.doc.shapes[i].knobs(self.scale()).iter().position(|h| (h.0 - p.0).hypot(h.1 - p.1) <= tol)?;
        Some((i, h))
    }

    /// Compute the patch for a hidden area in the background.
    fn refresh_patch(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(src) = self.source.clone() else { return };
        let Some((how, a, b)) = self.doc.shapes.get(index).and_then(|s| match s.kind {
            Kind::Blur(how, _) => Some((how, s.a, s.b)),
            _ => None,
        }) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let patch = cx.background_executor().spawn(async move { hide_png(&src, a, b, how) }).await;
            let _ = this.update(cx, |editor, cx| {
                // Only if the shape is still where and how the patch was made.
                if let Some(Shape { kind: Kind::Blur(now, slot), a: sa, b: sb, .. }) = editor.doc.shapes.get_mut(index)
                    && (*sa, *sb, *now) == (a, b, how)
                {
                    *slot = patch.map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        // A typed crop size counts once the canvas is clicked.
        self.apply_size_field();
        let Some(p) = self.to_image(e.position, false) else { return };
        let s = self.scale();

        // Double-click a text to type into it again.
        if e.click_count == 2
            && let Some(i) = self.shape_at(p)
            && matches!(self.doc.shapes[i].kind, Kind::Text(_))
        {
            self.finish_text();
            self.checkpoint();
            self.selected = Some(i);
            self.editing = Some(i);
            return cx.notify();
        }
        self.finish_text();

        // The selection's handles and body work with any tool.
        if let Some((index, handle)) = self.handle_at(p) {
            self.checkpoint();
            let shape = &self.doc.shapes[index];
            self.drag = Some(if shape.scales() { Drag::Scale { index, handle, orig: shape.clone() } } else { Drag::Handle { index, handle } });
            return cx.notify();
        }
        if self.tool == Tool::Crop {
            self.checkpoint();
            // An existing crop is resized at its knobs and moved from inside;
            // elsewhere a new one starts.
            if let Some(orig) = self.doc.crop.map(|(a, b)| rect(a, b)) {
                let handle = self.crop_handle_at(p);
                let inside = p.0 > orig.0 && p.0 < orig.2 && p.1 > orig.1 && p.1 < orig.3;
                if handle.is_some() || inside {
                    self.drag = Some(Drag::CropEdit { handle, orig, from: p });
                    return cx.notify();
                }
            }
            self.doc.crop = Some((p, p));
            self.drag = Some(Drag::Crop { start: p });
            return cx.notify();
        }
        let grabbed = match self.tool {
            Tool::Select => self.shape_at(p),
            _ => self.selected.filter(|&i| self.doc.shapes[i].hit(p, GRAB / s)),
        };
        if let Some(index) = grabbed {
            self.checkpoint();
            self.selected = Some(index);
            self.drag = Some(Drag::Move { index, last: p, moved: false });
            return cx.notify();
        }
        self.selected = None;

        let color = self.current_color();
        let kind = match self.tool {
            Tool::Select | Tool::Crop => return cx.notify(),
            Tool::Text => {
                self.checkpoint();
                self.doc.shapes.push(Shape { kind: Kind::Text(String::new()), a: p, b: p, color, size: sized(FONT_SIZES, self.weight) / s, filled: false, text: self.text_style, bend: None });
                self.editing = Some(self.doc.shapes.len() - 1);
                self.selected = self.editing;
                return cx.notify();
            }
            Tool::Counter => {
                let n = self.doc.shapes.iter().filter(|s| matches!(s.kind, Kind::Counter(_))).count() as u32 + 1;
                self.checkpoint();
                self.doc.shapes.push(Shape { kind: Kind::Counter(n), a: p, b: p, color, size: sized(COUNTER_SIZES, self.weight) / s, filled: false, text: TextStyle::Plain, bend: None });
                self.selected = Some(self.doc.shapes.len() - 1);
                return cx.notify();
            }
            Tool::Arrow => Kind::Arrow,
            Tool::Rect => Kind::Rect,
            Tool::Ellipse => Kind::Ellipse,
            Tool::Line => Kind::Line,
            Tool::Pen => Kind::Pen(vec![p]),
            Tool::Marker => Kind::Marker(vec![p]),
            Tool::Blur => Kind::Blur(self.hide, None),
            Tool::Spotlight => Kind::Spotlight,
        };
        let width = if self.tool == Tool::Marker { sized(WIDTHS, self.weight) * 3. } else { sized(WIDTHS, self.weight) };
        let filled = self.fill && matches!(kind, Kind::Rect | Kind::Ellipse);
        self.draft = Some(Shape { kind, a: p, b: p, color, size: width / s, filled, text: TextStyle::Plain, bend: None });
        self.drag = Some(Drag::Draw);
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        if e.pressed_button != Some(MouseButton::Left) {
            // Show what a press would do: resize at a knob, move over a shape.
            let hover = self.to_image(e.position, false).and_then(|p| {
                if self.tool == Tool::Crop
                    && let Some((l, t, r, b)) = self.doc.crop.map(|(a, b)| rect(a, b))
                {
                    return match self.crop_handle_at(p) {
                        Some(0 | 2) => Some(CursorStyle::ResizeUpLeftDownRight),
                        Some(1 | 3) => Some(CursorStyle::ResizeUpRightDownLeft),
                        Some(4 | 6) => Some(CursorStyle::ResizeUpDown),
                        Some(_) => Some(CursorStyle::ResizeLeftRight),
                        None => (p.0 > l && p.0 < r && p.1 > t && p.1 < b).then_some(CursorStyle::OpenHand),
                    };
                }
                if let Some((index, handle)) = self.handle_at(p) {
                    let shape = &self.doc.shapes[index];
                    let corners = shape.handles();
                    let (h, mid) = (corners[handle], shape.bounds());
                    let centre = ((mid.0 + mid.2) / 2., (mid.1 + mid.3) / 2.);
                    return Some(match (shape.kind.clone(), handle) {
                        (Kind::Line | Kind::Arrow, _) => CursorStyle::Crosshair,
                        (_, 4 | 6) => CursorStyle::ResizeUpDown,
                        (_, 5 | 7) => CursorStyle::ResizeLeftRight,
                        _ if (h.0 - centre.0) * (h.1 - centre.1) > 0. => CursorStyle::ResizeUpLeftDownRight,
                        _ => CursorStyle::ResizeUpRightDownLeft,
                    });
                }
                let over = match self.tool {
                    Tool::Select => self.shape_at(p).is_some(),
                    _ => self.selected.is_some_and(|i| self.doc.shapes[i].hit(p, GRAB / self.scale())),
                };
                over.then_some(CursorStyle::OpenHand)
            });
            if hover != self.hover {
                self.hover = hover;
                cx.notify();
            }
            return;
        }
        if let Some(Drag::Slider(knob)) = self.drag {
            return self.slide(knob, f32::from(e.position.x), cx);
        }
        let Some(p) = self.to_image(e.position, true) else { return };
        match &mut self.drag {
            Some(Drag::Move { index, last, moved }) => {
                let d = (p.0 - last.0, p.1 - last.1);
                *last = p;
                *moved = true;
                self.doc.shapes[*index].translate(d);
            }
            Some(Drag::Handle { index, handle }) => {
                let (index, handle) = (*index, *handle);
                self.doc.shapes[index].set_handle(handle, p);
            }
            Some(Drag::Scale { index, handle, orig }) => {
                let (index, handle) = (*index, *handle);
                self.doc.shapes[index].scale_from(orig, handle, p);
            }
            Some(Drag::Crop { .. } | Drag::CropEdit { .. }) => self.drag_crop(p, e.modifiers.control),
            Some(Drag::Slider(_)) => {}
            Some(Drag::Draw) => {
                let min_step = 2.5 / self.scale();
                let Some(d) = self.draft.as_mut() else { return };
                d.b = constrain(d, p, e.modifiers.shift);
                // Sample every few screen pixels: mouse events arrive with
                // sub-pixel jitter that otherwise turns into a ragged edge.
                if let Kind::Pen(points) | Kind::Marker(points) = &mut d.kind
                    && points.last().is_none_or(|l| (l.0 - p.0).hypot(l.1 - p.1) > min_step)
                {
                    points.push(p);
                }
            }
            None => return,
        }
        cx.notify();
    }

    fn mouse_up(&mut self, cx: &mut Context<Self>) {
        match self.drag.take() {
            Some(Drag::Slider(_)) => {}
            // A click without movement isn't an edit.
            Some(Drag::Move { moved: false, .. }) => {
                self.history.pop();
            }
            Some(Drag::Move { index, .. } | Drag::Handle { index, .. } | Drag::Scale { index, .. }) => self.refresh_patch(index, cx),
            Some(Drag::Crop { .. } | Drag::CropEdit { .. }) => {
                if let Some((a, b)) = self.doc.crop
                    && ((a.0 - b.0).abs() < 4. || (a.1 - b.1).abs() < 4.)
                {
                    // A click with the crop tool resets the crop.
                    self.doc.crop = None;
                }
            }
            Some(Drag::Draw) => {
                if let Some(shape) = self.draft.take() {
                    let tiny = (shape.a.0 - shape.b.0).abs() < 3. && (shape.a.1 - shape.b.1).abs() < 3.;
                    let freehand = matches!(shape.kind, Kind::Pen(ref p) | Kind::Marker(ref p) if p.len() > 1);
                    if !tiny || freehand {
                        self.checkpoint();
                        self.doc.shapes.push(shape);
                        let index = self.doc.shapes.len() - 1;
                        self.selected = Some(index);
                        self.refresh_patch(index, cx);
                    }
                }
            }
            None => return,
        }
        cx.notify();
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let (ctrl, shift) = (k.modifiers.control || k.modifiers.platform, k.modifiers.shift);

        if let Some((height, text)) = &mut self.size_field {
            match k.key.as_str() {
                "escape" => self.size_field = None,
                "enter" => self.apply_size_field(),
                // On to the other side, like Tab in a form.
                "tab" | "x" => {
                    let other = !*height;
                    self.apply_size_field();
                    self.size_field = Some((other, String::new()));
                }
                "backspace" => {
                    text.pop();
                }
                key if key.len() == 1 && key.chars().all(|c| c.is_ascii_digit()) && text.len() < 6 => text.push_str(key),
                _ => {}
            }
            return cx.notify();
        }

        if let Some(i) = self.editing {
            match k.key.as_str() {
                "escape" => self.finish_text(),
                "enter" if !shift => self.finish_text(),
                "enter" => push_text(&mut self.doc.shapes[i], "\n"),
                "backspace" => {
                    if let Kind::Text(t) = &mut self.doc.shapes[i].kind {
                        t.pop();
                    }
                }
                _ if !ctrl => {
                    if let Some(ch) = &k.key_char {
                        push_text(&mut self.doc.shapes[i], ch);
                    }
                }
                _ => {}
            }
            return cx.notify();
        }

        match (k.key.as_str(), ctrl, shift) {
            // Esc steps back: the colour menu, the selection, then the editor.
            ("escape", _, _) if self.colors_open => self.colors_open = false,
            ("escape", _, _) if self.selected.is_some() => self.selected = None,
            // Done with the crop: it stays, the crop tool steps aside.
            ("escape", _, _) if self.crop_menu.is_some() => self.crop_menu = None,
            ("escape", _, _) if self.cropping.is_some() => self.cancel_crop(cx),
            ("enter", _, _) if self.cropping.is_some() => self.apply_crop(),
            ("delete" | "backspace", false, _) if self.cropping.is_some() && self.doc.crop.is_some() => {
                self.checkpoint();
                self.doc.crop = None;
            }
            ("escape", _, _) => window.remove_window(),
            ("delete" | "backspace", false, _) => self.delete_selected(),
            ("d", true, _) => self.duplicate(cx),
            ("left" | "right" | "up" | "down", false, _) if self.selected.is_some() => {
                // A screen pixel, or ten with Shift.
                let step = if shift { 10. } else { 1. } / self.scale();
                let d = match k.key.as_str() {
                    "left" => (-step, 0.),
                    "right" => (step, 0.),
                    "up" => (0., -step),
                    _ => (0., step),
                };
                if let Some(index) = self.selected {
                    self.checkpoint();
                    self.doc.shapes[index].translate(d);
                    self.refresh_patch(index, cx);
                }
            }
            ("z", true, false) => self.undo(cx),
            ("z", true, true) | ("y", true, _) => self.redo(cx),
            ("c", true, _) => self.export(Export::Copy, window, cx),
            ("v", true, _) => self.paste(cx),
            ("=" | "+", true, _) => self.zoom_by(1.25),
            ("-", true, _) => self.zoom_by(0.8),
            ("0", true, _) => self.zoom = 1.,
            ("s", true, _) => self.export(Export::Save, window, cx),
            (key, false, false) => {
                if let Some(tool) = Tool::ALL.into_iter().find(|t| t.key() == key) {
                    // C again in the crop mode is Crop.
                    if tool == Tool::Crop && self.cropping.is_some() {
                        self.apply_crop();
                    } else {
                        self.set_tool(tool, cx);
                    }
                } else if let Some(n) = key.parse::<usize>().ok().filter(|n| (1..=self.colors.len()).contains(n)) {
                    self.pick_color(n - 1);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Toolbar colors and weights also restyle the selected shape.
    fn pick_color(&mut self, i: usize) {
        self.color = i;
        if let Some(index) = self.selected {
            self.checkpoint();
            self.doc.shapes[index].color = self.colors[i];
        }
    }

    /// Undo restores the size from before the whole slide, not each step.
    fn pick_weight(&mut self, weight: f32) {
        self.weight = weight;
        if let Some(index) = self.selected {
            let s = self.scale();
            self.doc.shapes[index].set_weight(weight, s);
        }
    }

    fn backdrops(&self) -> Vec<(Hsla, Hsla)> {
        palette::backdrops()
    }

    /// Solid or outline for boxes and ellipses; restyles the selection too.
    fn pick_fill(&mut self, fill: bool) {
        self.fill = fill;
        if let Some(index) = self.selected
            && matches!(self.doc.shapes[index].kind, Kind::Rect | Kind::Ellipse)
        {
            self.checkpoint();
            self.doc.shapes[index].filled = fill;
        }
    }

    /// Copy the selection a little down and right, and select the copy.
    fn duplicate(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected else { return };
        self.checkpoint();
        let mut copy = self.doc.shapes[index].clone();
        if let Kind::Counter(_) = copy.kind {
            let n = self.doc.shapes.iter().filter(|s| matches!(s.kind, Kind::Counter(_))).count() as u32 + 1;
            copy.kind = Kind::Counter(n);
        }
        let step = 16. / self.scale();
        copy.translate((step, step));
        self.doc.shapes.push(copy);
        let new = self.doc.shapes.len() - 1;
        self.selected = Some(new);
        self.refresh_patch(new, cx);
    }

    /// Lay a picture over the screenshot, centred, at most half its size,
    /// and select it.
    fn add_picture(&mut self, picture: Picture) {
        let (pw, ph) = (picture.pixels.width() as f32, picture.pixels.height() as f32);
        let (cw, ch) = (self.px.0 as f32, self.px.1 as f32);
        let fit = (cw * 0.5 / pw).min(ch * 0.5 / ph).min(1.);
        let (w, h) = (pw * fit, ph * fit);
        let (x, y) = ((cw - w) / 2., (ch - h) / 2.);
        self.checkpoint();
        self.doc.shapes.push(Shape {
            kind: Kind::Picture(picture),
            a: (x, y),
            b: (x + w, y + h),
            color: self.colors[self.color],
            size: 0.,
            filled: false,
            text: TextStyle::Plain,
            bend: None,
        });
        self.selected = Some(self.doc.shapes.len() - 1);
        self.apply_crop();
        self.tool = Tool::Select;
    }

    /// Ctrl+V: a copied picture, or a copied image file.
    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else { return };
        let picture = item.entries().iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::Image(image) => Picture::decode(image.bytes()),
            gpui::ClipboardEntry::ExternalPaths(paths) => paths.paths().iter().find_map(|p| Picture::decode(&std::fs::read(p).ok()?)),
            gpui::ClipboardEntry::String(text) => {
                let line = text.text().lines().next()?.trim();
                let path = line.strip_prefix("file://").unwrap_or(line);
                Picture::decode(&std::fs::read(path).ok()?)
            }
        });
        if let Some(picture) = picture {
            self.add_picture(picture);
        }
    }

    /// Files dropped on the editor: each image goes on as a picture.
    fn drop_files(&mut self, paths: &gpui::ExternalPaths) {
        for path in paths.paths() {
            if let Some(picture) = std::fs::read(path).ok().and_then(|bytes| Picture::decode(&bytes)) {
                self.add_picture(picture);
            }
        }
    }

    /// Text look; restyles the selected text too.
    fn pick_text_style(&mut self, style: TextStyle) {
        self.text_style = style;
        if let Some(index) = self.selected
            && matches!(self.doc.shapes[index].kind, Kind::Text(_))
        {
            self.checkpoint();
            self.doc.shapes[index].text = style;
        }
    }

    /// Pixelate or blur; restyles the selected hidden area too.
    fn pick_hide(&mut self, how: Hide, cx: &mut Context<Self>) {
        self.hide = how;
        if let Some(index) = self.selected
            && let Kind::Blur(now, _) = self.doc.shapes[index].kind
            && now != how
        {
            self.checkpoint();
            self.doc.shapes[index].kind = Kind::Blur(how, None);
            self.refresh_patch(index, cx);
        }
    }

    /// Read the picture and hide e-mails, keys, card and phone numbers and
    /// the like, each as an ordinary hidden area (one undo takes them all).
    fn hide_sensitive(&mut self, cx: &mut Context<Self>) {
        let Some(src) = self.source.clone() else { return };
        if self.redacting {
            return;
        }
        self.finish_text();
        self.redacting = true;
        cx.spawn(async move |this, cx| {
            let png = util::runtime_dir().join(format!("kadr-redact-{}.png", std::process::id()));
            let saved = {
                let png = png.clone();
                cx.background_executor().spawn(async move { src.save(&png).is_ok() }).await
            };
            let words = if saved { ocr::words(&png).await } else { None };
            let _ = std::fs::remove_file(&png);
            let _ = this.update(cx, |editor, cx| {
                editor.redacting = false;
                match words {
                    None => util::notify(t("editor.couldnt_read_picture"), t("editor.tesseract_needed")),
                    Some(words) => editor.cover(redact::sensitive(&words), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Hide each box (image pixels) the current way.
    fn cover(&mut self, boxes: Vec<(f32, f32, f32, f32)>, cx: &mut Context<Self>) {
        if boxes.is_empty() {
            return util::notify("kadr", t("editor.nothing_sensitive_found"));
        }
        self.checkpoint();
        let (color, size) = (self.current_color(), sized(WIDTHS, self.weight) / self.scale());
        for (l, t, r, b) in boxes {
            self.doc.shapes.push(Shape { kind: Kind::Blur(self.hide, None), a: (l, t), b: (r, b), color, size, filled: false, text: TextStyle::Plain, bend: None });
            self.refresh_patch(self.doc.shapes.len() - 1, cx);
        }
        self.selected = None;
    }

    /// Back to the picture as it was before any drawing (undoable).
    fn discard_edits(&mut self, cx: &mut Context<Self>) {
        self.finish_text();
        self.checkpoint();
        self.doc = Doc::default();
        self.selected = None;
        self.sync_view(cx);
    }

    /// Zoom the canvas by `factor`, between fitting and 4×.
    fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(1., 8.);
    }

    fn slide(&mut self, knob: Knob, x: f32, cx: &mut Context<Self>) {
        let Some(&(x0, w)) = self.sliders.borrow().get(&knob) else { return };
        let v = ((x - x0) / w.max(1.)).clamp(0., 1.);
        match knob {
            Knob::Padding => self.frame.padding = v * MAX_PAD,
            Knob::Radius => self.frame.radius = v * MAX_ROUND,
            Knob::Shadow => self.frame.shadow = v,
            Knob::Weight => {
                self.pick_weight(v);
                return cx.notify();
            }
        }
        if !self.frame.on() {
            self.frame.backdrop = Backdrop::Gradient(0);
        }
        cx.notify();
    }

    fn pick_wallpaper(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(t("common.choose_background_image").into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |editor, cx| {
                editor.frame.backdrop = Backdrop::Wallpaper(path);
                cx.notify();
            });
        })
        .detach();
    }

    fn export(&mut self, how: Export, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_text();
        let Some(src) = self.source.clone() else {
            return util::notify("kadr", t("editor.image_still_loading_try_again"));
        };
        let doc = self.doc.clone();
        let frame = self.frame.clone();
        let (path, copy) = match how {
            Export::SaveAs(path) => (path, false),
            Export::Save => (self.path.clone(), false),
            Export::Copy => (self.path.clone(), true),
        };
        let base = self.base.clone();
        let bundle = self.bundle.clone();
        window.remove_window();
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let (drawn, framed) = (doc.clone(), frame.clone());
            let png = cx.background_executor().spawn(async move { compose(&src, &drawn, &framed) }).await;
            let Some(png) = png else {
                return util::notify(t("common.couldnt_save"), &path.display().to_string());
            };
            match copy {
                false => {
                    if let Err(e) = std::fs::write(&path, &png) {
                        return util::notify(t("common.couldnt_save"), &e.to_string());
                    }
                    util::copy_image(&path);
                    let _ = cx.update(|cx| cards::show(path.clone(), cx));
                }
                true => {
                    let tmp = util::runtime_dir().join("kadr-annotated.png");
                    if std::fs::write(&tmp, &png).is_ok() {
                        util::copy_image(&tmp);
                    }
                }
            }
            // Keep the drawing editable for the next `kadr edit` of the file.
            cx.background_executor()
                .spawn(async move {
                    let output = if copy { std::fs::read(&path).unwrap_or_default() } else { png };
                    if let Some(bundle) = bundle
                        && let Err(e) = project::save_bundle(&bundle, &output, &base, &doc, &frame)
                    {
                        util::notify(t("bundle.couldnt_save"), &e.to_string());
                    }
                    let _ = project::save(&path, &output, &base, &doc, &frame);
                })
                .await;
        })
        .detach();
    }

    /// Save to a file picked in the system's dialog, next to the screenshot
    /// by default.
    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_text();
        let dir = self.path.parent().map(std::path::Path::to_path_buf).unwrap_or_else(|| crate::settings::Settings::load().screenshot_dir());
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned());
        let picked = cx.prompt_for_new_path(&dir, name.as_deref());
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = picked.await else { return };
            let _ = this.update_in(cx, |editor, window, cx| editor.export(Export::SaveAs(path), window, cx));
        })
        .detach();
    }

    /// Pin the annotated picture above the windows; the editor stays.
    fn pin(&mut self, cx: &mut Context<Self>) {
        self.finish_text();
        let Some(src) = self.source.clone() else {
            return util::notify("kadr", t("editor.image_still_loading_try_again"));
        };
        let (doc, frame) = (self.doc.clone(), self.frame.clone());
        let name = format!("kadr-pin-{}-{}.png", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()));
        let out = util::runtime_dir().join(name);
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let png = cx.background_executor().spawn(async move { compose(&src, &doc, &frame) }).await;
            if let Some(png) = png
                && std::fs::write(&out, png).is_ok()
            {
                let _ = cx.update(|cx| crate::pin::open(out, cx));
            }
        })
        .detach();
    }

    /// A picture from a file, put on top like a dropped one.
    fn pick_picture(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some(t("editor.choose_picture").into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let _ = this.update(cx, |editor, cx| {
                for path in paths {
                    if let Some(picture) = std::fs::read(&path).ok().and_then(|bytes| Picture::decode(&bytes)) {
                        editor.add_picture(picture);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Keep the whole edit as a `.kadr` file, in the screenshots folder (or
    /// the one this session came from); the editor stays open.
    fn save_project(&mut self, cx: &mut Context<Self>) {
        self.finish_text();
        let Some(src) = self.source.clone() else {
            return util::notify("kadr", t("editor.image_still_loading_try_again"));
        };
        let target = self.bundle.clone().unwrap_or_else(|| crate::bundle::target_for(&self.path, &crate::settings::Settings::load().screenshot_dir()));
        self.bundle = Some(target.clone());
        let (doc, frame, base) = (self.doc.clone(), self.frame.clone(), self.base.clone());
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let result = cx
                .background_executor()
                .spawn(async move {
                    let png = compose(&src, &doc, &frame).ok_or_else(|| std::io::Error::other(t("common.couldnt_save")))?;
                    project::save_bundle(&target, &png, &base, &doc, &frame).map(|_| target)
                })
                .await;
            match result {
                Ok(target) => util::notify(t("bundle.project_saved"), &target.display().to_string()),
                Err(e) => util::notify(t("bundle.couldnt_save"), &e.to_string()),
            }
        })
        .detach();
    }

    /// Recognize text in the picture (within the crop, if any).
    fn read_text(&self, cx: &mut Context<Self>) {
        let Some(src) = self.source.clone() else { return };
        let crop = self.doc.crop;
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let tmp = util::runtime_dir().join(format!("kadr-ocr-editor-{}.png", std::process::id()));
            let doc = Doc { shapes: Vec::new(), crop, ..Doc::default() };
            let png = cx.background_executor().spawn(async move { compose(&src, &doc, &Frame::default()) }).await;
            if let Some(png) = png
                && std::fs::write(&tmp, png).is_ok()
            {
                let _ = cx.update(|cx| ocr::image(tmp, cx));
            }
        })
        .detach();
    }

    /// CleanShot-like top bar: history on the left, tools in the middle, the
    /// current style (color, weight) on the right.
    fn render_toolbar(&self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        if self.cropping.is_some() {
            return self.render_crop_bar(width, cx);
        }
        let tool_button = |tool: Tool, cx: &mut Context<Self>| {
            let on = self.tool == tool;
            let icon = tool.icon().size(IconSize::Tool);
            ButtonLike::new(SharedString::from(format!("tool-{}", tool.key())))
                .style(if on { ButtonStyle::Accent } else { ButtonStyle::Subtle })
                .size(ButtonSize::Large)
                .square()
                .child(if on { icon.color(Color::Custom(gpui::white())) } else { icon })
                .tooltip(Tooltip::text(format!("{} ({})", tool.label(), tool.key().to_uppercase())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_tool(tool, cx);
                    cx.notify();
                }))
        };
        // The crop is a mode of its own, beside the background at the left.
        let groups: Vec<_> = Tool::GROUPS
            .iter()
            .filter(|group| !group.contains(&Tool::Crop))
            .map(|group| h_flex().gap_0p5().children(group.iter().map(|&t| tool_button(t, cx))))
            .collect();

        let ring = cx.theme().colors().text;
        let current = self.current_color();
        // Colours sit in a dropdown under one swatch of the current colour.
        let swatches = self.colors.iter().enumerate().map(|(i, &c)| {
            let selected = i == self.color;
            div()
                .id(("color", i))
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_2()
                .border_color(if selected { ring } else { gpui::transparent_black() })
                .cursor_pointer()
                .child(div().size(px(20.)).rounded_full().bg(c))
                .tooltip(Tooltip::text(tr!("editor.color_n", i + 1)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pick_color(i);
                    this.colors_open = false;
                    cx.notify();
                }))
        })
            .collect::<Vec<_>>();
        let palette = div()
            .relative()
            .on_mouse_down_out(cx.listener(|this, e: &MouseDownEvent, _, cx| {
                if this.colors_open && !this.colors_menu.get().contains(&e.position) {
                    this.colors_open = false;
                    cx.notify();
                }
            }))
            .child(
            ButtonLike::new("colors")
                .style(if self.colors_open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                .size(ButtonSize::Large)
                .toggle_state(self.colors_open)
                .child(
                    h_flex()
                        .gap_1()
                        .px_1()
                        .child(div().size(px(16.)).rounded_full().bg(current).border_1().border_color(cx.theme().colors().border))
                        .child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted)),
                )
                .tooltip(Tooltip::text(t("editor.color_1_7")))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.colors_open = !this.colors_open;
                    cx.notify();
                })),
        )
        .when(self.colors_open, |d| {
            d.child(gpui::deferred(
                h_flex()
                    .id("color-menu")
                    .occlude()
                    .absolute()
                    .top(px(38.))
                    .right_0()
                    .gap_1()
                    .p_1p5()
                    .elevation_3(cx)
                    .children(swatches)
                    .child(crate::ui::track_bounds(&self.colors_menu)),
            ))
        });
        // A dot as thick as the stroke, in the current colour, then the slider.
        let dot_size = sized(WIDTHS, self.weight).min(16.) + 1.;
        let weights = h_flex()
            .id("weight")
            .w(px(140.))
            .h(px(32.))
            .gap_1()
            .child(div().w(px(18.)).flex().justify_center().child(div().size(px(dot_size)).rounded_full().bg(current)))
            .child(self.slider(Knob::Weight, self.weight, String::new(), cx))
            .tooltip(Tooltip::text(t("common.size")));
        // The modes, like CleanShot's: crop, a picture on top, the background.
        let mode = |id: &'static str, icon: Icon, on: bool, tip: &'static str| {
            ButtonLike::new(id).style(ButtonStyle::Subtle).size(ButtonSize::Large).square().toggle_state(on).child(icon).tooltip(Tooltip::text(tip))
        };
        let modes = h_flex()
            .gap_0p5()
            .child(mode("mode-crop", Icon::from_path("icons/kadr/crop.svg").size(IconSize::Tool), false, t("editor.crop_tip")).on_click(cx.listener(|this, _, _, cx| {
                this.set_tool(Tool::Crop, cx);
                cx.notify();
            })))
            .child(mode("mode-picture", Icon::from_path("icons/image_plus.svg").size(IconSize::Tool), false, t("editor.add_picture")).on_click(cx.listener(|this, _, _, cx| this.pick_picture(cx))))
            .child(
                mode(
                    "mode-background",
                    Icon::from_path("icons/kadr/frame.svg").size(IconSize::Tool).color(if self.frame.on() { Color::Accent } else { Color::Default }),
                    self.panel,
                    t("editor.background_padding_corners_shadow_format"),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.panel = !this.panel;
                    cx.notify();
                })),
            );

        let history = h_flex()
            .gap_0p5()
            .child(
                IconButton::new("undo", IconName::Undo)
                    .shape(IconButtonShape::Square)
                    .size(ButtonSize::Large)
                    .disabled(self.history.is_empty())
                    .tooltip(Tooltip::text(t("editor.undo_ctrl_z")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.undo(cx);
                        cx.notify();
                    })),
            )
            .child(
                ButtonLike::new("redo")
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Large)
                    .disabled(self.future.is_empty())
                    .child(div().px_1().child(Icon::from_path("icons/kadr/redo.svg").size(IconSize::Small).color(if self.future.is_empty() { Color::Disabled } else { Color::Default })))
                    .tooltip(Tooltip::text(t("editor.redo_ctrl_shift_z")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.redo(cx);
                        cx.notify();
                    })),
            );
        // One pill, its groups apart with a little space.
        let tools = h_flex().p(px(2.)).gap(px(6.)).rounded(px(11.)).rounded_smoothing(crate::ui::SMOOTHING).bg(cx.theme().colors().element_background).children(groups);
        // Solid or outline, for boxes and ellipses.
        let shapes_fill = matches!(self.tool, Tool::Rect | Tool::Ellipse)
            || self.selected.and_then(|i| self.doc.shapes.get(i)).is_some_and(|sh| matches!(sh.kind, Kind::Rect | Kind::Ellipse));
        let fill_toggle = shapes_fill.then(|| {
            let option = |id: &'static str, solid: bool, icon: &'static str, tip: &'static str| {
                ButtonLike::new(id)
                    .style(if self.fill == solid { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(self.fill == solid)
                    .child(div().px_1().child(Icon::from_path(icon).size(IconSize::Small)))
                    .tooltip(Tooltip::text(tip))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_fill(solid);
                        cx.notify();
                    }))
            };
            h_flex()
                .gap_0p5()
                .child(option("outline", false, "icons/kadr/rect.svg", t("editor.outline")))
                .child(option("solid", true, "icons/kadr/fill.svg", t("editor.fill")))
                .child(Divider::vertical().h_5().mx_1())
        });
        // Plain, on a plate or outlined, for text.
        let texts = matches!(self.tool, Tool::Text) || self.selected.and_then(|i| self.doc.shapes.get(i)).is_some_and(|sh| matches!(sh.kind, Kind::Text(_)));
        let text_options = texts.then(|| {
            let fg = cx.theme().colors().text;
            let option = |id: &'static str, style: TextStyle, tip: &'static str| {
                let sample = div().px_1().text_size(px(14.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Aa");
                let sample = match style {
                    TextStyle::Plain => sample,
                    TextStyle::Plate => sample.rounded(px(4.)).bg(fg).text_color(cx.theme().colors().editor_background),
                    TextStyle::Outline => sample.rounded(px(4.)).border_1().border_color(fg),
                };
                ButtonLike::new(id)
                    .style(if self.text_style == style { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(self.text_style == style)
                    .child(div().px_0p5().child(sample))
                    .tooltip(Tooltip::text(tip))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_text_style(style);
                        cx.notify();
                    }))
            };
            h_flex()
                .gap_0p5()
                .child(option("text-plain", TextStyle::Plain, t("editor.plain_text")))
                .child(option("text-plate", TextStyle::Plate, t("editor.on_plate")))
                .child(option("text-outline", TextStyle::Outline, t("editor.outlined")))
                .child(Divider::vertical().h_5().mx_1())
        });
        // Pixelate or blur, and hiding what looks private, for the hide tool.
        let hiding = self.tool == Tool::Blur || self.selected.and_then(|i| self.doc.shapes.get(i)).is_some_and(|sh| matches!(sh.kind, Kind::Blur(..)));
        let hide_options = hiding.then(|| {
            let option = |id: &'static str, how: Hide, icon: &'static str, tip: &'static str| {
                ButtonLike::new(id)
                    .style(if self.hide == how { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(self.hide == how)
                    .child(div().px_1().child(Icon::from_path(icon).size(IconSize::Small)))
                    .tooltip(Tooltip::text(tip))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_hide(how, cx);
                        cx.notify();
                    }))
            };
            h_flex()
                .gap_0p5()
                .child(option("hide-pixelate", Hide::Pixelate, "icons/kadr/pixelate.svg", t("editor.pixelate")))
                .child(option("hide-blur", Hide::Blur, "icons/kadr/blur.svg", t("editor.blur")))
                .child({
                    // Short of room the label goes, the tooltip still says it.
                    let tip = Tooltip::text(t("editor.hide_sensitive_tip"));
                    let button = if width >= WIDE_TOOLBAR {
                        Button::new("hide-sensitive", if self.redacting { t("editor.looking") } else { t("editor.hide_sensitive") })
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Large)
                            .start_icon(Icon::from_path("icons/kadr/scan_text.svg").size(IconSize::Small))
                            .disabled(self.redacting || self.source.is_none())
                            .tooltip(tip)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.hide_sensitive(cx);
                                cx.notify();
                            }))
                            .into_any_element()
                    } else {
                        ButtonLike::new("hide-sensitive")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Large)
                            .disabled(self.redacting || self.source.is_none())
                            .child(div().px_1().child(Icon::from_path("icons/kadr/scan_text.svg").size(IconSize::Small)))
                            .tooltip(tip)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.hide_sensitive(cx);
                                cx.notify();
                            }))
                            .into_any_element()
                    };
                    button
                })
                .child(Divider::vertical().h_5().mx_1())
        });
        // A narrow window folds the style controls into a "⋯" menu, like
        // toolbars in the OS do.
        let compact = width < COMPACT_TOOLBAR;
        let style = if compact {
            let menu = self.more_open.then(|| {
                gpui::deferred(
                    v_flex()
                        .id("more-menu")
                        .occlude()
                        .absolute()
                        .top(px(38.))
                        .right_0()
                        .gap_2()
                        .p_2()
                        .elevation_3(cx)
                        .children(hide_options)
                        .children(text_options)
                        .children(fill_toggle)
                        .child(palette)
                        .child(weights),
                )
            });
            h_flex().child(
                div()
                    .relative()
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        if this.more_open {
                            this.more_open = false;
                            cx.notify();
                        }
                    }))
                    .child(
                        ButtonLike::new("more")
                            .style(if self.more_open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                            .size(ButtonSize::Large)
                            .toggle_state(self.more_open)
                            .child(div().px_1().child(Icon::from_path("icons/kadr/more.svg").size(IconSize::Medium)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.more_open = !this.more_open;
                                cx.notify();
                            })),
                    )
                    .children(menu),
            )
        } else {
            h_flex().gap_0p5().children(hide_options).children(text_options).children(fill_toggle).child(palette).child(weights)
        };

        // The window's controls (or room for them), the modes and history at
        // the left; the tools and their style in the middle.
        let left = h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_1()
            .children(kit::window_controls(false, cx))
            .child(modes)
            .child(div().w(px(6.)))
            .child(history)
            .child(kit::drags_window(div().flex_1().h_full()));
        let right = h_flex().flex_1().min_w_0().h_full().justify_end().child(kit::drags_window(div().flex_1().h_full())).children(kit::window_controls(true, cx));
        self.top_bar(width, cx).child(left).child(h_flex().flex_none().gap_3().child(tools).child(style)).child(right).into_any_element()
    }

    /// The bar along the window's top, for the editing and the crop modes.
    fn top_bar(&self, width: f32, cx: &mut Context<Self>) -> Div {
        h_flex()
            .w(px(width))
            .h(px(42.))
            .gap_2()
            // An even inset keeps corner buttons concentric with the window.
            .px_1()
            .bg(cx.theme().colors().title_bar_background)
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .when(!self.windowed, |d| d.rounded_t(px(kit::WINDOW_RADIUS - 1.)))
            // GPUI clips children to rectangles: the bars round their own
            // outer corners to stay inside the window's.
            .rounded_smoothing(crate::ui::SMOOTHING)
    }

    /// The zoom as a dropdown: fit, or a real size.
    fn zoom_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let percent = |k: f32| format!("{}%", (k * 100.).round());
        let mut zooms: Vec<MenuItem> = vec![(t("editor.fit").into(), self.zoom == 1., Rc::new(|this: &mut Editor, _: &mut Window, _: &mut Context<Editor>| this.zoom = 1.) as MenuPick)];
        zooms.extend([0.5f32, 1., 2.].map(|k| (percent(k), false, Rc::new(move |this: &mut Editor, _: &mut Window, _: &mut Context<Editor>| this.zoom_to_real(k)) as MenuPick)));
        self.crop_dropdown(CropMenu::Zoom, percent(self.disp.0 / self.real.0), t("editor.zoom"), zooms, true, cx)
    }

    /// CleanShot-like bottom bar.
    fn render_footer(&self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        if self.cropping.is_some() {
            return self.render_crop_footer(width, cx);
        }
        // Drag the annotated picture out, rendered at the moment the drag starts.
        let drag_state = self.source.clone().map(|src| (src, self.doc.clone(), self.frame.clone(), self.path.clone()));
        let drag_out = div()
            .id("drag-out")
            .flex()
            .items_center()
            .gap_1p5()
            .h(crate::ui::rems_from_px(32.))
            .px_3()
            .rounded(crate::ui::rems_from_px(9.))
            .bg(cx.theme().colors().element_background)
            .hover(|s| s.bg(cx.theme().colors().element_hover))
            .cursor_grab()
            .child(Icon::from_path("icons/kadr/grip.svg").size(IconSize::Small).color(Color::Muted))
            .when(width >= COMPACT_TOOLBAR, |d| {
                d.child(Label::new(t("editor.drag")).color(Color::Muted)).child(Icon::from_path("icons/kadr/grip.svg").size(IconSize::Small).color(Color::Muted))
            })
            .tooltip(Tooltip::text(t("editor.drag_annotated_image_into_chat")))
            .on_drag(DragOut, |_: &DragOut, _, _, cx| cx.new(|_| DragPreview))
            .external_drag_payload(move |_: &DragOut, _, _| {
                let (src, doc, frame, path) = drag_state.as_ref()?;
                let png = compose(src, doc, frame)?;
                let dir = util::runtime_dir().join("kadr-drag");
                let _ = std::fs::create_dir_all(&dir);
                let out = dir.join(path.file_name()?);
                std::fs::write(&out, png).ok()?;
                Some(gpui::ExternalDragPayload::Files(gpui::FileDragPaths::new([(out, false)])))
            });

        let action = |id: &'static str, icon: Icon, tip: String| ButtonLike::new(id).style(ButtonStyle::Subtle).size(ButtonSize::Large).square().child(icon).tooltip(Tooltip::text(tip));
        let actions = h_flex()
            .gap_0p5()
            .child(action("ocr", Icon::from_path("icons/kadr/scan_text.svg").size(IconSize::Medium), t("editor.copy_text_tip").into()).on_click(cx.listener(|this, _, _, cx| this.read_text(cx))))
            .child(action("pin", Icon::new(IconName::Pin).size(IconSize::Medium), t("editor.pin").into()).on_click(cx.listener(|this, _, _, cx| this.pin(cx))))
            .child(action("save-project", Icon::from_path("icons/kadr/project.svg").size(IconSize::Medium), t("bundle.save_project_tip").into()).on_click(cx.listener(|this, _, _, cx| this.save_project(cx))))
            .child(
                action("copy", Icon::from_path("icons/copy.svg").size(IconSize::Medium), format!("{} (Ctrl+C)", t("common.copy")))
                    .on_click(cx.listener(|this, _, window, cx| this.export(Export::Copy, window, cx))),
            );

        // Save, with its other ways in a menu off the chevron.
        let open = self.crop_menu == Some(CropMenu::Save);
        let saves: Vec<MenuItem> = vec![
            (t("common.save").into(), false, Rc::new(|this: &mut Editor, window: &mut Window, cx: &mut Context<Editor>| this.export(Export::Save, window, cx)) as MenuPick),
            (t("editor.save_as").into(), false, Rc::new(|this: &mut Editor, window: &mut Window, cx: &mut Context<Editor>| this.save_as(window, cx)) as MenuPick),
            (t("editor.save_project").into(), false, Rc::new(|this: &mut Editor, _: &mut Window, cx: &mut Context<Editor>| this.save_project(cx)) as MenuPick),
        ];
        let menu = open.then(|| self.menu_popup(saves, true, true, cx));
        let save = self
            .menu_anchor(CropMenu::Save, cx)
            .child(
                h_flex()
                    .gap(px(1.))
                    .child(
                        Button::new("save", t("common.save"))
                            .style(ButtonStyle::Accent)
                            .size(ButtonSize::Large)
                            .when(width >= COMPACT_TOOLBAR, |b| b.key_binding(kit::keys("ctrl-s")))
                            .on_click(cx.listener(|this, _, window, cx| this.export(Export::Save, window, cx))),
                    )
                    .child(
                        ButtonLike::new("save-more")
                            .style(ButtonStyle::Accent)
                            .size(ButtonSize::Large)
                            .child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Custom(gpui::white())))
                            .tooltip(Tooltip::text(t("editor.save_options")))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.crop_menu = if this.crop_menu == Some(CropMenu::Save) { None } else { Some(CropMenu::Save) };
                                cx.notify();
                            })),
                    ),
            )
            .children(menu);

        // Zoom (and undoing a crop or a reopened drawing) at the left, the
        // drag-out in the middle, the results at the right.
        h_flex()
            .w(px(width))
            .gap_2()
            .p_1()
            .bg(cx.theme().colors().status_bar_background)
            .border_t_1()
            .border_color(cx.theme().colors().border_variant)
            .when(!self.windowed, |d| d.rounded_b(px(kit::WINDOW_RADIUS - 1.)))
            .rounded_smoothing(crate::ui::SMOOTHING)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(self.zoom_menu(cx))
                    .when(self.doc.crop.is_some(), |row| {
                        row.child(
                            Button::new("uncrop", t("editor.uncrop")).style(ButtonStyle::Subtle).size(ButtonSize::Large).color(Color::Muted).on_click(cx.listener(|this, _, _, cx| {
                                this.checkpoint();
                                this.doc.crop = None;
                                cx.notify();
                            })),
                        )
                    })
                    // A reopened drawing can be dropped for the clean picture.
                    .when(self.restored && (!self.doc.shapes.is_empty() || self.doc.crop.is_some()), |row| {
                        row.child(
                            Button::new("discard-edits", t("editor.start_over"))
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Large)
                                .color(Color::Muted)
                                .tooltip(Tooltip::text(t("editor.start_over_tip")))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.discard_edits(cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(drag_out.flex_none())
            .child(h_flex().flex_1().min_w_0().justify_end().gap_2().child(actions).child(save))
            .into_any_element()
    }

    /// A dropdown's open list: the choices, under its button (or over it,
    /// in the footer), from its left edge or its right one.
    fn menu_popup(&self, items: Vec<MenuItem>, up: bool, right: bool, cx: &mut Context<Self>) -> AnyElement {
        let hover = cx.theme().colors().element_hover;
        let rows: Vec<_> = items
            .into_iter()
            .enumerate()
            .map(|(i, (text, on, pick))| {
                h_flex()
                    .id(SharedString::from(format!("menu-row-{i}")))
                    .w_full()
                    .gap_1p5()
                    .px_2()
                    .h(px(28.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(move |d| d.bg(hover))
                    .child(div().w(px(14.)).children(on.then(|| Icon::new(IconName::Check).size(IconSize::XSmall))))
                    .child(Label::new(text).size(LabelSize::Small))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.crop_menu = None;
                        pick(this, window, cx);
                        cx.notify();
                    }))
            })
            .collect();
        gpui::deferred(
            v_flex()
                .id("dropdown-menu")
                .occlude()
                .absolute()
                .when(up, |d| d.bottom(px(38.)))
                .when(!up, |d| d.top(px(38.)))
                .when(right, |d| d.right_0())
                .when(!right, |d| d.left_0())
                .min_w(px(150.))
                .p_1()
                .elevation_3(cx)
                .children(rows)
                .child(crate::ui::track_bounds(&self.crop_menu_bounds)),
        )
        .into_any_element()
    }

    /// Closes `which` on a click outside it and its list.
    fn menu_anchor(&self, which: CropMenu, cx: &mut Context<Self>) -> Div {
        div().relative().on_mouse_down_out(cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            if this.crop_menu == Some(which) && !this.crop_menu_bounds.get().contains(&e.position) {
                this.crop_menu = None;
                cx.notify();
            }
        }))
    }

    /// A dropdown: a button with the current choice, and the choices under
    /// it (or over it, in the footer).
    fn crop_dropdown(&self, which: CropMenu, label: String, tip: &'static str, items: Vec<MenuItem>, up: bool, cx: &mut Context<Self>) -> AnyElement {
        let open = self.crop_menu == Some(which);
        let menu = open.then(|| self.menu_popup(items, up, false, cx));
        self.menu_anchor(which, cx)
            .child(
                ButtonLike::new(SharedString::from(format!("crop-dropdown-{}", which as u8)))
                    .style(if open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(open)
                    .child(h_flex().gap_1().px_1().child(Label::new(label)).child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted)))
                    .tooltip(Tooltip::text(tip))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.crop_menu = if this.crop_menu == Some(which) { None } else { Some(which) };
                        cx.notify();
                    })),
            )
            .children(menu)
            .into_any_element()
    }

    /// The crop mode's top bar, like CleanShot's: the mode, the shape and
    /// size of the crop, turning, the result's size, then Cancel and Crop.
    fn render_crop_bar(&self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let (accent, field_bg) = (colors.text_accent, colors.element_background);
        let (left, top, right, bottom) = self.crop_or_picture();
        let (cw, ch) = ((right - left).round().max(1.) as u32, (bottom - top).round().max(1.) as u32);

        let leave = ButtonLike::new("crop-mode")
            .style(ButtonStyle::Filled)
            .size(ButtonSize::Large)
            .toggle_state(true)
            .child(div().px_1().child(Icon::from_path("icons/kadr/crop.svg").size(IconSize::Tool).color(Color::Accent)))
            .tooltip(Tooltip::text(t("editor.leave_crop")))
            .on_click(cx.listener(|this, _, _, cx| {
                this.apply_crop();
                cx.notify();
            }));
        let aspects: Vec<MenuItem> = crop::ASPECTS
            .iter()
            .map(|&a| (a.label(), self.aspect == a, Rc::new(move |this: &mut Editor, _: &mut Window, _: &mut Context<Editor>| this.pick_aspect(a)) as MenuPick))
            .collect();
        let aspect = self.crop_dropdown(CropMenu::Aspect, self.aspect.label(), t("editor.aspect_ratio"), aspects, false, cx);

        // Click to type an exact size in the picture's pixels.
        let field = |id: &'static str, height: bool, value: u32, cx: &mut Context<Self>| {
            let typing = self.size_field.as_ref().filter(|(h, _)| *h == height).map(|(_, s)| s.clone());
            let shown = match &typing {
                Some(s) if !s.is_empty() => Label::new(s.clone()).size(LabelSize::Small),
                _ => Label::new(value.to_string()).size(LabelSize::Small).color(if typing.is_some() { Color::Muted } else { Color::Default }),
            };
            div()
                .id(id)
                .w(px(68.))
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(1.))
                .rounded(px(7.))
                .bg(field_bg)
                .border_1()
                .border_color(if typing.is_some() { accent } else { gpui::transparent_black() })
                .cursor(CursorStyle::IBeam)
                .child(shown)
                .when(typing.is_some(), |d| d.child(div().w(px(1.)).h(px(14.)).bg(accent)))
                .tooltip(Tooltip::text(t(if height { "editor.height_px" } else { "editor.width_px" })))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.apply_size_field();
                    this.size_field = Some((height, String::new()));
                    cx.notify();
                }))
        };
        let icon_button = |id: &'static str, icon: &'static str, tip: &'static str| {
            ButtonLike::new(id).style(ButtonStyle::Subtle).size(ButtonSize::Large).child(div().px_1().child(Icon::from_path(icon).size(IconSize::Small))).tooltip(Tooltip::text(tip))
        };
        let swap = icon_button("crop-swap", "icons/kadr/swap.svg", t("editor.swap_sides")).on_click(cx.listener(|this, _, _, cx| {
            this.swap_crop_sides();
            cx.notify();
        }));
        let rotate = icon_button("crop-rotate", "icons/kadr/rotate.svg", t("editor.rotate")).on_click(cx.listener(|this, _, _, cx| {
            this.turn(crop::Turn::Clockwise, cx);
            cx.notify();
        }));
        let flip = icon_button("crop-flip", "icons/kadr/flip.svg", t("editor.flip")).on_click(cx.listener(|this, _, _, cx| {
            this.turn(crop::Turn::Mirror, cx);
            cx.notify();
        }));

        // The result's size; smaller ones from the menu.
        let (el, et, er, eb) = self.extent();
        let (ow, oh) = (er - el, eb - et);
        let sized = |k: f32| format!("{}×{} px", (ow * k).round().max(1.), (oh * k).round().max(1.));
        let scales: Vec<MenuItem> = SCALES
            .iter()
            .map(|&k| (format!("{}% · {}", (k * 100.).round(), sized(k)), (self.doc.scale - k).abs() < 1e-3, Rc::new(move |this: &mut Editor, _: &mut Window, _: &mut Context<Editor>| this.pick_scale(k)) as MenuPick))
            .collect();
        let scale = self.crop_dropdown(CropMenu::Scale, sized(self.doc.scale), t("editor.image_size_tip"), scales, false, cx);

        let narrow = width < COMPACT_TOOLBAR;
        let tight = width < 780.;
        let left = h_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .children(kit::window_controls(false, cx))
            .child(leave)
            .child(aspect)
            .when(!tight, |row| row.child(field("crop-width", false, cw, cx)).child(swap).child(field("crop-height", true, ch, cx)))
            .child(div().w(px(12.)))
            .child(rotate)
            .child(flip)
            .child(div().w(px(12.)))
            .when(!narrow, |row| row.child(Label::new(t("editor.image_size")).size(LabelSize::Small).color(Color::Muted)))
            .child(scale);
        let right = h_flex()
            .flex_none()
            .gap_1()
            .child(
                Button::new("crop-cancel", t("common.cancel"))
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Large)
                    .when(!narrow, |b| b.key_binding(kit::keys("escape")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_crop(cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("crop-apply", t("editor.crop"))
                    .style(ButtonStyle::Accent)
                    .size(ButtonSize::Large)
                    .when(!narrow, |b| b.key_binding(kit::keys("enter")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.apply_crop();
                        cx.notify();
                    })),
            )
            .children(kit::window_controls(true, cx));
        self.top_bar(width, cx).justify_between().child(left).child(right).into_any_element()
    }

    /// The crop mode's bottom bar: zoom, snapping, and going back to the
    /// picture as taken.
    fn render_crop_footer(&self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let (accent, edge) = (colors.text_accent, colors.border);
        let zoom = self.zoom_menu(cx);

        let on = self.snap;
        let check = div()
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(if on { accent } else { edge })
            .when(on, |d| d.bg(accent).child(gpui::svg().path("icons/check.svg").size(px(12.)).text_color(gpui::white())));
        let snap = h_flex()
            .id("crop-snap")
            .gap_1p5()
            .px_1()
            .h(px(28.))
            .cursor_pointer()
            .child(check)
            .child(Label::new(t("editor.snap_to_edges")).size(LabelSize::Small))
            .on_click(cx.listener(|this, _, _, cx| {
                this.snap = !this.snap;
                cx.notify();
            }));
        let hint = t(if on { "editor.hold_ctrl_to_move_freely" } else { "editor.hold_ctrl_to_snap" });
        let changed = self.doc.crop.is_some() || !self.doc.view.is_upright() || self.doc.scale != 1.;
        h_flex()
            .w(px(width))
            .gap_2()
            .p_1()
            .bg(cx.theme().colors().status_bar_background)
            .border_t_1()
            .border_color(cx.theme().colors().border_variant)
            .when(!self.windowed, |d| d.rounded_b(px(kit::WINDOW_RADIUS - 1.)))
            .rounded_smoothing(crate::ui::SMOOTHING)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(zoom)
                    .child(snap)
                    .when(width >= COMPACT_TOOLBAR, |row| row.child(Label::new(hint).size(LabelSize::XSmall).color(Color::Muted))),
            )
            .child(
                Button::new("crop-revert", t("editor.revert_to_original"))
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Large)
                    .disabled(!changed)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.revert_crop(cx);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn slider(&self, knob: Knob, value: f32, label: String, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let (track, filled, knob_color, knob_edge) = (colors.element_active, colors.text_accent, gpui::white(), colors.border);
        let sliders = self.sliders.clone();
        let v = value.clamp(0., 1.);
        let bar = canvas(
            move |bounds, _, _| {
                sliders.borrow_mut().insert(knob, (f32::from(bounds.origin.x), f32::from(bounds.size.width)));
                bounds
            },
            move |_, bounds, window, _| {
                let mid = bounds.origin.y + bounds.size.height / 2.;
                let (x0, w) = (bounds.origin.x, bounds.size.width);
                window.paint_quad(crate::ui::quad(Bounds::new(point(x0, mid - px(2.)), size(w, px(4.))), px(2.), track, px(0.), track, BorderStyle::Solid));
                window.paint_quad(crate::ui::quad(Bounds::new(point(x0, mid - px(2.)), size(w * v, px(4.))), px(2.), filled, px(0.), filled, BorderStyle::Solid));
                let c = x0 + w * v;
                window.paint_quad(crate::ui::quad(Bounds::new(point(c - px(9.), mid - px(8.)), size(px(18.), px(18.))), px(9.), gpui::hsla(0., 0., 0., 0.18), px(0.), knob_edge, BorderStyle::Solid));
                window.paint_quad(crate::ui::quad(Bounds::new(point(c - px(8.), mid - px(8.)), size(px(16.), px(16.))), px(8.), knob_color, px(0.5), knob_edge, BorderStyle::Solid));
            },
        )
        .size_full();
        h_flex()
            .w_full()
            .gap_3()
            .child(div().id(SharedString::from(format!("slider-{}", knob as u8))).flex_1().h(px(22.)).mx_2().cursor(CursorStyle::PointingHand).child(bar).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if knob == Knob::Weight && this.selected.is_some() {
                        this.checkpoint();
                    }
                    this.drag = Some(Drag::Slider(knob));
                    this.slide(knob, f32::from(e.position.x), cx);
                }),
            ))
            .when(!label.is_empty(), |row| row.child(div().w(px(44.)).flex().justify_end().child(Label::new(label).size(LabelSize::Small).color(Color::Muted))))
            .into_any_element()
    }

    /// The background tool as a sidebar, like CleanShot's: presets, then
    /// what's behind the picture (nothing, gradients, wallpapers, the
    /// screenshot blurred, plain colours), then its margins, corners, shadow
    /// and the output's shape.
    fn render_panel(&self, height: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let (accent, edge, field_bg, track_bg, thumb_bg) = (colors.text_accent, colors.border_variant, colors.element_background, colors.element_hover, colors.element_selected);
        let clear = gpui::transparent_black();
        let heading = |text: &'static str| Label::new(text).size(LabelSize::Small).color(Color::Muted);
        // A square tile with an accent ring when chosen; the ring, a gap and
        // the swatch keep their corners concentric.
        let tile = |id: SharedString, on: bool| {
            div().id(id).flex_none().p(px(2.)).rounded(px(TILE_RADIUS)).border_2().border_color(if on { accent } else { clear }).cursor_pointer()
        };
        let swatch = || div().size(px(TILE)).rounded(px(TILE_RADIUS - 4.)).overflow_hidden().border_1().border_color(edge);
        let grid = || h_flex().w_full().flex_wrap().gap(px(TILE_GAP));
        let pick = |backdrop: Backdrop| {
            cx.listener(move |this: &mut Editor, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Editor>| {
                this.frame.backdrop = backdrop.clone();
                cx.notify();
            })
        };

        let none_on = self.frame.backdrop == Backdrop::None;
        let none = div()
            .id("backdrop-none")
            .w_full()
            .h(px(34.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(9.))
            .rounded_smoothing(crate::ui::SMOOTHING)
            .bg(field_bg)
            .border_2()
            .border_color(if none_on { accent } else { clear })
            .cursor_pointer()
            .hover(|d| d.bg(track_bg))
            .child(Label::new(t("editor.none")).size(LabelSize::Small))
            .on_click(pick(Backdrop::None));

        let all = self.backdrops();
        let gradients: Vec<usize> = (0..all.len()).filter(|&i| all[i].0 != all[i].1).collect();
        let plains: Vec<usize> = (0..all.len()).filter(|&i| all[i].0 == all[i].1).collect();
        let more = gradients.len() > GRADIENT_ROWS;
        let shown = if self.more_gradients { gradients.len() } else { gradients.len().min(GRADIENT_ROWS) };
        let gradient_tiles = gradients[..shown].iter().map(|&i| {
            let (c0, c1) = all[i];
            tile(SharedString::from(format!("gradient-{i}")), self.frame.backdrop == Backdrop::Gradient(i))
                .child(swatch().bg(linear_gradient(135., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.))))
                .on_click(pick(Backdrop::Gradient(i)))
        })
        .collect::<Vec<_>>();
        let toggle_more = more.then(|| {
            Button::new("more-gradients", t(if self.more_gradients { "editor.show_less" } else { "editor.show_more" }))
                .style(ButtonStyle::Subtle)
                .label_size(LabelSize::Small)
                .color(Color::Muted)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.more_gradients = !this.more_gradients;
                    cx.notify();
                }))
        });

        // Wallpapers, and a tile for one's own picture.
        let mut walls: Vec<AnyElement> = self
            .walls
            .iter()
            .enumerate()
            .map(|(i, (path, thumb))| {
                tile(SharedString::from(format!("wall-{i}")), self.frame.backdrop == Backdrop::Wallpaper(path.clone()))
                    .child(swatch().child(img(thumb.clone()).size_full().object_fit(ObjectFit::Cover)))
                    .on_click(pick(Backdrop::Wallpaper(path.clone())))
                    .into_any_element()
            })
            .collect();
        let own = matches!(&self.frame.backdrop, Backdrop::Wallpaper(p) if !self.walls.iter().any(|(w, _)| w == p));
        walls.push(
            tile("own-wallpaper".into(), own)
                .child(
                    div()
                        .size(px(TILE))
                        .rounded(px(TILE_RADIUS - 4.))
                        .border_1()
                        .border_dashed()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(IconName::Plus).size(IconSize::Small).color(Color::Muted)),
                )
                .tooltip(Tooltip::text(t("common.your_own_image")))
                .on_click(cx.listener(|this, _, _, cx| this.pick_wallpaper(cx)))
                .into_any_element(),
        );

        let blurred = tile("backdrop-blurred".into(), self.frame.backdrop == Backdrop::Blurred)
            .child(swatch().bg(field_bg).children(self.blurred.clone().map(|b| img(b).size_full().object_fit(ObjectFit::Cover))))
            .tooltip(Tooltip::text(t("editor.blurred_tip")))
            .on_click(pick(Backdrop::Blurred));

        // Plain colours as dots, transparency first.
        let dot = |id: SharedString, on: bool| {
            div().id(id).flex_none().p(px(2.)).rounded_full().border_2().border_color(if on { accent } else { clear }).cursor_pointer()
        };
        let mut plain: Vec<AnyElement> = vec![dot("transparent".into(), self.frame.backdrop == Backdrop::Transparent)
            .child(div().size(px(DOT)).rounded_full().overflow_hidden().border_1().border_color(edge).child(checker_tile(DOT, 5.)))
            .tooltip(Tooltip::text(t("editor.transparent")))
            .on_click(pick(Backdrop::Transparent))
            .into_any_element()];
        plain.extend(plains.iter().map(|&i| {
            dot(SharedString::from(format!("plain-{i}")), self.frame.backdrop == Backdrop::Gradient(i))
                .child(div().size(px(DOT)).rounded_full().border_1().border_color(edge).bg(all[i].0))
                .on_click(pick(Backdrop::Gradient(i)))
                .into_any_element()
        }));

        // The output's shape, as a segmented control.
        let aspects: Vec<AnyElement> = ASPECTS
            .iter()
            .enumerate()
            .map(|(i, (label, _))| {
                let on = self.frame.aspect == i;
                div()
                    .id(SharedString::from(format!("aspect-{i}")))
                    .flex_grow(1.)
                    .px_1()
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(on, |d| d.bg(thumb_bg))
                    .when(!on, |d| d.hover(|d| d.bg(track_bg)))
                    .child(Label::new(t(label)).size(LabelSize::Small).color(if on { Color::Default } else { Color::Muted }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.frame.aspect = i;
                        if !this.frame.on() {
                            this.frame.backdrop = Backdrop::Gradient(0);
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        let slider = |text: &'static str, control: AnyElement| v_flex().w_full().gap_1().child(heading(text)).child(control);
        let padding = self.slider(Knob::Padding, self.frame.padding / MAX_PAD, format!("{}%", (self.frame.padding * 100.).round()), cx);
        let radius = self.slider(Knob::Radius, self.frame.radius / MAX_ROUND, format!("{}", (self.frame.radius / MAX_ROUND * 100.).round()), cx);
        let shadow = self.slider(Knob::Shadow, self.frame.shadow, format!("{}%", (self.frame.shadow * 100.).round()), cx);
        let size = self.source.as_ref().filter(|_| self.frame.on()).map(|_| {
            let (l, t, r, b) = self.extent();
            let ((ow, oh), _) = self.frame.layout(r - l, b - t);
            tr!("editor.output_size", ow, oh)
        });

        let section = || v_flex().w_full().gap_1p5();
        let content = v_flex()
            .id("background-panel")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.side_scroll)
            .gap_4()
            .p_3()
            .child(v_flex().w_full().gap_2().child(crate::presets::render(&self.presets, &self.frame.to_json(), cx)).child(none))
            .child(section().child(h_flex().w_full().justify_between().child(heading(t("editor.gradients"))).children(toggle_more)).child(grid().children(gradient_tiles)))
            .child(section().child(heading(t("editor.wallpapers"))).child(grid().children(walls)))
            .child(section().child(heading(t("editor.blurred"))).child(grid().child(blurred)))
            .child(section().child(heading(t("editor.plain_color"))).child(h_flex().w_full().flex_wrap().gap(px(2.)).children(plain)))
            .child(
                v_flex()
                    .w_full()
                    .gap_3()
                    .pt_1()
                    .child(slider(t("studio.row_padding"), padding))
                    .child(slider(t("studio.row_corners"), radius))
                    .child(slider(t("studio.row_shadow"), shadow)),
            )
            .child(
                section()
                    .child(heading(t("editor.aspect_ratio")))
                    .child(h_flex().w_full().p(px(2.)).gap(px(2.)).rounded(px(8.)).bg(field_bg).children(aspects))
                    .children(size.map(|s| Label::new(s).size(LabelSize::XSmall).color(Color::Muted))),
            );
        div()
            .relative()
            .flex_none()
            .w(px(PANEL))
            .h(px(height))
            .bg(colors.panel_background)
            .border_r_1()
            .border_color(edge)
            .child(content)
            .child(crate::ui::scrollbar(&self.side_scroll, &self.side_grab, cx))
    }

    /// Picture with blur patches, vector shapes, text and counters, then the
    /// crop shade and the selection on top.
    fn render_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.scale();
        // Everything is laid out in the work area, the picture `m` in.
        let m = self.margin();
        let off = m * s;
        let at = |p: P| (px(p.0 * s + off), px(p.1 * s + off));
        let (pw, ph) = (self.px.0 as f32, self.px.1 as f32);

        // Pixelation stays on the picture (it covers only what's there).
        let blurs = self.doc.shapes.iter().filter_map(|shape| match &shape.kind {
            Kind::Blur(_, Some(patch)) => {
                let (l, t, r, b) = rect(shape.a, shape.b);
                let (l, t, r, b) = (l.max(0.).floor(), t.max(0.).floor(), r.min(pw).ceil(), b.min(ph).ceil());
                Some(img(patch.clone()).absolute().left(px(l * s)).top(px(t * s)).w(px((r - l) * s)).h(px((b - t) * s)).object_fit(ObjectFit::Fill))
            }
            _ => None,
        });
        let pictures = self.doc.shapes.iter().filter_map(|shape| match &shape.kind {
            Kind::Picture(Picture { shown: patch, .. }) => {
                let (l, t, r, b) = rect(shape.a, shape.b);
                Some(img(patch.clone()).absolute().left(px(l * s + off)).top(px(t * s + off)).w(px((r - l) * s)).h(px((b - t) * s)).object_fit(ObjectFit::Fill))
            }
            _ => None,
        });

        let vector: Vec<Shape> = self.doc.shapes.iter().chain(self.draft.iter()).cloned().collect();
        let spots: Vec<(P, P)> = vector.iter().filter(|sh| matches!(sh.kind, Kind::Spotlight)).map(|sh| (sh.a, sh.b)).collect();
        let picture_size = (self.px.0 as f32, self.px.1 as f32);
        let origin = self.origin.clone();
        let strokes = canvas(
            move |bounds, _, _| {
                origin.set((f32::from(bounds.origin.x), f32::from(bounds.origin.y)));
                bounds
            },
            move |_, bounds, window, _| {
                let o = bounds.origin;
                let pt = |p: P| point(o.x + px(p.0 * s + off), o.y + px(p.1 * s + off));
                // The dimming goes under the other shapes.
                paint_spotlights(window, &spots, picture_size, &pt);
                for shape in &vector {
                    paint_shape(window, shape, s, &pt);
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        let labels = self.doc.shapes.iter().enumerate().filter_map(|(i, shape)| match &shape.kind {
            Kind::Text(text) => {
                let (x, y) = at(shape.a);
                let m = metrics(shape.size);
                let editing = self.editing == Some(i);
                let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
                let count = lines.len();
                let block = |color: Hsla| {
                    div().text_color(color).children(lines.iter().enumerate().map(|(li, line)| {
                        let caret = editing && li + 1 == count;
                        div().h(px(m.line * s)).line_height(px(m.line * s)).child(if caret { format!("{line}▏") } else { line.clone() })
                    }))
                };
                // The export draws text with the bundled font; the canvas must match it.
                let base = div().absolute().font_family(kit::ANNOTATION_FONT).text_size(px(shape.size * s)).font_weight(gpui::FontWeight::SEMIBOLD).whitespace_nowrap();
                Some(match shape.text {
                    TextStyle::Plain => base.left(x).top(y).child(block(shape.color)).into_any_element(),
                    TextStyle::Plate => {
                        let (padx, pady, radius) = TextStyle::plate(shape.size);
                        base.left(x - px(padx * s))
                            .top(y - px(pady * s))
                            .px(px(padx * s))
                            .py(px(pady * s))
                            .rounded(px(radius * s))
                            .bg(shape.color)
                            .child(block(on_color(shape.color)))
                            .into_any_element()
                    }
                    // GPUI has no text stroke: the outline is the text drawn
                    // around itself in the contrasting colour.
                    TextStyle::Outline => {
                        let o = TextStyle::outline(shape.size) * s;
                        let ring = on_color(shape.color);
                        let copies = (0..12).map(|k| {
                            let th = k as f32 / 12. * std::f32::consts::TAU;
                            div().absolute().left(px(o * th.cos())).top(px(o * th.sin())).child(block(ring))
                        });
                        base.left(x).top(y).child(div().relative().children(copies).child(block(shape.color))).into_any_element()
                    }
                })
            }
            Kind::Counter(n) => {
                let d = shape.size * s;
                Some(
                    div()
                        .absolute()
                        .left(px(shape.a.0 * s + off - d / 2.))
                        .top(px(shape.a.1 * s + off - d / 2.))
                        .size(px(d))
                        .rounded_full()
                        .bg(shape.color)
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family(kit::ANNOTATION_FONT)
                        .text_size(px(d * 0.55))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(on_color(shape.color))
                        .child(n.to_string())
                        .into_any_element(),
                )
            }
            _ => None,
        });

        let crop = self.doc.crop;
        let cropping = self.cropping.is_some();
        // Thirds while the crop is being dragged, like a camera's grid.
        let thirds = cropping && matches!(self.drag, Some(Drag::Crop { .. } | Drag::CropEdit { .. }));
        // The result's bounds, when drawing has grown it past the picture.
        let (el, et, er, eb) = self.extent();
        let grown = crop.is_none() && (el < 0. || et < 0. || er > pw || eb > ph);
        let selection = self.selected.filter(|_| self.editing.is_none()).and_then(|i| self.doc.shapes.get(i)).map(|sh| (sh.bounds(), sh.knobs(s), sh.band_pad(s), matches!(sh.kind, Kind::Line | Kind::Arrow)));
        let accent = cx.theme().colors().text_accent;
        let overlay = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                let pt = |p: P| point(o.x + px(p.0 * s + off), o.y + px(p.1 * s + off));
                let area = |l: f32, t: f32, r: f32, b: f32| Bounds::from_corners(pt((l, t)), pt((r, b)));
                if let Some((a, b)) = crop {
                    let (l, t, r, bt) = rect(a, b);
                    // The whole work area outside the crop, in image pixels.
                    let (w, h) = (f32::from(bounds.size.width) / s - m, f32::from(bounds.size.height) / s - m);
                    let shade = rgba(0x000000a6);
                    for (l0, t0, r0, b0) in [(-m, -m, w, t), (-m, bt, w, h), (-m, t, l, bt), (r, t, w, bt)] {
                        if r0 > l0 && b0 > t0 {
                            window.paint_quad(fill(area(l0, t0, r0, b0), shade));
                        }
                    }
                    if cropping {
                        paint_crop_frame(window, area(l, t, r, bt), thirds);
                    } else {
                        window.paint_quad(outline(area(l, t, r, bt), gpui::white(), BorderStyle::Dashed));
                    }
                } else if grown {
                    window.paint_quad(outline(area(el, et, er, eb), gpui::hsla(0., 0., 1., 0.45), BorderStyle::Dashed));
                }
                // Like CleanShot: a soft, wide band just outside anything
                // boxy with knobs on its corners; lines show their ends.
                if let Some(((l, t, r, b), knobs, pad, line)) = selection {
                    if line {
                        for h in knobs {
                            crate::ui::paint_knob(window, pt(h), accent);
                        }
                    } else {
                        let mut band = accent;
                        band.lightness = (band.lightness + 0.12).min(0.8);
                        band.alpha = 0.8;
                        let half = BAND / 2. / s;
                        let r0 = area(l - pad - half, t - pad - half, r + pad + half, b + pad + half);
                        window.paint_quad(crate::ui::quad(r0, px(2.), gpui::transparent_black(), px(BAND), band, BorderStyle::Solid));
                        for &h in knobs.iter().take(4) {
                            crate::ui::paint_knob(window, pt(h), accent);
                        }
                    }
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        let pointer = match self.tool {
            _ if self.drag.is_none() && self.hover.is_some() => self.hover.unwrap_or(CursorStyle::Arrow),
            Tool::Select => CursorStyle::Arrow,
            Tool::Text => CursorStyle::IBeam,
            _ => CursorStyle::Crosshair,
        };
        let framed = self.frame.on();
        let radius = if framed { self.disp.0.max(self.disp.1) * self.frame.radius } else { 6. };
        let picture = div()
            .absolute()
            .left(px(off))
            .top(px(off))
            .w(px(self.disp.0))
            .h(px(self.disp.1))
            .rounded(px(radius))
            .overflow_hidden()
            .when(!framed, |d| d.shadow(kit::shadow()))
            .when(framed && self.frame.shadow > 0., |d| {
                let big = self.disp.0.max(self.disp.1);
                d.shadow(vec![BoxShadow {
                    color: gpui::hsla(0., 0., 0., 0.6 * self.frame.shadow).into(),
                    offset: point(px(0.), px(big * 0.012)),
                    blur_radius: px(big * 0.035),
                    spread_radius: px(0.),
                    inset: false,
                }])
            })
            .child(img(self.image.clone()).size_full().object_fit(ObjectFit::Fill).rounded(px(radius)))
            .children(blurs)
            // A hairline edge, so a dark screenshot doesn't melt into the
            // dark stage around it.
            .when(!framed, |d| d.child(div().absolute().inset_0().rounded(px(radius)).border_1().border_color(cx.theme().colors().border)));

        // The background preview mirrors what Save produces: around the
        // result, which may reach past the picture.
        let ((ow, oh), (left, top)) = self.frame.layout((er - el) * s, (eb - et) * s);
        let backdrop = match &self.frame.backdrop {
            Backdrop::None => None,
            Backdrop::Transparent => Some(checkerboard(10.).absolute().inset_0().size_full().into_any_element()),
            Backdrop::Gradient(i) => {
                let (c0, c1) = self.backdrops()[(*i).min(self.backdrops().len() - 1)];
                Some(div().absolute().inset_0().bg(linear_gradient(135., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.))).into_any_element())
            }
            Backdrop::Wallpaper(path) => Some(img(path.clone()).absolute().inset_0().size_full().object_fit(ObjectFit::Cover).into_any_element()),
            Backdrop::Blurred => Some(match &self.blurred {
                Some(blurred) => img(blurred.clone()).absolute().inset_0().size_full().object_fit(ObjectFit::Cover).into_any_element(),
                None => div().absolute().inset_0().bg(cx.theme().colors().element_background).into_any_element(),
            }),
        };
        let backdrop = backdrop.map(|fill| {
            div()
                .absolute()
                .left(px(el * s + off - left))
                .top(px(et * s + off - top))
                .w(px(ow))
                .h(px(oh))
                .rounded_lg()
                .overflow_hidden()
                .child(fill)
        });
        div()
            .id("canvas")
            .relative()
            .flex_none()
            .w(px(self.disp.0 + 2. * off))
            .h(px(self.disp.1 + 2. * off))
            .cursor(pointer)
            .children(backdrop)
            .child(picture)
            .children(pictures)
            .child(strokes)
            .children(labels)
            .child(overlay)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.mouse_down(e, cx)))
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The next screenshot starts with this background.
        let frame = self.frame.to_json();
        if frame != self.remembered {
            self.frame.remember();
            self.remembered = frame;
        }
        // A CleanShot-like window: tools on top, the picture on a stage
        // (with the background panel beside it when open), actions at the
        // bottom. The picture fits the stage, its background included.
        let viewport = window.viewport_size();
        let vw = f32::from(viewport.width);
        let panel = if self.panel { PANEL } else { 0. };
        let box_of = |frame: &Frame, (w, h): (f32, f32)| {
            if frame.on() {
                frame.layout(w, h).0
            } else {
                let pad = w.max(h) * WORK_PAD;
                (w + 2. * pad, h + 2. * pad)
            }
        };
        let full = box_of(&self.frame, self.natural);
        let room = (vw - 48. - panel, self.stage_h.get() - 48.);
        let k = (room.0 / full.0).min(room.1 / full.1).min(1.);
        let fit = (self.natural.0 * k, self.natural.1 * k);
        self.disp = (fit.0 * self.zoom, fit.1 * self.zoom);
        // The picture fits the stage; a zoomed canvas scrolls inside it.
        let width = if self.windowed { vw } else { self.size.0 };
        let stage_h = self.stage_h.clone();
        let measure = canvas(
            move |bounds, window, _| {
                let h = f32::from(bounds.size.height);
                if (h - stage_h.get()).abs() > 0.5 {
                    stage_h.set(h);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let stage = h_flex()
            .relative()
            .w_full()
            .flex_1()
            .min_h_0()
            .child(measure)
            // The background sidebar at the left, like CleanShot's.
            .when(self.panel, |row| row.child(self.render_panel(self.stage_h.get(), cx)))
            .child(
                div()
                    .id("stage")
                    .flex()
                    .items_start()
                    .flex_1()
                    .h_full()
                    .overflow_scroll()
                    .bg(cx.theme().colors().editor_background)
                    .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| {
                        if e.modifiers.control {
                            cx.stop_propagation();
                            let dy = f32::from(e.delta.pixel_delta(px(20.)).y);
                            if dy != 0. {
                                this.zoom_by(if dy > 0. { 1.1 } else { 1. / 1.1 });
                                cx.notify();
                            }
                        }
                    }))
                    // Centered with auto margins, which stop at the edge when the
                    // zoomed canvas is bigger, so all of it can be scrolled to.
                    .child(div().flex_none().min_w(relative(1.)).min_h(relative(1.)).flex().p(px(24.)).child(div().m_auto().child(self.render_canvas(cx)))),
            )
            ;
        let border = cx.theme().colors().border;
        let bars = |el: Div| el.child(self.render_toolbar(width, cx)).child(stage).child(self.render_footer(width, cx));
        let content = if self.windowed {
            bars(v_flex().size_full())
        } else {
            // Over everything: a panel on a dimmed screen.
            bars(
                v_flex()
                    .w(px(width))
                    .h(px(self.size.1))
                    .rounded(px(kit::WINDOW_RADIUS))
                    .rounded_smoothing(crate::ui::SMOOTHING)
                    .overflow_hidden()
                    .border_1()
                    .border_color(border)
                    .shadow(kit::shadow()),
            )
        };
        kit::root(cx)
            .when(self.windowed, |d| d.bg(cx.theme().colors().editor_background))
            .when(!self.windowed, |d| d.bg(rgba(0x00000040)).items_center().justify_center())
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.drop_files(paths);
                cx.notify();
            }))
            // The pointer stays with this surface while the button is held.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.mouse_up(cx)))
            .child(content)
    }
}

/// The crop mode's frame, like CleanShot's: a thin line, thick white
/// brackets at the corners and short bars on the edges' middles, and the
/// thirds while it's dragged.
fn paint_crop_frame(window: &mut Window, b: Bounds<gpui::Pixels>, thirds: bool) {
    let (l, t) = (f32::from(b.origin.x), f32::from(b.origin.y));
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    let bar = |x: f32, y: f32, bw: f32, bh: f32| Bounds::new(point(px(x), px(y)), size(px(bw), px(bh)));
    let white = gpui::white();
    let soft = gpui::hsla(0., 0., 1., 0.45);
    if thirds {
        for i in 1..3 {
            let k = i as f32 / 3.;
            window.paint_quad(fill(bar(l + w * k, t, 1., h), soft));
            window.paint_quad(fill(bar(l, t + h * k, w, 1.), soft));
        }
    }
    window.paint_quad(outline(b, gpui::hsla(0., 0., 1., 0.8), BorderStyle::Solid));
    // Brackets just outside the corners, so they don't cover the picture.
    let (arm, thick) = ((w.min(h) / 3.).clamp(6., 20.), 3.);
    let (r, bt) = (l + w, t + h);
    for (x, y, sx, sy) in [(l, t, 1., 1.), (r, t, -1., 1.), (r, bt, -1., -1.), (l, bt, 1., -1.)] {
        let ox = if sx > 0. { x - thick } else { x - arm };
        let oy = if sy > 0. { y - thick } else { y };
        window.paint_quad(fill(bar(ox, oy, arm + thick, thick), white).corner_radii(px(1.)));
        let ox = if sx > 0. { x - thick } else { x };
        let oy = if sy > 0. { y - thick } else { y - arm };
        window.paint_quad(fill(bar(ox, oy, thick, arm + thick), white).corner_radii(px(1.)));
    }
    // Bars on the edges' middles, where they fit.
    let len = 22f32;
    if w > len * 3. {
        window.paint_quad(fill(bar(l + (w - len) / 2., t - thick, len, thick), white).corner_radii(px(1.5)));
        window.paint_quad(fill(bar(l + (w - len) / 2., bt, len, thick), white).corner_radii(px(1.5)));
    }
    if h > len * 3. {
        window.paint_quad(fill(bar(l - thick, t + (h - len) / 2., thick, len), white).corner_radii(px(1.5)));
        window.paint_quad(fill(bar(r, t + (h - len) / 2., thick, len), white).corner_radii(px(1.5)));
    }
}

/// What's dragged from "editor.drag"; the file is rendered when the drag starts.
struct DragOut;

/// The platform draws its own icon for outbound file drags.
struct DragPreview;

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Shift while drawing: squares, circles and 45° lines.
fn constrain(d: &Shape, p: P, shift: bool) -> P {
    if !shift {
        return p;
    }
    let (dx, dy) = (p.0 - d.a.0, p.1 - d.a.1);
    match d.kind {
        Kind::Rect | Kind::Ellipse | Kind::Blur(..) | Kind::Spotlight => {
            let m = dx.abs().max(dy.abs());
            (d.a.0 + m * dx.signum(), d.a.1 + m * dy.signum())
        }
        Kind::Line | Kind::Arrow => {
            let step = std::f32::consts::FRAC_PI_4;
            let angle = (dy.atan2(dx) / step).round() * step;
            let len = dx.hypot(dy);
            (d.a.0 + len * angle.cos(), d.a.1 + len * angle.sin())
        }
        _ => p,
    }
}

/// A straight segment, or a quadratic curve from `a` to `b` passing through
/// `bend` halfway, as points close enough to draw as a polyline.
fn curve(a: P, b: P, bend: Option<P>) -> Vec<P> {
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

/// An arrow along `points`: the shaft, stopping where the head begins, and
/// the head pointing along the line's last stretch.
fn arrow_parts(points: &[P], w: f32) -> (Vec<P>, [P; 3]) {
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

fn segment_distance(p: P, a: P, b: P) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0. { 0. } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0., 1.) };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// The room around a picture of `size` to draw and crop in, in its pixels.
fn work_margin(size: (u32, u32)) -> f32 {
    (size.0.max(size.1) as f32 * WORK_PAD).round()
}

/// What the result covers, in image pixels (whole, within the work area):
/// the crop, or else the picture and everything drawn around it.
fn extent(doc: &Doc, size: (u32, u32)) -> (f32, f32, f32, f32) {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let m = work_margin(size);
    let (l, t, r, b) = match doc.crop {
        Some((a, b)) => rect(a, b),
        None => doc.shapes.iter().map(Shape::bounds).fold((0f32, 0f32, w, h), |e, s| (e.0.min(s.0), e.1.min(s.1), e.2.max(s.2), e.3.max(s.3))),
    };
    let (l, t) = (l.round().clamp(-m, w + m), t.round().clamp(-m, h + m));
    let (r, b) = (r.round().clamp(l + 1., w + m + 1.), b.round().clamp(t + 1., h + m + 1.));
    (l, t, r, b)
}

/// Annotated, cropped, optionally framed picture as PNG bytes.
fn compose(src: &RgbaImage, doc: &Doc, frame: &Frame) -> Option<Vec<u8>> {
    // The picture on a transparent canvas the size of the result, and the
    // shapes moved along with it.
    let (l, t, r, b) = extent(doc, src.dimensions());
    let (ew, eh) = ((r - l) as u32, (b - t) as u32);
    let mut canvas = RgbaImage::new(ew, eh);
    image::imageops::replace(&mut canvas, src, -l as i64, -t as i64);
    let shapes: Vec<Shape> = doc
        .shapes
        .iter()
        .map(|sh| {
            let mut sh = sh.clone();
            sh.translate((-l, -t));
            sh
        })
        .collect();
    let mut out = rasterize(&canvas, &shapes)?;
    if frame.on() {
        // The picture gets the rounded corners and the shadow; what's drawn
        // past its edges sits on the backdrop as it is.
        let drawn = rasterize(&RgbaImage::new(ew, eh), &shapes)?;
        let (w, h) = (src.width() as f32, src.height() as f32);
        let inner = ((-l).max(0.), (-t).max(0.), (w - l).min(ew as f32), (h - t).min(eh as f32));
        out = framed(&out, &drawn, inner, frame);
    }
    // The crop mode's image size: the whole result, smaller.
    if doc.scale < 0.999 {
        let (w, h) = out.dimensions();
        let (nw, nh) = (((w as f32 * doc.scale).round() as u32).max(1), ((h as f32 * doc.scale).round() as u32).max(1));
        out = image::imageops::resize(&out, nw, nh, image::imageops::FilterType::Lanczos3);
    }
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

fn frame_presets() -> Vec<(String, serde_json::Value)> {
    let saved = std::fs::read_to_string(util::config_dir().join("editor-presets.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    saved.as_ref().and_then(|v| v.as_array()).into_iter().flatten().filter_map(|p| Some((p["name"].as_str()?.to_owned(), p["look"].clone()))).collect()
}

fn save_frame_presets(presets: &[(String, serde_json::Value)]) {
    let v: Vec<_> = presets.iter().map(|(name, look)| serde_json::json!({ "name": name, "look": look })).collect();
    let _ = std::fs::create_dir_all(util::config_dir());
    let _ = std::fs::write(util::config_dir().join("editor-presets.json"), serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// The backdrop picture at `w`×`h`: the wallpaper cropped to fill, or the
/// gradient.
fn backdrop_pixels(backdrop: &Backdrop, picture: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    if *backdrop == Backdrop::Blurred {
        return blurred_pixels(picture, w, h);
    }
    if let Backdrop::Wallpaper(path) = backdrop
        && let Ok(img) = image::open(path)
    {
        return img.resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    }
    let all = palette::backdrops();
    let (c0, c1) = match backdrop {
        Backdrop::Gradient(i) => all[(*i).min(all.len() - 1)],
        _ => all[0],
    };
    let (a, b) = (crate::ui::rgb_of(c0), crate::ui::rgb_of(c1));
    RgbaImage::from_fn(w, h, |x, y| {
        let g = ((x as f32 / w as f32) * 0.6 + (y as f32 / h as f32) * 0.4).clamp(0., 1.);
        let mix = |p: f32, q: f32| ((p + (q - p) * g) * 255.).round() as u8;
        image::Rgba([mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), 255])
    })
}

/// `picture` blurred into a `w`×`h` fill: shrunk a lot, softened, grown
/// back, which is quick even for big outputs.
fn blurred_pixels(picture: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    use image::imageops::{FilterType, blur, resize};
    let (w, h) = (w.max(1), h.max(1));
    let k = (w.max(h) as f32 / 48.).max(1.);
    let (sw, sh) = (((w as f32 / k).round() as u32).max(2), ((h as f32 / k).round() as u32).max(2));
    let small = image::DynamicImage::ImageRgba8(picture.clone()).resize_to_fill(sw, sh, FilterType::Triangle).to_rgba8();
    let mut out = resize(&blur(&small, 1.6), w, h, FilterType::CatmullRom);
    for p in out.pixels_mut() {
        p.0[3] = 255;
    }
    out
}

/// The picture with rounded corners and a soft shadow on its background,
/// like the studio render does for recordings.
/// `img` on the backdrop: the `inner` part (the picture, in `img` pixels)
/// rounded and shadowed, and `drawn` (the shapes alone) wherever it reaches
/// past that.
fn framed(img: &RgbaImage, drawn: &RgbaImage, inner: (f32, f32, f32, f32), frame: &Frame) -> RgbaImage {
    let ((ow, oh), (left, top)) = frame.layout(img.width() as f32, img.height() as f32);
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let (w, h) = ((inner.2 - inner.0).max(1.), (inner.3 - inner.1).max(1.));
    let radius = w.max(h) * frame.radius;
    let blur = (w.max(h) * frame.padding * 0.45).max(w.max(h) * 0.02);
    let strength = frame.shadow.clamp(0., 1.);
    let clear = frame.backdrop == Backdrop::Transparent;
    let mut out = if clear { RgbaImage::new(ow as u32, oh as u32) } else { backdrop_pixels(&frame.backdrop, img, ow as u32, oh as u32) };
    // Signed distance to a rounded rectangle centered at (cx, cy).
    let sdf = |x: f32, y: f32, cx: f32, cy: f32| {
        let qx = (x - cx).abs() - (w / 2. - radius);
        let qy = (y - cy).abs() - (h / 2. - radius);
        qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - radius
    };
    let (cx, cy) = (left + inner.0 + w / 2., top + inner.1 + h / 2.);
    for (x, y, px) in out.enumerate_pixels_mut() {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        // Premultiplied colour and coverage: an opaque background stays
        // opaque, a transparent one picks up the shadow as dark alpha.
        let mut a = if clear { 0. } else { 1. };
        let mut c = [px.0[0] as f32 / 255. * a, px.0[1] as f32 / 255. * a, px.0[2] as f32 / 255. * a];
        if strength > 0. {
            let d = sdf(fx, fy, cx, cy + blur * 0.35);
            let u = ((d + blur * 0.4) / (blur * 1.4)).clamp(0., 1.);
            let k = u * u * (3. - 2. * u);
            let dark = 0.9 * strength * (1. - k);
            c.iter_mut().for_each(|v| *v *= 1. - dark);
            a = a * (1. - dark) + dark;
        }
        // The picture itself, antialiased along its rounded edge.
        let cover = (0.5 - sdf(fx, fy, cx, cy)).clamp(0., 1.);
        let (ix, iy) = (fx - left, fy - top);
        if ix >= 0. && iy >= 0. && ix < iw && iy < ih {
            let (ix, iy) = (ix as u32, iy as u32);
            // The picture with its drawing inside the rounded edge, the
            // drawing alone outside it.
            let mut over = |p: [u8; 4], k: f32| {
                let k = k * p[3] as f32 / 255.;
                for (v, s) in c.iter_mut().zip(p) {
                    *v = *v * (1. - k) + s as f32 / 255. * k;
                }
                a = a * (1. - k) + k;
            };
            if cover < 1. {
                over(drawn.get_pixel(ix, iy).0, 1.);
            }
            if cover > 0. {
                over(img.get_pixel(ix, iy).0, cover);
            }
        }
        let un = |v: f32| if a > 0. { (v / a * 255.).round().clamp(0., 255.) as u8 } else { 0 };
        *px = image::Rgba([un(c[0]), un(c[1]), un(c[2]), (a * 255.).round() as u8]);
    }
    out
}

/// A small rounded checkerboard for the "transparent" swatch: 3×3 cells,
/// the corner ones rounded (GPUI clips children to rectangles only).
pub(crate) fn checker_tile(side: f32, radius: f32) -> gpui::Div {
    let cell = side / 3.;
    let (light, dark) = (gpui::hsla(0., 0., 0.92, 1.), gpui::hsla(0., 0., 0.72, 1.));
    v_flex().size(px(side)).children((0..3).map(|r| {
        h_flex().children((0..3).map(move |c| {
            let d = div().size(px(cell)).bg(if (r + c) % 2 == 0 { light } else { dark });
            match (r, c) {
                (0, 0) => d.rounded_tl(px(radius)),
                (0, 2) => d.rounded_tr(px(radius)),
                (2, 0) => d.rounded_bl(px(radius)),
                (2, 2) => d.rounded_br(px(radius)),
                _ => d,
            }
        }))
    }))
}

/// Light and dark squares, the usual sign of transparency.
fn checkerboard(cell: f32) -> gpui::Canvas<()> {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (light, dark) = (gpui::hsla(0., 0., 0.92, 1.), gpui::hsla(0., 0., 0.78, 1.));
            window.paint_quad(fill(bounds, light));
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let (cols, rows) = ((w / cell).ceil() as i32, (h / cell).ceil() as i32);
            for r in 0..rows {
                for c in (r % 2..cols).step_by(2) {
                    let x = c as f32 * cell;
                    let y = r as f32 * cell;
                    let cell_bounds = Bounds::new(point(bounds.origin.x + px(x), bounds.origin.y + px(y)), size(px(cell.min(w - x)), px(cell.min(h - y))));
                    window.paint_quad(fill(cell_bounds, dark));
                }
            }
        },
    )
}

fn push_text(shape: &mut Shape, s: &str) {
    if let Kind::Text(t) = &mut shape.kind {
        t.push_str(s);
    }
}

/// Left, top, right, bottom.
fn rect(a: P, b: P) -> (f32, f32, f32, f32) {
    (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
}

/// Points around an ellipse inscribed in the a–b rectangle.
fn ellipse_points(a: P, b: P) -> Vec<P> {
    let (l, t, r, bt) = rect(a, b);
    let (cx, cy, rx, ry) = ((l + r) / 2., (t + bt) / 2., (r - l) / 2., (bt - t) / 2.);
    (0..=72).map(|i| {
        let th = i as f32 / 72. * std::f32::consts::TAU;
        (cx + rx * th.cos(), cy + ry * th.sin())
    })
    .collect()
}

/// Smooth freehand points into a curve: quadratic segments between the
/// midpoints of consecutive points, with each point as the control. Shared by
/// screen and export so both draw the same line.
fn smooth(raw: &[P]) -> Vec<P> {
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
            out.push((
                u * u * start.0 + 2. * u * t * ctrl.0 + t * t * end.0,
                u * u * start.1 + 2. * u * t * ctrl.1 + t * t * end.1,
            ));
        }
    }
    out.push(*points.last().unwrap());
    out
}

/// Where the shaft ends and the head triangle, shared by screen and export.
fn arrow_geometry(a: P, b: P, w: f32) -> (P, [P; 3]) {
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

fn paint_shape(window: &mut Window, shape: &Shape, s: f32, pt: &impl Fn(P) -> Point<gpui::Pixels>) {
    // Outline the stroke the way the export does (round caps and joins) and
    // fill it with the nonzero rule: GPUI's own stroke overlaps itself at sharp
    // turns, which shows as dark knots in translucent marker lines.
    let stroke = |window: &mut Window, points: &[P], closed: bool, color: Hsla, width: f32| {
        let Some((first, rest)) = points.split_first() else { return };
        let mut line = tiny_skia::PathBuilder::new();
        line.move_to(first.0 * s, first.1 * s);
        for p in rest {
            line.line_to(p.0 * s, p.1 * s);
        }
        if rest.is_empty() {
            line.line_to(first.0 * s + 0.01, first.1 * s);
        }
        if closed {
            line.close();
        }
        let style = tiny_skia::Stroke { width: width * s, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        let Some(outline) = line.finish().and_then(|path| path.stroke(&style, 1.)) else { return };
        let at = |p: tiny_skia::Point| pt((p.x / s, p.y / s));
        let mut b = PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::non_zero()));
        for segment in outline.segments() {
            match segment {
                tiny_skia::PathSegment::MoveTo(p) => b.move_to(at(p)),
                tiny_skia::PathSegment::LineTo(p) => b.line_to(at(p)),
                tiny_skia::PathSegment::QuadTo(c, p) => b.curve_to(at(p), at(c)),
                tiny_skia::PathSegment::CubicTo(c1, c2, p) => b.cubic_bezier_to(at(p), at(c1), at(c2)),
                tiny_skia::PathSegment::Close => b.close(),
            }
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
    };
    let (a, b, c, w) = (shape.a, shape.b, shape.color, shape.size);
    match &shape.kind {
        Kind::Rect | Kind::Ellipse if shape.filled => {
            let points = if matches!(shape.kind, Kind::Rect) { vec![a, (b.0, a.1), b, (a.0, b.1)] } else { ellipse_points(a, b) };
            let mut f = PathBuilder::fill();
            if let Some((first, rest)) = points.split_first() {
                f.move_to(pt(*first));
                for p in rest {
                    f.line_to(pt(*p));
                }
                f.close();
            }
            if let Ok(path) = f.build() {
                window.paint_path(path, c);
            }
        }
        Kind::Rect => stroke(window, &[a, (b.0, a.1), b, (a.0, b.1)], true, c, w),
        Kind::Ellipse => stroke(window, &ellipse_points(a, b), true, c, w),
        Kind::Line => stroke(window, &shape.path(), false, c, w),
        Kind::Pen(points) => stroke(window, &smooth(points), false, c, w),
        Kind::Marker(points) => {
            let mut faded = c;
            faded.alpha = MARKER_ALPHA;
            stroke(window, &smooth(points), false, faded, w)
        }
        Kind::Arrow => {
            let (shaft, head) = arrow_parts(&shape.path(), w);
            stroke(window, &shaft, false, c, w);
            let mut f = PathBuilder::fill();
            f.move_to(pt(head[0]));
            f.line_to(pt(head[1]));
            f.line_to(pt(head[2]));
            f.close();
            if let Ok(path) = f.build() {
                window.paint_path(path, c);
            }
        }
        // While dragging, outline the area to be hidden.
        Kind::Blur(_, None) => {
            let mut outline = c;
            outline.alpha = 0.8;
            stroke(window, &[a, (b.0, a.1), b, (a.0, b.1)], true, outline, 1.5 / s);
        }
        Kind::Blur(_, Some(_)) | Kind::Text(_) | Kind::Counter(_) | Kind::Spotlight | Kind::Picture(_) => {}
    }
}

/// How dark a spotlight leaves everything around it.
const SPOT_SHADE: f32 = 0.55;

/// Dim the picture outside every spotlight box, one even-odd path.
fn paint_spotlights(window: &mut Window, spots: &[(P, P)], size: P, pt: &impl Fn(P) -> Point<gpui::Pixels>) {
    if spots.is_empty() {
        return;
    }
    let mut b = PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::even_odd()));
    let mut add = |l: f32, t: f32, r: f32, bt: f32| {
        b.move_to(pt((l, t)));
        b.line_to(pt((r, t)));
        b.line_to(pt((r, bt)));
        b.line_to(pt((l, bt)));
        b.close();
    };
    add(0., 0., size.0, size.1);
    for &(a, c) in spots {
        let (l, t, r, bt) = rect(a, c);
        add(l, t, r, bt);
    }
    if let Ok(path) = b.build() {
        window.paint_path(path, gpui::hsla(0., 0., 0., SPOT_SHADE));
    }
}

/// White or near-black text, whichever reads better on `c`.
fn on_color(c: Hsla) -> Hsla {
    if c.lightness > 0.65 { gpui::hsla(0., 0., 0.08, 1.) } else { gpui::white() }
}

pub(crate) fn font() -> &'static FontRef<'static> {
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| FontRef::try_from_slice(include_bytes!("../data/fonts/IBMPlexSans-SemiBold.ttf")).expect("bundled font"))
}

/// ab_glyph scales by the font's whole height (ascender to descender), GPUI
/// and everyone else by the em; this turns an em size into ab_glyph's scale
/// so text measures and draws the size it shows on screen.
fn em(size: f32) -> PxScale {
    let f = font();
    let height = f.ascent_unscaled() - f.descent_unscaled();
    PxScale::from(size * height / f.units_per_em().unwrap_or(1000.))
}

pub(crate) struct Metrics {
    pub ascent: f32,
    pub descent: f32,
    /// Line box height; text sits centered in it, as in GPUI.
    pub line: f32,
}

pub(crate) fn metrics(size: f32) -> Metrics {
    let f = font().as_scaled(em(size));
    let (ascent, descent) = (f.ascent(), f.descent());
    Metrics { ascent, descent, line: ((ascent - descent) * 1.2).ceil() }
}

/// The hidden a–b area of `src` as PNG bytes, for the canvas.
fn hide_png(src: &RgbaImage, a: P, b: P, how: Hide) -> Option<Vec<u8>> {
    let patch = hide(src, a, b, how)?;
    let mut png = Vec::new();
    patch.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// The a–b area of `src` pixelated or blurred, placed at the area's top
/// left corner clamped to the picture. Screen and export share it.
fn hide(src: &RgbaImage, a: P, b: P, how: Hide) -> Option<RgbaImage> {
    let (l, t, r, bt) = rect(a, b);
    let (x, y) = (l.floor().max(0.) as u32, t.floor().max(0.) as u32);
    let w = (r.ceil() as u32).min(src.width()).saturating_sub(x);
    let h = (bt.ceil() as u32).min(src.height()).saturating_sub(y);
    if w < 2 || h < 2 {
        return None;
    }
    // Blocks and blur scale with the screenshot so text stays unreadable at any size.
    let long = src.width().max(src.height());
    match how {
        Hide::Pixelate => {
            let block = (long / 120).clamp(8, 32);
            let region = image::imageops::crop_imm(src, x, y, w, h).to_image();
            let small = image::imageops::resize(&region, (w / block).max(1), (h / block).max(1), image::imageops::FilterType::Triangle);
            Some(image::imageops::resize(&small, w, h, image::imageops::FilterType::Nearest))
        }
        Hide::Blur => {
            // Blurred with what's around it, so the edges don't sharpen up.
            let sigma = (long as f32 / 140.).clamp(6., 24.);
            let reach = (sigma * 2.).ceil() as u32;
            let (bx, by) = (x.saturating_sub(reach), y.saturating_sub(reach));
            let (bw, bh) = ((x + w + reach).min(src.width()) - bx, (y + h + reach).min(src.height()) - by);
            let region = image::imageops::crop_imm(src, bx, by, bw, bh).to_image();
            let blurred = image::imageops::fast_blur(&region, sigma);
            Some(image::imageops::crop_imm(&blurred, x - bx, y - by, w, h).to_image())
        }
    }
}

/// Draw everything onto the original at full resolution.
fn rasterize(src: &RgbaImage, shapes: &[Shape]) -> Option<RgbaImage> {
    use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

    // Hiding edits the pixels themselves, beneath every other shape.
    let mut base = src.clone();
    for shape in shapes {
        if let Kind::Blur(how, _) = shape.kind
            && let Some(patch) = hide(src, shape.a, shape.b, how)
        {
            let (l, t, _, _) = rect(shape.a, shape.b);
            image::imageops::replace(&mut base, &patch, l.floor().max(0.) as i64, t.floor().max(0.) as i64);
        }
    }
    let spots: Vec<(f32, f32, f32, f32)> = shapes.iter().filter(|sh| matches!(sh.kind, Kind::Spotlight)).map(|sh| rect(sh.a, sh.b)).collect();
    if !spots.is_empty() {
        let keep = 1. - SPOT_SHADE;
        for (x, y, px) in base.enumerate_pixels_mut() {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if !spots.iter().any(|&(l, t, r, b)| fx >= l && fx < r && fy >= t && fy < b) {
                for c in &mut px.0[..3] {
                    *c = (*c as f32 * keep) as u8;
                }
            }
        }
    }
    // Pictures on top of the screenshot, under the drawing.
    for shape in shapes {
        if let Kind::Picture(picture) = &shape.kind {
            let (l, t, r, b) = rect(shape.a, shape.b);
            let (w, h) = ((r - l).round().max(1.) as u32, (b - t).round().max(1.) as u32);
            let scaled = image::imageops::resize(picture.pixels.as_ref(), w, h, image::imageops::FilterType::Lanczos3);
            image::imageops::overlay(&mut base, &scaled, l.round() as i64, t.round() as i64);
        }
    }
    let (w, h) = base.dimensions();
    let mut data = base.into_raw();
    // tiny-skia works on premultiplied pixels.
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = (*c as u32 * a / 255) as u8;
        }
    }
    let mut pm = Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h)?)?;

    let paint_for = |c: Hsla, alpha: f32| {
        let rgb = crate::ui::rgb_of(c);
        let mut p = Paint::default();
        p.set_color_rgba8((rgb.r * 255.) as u8, (rgb.g * 255.) as u8, (rgb.b * 255.) as u8, (alpha * 255.) as u8);
        p.anti_alias = true;
        p
    };
    let stroke_for = |width: f32| Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
    let polyline = |points: &[P], closed: bool| {
        let mut b = PathBuilder::new();
        let (first, rest) = points.split_first()?;
        b.move_to(first.0, first.1);
        for p in rest {
            b.line_to(p.0, p.1);
        }
        if closed {
            b.close();
        }
        b.finish()
    };

    for shape in shapes {
        let (a, b, c, size) = (shape.a, shape.b, shape.color, shape.size);
        let solid = paint_for(c, 1.);
        match &shape.kind {
            Kind::Rect => {
                if let Some(path) = polyline(&[a, (b.0, a.1), b, (a.0, b.1)], true) {
                    if shape.filled {
                        pm.fill_path(&path, &solid, FillRule::Winding, Transform::identity(), None);
                    } else {
                        pm.stroke_path(&path, &solid, &stroke_for(size), Transform::identity(), None);
                    }
                }
            }
            Kind::Ellipse => {
                let (l, t, r, bt) = rect(a, b);
                if let Some(path) = Rect::from_ltrb(l, t, r, bt).and_then(PathBuilder::from_oval) {
                    if shape.filled {
                        pm.fill_path(&path, &solid, FillRule::Winding, Transform::identity(), None);
                    } else {
                        pm.stroke_path(&path, &solid, &stroke_for(size), Transform::identity(), None);
                    }
                }
            }
            Kind::Line => {
                if let Some(path) = polyline(&shape.path(), false) {
                    pm.stroke_path(&path, &solid, &stroke_for(size), Transform::identity(), None);
                }
            }
            Kind::Pen(points) => {
                if let Some(path) = polyline(&smooth(points), false) {
                    pm.stroke_path(&path, &solid, &stroke_for(size), Transform::identity(), None);
                }
            }
            Kind::Marker(points) => {
                if let Some(path) = polyline(&smooth(points), false) {
                    pm.stroke_path(&path, &paint_for(c, MARKER_ALPHA), &stroke_for(size), Transform::identity(), None);
                }
            }
            Kind::Arrow => {
                let (shaft, head) = arrow_parts(&shape.path(), size);
                if let Some(path) = polyline(&shaft, false) {
                    pm.stroke_path(&path, &solid, &stroke_for(size), Transform::identity(), None);
                }
                if let Some(path) = polyline(&head, true) {
                    pm.fill_path(&path, &solid, FillRule::Winding, Transform::identity(), None);
                }
            }
            Kind::Counter(n) => {
                if let Some(path) = PathBuilder::from_circle(a.0, a.1, size / 2.) {
                    pm.fill_path(&path, &solid, FillRule::Winding, Transform::identity(), None);
                }
                let label = n.to_string();
                let fs = size * 0.55;
                let m = metrics(fs);
                let width = text_width(&label, fs);
                draw_line(&mut pm, &label, (a.0 - width / 2., a.1 - (m.ascent - m.descent) / 2. + m.ascent), fs, on_color(c));
            }
            Kind::Text(text) => {
                let m = metrics(size);
                if shape.text == TextStyle::Plate {
                    let (l, t, r, bt) = shape.bounds();
                    let (_, _, radius) = TextStyle::plate(size);
                    if let Some(path) = rounded_rect(l, t, r, bt, radius) {
                        pm.fill_path(&path, &solid, FillRule::Winding, Transform::identity(), None);
                    }
                }
                let ink = if shape.text == TextStyle::Plate { on_color(c) } else { c };
                for (i, line) in text.split('\n').enumerate() {
                    let top = a.1 + i as f32 * m.line;
                    let baseline = top + (m.line - (m.ascent - m.descent)) / 2. + m.ascent;
                    if shape.text == TextStyle::Outline {
                        let o = TextStyle::outline(size);
                        for k in 0..16 {
                            let th = k as f32 / 16. * std::f32::consts::TAU;
                            draw_line(&mut pm, line, (a.0 + o * th.cos(), baseline + o * th.sin()), size, on_color(c));
                        }
                    }
                    draw_line(&mut pm, line, (a.0, baseline), size, ink);
                }
            }
            Kind::Blur(..) | Kind::Spotlight | Kind::Picture(_) => {}
        }
    }
    let mut data = pm.take();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = (*c as u32 * 255 / a).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(w, h, data)
}

fn rounded_rect(l: f32, t: f32, r: f32, b: f32, radius: f32) -> Option<tiny_skia::Path> {
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

pub(crate) fn text_width(text: &str, size: f32) -> f32 {
    let f = font().as_scaled(em(size));
    let mut prev = None;
    let mut w = 0.;
    for ch in text.chars() {
        let id = font().glyph_id(ch);
        if let Some(p) = prev {
            w += f.kern(p, id);
        }
        w += f.h_advance(id);
        prev = Some(id);
    }
    w
}

/// One line of text with its baseline at `origin.1`.
pub(crate) fn draw_line(pm: &mut tiny_skia::Pixmap, text: &str, origin: P, size: f32, color: Hsla) {
    let font = font();
    let scale = em(size);
    let f = font.as_scaled(scale);
    let rgb = crate::ui::rgb_of(color);
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    let mut x = origin.0;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            x += f.kern(p, id);
        }
        let glyph = id.with_scale_and_position(scale, ab_glyph::point(x, origin.1));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bb = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let (px, py) = (bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32);
                if px < 0 || py < 0 || px >= w || py >= h {
                    return;
                }
                let i = ((py * w + px) * 4) as usize;
                let k = coverage.clamp(0., 1.) * rgb.a;
                for (ch, v) in [rgb.r, rgb.g, rgb.b].into_iter().enumerate() {
                    data[i + ch] = (data[i + ch] as f32 * (1. - k) + v * 255. * k) as u8;
                }
                data[i + 3] = (data[i + 3] as f32 * (1. - k) + 255. * k) as u8;
            });
        }
        x += f.h_advance(id);
        prev = Some(id);
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A wobbly mouse trace, raw and smoothed, rendered side by side.
    #[test]
    fn smooth_pen_render() {
        let src = RgbaImage::from_pixel(600, 220, image::Rgba([16, 15, 15, 255]));
        let trace = |y0: f32| -> Vec<P> {
            (0..520).map(|i| {
                let x = 40. + i as f32;
                let jitter = ((i * 7919) % 13) as f32 / 13. - 0.5;
                (x, y0 + (x / 40.).sin() * 25. + jitter * 1.4)
            })
            // Mimic the 2.5 px sampling.
            .step_by(3)
            .collect()
        };
        let red = gpui::hsla(0.01, 0.6, 0.54, 1.);
        let mut shapes = Vec::new();
        // Raw polyline (old behaviour) via a chain of lines, then the smoothed pen.
        let raw = trace(60.);
        for w in raw.windows(2) {
            shapes.push(Shape { kind: Kind::Line, a: w[0], b: w[1], color: red, size: 6., filled: false, text: TextStyle::Plain, bend: None });
        }
        shapes.push(Shape { kind: Kind::Pen(trace(160.)), a: (0., 0.), b: (0., 0.), color: red, size: 6., filled: false, text: TextStyle::Plain, bend: None });
        let png = compose(&src, &Doc { shapes, crop: None, ..Doc::default() }, &Frame::default()).unwrap();
        assert!(!png.is_empty());
        // KADR_TEST_OUT=/path/out.png to look at the result.
        if let Ok(out) = std::env::var("KADR_TEST_OUT") {
            std::fs::write(out, png).unwrap();
        }
    }

    fn red() -> Hsla {
        gpui::hsla(0.01, 0.6, 0.54, 1.)
    }

    #[test]
    fn a_transparent_background_leaves_the_margin_clear() {
        let src = RgbaImage::from_pixel(100, 80, image::Rgba([200, 30, 30, 255]));
        let frame = Frame { backdrop: Backdrop::Transparent, padding: 0.1, radius: 0., shadow: 0., aspect: 0 };
        let out = framed(&src, &RgbaImage::new(100, 80), (0., 0., 100., 80.), &frame);
        assert_eq!(out.get_pixel(0, 0).0[3], 0, "the margin is see-through");
        let (w, h) = out.dimensions();
        assert_eq!(out.get_pixel(w / 2, h / 2).0, [200, 30, 30, 255], "the picture stays as it was");
    }

    #[test]
    fn drawing_past_the_edge_grows_the_result() {
        let src = RgbaImage::from_pixel(200, 100, image::Rgba([200, 200, 200, 255]));
        // A line from inside to 15 px past the right edge (the room is 20).
        let line = Shape { kind: Kind::Line, a: (150., 50.), b: (215., 50.), color: red(), size: 4., filled: false, text: TextStyle::Plain, bend: None };
        let doc = Doc { shapes: vec![line], crop: None, ..Doc::default() };
        let out = image::load_from_memory(&compose(&src, &doc, &Frame::default()).unwrap()).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (217, 100));
        assert!(out.get_pixel(210, 50).0[3] > 200, "the line goes on past the picture");
        assert_eq!(out.get_pixel(210, 10).0[3], 0, "and the new room is clear");
        // A crop reaching past the picture makes it bigger too.
        let doc = Doc { shapes: Vec::new(), crop: Some(((-10., -10.), (210., 100.))), ..Doc::default() };
        let out = image::load_from_memory(&compose(&src, &doc, &Frame::default()).unwrap()).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (220, 110));
        assert_eq!(out.get_pixel(5, 5).0[3], 0);
        assert_eq!(out.get_pixel(15, 15).0, [200, 200, 200, 255]);
        // On a backdrop the line still shows past the rounded picture.
        let frame = Frame { backdrop: Backdrop::Gradient(0), ..Frame::default() };
        let doc = Doc { shapes: doc_line(), crop: None, ..Doc::default() };
        let png = compose(&src, &doc, &frame).unwrap();
        let framed = image::load_from_memory(&png).unwrap().to_rgba8();
        let ((_, _), (left, top)) = frame.layout(217., 100.);
        let p = framed.get_pixel(left as u32 + 210, top as u32 + 50).0;
        assert!(p[0] > 150 && p[1] < 120, "red line on the backdrop: {p:?}");
    }

    fn doc_line() -> Vec<Shape> {
        vec![Shape { kind: Kind::Line, a: (150., 50.), b: (215., 50.), color: red(), size: 4., filled: false, text: TextStyle::Plain, bend: None }]
    }

    #[test]
    fn pictures_go_into_the_export_at_their_box() {
        let src = RgbaImage::from_pixel(100, 80, image::Rgba([255, 255, 255, 255]));
        let mut png = Vec::new();
        RgbaImage::from_pixel(10, 10, image::Rgba([200, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let picture = Picture::decode(&png).unwrap();
        let shape = Shape { kind: Kind::Picture(picture), a: (20., 20.), b: (40., 40.), color: gpui::white(), size: 0., filled: false, text: TextStyle::Plain, bend: None };
        let out = rasterize(&src, &[shape]).unwrap();
        assert_eq!(out.get_pixel(30, 30).0, [200, 0, 0, 255]);
        assert_eq!(out.get_pixel(45, 30).0, [255, 255, 255, 255]);
    }

    #[test]
    fn hiding_pixelates_or_blurs_the_same_on_screen_and_in_the_export() {
        // Fine stripes: text-like detail both ways must wash out.
        let src = RgbaImage::from_fn(400, 300, |x, _| if x % 4 < 2 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255, 255, 255, 255]) });
        let spread = |img: &RgbaImage, (x0, y0, x1, y1): (u32, u32, u32, u32)| {
            let values: Vec<u8> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| img.get_pixel(x, y).0[0]).collect();
            values.iter().max().unwrap() - values.iter().min().unwrap()
        };
        for how in [Hide::Pixelate, Hide::Blur] {
            let patch = hide(&src, (100., 50.), (300., 250.), how).unwrap();
            assert_eq!(patch.dimensions(), (200, 200));
            let shape = Shape { kind: Kind::Blur(how, None), a: (100., 50.), b: (300., 250.), color: red(), size: 2., filled: false, text: TextStyle::Plain, bend: None };
            let out = rasterize(&src, &[shape]).unwrap();
            // The export holds exactly the canvas's patch.
            for (x, y) in [(0, 0), (57, 91), (199, 199)] {
                assert_eq!(out.get_pixel(100 + x, 50 + y), patch.get_pixel(x, y));
            }
            assert_eq!(out.get_pixel(50, 50), src.get_pixel(50, 50), "outside stays sharp");
            assert!(spread(&out, (140, 100, 260, 200)) < 60, "{} hides the stripes", if how == Hide::Blur { "blur" } else { "pixelate" });
        }
        // The blur is smooth: neighbours differ little, unlike blocks.
        let blurred = hide(&src, (100., 50.), (300., 250.), Hide::Blur).unwrap();
        let jumps = (1..200).map(|x| (blurred.get_pixel(x, 100).0[0] as i32 - blurred.get_pixel(x - 1, 100).0[0] as i32).abs()).max().unwrap();
        assert!(jumps < 8, "blur has no steps: {jumps}");
    }

    #[test]
    fn hit_handles_and_move() {
        let mut line = Shape { kind: Kind::Arrow, a: (10., 10.), b: (110., 10.), color: red(), size: 4., filled: false, text: TextStyle::Plain, bend: None };
        assert!(line.hit((60., 13.), 2.));
        assert!(!line.hit((60., 30.), 2.));
        assert_eq!(line.handles(), vec![(10., 10.), (110., 10.), (60., 10.)]);
        // The middle handle bends it: the curve runs through the point.
        line.set_handle(2, (60., 50.));
        assert_eq!(line.bend, Some((60., 50.)));
        assert!(line.hit((60., 50.), 2.) && !line.hit((60., 10.), 2.));
        let (shaft, head) = arrow_parts(&line.path(), line.size);
        assert_eq!(head[0], (110., 10.));
        assert!(shaft.last().unwrap().1 > 10., "the head comes in along the curve, from below");
        // Dragged back onto the line, it's straight again.
        line.set_handle(2, (61., 10.));
        assert_eq!(line.bend, None);
        line.set_handle(1, (110., 60.));
        assert_eq!(line.b, (110., 60.));
        line.translate((5., 5.));
        assert_eq!((line.a, line.b), ((15., 15.), (115., 65.)));

        let mut boxy = Shape { kind: Kind::Rect, a: (0., 0.), b: (50., 40.), color: red(), size: 3., filled: false, text: TextStyle::Plain, bend: None };
        assert!(boxy.hit((25., 20.), 1.), "inside a box selects it");
        boxy.set_handle(1, (80., -10.)); // top-right corner
        assert_eq!((boxy.a, boxy.b), ((0., -10.), (80., 40.)));

        let pen = Shape { kind: Kind::Pen(vec![(0., 0.), (10., 10.), (20., 0.)]), a: (0., 0.), b: (20., 0.), color: red(), size: 2., filled: false, text: TextStyle::Plain, bend: None };
        assert!(pen.hit((15., 5.), 1.));
        assert_eq!(pen.handles().len(), 4, "freehand scales from its corners");
    }

    #[test]
    fn spotlight_dims_around_and_fill_is_solid() {
        let src = RgbaImage::from_pixel(200, 100, image::Rgba([200, 200, 200, 255]));
        let shapes = vec![
            Shape { kind: Kind::Spotlight, a: (10., 10.), b: (60., 60.), color: red(), size: 3., filled: false, text: TextStyle::Plain, bend: None },
            Shape { kind: Kind::Rect, a: (120., 20.), b: (180., 80.), color: red(), size: 3., filled: true, text: TextStyle::Plain, bend: None },
        ];
        let out = image::load_from_memory(&compose(&src, &Doc { shapes, crop: None, ..Doc::default() }, &Frame::default()).unwrap()).unwrap().to_rgba8();
        assert_eq!(out.get_pixel(30, 30).0[0], 200, "inside the spotlight stays");
        assert!(out.get_pixel(90, 90).0[0] < 100, "outside is dimmed");
        let inner = out.get_pixel(150, 50).0;
        assert!(inner[0] > inner[1] + 40, "the box is filled with its colour: {inner:?}");
    }

    #[test]
    fn text_on_a_plate_and_outlined() {
        let src = RgbaImage::from_pixel(300, 120, image::Rgba([240, 240, 240, 255]));
        let text = |a: P, style| Shape { kind: Kind::Text("Hi".into()), a, b: a, color: red(), size: 30., filled: false, text: style, bend: None };
        let plate = text((20., 20.), TextStyle::Plate);
        let (l, t, _, _) = plate.bounds();
        assert!(l < 20. && t < 20., "the plate reaches around the text");
        let shapes = vec![plate, text((160., 20.), TextStyle::Outline)];
        let out = image::load_from_memory(&compose(&src, &Doc { shapes, crop: None, ..Doc::default() }, &Frame::default()).unwrap()).unwrap().to_rgba8();
        let corner = out.get_pixel((l + 3.) as u32, 28).0;
        assert!(corner[0] > corner[1] + 40, "plate is the text colour: {corner:?}");
        if let Ok(path) = std::env::var("KADR_TEST_TEXT") {
            out.save(path).unwrap();
        }
    }

    #[test]
    fn corners_scale_text_and_freehand() {
        let text = Shape { kind: Kind::Text("Hello".into()), a: (100., 100.), b: (100., 100.), color: red(), size: 20., filled: false, text: TextStyle::Plain, bend: None };
        let (l, t, r, b) = text.bounds();
        let mut big = text.clone();
        // Drag the bottom-right corner to twice the box.
        big.scale_from(&text, 2, (l + (r - l) * 2., t + (b - t) * 2.));
        assert!((big.size - 40.).abs() < 0.5, "font doubles: {}", big.size);
        assert_eq!(big.a, (100., 100.), "top-left stays");

        let pen = Shape { kind: Kind::Pen(vec![(0., 0.), (10., 20.)]), a: (0., 0.), b: (10., 20.), color: red(), size: 2., filled: false, text: TextStyle::Plain, bend: None };
        let (pl, pt, pr, pb) = pen.bounds();
        let mut wide = pen.clone();
        wide.scale_from(&pen, 2, (pl + (pr - pl) * 3., pb));
        let (wl, _, wr, wb) = wide.bounds();
        // The stroke's own width doesn't scale, so the box grows a little less.
        let ratio = (wr - wl) / (pr - pl);
        assert!(ratio > 2.6 && ratio <= 3. && (wb - pb).abs() < 0.5, "freehand stretches one way: {ratio}");
        let _ = pt;
    }

    #[test]
    fn background_round_trips_through_json() {
        let frame = Frame { backdrop: Backdrop::Gradient(2), padding: 0.03, radius: 0.02, shadow: 0.7, aspect: 3 };
        let back = Frame::from_json(&frame.to_json());
        assert!(back.backdrop == frame.backdrop && back.padding == frame.padding && back.radius == frame.radius && back.shadow == frame.shadow && back.aspect == frame.aspect);
        assert!(!Frame::from_json(&Frame::default().to_json()).on());
    }

    #[test]
    fn crop_and_backdrop_sizes() {
        let src = RgbaImage::from_pixel(400, 300, image::Rgba([200, 200, 200, 255]));
        let shapes = vec![Shape { kind: Kind::Rect, a: (60., 60.), b: (220., 180.), color: red(), size: 6., filled: false, text: TextStyle::Plain, bend: None }];
        let doc = Doc { shapes, crop: Some(((50., 40.), (250., 200.))), ..Doc::default() };
        let plain = image::load_from_memory(&compose(&src, &doc, &Frame::default()).unwrap()).unwrap();
        assert_eq!((plain.width(), plain.height()), (200, 160));
        let frame = Frame { backdrop: Backdrop::Gradient(0), ..Frame::default() };
        let png = compose(&src, &doc, &frame).unwrap();
        let framed = image::load_from_memory(&png).unwrap();
        // 6% of the longer side on each edge.
        assert_eq!((framed.width(), framed.height()), (224, 184));
        // A square output grows the short side.
        let square = Frame { aspect: 3, ..frame.clone() };
        let sq = image::load_from_memory(&compose(&src, &doc, &square).unwrap()).unwrap();
        assert_eq!((sq.width(), sq.height()), (224, 224));
        if let Ok(out) = std::env::var("KADR_TEST_FRAME") {
            let big = Doc { shapes: doc.shapes.clone(), crop: None, ..Doc::default() };
            std::fs::write(out, compose(&src, &big, &frame).unwrap()).unwrap();
        }
    }
}

impl crate::presets::HasPresets for Editor {
    fn presets(&mut self) -> &mut crate::presets::Presets {
        &mut self.presets
    }

    fn current_look(&self) -> serde_json::Value {
        self.frame.to_json()
    }

    fn apply_look(&mut self, look: &serde_json::Value, cx: &mut Context<Self>) {
        self.frame = Frame::from_json(look);
        cx.notify();
    }
}
