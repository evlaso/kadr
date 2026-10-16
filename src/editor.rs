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
    Window, canvas, div, fill, img, layer_shell::*, linear_color_stop, linear_gradient, outline,
    point, px, rgba, size,
};
use image::RgbaImage;
use ui::{ButtonLike, Divider, IconButtonShape, Switch, ToggleState, Tooltip, prelude::*};

use crate::kit::{self, LayerSpec};
use crate::{cards, ocr, palette, studio_editor, util};

/// A point in image pixels.
type P = (f32, f32);

/// Stroke widths, font sizes and counter sizes, in screen pixels.
const WIDTHS: [f32; 3] = [3., 5., 9.];
const FONT_SIZES: [f32; 3] = [16., 22., 32.];
const COUNTER_SIZES: [f32; 3] = [22., 28., 36.];
/// Opacity of the highlighter.
const MARKER_ALPHA: f32 = 0.4;
/// Backdrop margin, as a fraction of the picture's longer side.
const FRAME_PAD: f32 = 0.06;
/// Grab distance for shapes and handles, in screen pixels.
const GRAB: f32 = 7.;
/// Width of the background panel.
const PANEL: f32 = 300. * kit::SCALE;
/// Slider ranges, as fractions of the picture's longer side.
const MAX_PAD: f32 = 0.2;
const MAX_ROUND: f32 = 0.05;
/// Output shapes for the framed picture.
const ASPECTS: [(&str, Option<f32>); 5] = [("Авто", None), ("16:9", Some(16. / 9.)), ("4:3", Some(4. / 3.)), ("1:1", Some(1.)), ("9:16", Some(9. / 16.))];

#[derive(Clone, PartialEq)]
enum Backdrop {
    None,
    /// Index into `palette::backdrops()`.
    Gradient(usize),
    Wallpaper(PathBuf),
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
        Tool::Rect,
        Tool::Ellipse,
        Tool::Line,
        Tool::Pen,
        Tool::Marker,
        Tool::Text,
        Tool::Counter,
        Tool::Blur,
        Tool::Spotlight,
        Tool::Crop,
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
            Tool::Select => "Выделить и двигать",
            Tool::Arrow => "Стрелка",
            Tool::Rect => "Прямоугольник",
            Tool::Ellipse => "Эллипс",
            Tool::Line => "Линия",
            Tool::Pen => "Карандаш",
            Tool::Marker => "Маркер",
            Tool::Text => "Текст",
            Tool::Counter => "Нумерация шагов",
            Tool::Blur => "Скрыть (пикселизация)",
            Tool::Spotlight => "Прожектор: затемнить всё вокруг",
            Tool::Crop => "Обрезать",
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
    /// The pixelated patch, once computed for the current position.
    Blur(Option<Arc<Image>>),
    /// Everything outside the a–b box is dimmed.
    Spotlight,
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
                let (l, t, r, b) = rect(self.a, self.b);
                let w = self.size / 2.;
                (l - w, t - w, r + w, b + w)
            }
            Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight => rect(self.a, self.b),
        }
    }

    fn hit(&self, p: P, tol: f32) -> bool {
        let reach = tol + self.size / 2.;
        match &self.kind {
            Kind::Line | Kind::Arrow => segment_distance(p, self.a, self.b) <= reach,
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
            Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight => vec![a, (b.0, a.1), b, (a.0, b.1), (mid.0, a.1), (b.0, mid.1), (mid.0, b.1), (a.0, mid.1)],
            Kind::Line | Kind::Arrow => vec![a, b],
            // Text, counters and freehand scale from their box's corners.
            _ => {
                let (l, t, r, bt) = self.bounds();
                vec![(l, t), (r, t), (r, bt), (l, bt)]
            }
        }
    }

    /// Shapes whose corner handles scale them rather than move a point.
    fn scales(&self) -> bool {
        matches!(self.kind, Kind::Text(_) | Kind::Counter(_) | Kind::Pen(_) | Kind::Marker(_))
    }

    /// Scale `orig` by dragging corner `handle` to `p`, the opposite corner
    /// staying put: text and counters keep their proportions (the size grows
    /// with them), freehand stretches each way.
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
            (Kind::Line | Kind::Arrow, 0) | (Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight, 0) => self.a = p,
            (Kind::Line | Kind::Arrow, 1) | (Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight, 2) => self.b = p,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight, 1) => (self.b.0, self.a.1) = p,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight, 3) => (self.a.0, self.b.1) = p,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight, 4) => self.a.1 = p.1,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight, 5) => self.b.0 = p.0,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight, 6) => self.b.1 = p.1,
            (Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight, 7) => self.a.0 = p.0,
            _ => {}
        }
        self.stale_patch();
    }

    fn translate(&mut self, d: P) {
        let shift = |p: &mut P| *p = (p.0 + d.0, p.1 + d.1);
        shift(&mut self.a);
        shift(&mut self.b);
        if let Kind::Pen(points) | Kind::Marker(points) = &mut self.kind {
            points.iter_mut().for_each(shift);
        }
        self.stale_patch();
    }

    /// A moved pixelation shows its outline until the new patch is ready.
    fn stale_patch(&mut self) {
        if let Kind::Blur(patch) = &mut self.kind {
            *patch = None;
        }
    }

    /// Apply the toolbar's weight to an existing shape.
    fn set_weight(&mut self, weight: usize, s: f32) {
        self.size = match self.kind {
            Kind::Text(_) => FONT_SIZES[weight],
            Kind::Counter(_) => COUNTER_SIZES[weight],
            Kind::Marker(_) => WIDTHS[weight] * 3.,
            Kind::Blur(_) | Kind::Spotlight => return,
            _ => WIDTHS[weight],
        } / s;
    }
}

/// Everything undo/redo restores.
#[derive(Clone)]
struct Doc {
    shapes: Vec<Shape>,
    crop: Option<(P, P)>,
}

enum Drag {
    Draw,
    Move { index: usize, last: P, moved: bool },
    Handle { index: usize, handle: usize },
    Scale { index: usize, handle: usize, orig: Shape },
    Crop { start: P },
    Slider(Knob),
}

enum Export {
    Copy,
    Save,
}

pub struct Editor {
    path: PathBuf,
    image: Arc<Image>,
    px: (u32, u32),
    /// Decoded original, for pixelation and the final render.
    source: Option<Arc<RgbaImage>>,
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
    weight: usize,
    /// Boxes and ellipses are drawn solid.
    fill: bool,
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
    walls: Vec<(PathBuf, Arc<Image>)>,
    /// The picture's largest on-screen size; `disp` shrinks from it to make
    /// room for the background.
    natural: (f32, f32),
    /// Slider tracks (x, width) in window coordinates.
    sliders: Rc<RefCell<HashMap<Knob, (f32, f32)>>>,
    /// Picture origin in window coordinates, updated every frame.
    origin: Rc<Cell<(f32, f32)>>,
    focus: FocusHandle,
}

/// Open a screenshot for annotation.
pub fn open(path: PathBuf, cx: &mut App) {
    let Ok(bytes) = std::fs::read(&path) else {
        return util::notify("Не удалось открыть картинку", &path.display().to_string());
    };
    let Some((w, h)) = cards::png_size(&bytes) else {
        return util::notify("Редактор открывает только PNG", &path.display().to_string());
    };
    let Some(mon) = util::focused_monitor() else { return };
    let (mw, mh) = (mon.w as f32, mon.h as f32);

    // Real size on screen (screenshots are in physical pixels), shrunk to
    // leave room for the toolbars and a backdrop.
    let natural = (w as f32 / mon.scale as f32, h as f32 / mon.scale as f32);
    let fit = ((mw - 160.) / natural.0).min((mh - 260. * kit::SCALE) / natural.1).min(1.0);
    let disp = (natural.0 * fit, natural.1 * fit);

    let image = Arc::new(Image::from_bytes(ImageFormat::Png, bytes.clone()));
    let spec = LayerSpec {
        namespace: "kadr-editor",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mw, mh),
        keyboard: KeyboardInteractivity::Exclusive,
        ignore_zones: true,
        monitor: Some(mon),
    };
    kit::open_layer(cx, spec, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move { image::load_from_memory(&bytes).ok().map(|i| Arc::new(i.to_rgba8())) })
                    .await;
                let _ = this.update(cx, |editor: &mut Editor, cx| {
                    editor.source = decoded;
                    cx.notify();
                });
                let walls = studio_editor::wallpaper_thumbs().await;
                let _ = this.update(cx, |editor: &mut Editor, cx| {
                    editor.walls = walls;
                    cx.notify();
                });
            })
            .detach();
            Editor {
                path,
                image,
                px: (w, h),
                source: None,
                disp,
                doc: Doc { shapes: Vec::new(), crop: None },
                history: Vec::new(),
                future: Vec::new(),
                draft: None,
                drag: None,
                selected: None,
                editing: None,
                tool: Tool::Arrow,
                colors: palette::annotation_colors(),
                color: 0,
                weight: 1,
                fill: false,
                hover: None,
                text_style: TextStyle::Plain,
                zoom: 1.,
                real: natural,
                frame: Frame::default(),
                panel: false,
                walls: Vec::new(),
                natural: disp,
                sliders: Rc::new(RefCell::new(HashMap::new())),
                origin: Rc::new(Cell::new((0., 0.))),
                focus,
            }
        })
    });
}

impl Editor {
    /// Screen pixels per image pixel.
    fn scale(&self) -> f32 {
        self.disp.0 / self.px.0 as f32
    }

    fn to_image(&self, pos: Point<gpui::Pixels>, clamp: bool) -> Option<P> {
        let (ox, oy) = self.origin.get();
        let s = self.scale();
        let (x, y) = ((f32::from(pos.x) - ox) / s, (f32::from(pos.y) - oy) / s);
        let (w, h) = (self.px.0 as f32, self.px.1 as f32);
        if clamp {
            Some((x.clamp(0., w), y.clamp(0., h)))
        } else {
            (x >= 0. && y >= 0. && x <= w && y <= h).then_some((x, y))
        }
    }

    fn current_color(&self) -> Hsla {
        self.colors[self.color]
    }

    /// Remember the current state for undo, before changing it.
    fn checkpoint(&mut self) {
        self.history.push(self.doc.clone());
        self.future.clear();
    }

    fn undo(&mut self) {
        self.finish_text();
        if let Some(doc) = self.history.pop() {
            self.future.push(std::mem::replace(&mut self.doc, doc));
            self.selected = None;
        }
    }

    fn redo(&mut self) {
        if let Some(doc) = self.future.pop() {
            self.history.push(std::mem::replace(&mut self.doc, doc));
            self.selected = None;
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
        let tol = (GRAB + 2.) / self.scale();
        let h = self.doc.shapes[i].handles().iter().position(|h| (h.0 - p.0).hypot(h.1 - p.1) <= tol)?;
        Some((i, h))
    }

    /// Compute the pixelation patch for a blur shape in the background.
    fn refresh_patch(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(src) = self.source.clone() else { return };
        let Some(shape) = self.doc.shapes.get(index).filter(|s| matches!(s.kind, Kind::Blur(_))) else { return };
        let (a, b) = (shape.a, shape.b);
        cx.spawn(async move |this, cx| {
            let patch = cx.background_executor().spawn(async move { pixelate_png(&src, a, b) }).await;
            let _ = this.update(cx, |editor, cx| {
                // Only if the shape is still where the patch was cut from.
                if let Some(Shape { kind: Kind::Blur(slot), a: sa, b: sb, .. }) = editor.doc.shapes.get_mut(index)
                    && (*sa, *sb) == (a, b)
                {
                    *slot = patch.map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
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
                self.doc.shapes.push(Shape { kind: Kind::Text(String::new()), a: p, b: p, color, size: FONT_SIZES[self.weight] / s, filled: false, text: self.text_style });
                self.editing = Some(self.doc.shapes.len() - 1);
                self.selected = self.editing;
                return cx.notify();
            }
            Tool::Counter => {
                let n = self.doc.shapes.iter().filter(|s| matches!(s.kind, Kind::Counter(_))).count() as u32 + 1;
                self.checkpoint();
                self.doc.shapes.push(Shape { kind: Kind::Counter(n), a: p, b: p, color, size: COUNTER_SIZES[self.weight] / s, filled: false, text: TextStyle::Plain });
                self.selected = Some(self.doc.shapes.len() - 1);
                return cx.notify();
            }
            Tool::Arrow => Kind::Arrow,
            Tool::Rect => Kind::Rect,
            Tool::Ellipse => Kind::Ellipse,
            Tool::Line => Kind::Line,
            Tool::Pen => Kind::Pen(vec![p]),
            Tool::Marker => Kind::Marker(vec![p]),
            Tool::Blur => Kind::Blur(None),
            Tool::Spotlight => Kind::Spotlight,
        };
        let width = if self.tool == Tool::Marker { WIDTHS[self.weight] * 3. } else { WIDTHS[self.weight] };
        let filled = self.fill && matches!(kind, Kind::Rect | Kind::Ellipse);
        self.draft = Some(Shape { kind, a: p, b: p, color, size: width / s, filled, text: TextStyle::Plain });
        self.drag = Some(Drag::Draw);
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        if e.pressed_button != Some(MouseButton::Left) {
            // Show what a press would do: resize at a knob, move over a shape.
            let hover = self.to_image(e.position, false).and_then(|p| {
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
            Some(Drag::Crop { start }) => self.doc.crop = Some((*start, p)),
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
            Some(Drag::Crop { .. }) => {
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
            // Esc steps back: selection first, then the editor.
            ("escape", _, _) if self.selected.is_some() => self.selected = None,
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
            ("z", true, false) => self.undo(),
            ("z", true, true) | ("y", true, _) => self.redo(),
            ("c", true, _) => self.export(Export::Copy, window, cx),
            ("=" | "+", true, _) => self.zoom_by(1.25),
            ("-", true, _) => self.zoom_by(0.8),
            ("0", true, _) => self.zoom = 1.,
            ("s", true, _) => self.export(Export::Save, window, cx),
            (key, false, false) => {
                if let Some(tool) = Tool::ALL.into_iter().find(|t| t.key() == key) {
                    self.tool = tool;
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

    fn pick_weight(&mut self, i: usize) {
        self.weight = i;
        if let Some(index) = self.selected {
            self.checkpoint();
            let s = self.scale();
            self.doc.shapes[index].set_weight(i, s);
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
        }
        if !self.frame.on() {
            self.frame.backdrop = Backdrop::Gradient(0);
        }
        cx.notify();
    }

    fn pick_wallpaper(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Выбрать картинку для фона".into()) });
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
            return util::notify("kadr", "Картинка ещё загружается, попробуй через секунду");
        };
        let doc = self.doc.clone();
        let frame = self.frame.clone();
        let path = self.path.clone();
        window.remove_window();
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let png = cx.background_executor().spawn(async move { compose(&src, &doc, &frame) }).await;
            let Some(png) = png else {
                return util::notify("Не удалось сохранить", &path.display().to_string());
            };
            match how {
                Export::Save => {
                    if let Err(e) = std::fs::write(&path, png) {
                        return util::notify("Не удалось сохранить", &e.to_string());
                    }
                    util::copy_image(&path);
                    let _ = cx.update(|cx| cards::show(path, cx));
                }
                Export::Copy => {
                    let tmp = util::runtime_dir().join("kadr-annotated.png");
                    if std::fs::write(&tmp, png).is_ok() {
                        util::copy_image(&tmp);
                    }
                }
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
            let doc = Doc { shapes: Vec::new(), crop };
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
    fn render_toolbar(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let tool_button = |tool: Tool, cx: &mut Context<Self>| {
            ButtonLike::new(SharedString::from(format!("tool-{}", tool.key())))
                .style(if self.tool == tool { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                .size(ButtonSize::Large)
                .toggle_state(self.tool == tool)
                .child(div().px_1().child(tool.icon().size(IconSize::Small)))
                .tooltip(Tooltip::text(format!("{} ({})", tool.label(), tool.key().to_uppercase())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.finish_text();
                    this.tool = tool;
                    cx.notify();
                }))
        };
        let drawing: Vec<_> = Tool::ALL[1..11].iter().map(|&t| tool_button(t, cx)).collect();
        let select = tool_button(Tool::Select, cx);
        let crop = tool_button(Tool::Crop, cx);

        let ring = cx.theme().colors().text;
        let swatches = self.colors.iter().enumerate().map(|(i, &c)| {
            let selected = i == self.color;
            div()
                .id(("color", i))
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_2()
                .border_color(if selected { ring } else { gpui::transparent_black() })
                .cursor_pointer()
                .child(div().size(px(16.)).rounded_full().bg(c))
                .tooltip(Tooltip::text(format!("Цвет ({})", i + 1)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pick_color(i);
                    cx.notify();
                }))
        });
        let dot = cx.theme().colors().text;
        let weights = WIDTHS.iter().enumerate().map(|(i, w)| {
            ButtonLike::new(("weight", i))
                .style(if i == self.weight { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                .size(ButtonSize::Large)
                .toggle_state(i == self.weight)
                .child(div().w(px(18.)).flex().justify_center().child(div().size(px(w + 1.)).rounded_full().bg(dot)))
                .tooltip(Tooltip::text(["Тонко", "Средне", "Толсто"][i]))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pick_weight(i);
                    cx.notify();
                }))
        });

        let history = h_flex()
            .gap_0p5()
            .child(
                IconButton::new("undo", IconName::Undo)
                    .shape(IconButtonShape::Square)
                    .size(ButtonSize::Large)
                    .disabled(self.history.is_empty())
                    .tooltip(Tooltip::text("Отменить (Ctrl+Z)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.undo();
                        cx.notify();
                    })),
            )
            .child(
                ButtonLike::new("redo")
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Large)
                    .disabled(self.future.is_empty())
                    .child(div().px_1().child(Icon::from_path("icons/kadr/redo.svg").size(IconSize::Small).color(if self.future.is_empty() { Color::Disabled } else { Color::Default })))
                    .tooltip(Tooltip::text("Повторить (Ctrl+Shift+Z)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.redo();
                        cx.notify();
                    })),
            );
        let tools = h_flex()
            .gap_0p5()
            .child(select)
            .child(Divider::vertical().h_5().mx_1())
            .children(drawing)
            .child(Divider::vertical().h_5().mx_1())
            .child(crop);
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
                .child(option("outline", false, "icons/kadr/rect.svg", "Контур"))
                .child(option("solid", true, "icons/kadr/fill.svg", "Заливка"))
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
                .child(option("text-plain", TextStyle::Plain, "Обычный текст"))
                .child(option("text-plate", TextStyle::Plate, "На плашке"))
                .child(option("text-outline", TextStyle::Outline, "С обводкой"))
                .child(Divider::vertical().h_5().mx_1())
        });
        let style = h_flex().gap_0p5().children(text_options).children(fill_toggle).child(h_flex().gap_0p5().children(swatches)).child(Divider::vertical().h_5().mx_1()).children(weights);

        h_flex()
            .w(px(width))
            .justify_between()
            .gap_2()
            .px_1p5()
            .py_1()
            .bg(cx.theme().colors().title_bar_background)
            // GPUI clips children to rectangles: the bars round their own
            // outer corners to stay inside the window's.
            .rounded_t(px(kit::RADIUS - 1.))
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .child(history)
            .child(tools)
            .child(style)
    }

    /// CleanShot-like bottom bar: backdrop on the left, the file in the
    /// middle, and drag-out, copy and save on the right.
    fn render_footer(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // Drag the annotated picture out, rendered at the moment the drag starts.
        let drag_state = self.source.clone().map(|src| (src, self.doc.clone(), self.frame.clone(), self.path.clone()));
        let drag_out = div()
            .id("drag-out")
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_dashed()
            .border_color(cx.theme().colors().border)
            .cursor_grab()
            .child(Icon::new(IconName::Image).size(IconSize::Small).color(Color::Muted))
            .child(Label::new("Перетащи").size(LabelSize::Small).color(Color::Muted))
            .tooltip(Tooltip::text("Утащи картинку с пометками в чат, браузер или папку"))
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

        h_flex()
            .w(px(width))
            .justify_between()
            .gap_2()
            .pl_2p5()
            .pr_1p5()
            .py_1()
            .bg(cx.theme().colors().status_bar_background)
            .rounded_b(px(kit::RADIUS - 1.))
            .border_t_1()
            .border_color(cx.theme().colors().border_variant)
            .child(
                h_flex()
                    .flex_1()
                    .gap_1()
                    .child(
                        ButtonLike::new("background")
                            .style(if self.panel { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                            .toggle_state(self.panel)
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .px_1()
                                    .child(Icon::from_path("icons/kadr/frame.svg").size(IconSize::Small).color(if self.frame.on() { Color::Accent } else { Color::Muted }))
                                    .child(Label::new("Фон").size(LabelSize::Small)),
                            )
                            .tooltip(Tooltip::text("Фон, отступы, скругление, тень и формат"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.panel = !this.panel;
                                cx.notify();
                            })),
                    )
                    .when(self.doc.crop.is_some(), |row| {
                        row.child(
                            Button::new("uncrop", "Без обрезки").style(ButtonStyle::Subtle).color(Color::Muted).on_click(cx.listener(|this, _, _, cx| {
                                this.checkpoint();
                                this.doc.crop = None;
                                cx.notify();
                            })),
                        )
                    }),
            )
            .child(
                h_flex()
                    .flex_none()
                    .max_w(px(width * 0.3))
                    .gap_2()
                    .child(div().min_w_0().child(Label::new(name).size(LabelSize::Small).color(Color::Muted).truncate()))
                    .child(Label::new(self.size_label()).size(LabelSize::Small).color(Color::Muted))
                    .child(
                        h_flex()
                            .child(
                                IconButton::new("zoom-out", IconName::Dash)
                                    .shape(IconButtonShape::Square)
                                    .icon_size(IconSize::XSmall)
                                    .tooltip(Tooltip::text("Отдалить (Ctrl+−)"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.zoom_by(0.8);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("zoom-fit", format!("{}%", (self.disp.0 / self.real.0 * 100.).round()))
                                    .style(ButtonStyle::Subtle)
                                    .label_size(LabelSize::Small)
                                    .tooltip(Tooltip::text("По размеру окна (Ctrl+0)"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.zoom = 1.;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                IconButton::new("zoom-in", IconName::Plus)
                                    .shape(IconButtonShape::Square)
                                    .icon_size(IconSize::XSmall)
                                    .tooltip(Tooltip::text("Приблизить (Ctrl+= или Ctrl+колесо)"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.zoom_by(1.25);
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .justify_end()
                    .gap_1()
                    .child(
                        IconButton::new("ocr", IconName::CaseSensitive)
                            .shape(IconButtonShape::Square)
                            .tooltip(Tooltip::text("Распознать текст"))
                            .on_click(cx.listener(|this, _, _, cx| this.read_text(cx))),
                    )
                    .child(drag_out)
                    .child(Divider::vertical().h_5().mx_1())
                    .child(
                        Button::new("cancel", "Отмена")
                            .style(ButtonStyle::Subtle)
                            .color(Color::Muted)
                            .key_binding(kit::keys("escape"))
                            .on_click(|_, window, _| window.remove_window()),
                    )
                    .child(
                        Button::new("copy", "Копировать")
                            .style(ButtonStyle::Subtle)
                            .key_binding(kit::keys("ctrl-c"))
                            .on_click(cx.listener(|this, _, window, cx| this.export(Export::Copy, window, cx))),
                    )
                    .child(
                        Button::new("save", "Сохранить")
                            .style(ButtonStyle::Filled)
                            .key_binding(kit::keys("ctrl-s"))
                            .on_click(cx.listener(|this, _, window, cx| this.export(Export::Save, window, cx))),
                    ),
            )
    }

    fn slider(&self, knob: Knob, value: f32, label: String, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let (track, filled, knob_color) = (colors.element_background, colors.text_accent, colors.text);
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
                window.paint_quad(gpui::quad(Bounds::new(point(x0, mid - px(2.)), size(w, px(4.))), px(2.), track, px(0.), track, BorderStyle::Solid));
                window.paint_quad(gpui::quad(Bounds::new(point(x0, mid - px(2.)), size(w * v, px(4.))), px(2.), filled, px(0.), filled, BorderStyle::Solid));
                let c = x0 + w * v;
                window.paint_quad(gpui::quad(Bounds::new(point(c - px(8.), mid - px(8.)), size(px(16.), px(16.))), px(8.), knob_color, px(0.), knob_color, BorderStyle::Solid));
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
                    this.drag = Some(Drag::Slider(knob));
                    this.slide(knob, f32::from(e.position.x), cx);
                }),
            ))
            .child(div().w(px(44.)).flex().justify_end().child(Label::new(label).size(LabelSize::Small).color(Color::Muted)))
            .into_any_element()
    }

    /// The background tool, like CleanShot's: what's behind the picture, its
    /// margins, corners, shadow and the output's shape.
    fn render_panel(&self, height: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let line = cx.theme().colors().border_variant;
        let ring = cx.theme().colors().text;
        let section = |title: &'static str| v_flex().w_full().gap_2().pb_4().border_b_1().border_color(line).child(Label::new(title).size(LabelSize::Small).color(Color::Muted));
        let caption = |text: &'static str| Label::new(text).size(LabelSize::Small);
        let chip = |id: SharedString, label: &str, on: bool| {
            ButtonLike::new(id).style(if on { ButtonStyle::Filled } else { ButtonStyle::Subtle }).toggle_state(on).child(div().px_1p5().child(Label::new(label.to_owned()).size(LabelSize::Small)))
        };
        let swatch = |id: SharedString, selected: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .border_2()
                .border_color(if selected { ring } else { gpui::transparent_black() })
                .cursor_pointer()
        };
        let gradients = self
            .backdrops()
            .into_iter()
            .enumerate()
            .map(|(i, (c0, c1))| {
                swatch(SharedString::from(format!("gradient-{i}")), self.frame.backdrop == Backdrop::Gradient(i))
                    .size(px(40.))
                    .child(div().size(px(30.)).rounded(px(7.)).bg(linear_gradient(135., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.frame.backdrop = Backdrop::Gradient(i);
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        let walls = self
            .walls
            .iter()
            .enumerate()
            .map(|(i, (path, thumb))| {
                let path = path.clone();
                swatch(SharedString::from(format!("wall-{i}")), self.frame.backdrop == Backdrop::Wallpaper(path.clone()))
                    .w(px(64.))
                    .h(px(44.))
                    .child(img(thumb.clone()).w(px(56.)).h(px(36.)).rounded(px(6.)).object_fit(ObjectFit::Cover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.frame.backdrop = Backdrop::Wallpaper(path.clone());
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();
        let aspects = ASPECTS
            .iter()
            .enumerate()
            .map(|(i, (label, _))| {
                chip(SharedString::from(format!("aspect-{i}")), label, self.frame.aspect == i).on_click(cx.listener(move |this, _, _, cx| {
                    this.frame.aspect = i;
                    if !this.frame.on() {
                        this.frame.backdrop = Backdrop::Gradient(0);
                    }
                    cx.notify();
                }))
            })
            .collect::<Vec<_>>();
        let padding = self.slider(Knob::Padding, self.frame.padding / MAX_PAD, format!("{}%", (self.frame.padding * 100.).round()), cx);
        let radius = self.slider(Knob::Radius, self.frame.radius / MAX_ROUND, format!("{}", (self.frame.radius / MAX_ROUND * 100.).round()), cx);
        let shadow = self.slider(Knob::Shadow, self.frame.shadow, format!("{}%", (self.frame.shadow * 100.).round()), cx);
        let size = self.source.as_ref().map(|src| {
            let (w, h) = match self.doc.crop {
                Some((a, b)) => {
                    let (l, t, r, bt) = rect(a, b);
                    (r - l, bt - t)
                }
                None => (src.width() as f32, src.height() as f32),
            };
            let ((ow, oh), _) = self.frame.layout(w.round(), h.round());
            format!("Итог: {}×{}", ow, oh)
        });

        v_flex()
            .id("background-panel")
            .flex_none()
            .w(px(PANEL))
            .h(px(height))
            .overflow_y_scroll()
            .gap_4()
            .p_4()
            .border_l_1()
            .border_color(line)
            .bg(cx.theme().colors().panel_background)
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(Label::new("Фон"))
                    .child(Switch::new("frame-on", ToggleState::from(self.frame.on())).on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                        this.frame.backdrop = if *state == ToggleState::Selected { Backdrop::Gradient(0) } else { Backdrop::None };
                        cx.notify();
                    }))),
            )
            .child(
                section("Подложка")
                    .child(h_flex().flex_wrap().gap_1().children(gradients))
                    .child(h_flex().flex_wrap().gap_1().children(walls))
                    .child(
                        Button::new("own-wallpaper", "Своя картинка…")
                            .style(ButtonStyle::Subtle)
                            .start_icon(Icon::new(IconName::Image).size(IconSize::Small))
                            .on_click(cx.listener(|this, _, _, cx| this.pick_wallpaper(cx))),
                    ),
            )
            .child(
                section("Оформление")
                    .child(caption("Отступ"))
                    .child(padding)
                    .child(caption("Скругление углов"))
                    .child(radius)
                    .child(caption("Тень"))
                    .child(shadow),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(Label::new("Формат").size(LabelSize::Small).color(Color::Muted))
                    .child(h_flex().flex_wrap().gap_1().children(aspects))
                    .children(size.filter(|_| self.frame.on()).map(|s| Label::new(s).size(LabelSize::XSmall).color(Color::Muted))),
            )
    }

    /// Output size after cropping, as shown in the footer.
    fn size_label(&self) -> String {
        match self.doc.crop {
            Some((a, b)) => {
                let (l, t, r, bt) = rect(a, b);
                format!("{}×{} (обрезано)", (r - l).round(), (bt - t).round())
            }
            None => format!("{}×{}", self.px.0, self.px.1),
        }
    }

    /// Picture with blur patches, vector shapes, text and counters, then the
    /// crop shade and the selection on top.
    fn render_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.scale();
        let at = |p: P| (px(p.0 * s), px(p.1 * s));

        let patches = self.doc.shapes.iter().filter_map(|shape| match &shape.kind {
            Kind::Blur(Some(patch)) => {
                let (l, t, r, b) = rect(shape.a, shape.b);
                Some(img(patch.clone()).absolute().left(px(l * s)).top(px(t * s)).w(px((r - l) * s)).h(px((b - t) * s)).object_fit(ObjectFit::Fill))
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
                let pt = |p: P| point(o.x + px(p.0 * s), o.y + px(p.1 * s));
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
                let base = div().absolute().text_size(px(shape.size * s)).font_weight(gpui::FontWeight::SEMIBOLD).whitespace_nowrap();
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
                        .left(px(shape.a.0 * s - d / 2.))
                        .top(px(shape.a.1 * s - d / 2.))
                        .size(px(d))
                        .rounded_full()
                        .bg(shape.color)
                        .flex()
                        .items_center()
                        .justify_center()
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
        let selection = self.selected.filter(|_| self.editing.is_none()).and_then(|i| self.doc.shapes.get(i)).map(|sh| (sh.bounds(), sh.handles(), sh.scales()));
        let accent = cx.theme().colors().text_accent;
        let overlay = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                let pt = |p: P| point(o.x + px(p.0 * s), o.y + px(p.1 * s));
                let area = |l: f32, t: f32, r: f32, b: f32| Bounds::from_corners(pt((l, t)), pt((r, b)));
                if let Some((a, b)) = crop {
                    let (l, t, r, bt) = rect(a, b);
                    let (w, h) = (f32::from(bounds.size.width) / s, f32::from(bounds.size.height) / s);
                    let shade = rgba(0x000000a6);
                    for (l0, t0, r0, b0) in [(0., 0., w, t), (0., bt, w, h), (0., t, l, bt), (r, t, w, bt)] {
                        if r0 > l0 && b0 > t0 {
                            window.paint_quad(fill(area(l0, t0, r0, b0), shade));
                        }
                    }
                    window.paint_quad(outline(area(l, t, r, bt), gpui::white(), BorderStyle::Solid));
                }
                // Like CleanShot: shapes that resize show only round knobs;
                // the rest (pen, text, counters) get a light dashed frame.
                if let Some(((l, t, r, b), handles, framed)) = selection {
                    if framed {
                        window.paint_quad(outline(area(l, t, r, b), accent, BorderStyle::Dashed));
                    }
                    for h in handles {
                        let c = pt(h);
                        let disc = |r: f32| Bounds::new(point(c.x - px(r), c.y - px(r)), size(px(2. * r), px(2. * r)));
                        window.paint_quad(fill(disc(8.), rgba(0x00000033)).corner_radii(px(8.)));
                        window.paint_quad(gpui::quad(disc(6.), px(6.), gpui::white(), px(1.5), accent, BorderStyle::Solid));
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
            .id("canvas")
            .relative()
            .flex_none()
            .w(px(self.disp.0))
            .h(px(self.disp.1))
            .rounded(px(radius))
            .overflow_hidden()
            .when(!framed, |d| d.shadow(kit::shadow()))
            .when(framed && self.frame.shadow > 0., |d| {
                let big = self.disp.0.max(self.disp.1);
                d.shadow(vec![BoxShadow {
                    color: gpui::hsla(0., 0., 0., 0.6 * self.frame.shadow),
                    offset: point(px(0.), px(big * 0.012)),
                    blur_radius: px(big * 0.035),
                    spread_radius: px(0.),
                    inset: false,
                }])
            })
            .cursor(pointer)
            .child(img(self.image.clone()).size_full().object_fit(ObjectFit::Fill).rounded(px(radius)))
            .children(patches)
            .child(strokes)
            .children(labels)
            .child(overlay)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.mouse_down(e, cx)));

        // The background preview mirrors what Save produces.
        let ((ow, oh), _) = self.frame.layout(self.disp.0, self.disp.1);
        let backdrop = match &self.frame.backdrop {
            Backdrop::None => None,
            Backdrop::Gradient(i) => {
                let (c0, c1) = self.backdrops()[(*i).min(self.backdrops().len() - 1)];
                Some(div().absolute().inset_0().bg(linear_gradient(135., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.))).into_any_element())
            }
            Backdrop::Wallpaper(path) => Some(img(path.clone()).absolute().inset_0().size_full().object_fit(ObjectFit::Cover).into_any_element()),
        };
        match backdrop {
            Some(backdrop) => div()
                .relative()
                .flex_none()
                .w(px(ow))
                .h(px(oh))
                .rounded_lg()
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .child(backdrop)
                .child(picture)
                .into_any_element(),
            None => div().p(px(self.disp.0.max(self.disp.1) * FRAME_PAD)).child(picture).into_any_element(),
        }
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // One CleanShot-like window: tools on top, the picture on a stage
        // (with the background panel beside it when open), actions at the
        // bottom. The picture shrinks to make room for its background.
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let panel = if self.panel { PANEL } else { 0. };
        let box_of = |frame: &Frame, (w, h): (f32, f32)| {
            if frame.on() {
                frame.layout(w, h).0
            } else {
                let pad = w.max(h) * FRAME_PAD;
                (w + 2. * pad, h + 2. * pad)
            }
        };
        let full = box_of(&self.frame, self.natural);
        let room = (vw - 32. - 48. - panel, vh - 230. * kit::SCALE);
        let k = (room.0 / full.0).min(room.1 / full.1).min(1.);
        let fit = (self.natural.0 * k, self.natural.1 * k);
        self.disp = (fit.0 * self.zoom, fit.1 * self.zoom);
        // The window keeps its fitted size; a zoomed canvas scrolls inside it.
        let frame = box_of(&self.frame, fit);
        let stage_w = (frame.0 + 48.).max(1180. * kit::SCALE - panel);
        let width = (stage_w + panel).min(vw - 32.);
        let stage_h = frame.1 + 48.;
        let stage = h_flex()
            .w_full()
            .h(px(stage_h))
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
            .when(self.panel, |row| row.child(self.render_panel(stage_h, cx)));
        kit::root(cx)
            .bg(rgba(0x00000040))
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            // The pointer stays with this surface while the button is held.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.mouse_up(cx)))
            .child(
                v_flex()
                    .w(px(width))
                    .rounded(px(kit::RADIUS))
                    .overflow_hidden()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .shadow(kit::shadow())
                    .child(self.render_toolbar(width, cx))
                    .child(stage)
                    .child(self.render_footer(width, cx)),
            )
    }
}

/// What's dragged from "Перетащи"; the file is rendered when the drag starts.
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
        Kind::Rect | Kind::Ellipse | Kind::Blur(_) | Kind::Spotlight => {
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

fn segment_distance(p: P, a: P, b: P) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0. { 0. } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0., 1.) };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// Annotated, cropped, optionally framed picture as PNG bytes.
fn compose(src: &RgbaImage, doc: &Doc, frame: &Frame) -> Option<Vec<u8>> {
    let mut out = rasterize(src, &doc.shapes)?;
    if let Some((a, b)) = doc.crop {
        let (l, t, r, bt) = rect(a, b);
        let (x, y) = (l.max(0.).round() as u32, t.max(0.).round() as u32);
        let w = (r.round() as u32).min(out.width()).saturating_sub(x);
        let h = (bt.round() as u32).min(out.height()).saturating_sub(y);
        if w > 0 && h > 0 {
            out = image::imageops::crop_imm(&out, x, y, w, h).to_image();
        }
    }
    if frame.on() {
        out = framed(&out, frame);
    }
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// The backdrop picture at `w`×`h`: the wallpaper cropped to fill, or the
/// gradient.
fn backdrop_pixels(backdrop: &Backdrop, w: u32, h: u32) -> RgbaImage {
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
    let (a, b) = (c0.to_rgb(), c1.to_rgb());
    RgbaImage::from_fn(w, h, |x, y| {
        let g = ((x as f32 / w as f32) * 0.6 + (y as f32 / h as f32) * 0.4).clamp(0., 1.);
        let mix = |p: f32, q: f32| ((p + (q - p) * g) * 255.).round() as u8;
        image::Rgba([mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), 255])
    })
}

/// The picture with rounded corners and a soft shadow on its background,
/// like the studio render does for recordings.
fn framed(img: &RgbaImage, frame: &Frame) -> RgbaImage {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let ((ow, oh), (left, top)) = frame.layout(w, h);
    let radius = w.max(h) * frame.radius;
    let blur = (w.max(h) * frame.padding * 0.45).max(w.max(h) * 0.02);
    let strength = frame.shadow.clamp(0., 1.);
    let mut out = backdrop_pixels(&frame.backdrop, ow as u32, oh as u32);
    // Signed distance to a rounded rectangle centered at (cx, cy).
    let sdf = |x: f32, y: f32, cx: f32, cy: f32| {
        let qx = (x - cx).abs() - (w / 2. - radius);
        let qy = (y - cy).abs() - (h / 2. - radius);
        qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - radius
    };
    let (cx, cy) = (left + w / 2., top + h / 2.);
    for (x, y, px) in out.enumerate_pixels_mut() {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut c = [px.0[0] as f32 / 255., px.0[1] as f32 / 255., px.0[2] as f32 / 255.];
        if strength > 0. {
            let d = sdf(fx, fy, cx, cy + blur * 0.35);
            let u = ((d + blur * 0.4) / (blur * 1.4)).clamp(0., 1.);
            let k = u * u * (3. - 2. * u);
            let shade = 1. - 0.9 * strength * (1. - k);
            c.iter_mut().for_each(|v| *v *= shade);
        }
        // The picture itself, antialiased along its rounded edge.
        let cover = (0.5 - sdf(fx, fy, cx, cy)).clamp(0., 1.);
        if cover > 0. {
            let (ix, iy) = ((fx - left).clamp(0., w - 1.) as u32, (fy - top).clamp(0., h - 1.) as u32);
            let p = img.get_pixel(ix, iy).0;
            let k = cover * p[3] as f32 / 255.;
            for (v, s) in c.iter_mut().zip(p) {
                *v = *v * (1. - k) + s as f32 / 255. * k;
            }
        }
        *px = image::Rgba([(c[0] * 255.) as u8, (c[1] * 255.) as u8, (c[2] * 255.) as u8, 255]);
    }
    out
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
        Kind::Line => stroke(window, &[a, b], false, c, w),
        Kind::Pen(points) => stroke(window, &smooth(points), false, c, w),
        Kind::Marker(points) => {
            let mut faded = c;
            faded.a = MARKER_ALPHA;
            stroke(window, &smooth(points), false, faded, w)
        }
        Kind::Arrow => {
            let (shaft, head) = arrow_geometry(a, b, w);
            stroke(window, &[a, shaft], false, c, w);
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
        Kind::Blur(None) => {
            let mut outline = c;
            outline.a = 0.8;
            stroke(window, &[a, (b.0, a.1), b, (a.0, b.1)], true, outline, 1.5 / s);
        }
        Kind::Blur(Some(_)) | Kind::Text(_) | Kind::Counter(_) | Kind::Spotlight => {}
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
    if c.l > 0.65 { gpui::hsla(0., 0., 0.08, 1.) } else { gpui::white() }
}

pub(crate) fn font() -> &'static FontRef<'static> {
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| FontRef::try_from_slice(include_bytes!("../data/zed/fonts/IBMPlexSans-SemiBold.ttf")).expect("bundled font"))
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

/// Pixelate the a–b area of `src`; returns the patch as PNG bytes.
fn pixelate_png(src: &RgbaImage, a: P, b: P) -> Option<Vec<u8>> {
    let patch = pixelate(src, a, b)?;
    let mut png = Vec::new();
    patch.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

fn pixelate(src: &RgbaImage, a: P, b: P) -> Option<RgbaImage> {
    let (l, t, r, bt) = rect(a, b);
    let (x, y) = (l.floor().max(0.) as u32, t.floor().max(0.) as u32);
    let w = (r.ceil() as u32).min(src.width()).saturating_sub(x);
    let h = (bt.ceil() as u32).min(src.height()).saturating_sub(y);
    if w < 2 || h < 2 {
        return None;
    }
    // Blocks scale with the screenshot so text stays unreadable at any size.
    let block = (src.width().max(src.height()) / 120).clamp(8, 32);
    let region = image::imageops::crop_imm(src, x, y, w, h).to_image();
    let small = image::imageops::resize(&region, (w / block).max(1), (h / block).max(1), image::imageops::FilterType::Triangle);
    Some(image::imageops::resize(&small, w, h, image::imageops::FilterType::Nearest))
}

/// Draw everything onto the original at full resolution.
fn rasterize(src: &RgbaImage, shapes: &[Shape]) -> Option<RgbaImage> {
    use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

    // Pixelation edits the pixels themselves, beneath every other shape.
    let mut base = src.clone();
    for shape in shapes {
        if let Kind::Blur(_) = shape.kind
            && let Some(patch) = pixelate(src, shape.a, shape.b)
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
        let rgb = c.to_rgb();
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
                if let Some(path) = polyline(&[a, b], false) {
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
                let (shaft, head) = arrow_geometry(a, b, size);
                if let Some(path) = polyline(&[a, shaft], false) {
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
            Kind::Blur(_) | Kind::Spotlight => {}
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
    let rgb = color.to_rgb();
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
            shapes.push(Shape { kind: Kind::Line, a: w[0], b: w[1], color: red, size: 6., filled: false, text: TextStyle::Plain });
        }
        shapes.push(Shape { kind: Kind::Pen(trace(160.)), a: (0., 0.), b: (0., 0.), color: red, size: 6., filled: false, text: TextStyle::Plain });
        let png = compose(&src, &Doc { shapes, crop: None }, &Frame::default()).unwrap();
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
    fn hit_handles_and_move() {
        let mut line = Shape { kind: Kind::Arrow, a: (10., 10.), b: (110., 10.), color: red(), size: 4., filled: false, text: TextStyle::Plain };
        assert!(line.hit((60., 13.), 2.));
        assert!(!line.hit((60., 30.), 2.));
        assert_eq!(line.handles(), vec![(10., 10.), (110., 10.)]);
        line.set_handle(1, (110., 60.));
        assert_eq!(line.b, (110., 60.));
        line.translate((5., 5.));
        assert_eq!((line.a, line.b), ((15., 15.), (115., 65.)));

        let mut boxy = Shape { kind: Kind::Rect, a: (0., 0.), b: (50., 40.), color: red(), size: 3., filled: false, text: TextStyle::Plain };
        assert!(boxy.hit((25., 20.), 1.), "inside a box selects it");
        boxy.set_handle(1, (80., -10.)); // top-right corner
        assert_eq!((boxy.a, boxy.b), ((0., -10.), (80., 40.)));

        let pen = Shape { kind: Kind::Pen(vec![(0., 0.), (10., 10.), (20., 0.)]), a: (0., 0.), b: (20., 0.), color: red(), size: 2., filled: false, text: TextStyle::Plain };
        assert!(pen.hit((15., 5.), 1.));
        assert_eq!(pen.handles().len(), 4, "freehand scales from its corners");
    }

    #[test]
    fn spotlight_dims_around_and_fill_is_solid() {
        let src = RgbaImage::from_pixel(200, 100, image::Rgba([200, 200, 200, 255]));
        let shapes = vec![
            Shape { kind: Kind::Spotlight, a: (10., 10.), b: (60., 60.), color: red(), size: 3., filled: false, text: TextStyle::Plain },
            Shape { kind: Kind::Rect, a: (120., 20.), b: (180., 80.), color: red(), size: 3., filled: true, text: TextStyle::Plain },
        ];
        let out = image::load_from_memory(&compose(&src, &Doc { shapes, crop: None }, &Frame::default()).unwrap()).unwrap().to_rgba8();
        assert_eq!(out.get_pixel(30, 30).0[0], 200, "inside the spotlight stays");
        assert!(out.get_pixel(90, 90).0[0] < 100, "outside is dimmed");
        let inner = out.get_pixel(150, 50).0;
        assert!(inner[0] > inner[1] + 40, "the box is filled with its colour: {inner:?}");
    }

    #[test]
    fn text_on_a_plate_and_outlined() {
        let src = RgbaImage::from_pixel(300, 120, image::Rgba([240, 240, 240, 255]));
        let text = |a: P, style| Shape { kind: Kind::Text("Hi".into()), a, b: a, color: red(), size: 30., filled: false, text: style };
        let plate = text((20., 20.), TextStyle::Plate);
        let (l, t, _, _) = plate.bounds();
        assert!(l < 20. && t < 20., "the plate reaches around the text");
        let shapes = vec![plate, text((160., 20.), TextStyle::Outline)];
        let out = image::load_from_memory(&compose(&src, &Doc { shapes, crop: None }, &Frame::default()).unwrap()).unwrap().to_rgba8();
        let corner = out.get_pixel((l + 3.) as u32, 28).0;
        assert!(corner[0] > corner[1] + 40, "plate is the text colour: {corner:?}");
        if let Ok(path) = std::env::var("KADR_TEST_TEXT") {
            out.save(path).unwrap();
        }
    }

    #[test]
    fn corners_scale_text_and_freehand() {
        let text = Shape { kind: Kind::Text("Hello".into()), a: (100., 100.), b: (100., 100.), color: red(), size: 20., filled: false, text: TextStyle::Plain };
        let (l, t, r, b) = text.bounds();
        let mut big = text.clone();
        // Drag the bottom-right corner to twice the box.
        big.scale_from(&text, 2, (l + (r - l) * 2., t + (b - t) * 2.));
        assert!((big.size - 40.).abs() < 0.5, "font doubles: {}", big.size);
        assert_eq!(big.a, (100., 100.), "top-left stays");

        let pen = Shape { kind: Kind::Pen(vec![(0., 0.), (10., 20.)]), a: (0., 0.), b: (10., 20.), color: red(), size: 2., filled: false, text: TextStyle::Plain };
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
    fn crop_and_backdrop_sizes() {
        let src = RgbaImage::from_pixel(400, 300, image::Rgba([200, 200, 200, 255]));
        let shapes = vec![Shape { kind: Kind::Rect, a: (60., 60.), b: (220., 180.), color: red(), size: 6., filled: false, text: TextStyle::Plain }];
        let doc = Doc { shapes, crop: Some(((50., 40.), (250., 200.))) };
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
            let big = Doc { shapes: doc.shapes.clone(), crop: None };
            std::fs::write(out, compose(&src, &big, &frame).unwrap()).unwrap();
        }
    }
}
