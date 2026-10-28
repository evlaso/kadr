//! Annotation editor: draw on a
//! screenshot, select and move what you drew, crop, put it on a backdrop,
//! then copy or save. The view lives on a full-monitor layer (GPUI can't
//! move or resize layers, and a modal overlay suits it anyway). Saving
//! re-draws every shape at full resolution with tiny-skia, so the file
//! matches what was on screen without the screen's scaling.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    FillOptions, PathStyle,
    App, BorderStyle, Bounds, BoxShadow, Context, CursorStyle, FocusHandle, Hsla, Image, ImageFormat,
    KeyDownEvent, MouseButton, ScrollWheelEvent, MouseDownEvent, MouseMoveEvent, ObjectFit, PathBuilder, Point,
    Window, canvas, div, img, linear_color_stop, linear_gradient, 
    point, px, rgba, size,
};
use image::RgbaImage;
use kadr_core::color::Rgba;
use kadr_core::project;
use kadr_core::document::{ArrowStyle, Backdrop, Doc, Frame, Hide, Kind, MARKER_ALPHA, MAX_PAD, MAX_REACH, MAX_ROUND, P, SPOT_SHADE, Shape, TextStyle, rect, work_margin_at};
use kadr_raster::shapes::{Piece, arrow_pieces, draw_vector, ellipse_points, smooth};
use kadr_raster::text::metrics;
use kadr_raster::{Bounds as _, cursors};
use kadr_ui::editor::{COUNTER_SIZES, CURSOR_SIZES, Edit, EditorVm, FONT_SIZES, WIDTHS, sized};
use crate::ui::{ButtonLike, Divider, IconButtonShape, Tooltip, prelude::*};

use crate::kit::{self, Anchor, KeyboardInteractivity, LayerSpec};
use crate::ui::{fill, outline};
use crate::{cards, ocr, palette, studio_editor, util};
use crate::i18n::t;

mod crop;
mod redact;


/// Where the size slider starts.
const WEIGHT: f32 = 0.22;


/// Room around the picture to draw and crop in, the same way: the
/// result grows to take in what's drawn outside the picture.
const WORK_PAD: f32 = 0.1;
/// How far the canvas zooms out from fitting the stage.
const MIN_ZOOM: f32 = 0.2;
/// Points this close to the picture's edge (screen pixels) snap onto it.
const EDGE_SNAP: f32 = 6.;
/// Grab distance for shapes and handles, in screen pixels.
const GRAB: f32 = 7.;
/// The selection band's width, in screen pixels (its gap from the shape is
/// `kadr_ui::editor::BAND_GAP`).
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
/// Output shapes for the framed picture.
const ASPECTS: [(&str, Option<f32>); 5] = [("editor.auto", None), ("16:9", Some(16. / 9.)), ("4:3", Some(4. / 3.)), ("1:1", Some(1.)), ("9:16", Some(9. / 16.))];




/// The background last used, for the next screenshot.
fn last_frame() -> Frame {
    let saved = std::fs::read_to_string(util::config_dir().join("editor-frame.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    saved.map(|v| Frame::from_json(&v)).unwrap_or_default()
}

fn remember_frame(frame: &Frame) {
    let _ = std::fs::create_dir_all(util::config_dir());
    let _ = std::fs::write(util::config_dir().join("editor-frame.json"), frame.to_json().to_string());
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Knob {
    Padding,
    Radius,
    Shadow,
    /// The toolbar's size slider.
    Weight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    Cursor,
    Blur,
    Spotlight,
    Crop,
}

impl Tool {
    const ALL: [Tool; 13] = [
        Tool::Select,
        Tool::Arrow,
        Tool::Line,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Pen,
        Tool::Marker,
        Tool::Text,
        Tool::Counter,
        Tool::Cursor,
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
        &[Tool::Text, Tool::Counter, Tool::Cursor],
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
            Tool::Cursor => Icon::from_path("icons/kadr/cursor.svg"),
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
            Tool::Cursor => t("editor.cursor"),
            Tool::Blur => t("editor.hide_tool"),
            Tool::Spotlight => t("editor.spotlight_dim_everything_around"),
            Tool::Crop => t("common.trim"),
        }
    }

    /// Single-key shortcut.
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
            Tool::Cursor => "m",
            Tool::Blur => "b",
            Tool::Spotlight => "s",
            Tool::Crop => "c",
        }
    }
}



/// The finished strokes drawn once into a picture, so a frame paints that
/// instead of outlining every stroke again; the shapes stay shapes. Redrawn
/// in the background whenever they change, while the canvas paints what the
/// picture lacks on top of it.
#[derive(Default)]
struct Strokes {
    shown: Option<StrokesImage>,
    busy: bool,
    /// The shape being moved or resized, drawn on its own: moving only
    /// shifts it, resizing stretches it until it's drawn again.
    lifted: Option<Lifted>,
    lift_busy: bool,
    /// Replaced pictures, for the next frame to free.
    retired: Vec<Arc<gpui::RenderImage>>,
}

struct Lifted {
    index: usize,
    /// The shape as drawn, and the picture's top left in image pixels.
    shape: Shape,
    at: P,
    /// The picture's size in image pixels.
    size: P,
    image: Arc<gpui::RenderImage>,
}

/// How far `now` is `then` moved, if it's only moved.
fn moved_by(now: &Shape, then: &Shape) -> Option<P> {
    let d = (now.a.0 - then.a.0, now.a.1 - then.a.1);
    // Loose enough for the rounding a long drag piles up.
    let near = |p: P, q: P| (p.0 - q.0 - d.0).abs() < 0.05 && (p.1 - q.1 - d.1).abs() < 0.05;
    let same = near(now.b, then.b)
        && now.size == then.size
        && now.filled == then.filled
        && now.color == then.color
        && match (now.bend, then.bend) {
            (Some(p), Some(q)) => near(p, q),
            (None, None) => true,
            _ => false,
        }
        && match (&now.kind, &then.kind) {
            (Kind::Pen(p), Kind::Pen(q)) | (Kind::Marker(p), Kind::Marker(q)) => p.len() == q.len() && p.iter().zip(q).all(|(p, q)| near(*p, *q)),
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        };
    same.then_some(d)
}

struct StrokesImage {
    /// It holds the vector shapes before `count`, but `skip`.
    count: usize,
    skip: Option<usize>,
    hash: u64,
    /// Its pixels per image pixel, and the box it covers in image pixels:
    /// round the strokes, so it doesn't change with the room round the
    /// picture.
    density: f32,
    at: P,
    size: P,
    image: Arc<gpui::RenderImage>,
}

/// A fingerprint of the vector shapes before `count`, but `skip`.
fn strokes_hash(shapes: &[Shape], count: usize, skip: Option<usize>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut mix = |v: u32| h = (h ^ v as u64).wrapping_mul(0x100000001b3);
    for (i, sh) in shapes.iter().enumerate().take(count) {
        if Some(i) == skip || !sh.kind.is_vector() {
            continue;
        }
        mix(i as u32);
        let tag = match &sh.kind {
            Kind::Arrow(_) => 1,
            Kind::Rect => 2,
            Kind::Ellipse => 3,
            Kind::Line => 4,
            Kind::Pen(_) => 5,
            _ => 6,
        };
        mix(tag);
        let c = sh.color;
        for v in [sh.a.0, sh.a.1, sh.b.0, sh.b.1, sh.size, c.r, c.g, c.b, c.a] {
            mix(v.to_bits());
        }
        mix(sh.filled as u32);
        if let Some(bend) = sh.bend {
            mix(bend.0.to_bits());
            mix(bend.1.to_bits());
        }
        if let Kind::Pen(points) | Kind::Marker(points) = &sh.kind {
            mix(points.len() as u32);
            for p in points {
                mix(p.0.to_bits());
                mix(p.1.to_bits());
            }
        }
    }
    h
}

/// What a cursor shape is called in the toolbar.
fn cursor_label(name: &str) -> &'static str {
    match name {
        "pointer" => t("editor.cursor_pointer"),
        "text" => t("editor.cursor_text"),
        "grab" => t("editor.cursor_grab"),
        "grabbing" => t("editor.cursor_grabbing"),
        "crosshair" => t("editor.cursor_crosshair"),
        "wait" => t("editor.cursor_wait"),
        "not-allowed" => t("editor.cursor_not_allowed"),
        _ => t("editor.cursor_arrow"),
    }
}

/// A cursor shape with its arrow `height` pixels tall, as GPUI wants an image.
fn cursor_image(name: &str, height: f32) -> Option<Arc<gpui::RenderImage>> {
    let (hot, (w, h)) = cursors::geometry(name, height);
    let (w, h) = (w.ceil().max(1.) as u32, h.ceil().max(1.) as u32);
    let mut pm = tiny_skia::Pixmap::new(w, h)?;
    cursors::draw(name, height, hot, &mut pm.as_mut());
    let mut data = pm.take();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = (*c as u32 * 255 / a).min(255) as u8;
            }
        }
    }
    let image = RgbaImage::from_raw(w, h, data)?;
    Some(Arc::new(gpui::RenderImage::new([image::Frame::new(image)])))
}

/// The box round `shapes` in image pixels, with room for their line ends.
fn shapes_box(shapes: &[Shape]) -> Option<(P, P)> {
    let (l, t, r, b) = shapes.iter().map(|sh| {
        let (l, t, r, b) = sh.bounds();
        let pad = sh.size + 2.;
        (l - pad, t - pad, r + pad, b + pad)
    }).reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)))?;
    Some(((l.floor(), t.floor()), ((r - l).ceil() + 1., (b - t).ceil() + 1.)))
}

/// Pixels per image pixel for a picture of a `size` box: `wanted`, within
/// what a texture holds.
fn box_density(size: P, wanted: f32) -> f32 {
    wanted.min(8192. / size.0.max(size.1).max(1.)).min((24e6 / (size.0 * size.1).max(1.)).sqrt())
}

/// The shapes' picture round them, at about `density`, as GPUI wants an
/// image: BGRA, not premultiplied. Also the box it covers and its density.
fn strokes_image(shapes: &[Shape], density: f32) -> Option<(P, P, f32, Arc<gpui::RenderImage>)> {
    let (at, size) = shapes_box(shapes)?;
    let density = box_density(size, density);
    shapes_image(shapes, at, size, density).map(|(_, image)| (at, size, density, image))
}

/// The shapes in the `size` box at `at` (image pixels), at `density`.
fn shapes_image(shapes: &[Shape], at: P, size: P, density: f32) -> Option<(u32, Arc<gpui::RenderImage>)> {
    let w = (size.0 * density).ceil().max(1.) as u32;
    let h = (size.1 * density).ceil().max(1.) as u32;
    let mut pm = tiny_skia::Pixmap::new(w, h)?;
    let tf = tiny_skia::Transform::from_row(density, 0., 0., density, -at.0 * density, -at.1 * density);
    for shape in shapes {
        draw_vector(&mut pm, shape, tf);
    }
    let mut data = pm.take();
    for px in data.chunks_exact_mut(4) {
        px.swap(0, 2);
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = (*c as u32 * 255 / a).min(255) as u8;
            }
        }
    }
    let image = RgbaImage::from_raw(w, h, data)?;
    Some((w, Arc::new(gpui::RenderImage::new([image::Frame::new(image)]))))
}










enum Drag {
    Draw,
    Move { index: usize, last: P, moved: bool },
    Handle { index: usize, handle: usize },
    Scale { index: usize, handle: usize, orig: Shape },
    Crop { start: P, grab: Grab },
    /// A box dragged over the empty canvas with the select tool: the shapes
    /// it touches get marked.
    Marquee { start: P, now: P },
    /// A crop's knob (corners, then edge middles) dragged, or the crop
    /// moved whole (`handle` None) from `from`.
    CropEdit { handle: Option<usize>, orig: (f32, f32, f32, f32), from: P, grab: Grab },
    Slider(Knob),
}

/// Where a crop drag started: the pointer on screen, the canvas scale then,
/// and the picture point under it. The crop follows the pointer's travel at
/// that scale, so it moves evenly whatever the room round it does.
#[derive(Clone, Copy)]
struct Grab {
    at: (f32, f32),
    scale: f32,
    point: P,
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
    /// The tools that don't fit in a narrow toolbar.
    Tools,
    /// The footer's actions, in a narrow window.
    Actions,
    Aspect,
    Scale,
    Zoom,
    /// What fills a crop past the picture.
    Fill,
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
    /// Show the file's card again on closing, unless a save showed one.
    card_on_close: bool,
    /// The picture as shown, its edge pixels repeated round it: drawn
    /// scaled, GPUI blends a texture's outer texels with what lies past
    /// them, a grey rim at the right and bottom; with the rim repeated and
    /// only the picture painted, there's none. Until it's decoded, nothing.
    picture: Option<Arc<gpui::RenderImage>>,
    /// Pictures replaced by a turn, to free from the window's atlas.
    stale_pictures: Vec<Arc<gpui::RenderImage>>,
    /// The finished strokes as one picture, for the canvas to show.
    strokes: Rc<RefCell<Strokes>>,
    px: (u32, u32),
    /// Decoded picture as shown (turned as `shown` says), for pixelation and
    /// the final render.
    source: Option<Arc<RgbaImage>>,
    /// The picture as taken, decoded, and its size; what `source`
    /// are turned from.
    original: Option<Arc<RgbaImage>>,
    px_taken: (u32, u32),
    /// How `image` and `source` are turned; catches up with `doc.view`.
    shown: crop::Orientation,
    /// A window shot's window inside its baked-in shadow, for how `source`
    /// is turned; None when the picture has no such margin.
    window: Option<(crop::Orientation, Option<crop::Rect>)>,
    /// While cropping: the stage's free room round the picture, in image
    /// pixels, which a crop can grow the canvas into.
    crop_room: std::cell::Cell<f32>,
    /// The room the canvas was last laid out with. Pointer positions map
    /// through it: the room follows the crop, and events between frames
    /// would otherwise land on a picture that isn't where they think.
    laid_margin: std::cell::Cell<f32>,
    /// The canvas sits in the middle of the stage, moved by this much: a
    /// camera, panned with Space or the wheel, kept round the point zoomed
    /// at. A canvas growing round the picture (a crop pulled out) leaves the
    /// picture where it is.
    view_pan: (f32, f32),
    /// While a crop is dragged past the stage, how far the view follows it.
    crop_shift: (f32, f32),
    /// The stage's bounds in the window, last laid out: x, y, w, h.
    stage_box: Rc<Cell<(f32, f32, f32, f32)>>,
    /// Space is held: a press pans the stage.
    space: bool,
    /// While Space is held: the pointer's last position; moving it pans,
    /// with or without a button down.
    pan: Option<(f32, f32)>,
    /// The crop was set to `window` by turning a background on.
    window_fitted: bool,
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
    /// The footer's sides as last drawn: the left one, and the right one
    /// with its actions unfolded, to know when they must fold.
    footer_left: crate::ui::Width,
    footer_right: crate::ui::Width,
    /// How compact the crop mode's top bar must be.
    crop_fit: crate::ui::Fit,
    /// On-screen size of the picture.
    disp: (f32, f32),
    /// The drawing and its undo history.
    vm: EditorVm,
    draft: Option<Shape>,
    drag: Option<Drag>,
    selected: Option<usize>,
    /// Shapes picked together by dragging a box round them (or Ctrl+A),
    /// for deleting at once.
    marked: Vec<usize>,
    /// Index of the text shape being typed.
    editing: Option<usize>,
    /// The text being typed into it: caret, selection, the input method's
    /// marked text. Its text is the shape's.
    field: kadr_ui::text::Field,
    /// Where the caret was drawn, in the stage's pixels: the input method
    /// puts its candidates there.
    caret_at: Cell<Option<Bounds<gpui::Pixels>>>,
    tool: Tool,
    /// The colour menu's own colours, the current one (any colour), the
    /// ones picked lately and the ones kept (both in the settings).
    colors: Vec<Hsla>,
    color: Hsla,
    recent: Vec<Hsla>,
    saved: Vec<Hsla>,
    /// The next click on the picture takes its colour.
    picking: bool,
    /// The size slider, 0..1.
    weight: f32,
    /// Boxes and ellipses are drawn solid.
    fill: bool,
    /// Pixelate or blur, for the hide tool.
    hide: Hide,
    /// The arrow tool's style.
    arrow: ArrowStyle,
    /// "Hide sensitive" is reading the picture.
    redacting: bool,
    /// The file's density (PNG pHYs), kept on what's saved. The picture
    /// itself is `original`; the file's bytes aren't kept.
    density: Option<f64>,
    /// Opened with the drawing from an earlier save.
    restored: bool,
    /// The `.kadr` this session keeps the drawing in: the one it was opened
    /// from, or the one "Save project" wrote. Saving writes it again.
    bundle: Option<PathBuf>,
    /// Pointer shape over a handle or a shape, while no button is held.
    hover: Option<CursorStyle>,
    text_style: TextStyle,
    /// The cursor tool's shape, by `cursors` name.
    cursor_kind: &'static str,
    /// The cursor tool's shapes for its toolbar, drawn once.
    cursor_icons: Vec<(&'static str, Arc<gpui::RenderImage>)>,
    /// Cursors drawn on the canvas, by shape and height in device pixels.
    cursor_images: RefCell<HashMap<(&'static str, u32), Arc<gpui::RenderImage>>>,
    /// Hidden areas' patches for the canvas, by `patch_key`: they outlive a
    /// move or an undo, so going back shows them at once.
    patches: HashMap<u64, Arc<Image>>,
    /// Laid-on pictures as the canvas shows them, by the picture's address
    /// (the `Arc` is kept, so the address stays its own).
    pictures: RefCell<HashMap<usize, (Arc<RgbaImage>, Arc<Image>)>>,
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
    /// The text styles' dropdown is open.
    text_styles_open: bool,
    text_styles_menu: crate::ui::MenuBounds,
    cursors_open: bool,
    cursors_menu: crate::ui::MenuBounds,
    /// The "⋯" menu with the style controls, when the toolbar is too narrow.
    more_open: bool,
    size: (f32, f32),
    focus: FocusHandle,
}

/// Open a screenshot for annotation, or a `.kadr` project made here.
pub fn open(path: PathBuf, cx: &mut App) {
    open_as(path, false, cx);
}

/// Opened from a capture's card: closing without saving brings the card
/// back (saving shows one for the result anyway).
pub fn open_from_card(path: PathBuf, cx: &mut App) {
    open_as(path, true, cx);
}

fn open_as(path: PathBuf, card: bool, cx: &mut App) {
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
    // The file's bytes, read once for the picture and then let go.
    let bytes = restored.as_mut().map(|r| std::mem::take(&mut r.base)).unwrap_or(bytes);
    let file_density = cards::png_density(&bytes);
    // PNG's size is read straight from its header; JPEG and WebP through
    // their decoders, without decoding the pixels.
    let size = cards::png_size(&bytes).or_else(|| image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().ok()?.into_dimensions().ok());
    let Some((w, h)) = size else {
        return util::notify(t("common.couldnt_open_image"), &path.display().to_string());
    };
    let Some(mon) = util::focused_monitor() else { return };
    let (mw, mh) = (mon.w as f32, mon.h as f32);

    // Real size on screen (the PNG's density, else screenshots are in
    // physical pixels), shrunk to leave room for the toolbars and a backdrop.
    let density = cards::png_density(&bytes).unwrap_or(mon.scale) as f32;
    let natural = (w as f32 / density, h as f32 / density);
    // The window takes at most about two thirds of the screen; a bigger
    // picture opens smaller than 100% to fit it with
    // its work margin (`pad` below) and the bars.
    let (max_w, max_h) = (mw * 0.7, mh * 0.72);
    let grown = |n: f32| n + natural.0.max(natural.1) * WORK_PAD * 2.;
    let fit = ((max_w - 48.) / grown(natural.0)).min((max_h - 48. - CHROME) / grown(natural.1)).min(1.0);
    let disp = (natural.0 * fit, natural.1 * fit);

    // A window around the picture with its backdrop margin, the stage
    // padding and the bars; the picture refits whenever the window resizes.
    let pad = disp.0.max(disp.1) * WORK_PAD * 2.;
    // Never a letterbox: at least a comfortable height, whatever the picture.
    let size = ((disp.0 + pad + 48.).max(1180. * kit::SCALE).min(mw - 80.), (disp.1 + pad + 48. + CHROME).max(640. * kit::SCALE).min(max_h.max(640. * kit::SCALE)).min(mh - 80.));
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
        crate::dev::mark("editor:build");
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
                    editor.set_picture();
                    // A reopened drawing may be on a turned picture.
                    editor.sync_view(cx);
                    editor.refresh_blurred(cx);
                    // A reopened drawing's hidden areas.
                    for i in 0..editor.vm.doc.shapes.len() {
                        editor.refresh_patch(i, cx);
                    }
                    cx.notify();
                });
                let icons = cx.background_executor().spawn(async { cursors::STAMPS.iter().filter_map(|&name| Some((name, cursor_image(name, cursors::height_for(name, 44.))?))).collect::<Vec<_>>() }).await;
                let _ = this.update(cx, |editor: &mut Editor, cx| {
                    editor.cursor_icons = icons;
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
            let frame = frame.unwrap_or_else(last_frame);
            if card {
                cx.on_release(|editor: &mut Editor, cx: &mut App| {
                    if editor.card_on_close && editor.path.exists() {
                        cards::show(editor.path.clone(), cx);
                    }
                })
                .detach();
            }
            crate::dev::mark("editor:fields");
            let colors = crate::settings::Settings::load();
            let mut editor = Editor {
                path,
                card_on_close: card,
                picture: None,
                stale_pictures: Vec::new(),
                strokes: Default::default(),
                px: (w, h),
                source: None,
                original: None,
                px_taken: (w, h),
                shown: crop::Orientation::default(),
                window: None,
                crop_room: std::cell::Cell::new(0.),
                laid_margin: std::cell::Cell::new(0.),
                view_pan: (0., 0.),
                crop_shift: (0., 0.),
                stage_box: Rc::new(Cell::new((0., 0., 0., 0.))),
                space: false,
                pan: None,
                window_fitted: false,
                cropping: None,
                aspect: crop::Aspect::Free,
                snap: true,
                edges: None,
                size_field: None,
                crop_menu: None,
                crop_menu_bounds: Default::default(),
                footer_left: Default::default(),
                footer_right: Default::default(),
                crop_fit: Default::default(),
                disp,
                vm: EditorVm::new(doc),
                draft: None,
                drag: None,
                selected: None,
                marked: Vec::new(),
                editing: None,
                field: Default::default(),
                caret_at: Default::default(),
                tool: Tool::Arrow,
                colors: palette::annotation_colors(),
                color: palette::annotation_colors()[0],
                recent: colors.recent_colors.iter().map(|&c| crate::ui::from_rgba(c)).collect(),
                saved: colors.saved_colors.iter().map(|&c| crate::ui::from_rgba(c)).collect(),
                picking: false,
                weight: WEIGHT,
                fill: false,
                hide: Hide::Pixelate,
                arrow: ArrowStyle::default(),
                redacting: false,
                density: file_density,
                restored: reopened,
                bundle,
                hover: None,
                text_style: TextStyle::Plain,
                cursor_kind: cursors::STAMPS[0],
                // Their squares fill 22px slots at twice the pixels.
                // Drawn after the window shows (below): the system's cursor
                // pictures take a while the first time on Windows.
                cursor_icons: Vec::new(),
                cursor_images: Default::default(),
                patches: HashMap::new(),
                pictures: Default::default(),
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
                text_styles_open: false,
                text_styles_menu: Default::default(),
                cursors_open: false,
                cursors_menu: Default::default(),
                more_open: false,
                size,
                focus,
            };
            crate::dev::mark("editor:made");
            // The sizes of a reopened turned picture.
            editor.sync_geometry();
            crate::dev::mark("editor:fitted");
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
    /// The shown picture from `source`, padded with `PICTURE_PAD` repeated
    /// edge pixels (see `picture`).
    fn set_picture(&mut self) {
        let Some(src) = self.source.clone() else { return };
        let (w, h) = src.dimensions();
        let n = PICTURE_PAD;
        // GPUI's sprites are BGRA.
        let padded = RgbaImage::from_fn(w + 2 * n, h + 2 * n, |x, y| {
            let [r, g, b, a] = src.get_pixel(x.saturating_sub(n).min(w - 1), y.saturating_sub(n).min(h - 1)).0;
            image::Rgba([b, g, r, a])
        });
        let picture = Arc::new(gpui::RenderImage::new([image::Frame::new(padded)]));
        if let Some(old) = self.picture.replace(picture) {
            self.stale_pictures.push(old);
        }
    }

    /// Screen pixels per image pixel.
    fn scale(&self) -> f32 {
        self.disp.0 / self.px.0 as f32
    }

    /// The room around the picture, in image pixels: the usual room, or more
    /// for a crop that reaches past it. While cropping, the whole stage round
    /// the picture, and more as a crop is pulled past it.
    fn margin(&self) -> f32 {
        let resting = self.resting_margin();
        if self.cropping.is_none() {
            return resting;
        }
        // Past the stage's room the canvas grows with the crop, some ahead
        // of it, round the picture, which stays put.
        let ahead = self.crop_overhang() + work_margin_at(self.px, WORK_PAD);
        self.crop_room.get().max(resting).max(ahead).ceil().min(work_margin_at(self.px, MAX_REACH))
    }

    /// The room as last drawn; before the first frame, as it would be.
    fn laid_margin(&self) -> f32 {
        match self.laid_margin.get() {
            m if m > 0. => m,
            _ => self.margin(),
        }
    }

    /// The room outside the crop mode: the usual, or the crop's overhang.
    fn resting_margin(&self) -> f32 {
        work_margin_at(self.px, WORK_PAD).max(self.crop_overhang().ceil())
    }

    /// How far the crop reaches past the picture, in image pixels.
    fn crop_overhang(&self) -> f32 {
        let (w, h) = (self.px.0 as f32, self.px.1 as f32);
        self.vm.doc.crop.map_or(0., |(a, b)| {
            let (l, t, r, b) = rect(a, b);
            (-l).max(-t).max(r - w).max(b - h).max(0.)
        })
    }

    /// A point on screen in image pixels: anywhere in the work area around
    /// the picture, snapped onto the picture's edges when close.
    fn to_image(&self, pos: Point<gpui::Pixels>, clamp: bool) -> Option<P> {
        let (ox, oy) = self.origin.get();
        let (s, m) = (self.scale(), self.laid_margin());
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
        let Some((l, t, r, b)) = self.vm.doc.crop.map(|(a, b)| rect(a, b)) else { return Vec::new() };
        let (mx, my) = ((l + r) / 2., (t + b) / 2.);
        vec![(l, t), (r, t), (r, b), (l, b), (mx, t), (r, my), (mx, b), (l, my)]
    }

    /// Sizes that follow the picture's turn: its pixels and how big it
    /// shows.
    fn sync_geometry(&mut self) {
        let view = self.vm.doc.view;
        let side = |p: (f32, f32)| if view.turns % 2 == 1 { (p.1, p.0) } else { p };
        self.px = view.size(self.px_taken);
        self.natural = side(self.natural_taken);
        self.real = side(self.real_taken);
    }

    /// Bring the shown picture in line with `doc.view` after a turn, an undo
    /// or a revert, and redo what depends on its pixels.
    fn sync_view(&mut self, cx: &mut Context<Self>) {
        self.sync_geometry();
        let view = self.vm.doc.view;
        if view == self.shown {
            return;
        }
        let Some(original) = self.original.clone() else { return };
        // Upright, the shown picture is the original itself, not a copy.
        self.source = Some(if view.is_upright() { original } else { Arc::new(view.apply(&original)) });
        self.set_picture();
        self.shown = view;
        self.refresh_blurred(cx);
        self.edges = None;
        if self.cropping.is_some() {
            self.find_edges(cx);
        }
        for i in 0..self.vm.doc.shapes.len() {
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
                    kadr_raster::blurred_pixels(&src, 320, h).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
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
        self.marked.clear();
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
        let history = self.vm.history.mark(&self.vm.doc);
        self.cropping = Some(CropSession { entry: self.vm.doc.clone(), history, tool: self.tool });
        self.tool = Tool::Crop;
        self.selected = None;
        self.colors_open = false;
        self.more_open = false;
        self.panel = false;
        // The frame starts round the whole picture.
        if self.vm.doc.crop.is_none() {
            self.vm.doc.crop = Some(((0., 0.), (self.px.0 as f32, self.px.1 as f32)));
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
        if self.vm.doc.crop.is_some_and(|(a, b)| rect(a, b) == (whole.0.0, whole.0.1, whole.1.0, whole.1.1)) {
            self.vm.doc.crop = None;
        }
        self.tool = if session.tool == Tool::Crop { Tool::Select } else { session.tool };
    }

    /// Cancel: back to how it was when the crop mode opened.
    fn cancel_crop(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.cropping.take() else { return };
        self.size_field = None;
        self.crop_menu = None;
        self.vm.doc = session.entry;
        self.vm.history.rewind(session.history);
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

    /// A window shot carries its own shadow on a transparent margin; on a
    /// background that margin would show as a band round it, so the picture
    /// is cropped to the window while a background is on, and back after.
    fn fit_window(&mut self) {
        let Some(source) = self.source.clone() else { return };
        if self.window.is_none_or(|(view, _)| view != self.shown) {
            self.window = Some((self.shown, opaque_bounds(&source)));
        }
        let Some((_, Some(win))) = self.window else { return };
        let crop = Some(((win.0, win.1), (win.2, win.3)));
        // Once: Uncrop afterwards keeps the shadow.
        if self.frame.on() && !self.window_fitted && self.cropping.is_none() && self.vm.doc.crop.is_none() {
            self.vm.doc.crop = crop;
            self.window_fitted = true;
        } else if !self.frame.on() && self.window_fitted {
            if self.vm.doc.crop == crop {
                self.vm.doc.crop = None;
            }
            self.window_fitted = false;
        }
    }

    /// The crop, or the whole picture when there's none.
    fn crop_or_picture(&self) -> crop::Rect {
        self.vm.doc.crop.map(|(a, b)| rect(a, b)).unwrap_or((0., 0., self.px.0 as f32, self.px.1 as f32))
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
        for shape in &self.vm.doc.shapes {
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
        let m = work_margin_at(self.px, MAX_REACH);
        let area = (-m, -m, self.px.0 as f32 + m, self.px.1 as f32 + m);
        let r = match self.drag {
            Some(Drag::Crop { start, .. }) => {
                let p = if ratio.is_none() { (crop::snap(p.0, &lines.xs, lines.reach), crop::snap(p.1, &lines.ys, lines.reach)) } else { p };
                rect(start, ratio.map_or(p, |k| crop::keep_ratio(start, p, k)))
            }
            Some(Drag::CropEdit { handle, orig, from, .. }) => crop::edit(orig, handle, p, from, ratio, area, &lines),
            _ => return,
        };
        self.vm.doc.crop = Some(((r.0, r.1), (r.2, r.3)));
    }

    /// The aspect menu: a fixed shape fits the crop (or the picture) to it.
    fn pick_aspect(&mut self, aspect: crop::Aspect) {
        self.aspect = aspect;
        if let Some(k) = self.crop_ratio() {
            self.checkpoint();
            let r = crop::fit_ratio(self.crop_or_picture(), k);
            self.vm.doc.crop = Some(((r.0, r.1), (r.2, r.3)));
        }
    }

    /// The ↔ button: the crop and its shape on their side.
    fn swap_crop_sides(&mut self) {
        self.aspect = self.aspect.swapped(self.px);
        if let Some((a, b)) = self.vm.doc.crop {
            self.checkpoint();
            let r = crop::swap_sides(rect(a, b));
            self.vm.doc.crop = Some(((r.0, r.1), (r.2, r.3)));
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
        self.vm.doc.crop = Some(((l, t), (l + w.round(), t + h.round())));
    }

    /// Turn the picture a quarter clockwise, or mirror it, with everything
    /// on it.
    fn turn(&mut self, turn: crop::Turn, cx: &mut Context<Self>) {
        self.size_field = None;
        self.checkpoint();
        crop::turn_doc(&mut self.vm.doc, turn, self.px);
        self.sync_view(cx);
    }

    fn pick_scale(&mut self, scale: f32) {
        if (self.vm.doc.scale - scale).abs() > 1e-4 {
            self.checkpoint();
            self.vm.doc.scale = scale;
        }
    }

    /// Revert to Original: no crop, turn, mirror or resize; the drawing stays.
    fn revert_crop(&mut self, cx: &mut Context<Self>) {
        self.size_field = None;
        self.checkpoint();
        crop::upright(&mut self.vm.doc, self.px);
        self.vm.doc.crop = None;
        self.vm.doc.scale = 1.;
        self.aspect = crop::Aspect::Free;
        self.sync_view(cx);
    }

    /// The canvas zoom for a share of the real size, as far as it goes.
    fn zoom_to_real(&mut self, share: f32) {
        let fit = self.disp.0 / self.zoom;
        self.zoom_at(share * self.real.0 / fit.max(1.), (0., 0.));
    }

    fn crop_handle_at(&self, p: P) -> Option<usize> {
        let reach = GRAB * 2. / self.scale();
        self.crop_handles().iter().position(|h| (h.0 - p.0).hypot(h.1 - p.1) <= reach)
    }

    /// What Save produces, in image pixels: the crop, or the picture and
    /// everything drawn around it.
    fn extent(&self) -> (f32, f32, f32, f32) {
        kadr_raster::extent(&self.vm.doc, self.px, !self.frame.clips())
    }

    fn current_color(&self) -> Rgba {
        crate::ui::to_rgba(self.color)
    }

    /// Remember the current state for undo, before changing it.
    fn checkpoint(&mut self) {
        self.vm.checkpoint();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        self.finish_text();
        if self.vm.undo() {
            self.selected = None;
            self.marked.clear();
            self.sync_view(cx);
        }
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        if self.vm.redo() {
            self.selected = None;
            self.marked.clear();
            self.sync_view(cx);
        }
    }

    fn finish_text(&mut self) {
        if let Some(i) = self.editing.take()
            && matches!(&self.vm.doc.shapes[i].kind, Kind::Text(t) if t.trim().is_empty())
        {
            self.vm.doc.shapes.remove(i);
            self.selected = None;
        }
    }

    fn delete_selected(&mut self) {
        if !self.marked.is_empty() {
            self.checkpoint();
            let mut marked = std::mem::take(&mut self.marked);
            marked.sort_unstable();
            for i in marked.into_iter().rev() {
                self.vm.doc.shapes.remove(i);
            }
            self.selected = None;
            return;
        }
        if let Some(i) = self.selected.take() {
            self.checkpoint();
            self.vm.doc.shapes.remove(i);
        }
    }

    /// Topmost shape under the pointer.
    fn shape_at(&self, p: P) -> Option<usize> {
        let tol = GRAB / self.scale();
        self.vm.doc.shapes.iter().rposition(|s| s.hit(p, tol))
    }

    fn handle_at(&self, p: P) -> Option<(usize, usize)> {
        let i = self.selected?;
        let tol = (GRAB + 4.) / self.scale();
        let h = self.vm.doc.shapes[i].knobs(self.scale()).iter().position(|h| (h.0 - p.0).hypot(h.1 - p.1) <= tol)?;
        Some((i, h))
    }

    /// Every shape in the drawing, its undo history and redo.
    fn all_shapes(&self) -> impl Iterator<Item = &Shape> {
        self.vm.all_shapes().chain(self.draft.iter())
    }

    /// Keep a computed patch; ones no version of the drawing has go.
    fn keep_patch(&mut self, key: u64, patch: Arc<Image>) {
        self.patches.insert(key, patch);
        if self.patches.len() > 64 {
            let wanted: std::collections::HashSet<u64> = self
                .all_shapes()
                .filter_map(|sh| match sh.kind {
                    Kind::Blur(how) => Some(patch_key(how, sh.a, sh.b, sh.size)),
                    _ => None,
                })
                .collect();
            self.patches.retain(|k, _| wanted.contains(k));
        }
    }

    /// A laid-on picture as GPUI shows it, made the first time it's drawn.
    fn shown_picture(&self, picture: &Arc<RgbaImage>) -> Option<Arc<Image>> {
        let key = Arc::as_ptr(picture) as usize;
        if let Some((_, shown)) = self.pictures.borrow().get(&key) {
            return Some(shown.clone());
        }
        let mut png = Vec::new();
        picture.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        let shown = Arc::new(Image::from_bytes(ImageFormat::Png, png));
        let mut pictures = self.pictures.borrow_mut();
        if pictures.len() > 16 {
            let wanted: std::collections::HashSet<usize> = self
                .all_shapes()
                .filter_map(|sh| match &sh.kind {
                    Kind::Picture(p) => Some(Arc::as_ptr(p) as usize),
                    _ => None,
                })
                .collect();
            pictures.retain(|k, _| wanted.contains(k));
        }
        pictures.insert(key, (picture.clone(), shown.clone()));
        Some(shown)
    }

    /// Compute the patch for a hidden area in the background.
    fn refresh_patch(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(src) = self.source.clone() else { return };
        let Some((how, a, b, size)) = self.vm.doc.shapes.get(index).and_then(|s| match s.kind {
            Kind::Blur(how) => Some((how, s.a, s.b, s.size)),
            _ => None,
        }) else {
            return;
        };
        let key = patch_key(how, a, b, size);
        if self.patches.contains_key(&key) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let patch = cx.background_executor().spawn(async move { hide_png(&src, a, b, how, size) }).await;
            let _ = this.update(cx, |editor, cx| {
                if let Some(png) = patch {
                    editor.keep_patch(key, Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        // Space held: the pointer pans; a press doesn't draw.
        if self.space {
            return;
        }
        // A typed crop size counts once the canvas is clicked.
        self.apply_size_field();
        let Some(p) = self.to_image(e.position, false) else { return };
        if self.picking {
            self.picking = false;
            self.take_color(p);
            return cx.notify();
        }
        let s = self.scale();

        // Double-click a text to type into it again.
        if e.click_count == 2
            && let Some(i) = self.shape_at(p)
            && matches!(self.vm.doc.shapes[i].kind, Kind::Text(_))
        {
            self.finish_text();
            self.checkpoint();
            self.selected = Some(i);
            self.editing = Some(i);
            if let Kind::Text(t) = &self.vm.doc.shapes[i].kind {
                self.field = kadr_ui::text::Field::new(t);
            }
            return cx.notify();
        }
        self.finish_text();

        // The selection's handles and body work with any tool.
        if let Some((index, handle)) = self.handle_at(p) {
            self.checkpoint();
            let shape = &self.vm.doc.shapes[index];
            self.drag = Some(if shape.scales() { Drag::Scale { index, handle, orig: shape.clone() } } else { Drag::Handle { index, handle } });
            return cx.notify();
        }
        if self.tool == Tool::Crop {
            self.checkpoint();
            // An existing crop is resized at its knobs and moved from inside;
            // elsewhere a new one starts.
            if let Some(orig) = self.vm.doc.crop.map(|(a, b)| rect(a, b)) {
                let handle = self.crop_handle_at(p);
                let inside = p.0 > orig.0 && p.0 < orig.2 && p.1 > orig.1 && p.1 < orig.3;
                if handle.is_some() || inside {
                    let grab = Grab { at: (f32::from(e.position.x), f32::from(e.position.y)), scale: self.scale(), point: p };
                    self.drag = Some(Drag::CropEdit { handle, orig, from: p, grab });
                    return cx.notify();
                }
            }
            self.vm.doc.crop = Some((p, p));
            let grab = Grab { at: (f32::from(e.position.x), f32::from(e.position.y)), scale: self.scale(), point: p };
            self.drag = Some(Drag::Crop { start: p, grab });
            return cx.notify();
        }
        self.marked.clear();
        let grabbed = match self.tool {
            Tool::Select => self.shape_at(p),
            _ => self.selected.filter(|&i| self.vm.doc.shapes[i].hit(p, GRAB / s)),
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
            Tool::Select => {
                self.drag = Some(Drag::Marquee { start: p, now: p });
                return cx.notify();
            }
            Tool::Crop => return cx.notify(),
            Tool::Text => {
                self.checkpoint();
                self.vm.doc.shapes.push(Shape { kind: Kind::Text(String::new()), a: p, b: p, color, size: sized(FONT_SIZES, self.weight) / s, filled: false, text: self.text_style, bend: None });
                self.editing = Some(self.vm.doc.shapes.len() - 1);
                self.selected = self.editing;
                self.field = kadr_ui::text::Field::default();
                return cx.notify();
            }
            Tool::Cursor => {
                self.checkpoint();
                self.vm.doc.shapes.push(Shape { kind: Kind::Cursor(self.cursor_kind), a: p, b: p, color, size: sized(CURSOR_SIZES, self.weight) / s, filled: false, text: TextStyle::Plain, bend: None });
                self.selected = Some(self.vm.doc.shapes.len() - 1);
                return cx.notify();
            }
            Tool::Counter => {
                let n = self.vm.doc.shapes.iter().filter(|s| matches!(s.kind, Kind::Counter(_))).count() as u32 + 1;
                self.checkpoint();
                self.vm.doc.shapes.push(Shape { kind: Kind::Counter(n), a: p, b: p, color, size: sized(COUNTER_SIZES, self.weight) / s, filled: false, text: TextStyle::Plain, bend: None });
                self.selected = Some(self.vm.doc.shapes.len() - 1);
                return cx.notify();
            }
            Tool::Arrow => Kind::Arrow(self.arrow),
            Tool::Rect => Kind::Rect,
            Tool::Ellipse => Kind::Ellipse,
            Tool::Line => Kind::Line,
            Tool::Pen => Kind::Pen(vec![p]),
            Tool::Marker => Kind::Marker(vec![p]),
            Tool::Blur => Kind::Blur(self.hide),
            Tool::Spotlight => Kind::Spotlight,
        };
        let width = if self.tool == Tool::Marker { sized(WIDTHS, self.weight) * 3. } else { sized(WIDTHS, self.weight) };
        let filled = self.fill && matches!(kind, Kind::Rect | Kind::Ellipse);
        let size = if matches!(kind, Kind::Blur(..)) { self.weight } else { width / s };
        self.draft = Some(Shape { kind, a: p, b: p, color, size, filled, text: TextStyle::Plain, bend: None });
        self.drag = Some(Drag::Draw);
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.space {
            let (x, y) = (f32::from(e.position.x), f32::from(e.position.y));
            if let Some((x0, y0)) = self.pan {
                self.view_pan = (self.view_pan.0 + x - x0, self.view_pan.1 + y - y0);
            }
            self.pan = Some((x, y));
            return cx.notify();
        }
        if e.pressed_button != Some(MouseButton::Left) {
            // Show what a press would do: resize at a knob, move over a shape.
            let hover = self.to_image(e.position, false).and_then(|p| {
                if self.tool == Tool::Crop
                    && let Some((l, t, r, b)) = self.vm.doc.crop.map(|(a, b)| rect(a, b))
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
                    let shape = &self.vm.doc.shapes[index];
                    let corners = shape.handles();
                    let (h, mid) = (corners[handle], shape.bounds());
                    let centre = ((mid.0 + mid.2) / 2., (mid.1 + mid.3) / 2.);
                    return Some(match (shape.kind.clone(), handle) {
                        (Kind::Line | Kind::Arrow(_), _) => CursorStyle::Crosshair,
                        (_, 4 | 6) => CursorStyle::ResizeUpDown,
                        (_, 5 | 7) => CursorStyle::ResizeLeftRight,
                        _ if (h.0 - centre.0) * (h.1 - centre.1) > 0. => CursorStyle::ResizeUpLeftDownRight,
                        _ => CursorStyle::ResizeUpRightDownLeft,
                    });
                }
                let over = match self.tool {
                    Tool::Select => self.shape_at(p).is_some(),
                    _ => self.selected.is_some_and(|i| self.vm.doc.shapes[i].hit(p, GRAB / self.scale())),
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
        if let Some(Drag::Crop { grab, .. } | Drag::CropEdit { grab, .. }) = &self.drag {
            let g = *grab;
            let m = work_margin_at(self.px, MAX_REACH);
            let (x, y) = (f32::from(e.position.x), f32::from(e.position.y));
            let p = (
                (g.point.0 + (x - g.at.0) / g.scale).clamp(-m, self.px.0 as f32 + m),
                (g.point.1 + (y - g.at.1) / g.scale).clamp(-m, self.px.1 as f32 + m),
            );
            self.drag_crop(p, e.modifiers.control || e.modifiers.platform);
            return cx.notify();
        }
        let Some(p) = self.to_image(e.position, true) else { return };
        match &mut self.drag {
            Some(Drag::Move { index, last, moved }) => {
                let d = (p.0 - last.0, p.1 - last.1);
                *last = p;
                *moved = true;
                self.vm.doc.shapes[*index].translate(d);
            }
            Some(Drag::Handle { index, handle }) => {
                let (index, handle) = (*index, *handle);
                self.vm.doc.shapes[index].set_handle(handle, p);
            }
            Some(Drag::Scale { index, handle, orig }) => {
                let (index, handle) = (*index, *handle);
                self.vm.doc.shapes[index].scale_from(orig, handle, p);
            }
            Some(Drag::Crop { .. } | Drag::CropEdit { .. }) => {}
            Some(Drag::Marquee { now, .. }) => *now = p,
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
        if self.space {
            return;
        }
        match self.drag.take() {
            Some(Drag::Marquee { start, now }) => {
                let (l, t, r, b) = rect(start, now);
                self.marked = (0..self.vm.doc.shapes.len())
                    .filter(|&i| {
                        let (sl, st, sr, sb) = self.vm.doc.shapes[i].bounds();
                        sl <= r && sr >= l && st <= b && sb >= t
                    })
                    .collect();
                // One shape is a plain selection, with its handles.
                if self.marked.len() == 1 {
                    self.selected = self.marked.pop();
                }
            }
            Some(Drag::Slider(_)) => {}
            // A click without movement isn't an edit.
            Some(Drag::Move { moved: false, .. }) => self.vm.drop_checkpoint(),
            Some(Drag::Move { index, .. } | Drag::Handle { index, .. } | Drag::Scale { index, .. }) => self.refresh_patch(index, cx),
            Some(Drag::Crop { .. } | Drag::CropEdit { .. }) => {
                // The view stays where the crop took it.
                self.view_pan = (self.view_pan.0 + self.crop_shift.0, self.view_pan.1 + self.crop_shift.1);
                self.crop_shift = (0., 0.);
                if let Some((a, b)) = self.vm.doc.crop
                    && ((a.0 - b.0).abs() < 4. || (a.1 - b.1).abs() < 4.)
                {
                    // A click with the crop tool resets the crop.
                    self.vm.doc.crop = None;
                }
            }
            Some(Drag::Draw) => {
                if let Some(shape) = self.draft.take() {
                    // A click, or a stroke that barely moved, draws nothing.
                    let tiny = match &shape.kind {
                        Kind::Pen(_) | Kind::Marker(_) => {
                            let (l, t, r, b) = shape.bounds();
                            let pad = shape.size;
                            r - l - pad < 3. && b - t - pad < 3.
                        }
                        _ => (shape.a.0 - shape.b.0).abs() < 3. && (shape.a.1 - shape.b.1).abs() < 3.,
                    };
                    // What's just drawn isn't selected: the
                    // next stroke starts right away.
                    if !tiny {
                        self.checkpoint();
                        self.vm.doc.shapes.push(shape);
                        let index = self.vm.doc.shapes.len() - 1;
                        self.selected = None;
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

        if self.editing.is_some() {
            // The system's text keys; what types goes on to the input
            // handler (and the input method) below.
            use kadr_platform::keymap::Command as C;
            use kadr_ui::text::Move;
            let f = &mut self.field;
            let is = |c| crate::ui::is_command(k, c);
            let moves = [(C::WordLeft, Move::WordLeft), (C::WordRight, Move::WordRight), (C::LineStart, Move::LineStart), (C::LineEnd, Move::LineEnd)];
            if let Some(&(_, to)) = moves.iter().find(|(c, _)| is(*c)) {
                f.go(to, shift);
            } else if is(C::DeleteWordBack) {
                f.delete_word_back();
            } else if is(C::SelectAll) {
                f.select_all();
            } else if is(C::Paste) {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    f.insert(&text);
                }
            } else {
                match k.key.as_str() {
                    "escape" => self.finish_text(),
                    "enter" if !shift => self.finish_text(),
                    "enter" => f.insert("\n"),
                    "backspace" => f.backspace(),
                    "delete" => f.delete(),
                    "left" => f.go(Move::Left, shift),
                    "right" => f.go(Move::Right, shift),
                    _ => return,
                }
            }
            self.sync_field();
            cx.stop_propagation();
            return cx.notify();
        }

        match (k.key.as_str(), ctrl, shift) {
            // Held Space pans, like in design apps.
            ("space", false, false) => self.space = true,
            // Esc steps back: the colour menu, the selection, then the editor.
            ("escape", _, _) if self.colors_open => self.colors_open = false,
            ("escape", _, _) if self.text_styles_open => self.text_styles_open = false,
            ("escape", _, _) if self.picking => self.picking = false,
            ("i", false, false) if self.cropping.is_none() => self.picking = !self.picking,
            ("escape", _, _) if self.cursors_open => self.cursors_open = false,
            ("escape", _, _) if !self.marked.is_empty() => self.marked.clear(),
            ("escape", _, _) if self.selected.is_some() => self.selected = None,
            ("a", true, _) if self.cropping.is_none() => {
                self.selected = None;
                self.marked = (0..self.vm.doc.shapes.len()).collect();
            }
            // Done with the crop: it stays, the crop tool steps aside.
            ("escape", _, _) if self.crop_menu.is_some() => self.crop_menu = None,
            ("escape", _, _) if self.cropping.is_some() => self.cancel_crop(cx),
            ("enter", _, _) if self.cropping.is_some() => self.apply_crop(),
            ("delete" | "backspace", false, _) if self.cropping.is_some() && self.vm.doc.crop.is_some() => {
                self.checkpoint();
                self.vm.doc.crop = None;
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
                    self.vm.doc.shapes[index].translate(d);
                    self.refresh_patch(index, cx);
                }
            }
            ("z", true, false) => self.undo(cx),
            ("z", true, true) | ("y", true, _) => self.redo(cx),
            ("c", true, _) => self.export(Export::Copy, window, cx),
            ("v", true, _) => self.paste(cx),
            ("=" | "+", true, _) => self.zoom_by(1.25),
            ("-", true, _) => self.zoom_by(0.8),
            ("0", true, _) => self.zoom_at(1., (0., 0.)),
            ("s", true, false) => self.export(Export::Save, window, cx),
            ("s", true, true) => self.save_as(window, cx),
            (key, false, false) => {
                if let Some(tool) = Tool::ALL.into_iter().find(|t| t.key() == key) {
                    // C again in the crop mode is Crop.
                    if tool == Tool::Crop && self.cropping.is_some() {
                        self.apply_crop();
                    } else {
                        self.set_tool(tool, cx);
                    }
                } else if let Some(n) = key.parse::<usize>().ok().filter(|n| (1..=self.colors.len()).contains(n)) {
                    self.pick_color(self.colors[n - 1]);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Toolbar colors and weights also restyle the selected shape.
    fn pick_color(&mut self, color: Hsla) {
        self.color = color;
        if let Some(index) = self.selected {
            self.checkpoint();
            self.vm.doc.shapes[index].color = crate::ui::to_rgba(color);
        }
    }

    /// Take the colour of the picture where it was clicked, and keep it
    /// among the recent ones.
    fn take_color(&mut self, p: P) {
        let Some(src) = &self.source else { return };
        let (x, y) = (p.0.floor(), p.1.floor());
        if x < 0. || y < 0. || x >= src.width() as f32 || y >= src.height() as f32 {
            return;
        }
        let [r, g, b, _] = src.get_pixel(x as u32, y as u32).0;
        let color = crate::ui::from_hex((r as u32) << 16 | (g as u32) << 8 | b as u32);
        self.pick_color(color);
        self.remember_color(color);
    }

    /// Put a colour first among the recent ones (the menu's own colours
    /// aren't kept), in the settings too.
    fn remember_color(&mut self, color: Hsla) {
        let same = |a: Hsla, b: Hsla| crate::ui::to_rgba(a).to_hex() == crate::ui::to_rgba(b).to_hex();
        if self.colors.iter().any(|&c| same(c, color)) {
            return;
        }
        self.recent.retain(|&c| !same(c, color));
        self.recent.insert(0, color);
        self.recent.truncate(kadr_core::settings::RECENT_COLORS);
        let recent = self.recent.iter().map(|&c| crate::ui::to_rgba(c)).collect();
        let mut settings = crate::settings::Settings::load();
        settings.recent_colors = recent;
        settings.save();
    }

    /// Keep the current colour in the menu, or let go of a kept one.
    fn keep_color(&mut self, color: Hsla, keep: bool) {
        let same = |a: Hsla| crate::ui::to_rgba(a).to_hex() == crate::ui::to_rgba(color).to_hex();
        self.saved.retain(|&c| !same(c));
        if keep {
            self.saved.push(color);
            let n = self.saved.len().saturating_sub(kadr_core::settings::SAVED_COLORS);
            self.saved.drain(..n);
        }
        let saved = self.saved.iter().map(|&c| crate::ui::to_rgba(c)).collect();
        let mut settings = crate::settings::Settings::load();
        settings.saved_colors = saved;
        settings.save();
    }

    /// Undo restores the size from before the whole slide, not each step.
    fn pick_weight(&mut self, weight: f32, cx: &mut Context<Self>) {
        self.weight = weight;
        if let Some(index) = self.selected {
            let s = self.scale();
            self.vm.doc.shapes[index].set_weight(weight, s);
            self.refresh_patch(index, cx);
        }
    }

    fn backdrops(&self) -> Vec<(Hsla, Hsla)> {
        palette::backdrops()
    }

    /// Solid or outline for boxes and ellipses; restyles the selection too.
    fn pick_fill(&mut self, fill: bool) {
        self.fill = fill;
        if let Some(index) = self.selected
            && matches!(self.vm.doc.shapes[index].kind, Kind::Rect | Kind::Ellipse)
        {
            self.checkpoint();
            self.vm.doc.shapes[index].filled = fill;
        }
    }

    /// Copy the selection a little down and right, and select the copy.
    fn duplicate(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected else { return };
        self.checkpoint();
        let mut copy = self.vm.doc.shapes[index].clone();
        if let Kind::Counter(_) = copy.kind {
            let n = self.vm.doc.shapes.iter().filter(|s| matches!(s.kind, Kind::Counter(_))).count() as u32 + 1;
            copy.kind = Kind::Counter(n);
        }
        let step = 16. / self.scale();
        copy.translate((step, step));
        self.vm.doc.shapes.push(copy);
        let new = self.vm.doc.shapes.len() - 1;
        self.selected = Some(new);
        self.refresh_patch(new, cx);
    }

    /// Lay a picture over the screenshot, centred, at most half its size,
    /// and select it.
    fn add_picture(&mut self, picture: Arc<RgbaImage>) {
        let (pw, ph) = (picture.width() as f32, picture.height() as f32);
        let (cw, ch) = (self.px.0 as f32, self.px.1 as f32);
        let fit = (cw * 0.5 / pw).min(ch * 0.5 / ph).min(1.);
        let (w, h) = (pw * fit, ph * fit);
        let (x, y) = ((cw - w) / 2., (ch - h) / 2.);
        self.checkpoint();
        self.vm.doc.shapes.push(Shape {
            kind: Kind::Picture(picture),
            a: (x, y),
            b: (x + w, y + h),
            color: self.current_color(),
            size: 0.,
            filled: false,
            text: TextStyle::Plain,
            bend: None,
        });
        self.selected = Some(self.vm.doc.shapes.len() - 1);
        self.apply_crop();
        self.tool = Tool::Select;
    }

    /// Ctrl+V: a copied picture, or a copied image file.
    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else { return };
        let picture = item.entries().iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::Image(image) => decode_picture(image.bytes()),
            gpui::ClipboardEntry::ExternalPaths(paths) => paths.paths().iter().find_map(|p| decode_picture(&std::fs::read(p).ok()?)),
            gpui::ClipboardEntry::String(text) => {
                let line = text.text().lines().next()?.trim();
                let path = line.strip_prefix("file://").unwrap_or(line);
                decode_picture(&std::fs::read(path).ok()?)
            }
        });
        if let Some(picture) = picture {
            self.add_picture(picture);
        }
    }

    /// Files dropped on the editor: each image goes on as a picture.
    fn drop_files(&mut self, paths: &gpui::ExternalPaths) {
        for path in paths.paths() {
            if let Some(picture) = std::fs::read(path).ok().and_then(|bytes| decode_picture(&bytes)) {
                self.add_picture(picture);
            }
        }
    }

    /// Text look; restyles the selected text too.
    /// The arrow tool's style; restyles the selected arrow too.
    fn pick_arrow(&mut self, style: ArrowStyle) {
        self.arrow = style;
        if let Some(index) = self.selected
            && let Kind::Arrow(now) = self.vm.doc.shapes[index].kind
            && now != style
        {
            self.checkpoint();
            self.vm.doc.shapes[index].kind = Kind::Arrow(style);
        }
    }

    fn pick_text_style(&mut self, style: TextStyle) {
        self.text_style = style;
        if let Some(index) = self.selected
            && matches!(self.vm.doc.shapes[index].kind, Kind::Text(_))
        {
            self.checkpoint();
            self.vm.doc.shapes[index].text = style;
        }
    }

    /// The cursor tool's shape; restyles the selected cursor too.
    fn pick_cursor(&mut self, name: &'static str) {
        self.cursor_kind = name;
        if let Some(index) = self.selected
            && matches!(self.vm.doc.shapes[index].kind, Kind::Cursor(_))
        {
            self.checkpoint();
            self.vm.doc.shapes[index].kind = Kind::Cursor(name);
        }
    }

    /// Pixelate or blur; restyles the selected hidden area too.
    fn pick_hide(&mut self, how: Hide, cx: &mut Context<Self>) {
        self.hide = how;
        if let Some(index) = self.selected
            && let Kind::Blur(now) = self.vm.doc.shapes[index].kind
            && now != how
        {
            self.checkpoint();
            self.vm.doc.shapes[index].kind = Kind::Blur(how);
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
        let (color, size) = (self.current_color(), self.weight);
        for (l, t, r, b) in boxes {
            self.vm.doc.shapes.push(Shape { kind: Kind::Blur(self.hide), a: (l, t), b: (r, b), color, size, filled: false, text: TextStyle::Plain, bend: None });
            self.refresh_patch(self.vm.doc.shapes.len() - 1, cx);
        }
        self.selected = None;
    }

    /// Back to the picture as it was before any drawing (undoable).
    fn discard_edits(&mut self, cx: &mut Context<Self>) {
        self.finish_text();
        self.checkpoint();
        self.vm.doc = Doc::default();
        self.selected = None;
        self.sync_view(cx);
    }

    /// Zoom the canvas by `factor`, between fitting and 4×.
    fn zoom_by(&mut self, factor: f32) {
        self.zoom_at(self.zoom * factor, (0., 0.));
    }

    /// Zoom to `zoom`, keeping the point `at` (from the stage's middle, in
    /// window pixels) where it is; fitting the stage centres it again.
    fn zoom_at(&mut self, zoom: f32, at: (f32, f32)) {
        let zoom = zoom.clamp(MIN_ZOOM, 8.);
        let k = zoom / self.zoom;
        let (px_, py) = self.view_pan;
        self.view_pan = if zoom == 1. { (0., 0.) } else { (at.0 - (at.0 - px_) * k, at.1 - (at.1 - py) * k) };
        self.zoom = zoom;
    }

    fn slide(&mut self, knob: Knob, x: f32, cx: &mut Context<Self>) {
        let Some(&(x0, w)) = self.sliders.borrow().get(&knob) else { return };
        let v = ((x - x0) / w.max(1.)).clamp(0., 1.);
        match knob {
            Knob::Padding => self.frame.padding = v * MAX_PAD,
            Knob::Radius => self.frame.radius = v * MAX_ROUND,
            Knob::Shadow => self.frame.shadow = v,
            Knob::Weight => {
                self.pick_weight(v, cx);
                return cx.notify();
            }
        }
        if !self.frame.on() {
            self.frame.backdrop = Backdrop::Gradient(0);
        }
        cx.notify();
    }

    fn pick_wallpaper(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = kadr_platform::dialog::open(Some(window), t("common.choose_background_image"), &[kadr_platform::dialog::Filter { name: t("common.pictures"), extensions: kadr_platform::dialog::PICTURES }], false);
        cx.spawn(async move |this, cx| {
            let Some(paths) = paths.await else { return };
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
        let doc = self.vm.doc.clone();
        let frame = self.frame.clone();
        let (path, copy) = match how {
            Export::SaveAs(path) => (path, false),
            Export::Save => (self.path.clone(), false),
            Export::Copy => (self.path.clone(), true),
        };
        // A save shows the result's card itself.
        if !copy {
            self.card_on_close = false;
        }
        let (original, density) = (self.original.clone(), self.density);
        let bundle = self.bundle.clone();
        window.remove_window();
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let (drawn, framed) = (doc.clone(), frame.clone());
            // The result keeps the original's density, less the image size.
            let shrunk = density.map(|d| d * drawn.scale.min(1.) as f64);
            // Saved as its name says (a copy is always PNG); the project
            // keeps a PNG of the result either way.
            let format = if copy { kadr_raster::Format::Png } else { kadr_raster::Format::of(&path) };
            let rendered = cx
                .background_executor()
                .spawn(async move {
                    let picture = kadr_raster::render(&src, &drawn, &framed)?;
                    let png = kadr_raster::encode(&picture, kadr_raster::Format::Png)?;
                    let file = if format == kadr_raster::Format::Png { None } else { Some(kadr_raster::encode(&picture, format)?) };
                    Some((png, file))
                })
                .await;
            let Some((png, file)) = rendered else {
                return util::notify(t("common.couldnt_save"), &path.display().to_string());
            };
            let png = match shrunk {
                Some(d) => cards::with_density(png, d),
                None => png,
            };
            let file = file.unwrap_or_else(|| png.clone());
            match copy {
                false => {
                    if let Err(e) = std::fs::write(&path, &file) {
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
            let preview = png;
            cx.background_executor()
                .spawn(async move {
                    let output = if copy { std::fs::read(&path).unwrap_or_default() } else { file };
                    // Encoded once, for the .kadr and the editable copy both.
                    let mut base = None;
                    if let Some(bundle) = bundle {
                        match base_png(original.as_deref(), density, &doc) {
                            Ok(png) => {
                                if let Err(e) = project::save_bundle(&bundle, &preview, &png, &doc, &frame) {
                                    util::notify(t("bundle.couldnt_save"), &e.to_string());
                                }
                                base = Some(png);
                            }
                            Err(e) => util::notify(t("bundle.couldnt_save"), &e.to_string()),
                        }
                    }
                    let _ = project::save(&path, &output, || base.map_or_else(|| base_png(original.as_deref(), density, &doc), Ok), &doc, &frame);
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
        let filters = [
            kadr_platform::dialog::Filter { name: "PNG", extensions: &["png"] },
            kadr_platform::dialog::Filter { name: "JPEG", extensions: &["jpg", "jpeg"] },
            kadr_platform::dialog::Filter { name: "WebP", extensions: &["webp"] },
            kadr_platform::dialog::Filter { name: t("bundle.project"), extensions: &["kadr"] },
        ];
        let picked = kadr_platform::dialog::save(Some(window), t("editor.save_as"), &dir, name.as_deref(), &filters);
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = picked.await else { return };
            // A name ending in .kadr saves an editable project there.
            let project = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("kadr"));
            let _ = this.update_in(cx, |editor, window, cx| {
                if project {
                    editor.bundle = Some(path);
                    editor.save_project(cx);
                } else {
                    editor.export(Export::SaveAs(path), window, cx);
                }
            });
        })
        .detach();
    }

    /// Pin the annotated picture above the windows; the editor stays.
    fn pin(&mut self, cx: &mut Context<Self>) {
        self.finish_text();
        let Some(src) = self.source.clone() else {
            return util::notify("kadr", t("editor.image_still_loading_try_again"));
        };
        let (doc, frame) = (self.vm.doc.clone(), self.frame.clone());
        let name = format!("kadr-pin-{}-{}.png", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()));
        let out = util::runtime_dir().join(name);
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let png = cx.background_executor().spawn(async move { kadr_raster::render_png(&src, &doc, &frame) }).await;
            if let Some(png) = png
                && std::fs::write(&out, png).is_ok()
            {
                let _ = cx.update(|cx| crate::pin::open(out, cx));
            }
        })
        .detach();
    }

    /// A picture from a file, put on top like a dropped one.
    fn pick_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = kadr_platform::dialog::open(Some(window), t("editor.choose_picture"), &[kadr_platform::dialog::Filter { name: t("common.pictures"), extensions: kadr_platform::dialog::PICTURES }], true);
        cx.spawn(async move |this, cx| {
            let Some(paths) = paths.await else { return };
            let _ = this.update(cx, |editor, cx| {
                for path in paths {
                    if let Some(picture) = std::fs::read(&path).ok().and_then(|bytes| decode_picture(&bytes)) {
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
        let (doc, frame, original, density) = (self.vm.doc.clone(), self.frame.clone(), self.original.clone(), self.density);
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let result = cx
                .background_executor()
                .spawn(async move {
                    let png = kadr_raster::render_png(&src, &doc, &frame).ok_or_else(|| std::io::Error::other(t("common.couldnt_save")))?;
                    project::save_bundle(&target, &png, &base_png(original.as_deref(), density, &doc)?, &doc, &frame).map(|_| target)
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
        let crop = self.vm.doc.crop;
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let tmp = util::runtime_dir().join(format!("kadr-ocr-editor-{}.png", std::process::id()));
            let doc = Doc { shapes: Vec::new(), crop, ..Doc::default() };
            let png = cx.background_executor().spawn(async move { kadr_raster::render_png(&src, &doc, &Frame::default()) }).await;
            if let Some(png) = png
                && std::fs::write(&tmp, png).is_ok()
            {
                let _ = cx.update(|cx| ocr::image(tmp, cx));
            }
        })
        .detach();
    }

    /// The top bar: history on the left, tools in the middle, the
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
                .tip(Tooltip::text(format!("{} ({})", tool.label(), tool.key().to_uppercase())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_tool(tool, cx);
                    cx.notify();
                }))
        };
        // The crop is a mode of its own, beside the background at the left.
        let groups: Vec<&[Tool]> = Tool::GROUPS.iter().copied().filter(|group| !group.contains(&Tool::Crop)).collect();

        let ring = cx.theme().colors().text;
        let current = crate::ui::from_rgba(self.current_color());
        // Colours sit in a dropdown under one swatch of the current colour:
        // kadr's own and the pipette, then the ones picked lately, then the
        // ones kept, with + to keep the current one.
        let hex = |c: Hsla| crate::ui::to_rgba(c).to_hex();
        let now = hex(self.color);
        let swatch = |id: (&'static str, usize), c: Hsla, tip: String| {
            div()
                .id(id)
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_2()
                .border_color(if hex(c) == now { ring } else { gpui::transparent_black() })
                .cursor_pointer()
                .child(div().size(px(20.)).rounded_full().bg(c).border_1().border_color(cx.theme().colors().border))
                .tip(Tooltip::text(tip))
        };
        let own = self.colors.iter().enumerate().map(|(i, &c)| {
            swatch(("color", i), c, tr!("editor.color_n", i + 1)).on_click(cx.listener(move |this, _, _, cx| {
                this.pick_color(c);
                this.colors_open = false;
                cx.notify();
            }))
        });
        let pipette = ButtonLike::new("color-pipette")
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Large)
            .square()
            .toggle_state(self.picking)
            .child(Icon::from_path("icons/kadr/pipette.svg").size(IconSize::Small))
            .tip(Tooltip::text(t("editor.pick_color_tip")))
            .on_click(cx.listener(|this, _, _, cx| {
                this.picking = true;
                this.colors_open = false;
                cx.notify();
            }));
        let recent = self.recent.iter().enumerate().map(|(i, &c)| {
            swatch(("color-recent", i), c, hex(c)).on_click(cx.listener(move |this, _, _, cx| {
                this.pick_color(c);
                this.colors_open = false;
                cx.notify();
            }))
        });
        let saved = self.saved.iter().enumerate().map(|(i, &c)| {
            swatch(("color-saved", i), c, tr!("editor.saved_color_tip", hex(c)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pick_color(c);
                    this.colors_open = false;
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| {
                    this.keep_color(c, false);
                    cx.notify();
                }))
        });
        let kept = self.colors.iter().chain(&self.saved).any(|&c| hex(c) == now);
        let keep = ButtonLike::new("color-keep")
            .style(ButtonStyle::Subtle)
            .size(ButtonSize::Large)
            .square()
            .disabled(kept)
            .child(Icon::new(IconName::Plus).size(IconSize::Small).color(Color::Muted))
            .tip(Tooltip::text(t("editor.keep_color_tip")))
            .on_click(cx.listener(|this, _, _, cx| {
                this.keep_color(this.color, true);
                cx.notify();
            }));
        let heading = |text: &'static str| Label::new(text).size(LabelSize::XSmall).color(Color::Muted);
        let row = || h_flex().flex_wrap().gap_1();
        let swatches = v_flex()
            // As wide as kadr's colours and the pipette; the other rows wrap.
            .w(px(8. * 28. + 36. + 8. * 4.))
            .gap_1p5()
            .child(h_flex().gap_1().children(own).child(pipette))
            .when(!self.recent.is_empty(), |d| d.child(heading(t("editor.recent_colors"))).child(row().children(recent)))
            .child(heading(t("editor.saved_colors")))
            .child(row().children(saved).child(keep));
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
                .tip(Tooltip::text(t("editor.color_1_7")))
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
                    .p_2()
                    .elevation_3(cx)
                    .child(swatches)
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
            .tip(Tooltip::text(t("common.size")));
        // The modes: the background first, then crop and a picture on top.
        let mode = |id: &'static str, icon: Icon, on: bool, tip: SharedString| {
            ButtonLike::new(id).style(ButtonStyle::Subtle).size(ButtonSize::Large).square().toggle_state(on).child(icon).tip(Tooltip::text(tip))
        };
        let modes = h_flex()
            .gap_0p5()
            .child(
                mode(
                    "mode-background",
                    Icon::from_path("icons/kadr/frame.svg").size(IconSize::Tool),
                    self.panel,
                    t("editor.background_padding_corners_shadow_format").into(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.panel = !this.panel;
                    cx.notify();
                })),
            )
            .child(mode("mode-crop", Icon::from_path("icons/kadr/crop.svg").size(IconSize::Tool), false, t("editor.crop_tip").into()).on_click(cx.listener(|this, _, _, cx| {
                this.set_tool(Tool::Crop, cx);
                cx.notify();
            })))
            .child(mode("mode-picture", Icon::from_path("icons/image_plus.svg").size(IconSize::Tool), false, tr!("editor.add_picture_tip", crate::ui::shortcut(kadr_platform::keymap::Command::Paste)).into()).on_click(cx.listener(|this, _, window, cx| this.pick_picture(window, cx))));

        let history = h_flex()
            .gap_0p5()
            .child(
                IconButton::new("undo", IconName::Undo)
                    .shape(IconButtonShape::Square)
                    .size(ButtonSize::Large)
                    .disabled(self.vm.history.depth() == 0)
                    .tip(Tooltip::text(tr!("editor.undo_tip", crate::ui::shortcut(kadr_platform::keymap::Command::Undo))))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.undo(cx);
                        cx.notify();
                    })),
            )
            .child(
                ButtonLike::new("redo")
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Large)
                    .disabled(self.vm.history.redo_depth() == 0)
                    .child(div().px_1().child(Icon::from_path("icons/kadr/redo.svg").size(IconSize::Small).color(if self.vm.history.redo_depth() == 0 { Color::Disabled } else { Color::Default })))
                    .tip(Tooltip::text(tr!("editor.redo_tip", crate::ui::shortcut(kadr_platform::keymap::Command::Redo))))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.redo(cx);
                        cx.notify();
                    })),
            );
        // Solid or outline, for boxes and ellipses.
        let shapes_fill = matches!(self.tool, Tool::Rect | Tool::Ellipse)
            || self.selected.and_then(|i| self.vm.doc.shapes.get(i)).is_some_and(|sh| matches!(sh.kind, Kind::Rect | Kind::Ellipse));
        let fill_toggle = shapes_fill.then(|| {
            let option = |id: &'static str, solid: bool, icon: &'static str, tip: &'static str| {
                ButtonLike::new(id)
                    .style(if self.fill == solid { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(self.fill == solid)
                    .child(div().px_1().child(Icon::from_path(icon).size(IconSize::Small)))
                    .tip(Tooltip::text(tip))
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
        let texts = matches!(self.tool, Tool::Text) || self.selected.and_then(|i| self.vm.doc.shapes.get(i)).is_some_and(|sh| matches!(sh.kind, Kind::Text(_)));
        // Text styles in a dropdown under a sample of the current one,
        // each a sample in the current colour.
        let text_options = texts.then(|| {
            let picked = self.selected.and_then(|i| self.vm.doc.shapes.get(i)).filter(|sh| matches!(sh.kind, Kind::Text(_))).map(|sh| (sh.text, sh.color));
            let (current, color) = picked.unwrap_or((self.text_style, self.current_color()));
            let options = TextStyle::ALL.into_iter().map(|style| {
                ButtonLike::new(SharedString::from(format!("text-{}", style.key())))
                    .style(if current == style { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(current == style)
                    .child(div().px_1().child(text_sample(style, color)))
                    .tip(Tooltip::text(text_style_name(style)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_text_style(style);
                        this.text_styles_open = false;
                        cx.notify();
                    }))
            });
            h_flex()
                .relative()
                .on_mouse_down_out(cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    if this.text_styles_open && !this.text_styles_menu.get().contains(&e.position) {
                        this.text_styles_open = false;
                        cx.notify();
                    }
                }))
                .child(
                    ButtonLike::new("text-style")
                        .style(if self.text_styles_open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                        .size(ButtonSize::Large)
                        .toggle_state(self.text_styles_open)
                        .child(h_flex().gap_1().px_1().child(text_sample(current, color)).child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted)))
                        .tip(Tooltip::text(text_style_name(current)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.text_styles_open = !this.text_styles_open;
                            cx.notify();
                        })),
                )
                .when(self.text_styles_open, |d| {
                    d.child(gpui::deferred(
                        h_flex()
                            .id("text-style-menu")
                            .occlude()
                            .absolute()
                            .top(px(38.))
                            .right_0()
                            .gap_0p5()
                            .p_1p5()
                            .elevation_3(cx)
                            .children(options)
                            .child(crate::ui::track_bounds(&self.text_styles_menu)),
                    ))
                })
                .child(Divider::vertical().h_5().mx_1())
        });
        // The arrow's style, for the arrow tool or a selected arrow.
        let picked_arrow = self.selected.and_then(|i| self.vm.doc.shapes.get(i)).and_then(|sh| match sh.kind {
            Kind::Arrow(style) => Some(style),
            _ => None,
        });
        let arrow_options = (self.tool == Tool::Arrow || picked_arrow.is_some()).then(|| {
            let current = picked_arrow.unwrap_or(self.arrow);
            let option = |style: ArrowStyle, tip: &'static str| {
                ButtonLike::new(SharedString::from(format!("arrow-{}", style.key())))
                    .style(if current == style { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(current == style)
                    .child(div().px_1().child(Icon::from_path(SharedString::from(format!("icons/kadr/arrow_{}.svg", style.key()))).size(IconSize::Small)))
                    .tip(Tooltip::text(tip))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_arrow(style);
                        cx.notify();
                    }))
            };
            h_flex()
                .gap_0p5()
                .child(option(ArrowStyle::Solid, t("editor.arrow_solid")))
                .child(option(ArrowStyle::Open, t("editor.arrow_open")))
                .child(option(ArrowStyle::Double, t("editor.arrow_double")))
                .child(option(ArrowStyle::Tapered, t("editor.arrow_tapered")))
                .child(Divider::vertical().h_5().mx_1())
        });
        // Which pointer, for the cursor tool or a selected cursor.
        let picked_cursor = self.selected.and_then(|i| self.vm.doc.shapes.get(i)).and_then(|sh| match sh.kind {
            Kind::Cursor(name) => Some(name),
            _ => None,
        });
        // In a dropdown under the current one, like the colours.
        let cursor_options = (self.tool == Tool::Cursor || picked_cursor.is_some()).then(|| {
            let current = picked_cursor.unwrap_or(self.cursor_kind);
            let icon = |name: &str| {
                self.cursor_icons.iter().find(|(n, _)| *n == name).map(|(_, image)| img(gpui::ImageSource::Render(image.clone())).size(px(22.)).object_fit(ObjectFit::Contain))
            };
            let options = self.cursor_icons.iter().map(|(name, image)| {
                let name = *name;
                ButtonLike::new(("cursor-kind", name.as_ptr() as usize))
                    .style(if current == name { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(current == name)
                    .child(img(gpui::ImageSource::Render(image.clone())).size(px(22.)).object_fit(ObjectFit::Contain))
                    .tip(Tooltip::text(cursor_label(name)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_cursor(name);
                        this.cursors_open = false;
                        cx.notify();
                    }))
            }).collect::<Vec<_>>();
            h_flex()
                .child(
                    div()
                        .relative()
                        .on_mouse_down_out(cx.listener(|this, e: &MouseDownEvent, _, cx| {
                            if this.cursors_open && !this.cursors_menu.get().contains(&e.position) {
                                this.cursors_open = false;
                                cx.notify();
                            }
                        }))
                        .child(
                            ButtonLike::new("cursor-kinds")
                                .style(if self.cursors_open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                                .size(ButtonSize::Large)
                                .toggle_state(self.cursors_open)
                                .child(h_flex().gap_0p5().children(icon(current)).child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted)))
                                .tip(Tooltip::text(cursor_label(current)))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cursors_open = !this.cursors_open;
                                    cx.notify();
                                })),
                        )
                        .when(self.cursors_open, |d| {
                            d.child(gpui::deferred(
                                h_flex()
                                    .id("cursor-menu")
                                    .occlude()
                                    .absolute()
                                    .top(px(38.))
                                    .right_0()
                                    .gap_0p5()
                                    .p_1()
                                    .elevation_3(cx)
                                    .children(options)
                                    .child(crate::ui::track_bounds(&self.cursors_menu)),
                            ))
                        }),
                )
                .child(Divider::vertical().h_5().mx_1())
        });
        // Pixelate or blur, and hiding what looks private, for the hide tool.
        let hiding = self.tool == Tool::Blur || self.selected.and_then(|i| self.vm.doc.shapes.get(i)).is_some_and(|sh| matches!(sh.kind, Kind::Blur(..)));
        let hide_options = hiding.then(|| {
            let option = |id: &'static str, how: Hide, icon: &'static str, tip: &'static str| {
                ButtonLike::new(id)
                    .style(if self.hide == how { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(self.hide == how)
                    .child(div().px_1().child(Icon::from_path(icon).size(IconSize::Small)))
                    .tip(Tooltip::text(tip))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick_hide(how, cx);
                        cx.notify();
                    }))
            };
            h_flex()
                .gap_0p5()
                .child(option("hide-pixelate", Hide::Pixelate, "icons/kadr/pixelate.svg", t("editor.pixelate")))
                .child(option("hide-blur", Hide::Blur, "icons/kadr/blur.svg", t("editor.blur")))
                .child(option("hide-redact", Hide::Redact, "icons/kadr/redact.svg", t("editor.redact_tip")))
                .child({
                    // Short of room the label goes, the tooltip still says it.
                    let tip = Tooltip::text(t("editor.hide_sensitive_tip"));
                    let button = if width >= WIDE_TOOLBAR {
                        Button::new("hide-sensitive", if self.redacting { t("editor.looking") } else { t("editor.hide_sensitive") })
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Large)
                            .start_icon(Icon::from_path("icons/kadr/scan_text.svg").size(IconSize::Small))
                            .disabled(self.redacting || self.source.is_none())
                            .tip(tip)
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
                            .tip(tip)
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
        // What fits, from the widths of the bar's parts (measured, in rems
        // so they scale with the UI). Short of room, the style controls fold
        // into a "⋯" menu first, then the tools that don't fit go under a
        // chevron, last ones first; the window's minimum size keeps a few.
        let rem = kit::REM;
        let (button, g05, g1, g2) = (2. * rem, 0.125 * rem, 0.25 * rem, 0.5 * rem);
        let controls = |right: bool| match crate::platform::window_controls() {
            crate::platform::WindowControls::Left(w) if !right => w + g1,
            crate::platform::WindowControls::Right(w) if right => w,
            crate::platform::WindowControls::Own if right => button,
            _ => 0.,
        };
        // The modes, a spacer and undo/redo (redo's icon has a little more
        // room), in the bar's inset.
        let left_w = g1 + controls(false) + 5. * button + 3. * g05 + 3. * g1 + 13.;
        let hide_w = if width >= WIDE_TOOLBAR { 13. } else { 7.9 };
        let options: f32 = [(shapes_fill, 5.4), (texts, 9.), (cursor_options.is_some(), 3.6), (hide_options.is_some(), hide_w)].iter().filter(|o| o.0).map(|o| o.1 * rem).sum();
        let right_w = |compact: bool| (if compact { button } else { 10.8 * rem + options }) + g2 + controls(true) + g1;
        let flat: Vec<(usize, Tool)> = groups.iter().enumerate().flat_map(|(g, group)| group.iter().map(move |&t| (g, t))).collect();
        let tools_w = |n: usize| flat.iter().take(n).enumerate().map(|(i, &(g, _))| button + if i == 0 { 0. } else if g != flat[i - 1].0 { 6. } else { g05 }).sum::<f32>();
        let room = |compact: bool| width - left_w - right_w(compact) - 2. * g2;
        let compact = room(false) < tools_w(flat.len());
        let chevron_w = button + 0.75 * rem;
        // In the middle while the wider side leaves them room, else right
        // after the left side.
        let centred = tools_w(flat.len()) <= width - 2. * left_w.max(right_w(compact)) - 2. * g2;
        let shown = if tools_w(flat.len()) <= room(compact) {
            flat.len()
        } else {
            (0..flat.len()).rev().find(|&n| tools_w(n) + 6. + chevron_w <= room(compact)).unwrap_or(0)
        };
        // Bare icons, only the chosen one filled; the groups apart with a
        // little space.
        let mut rows: Vec<Vec<Tool>> = Vec::new();
        for (i, &(g, t)) in flat.iter().take(shown).enumerate() {
            if i == 0 || g != flat[i - 1].0 {
                rows.push(Vec::new());
            }
            rows.last_mut().expect("a group").push(t);
        }
        let hidden: Vec<Tool> = flat.iter().skip(shown).map(|&(_, t)| t).collect();
        let overflow = (!hidden.is_empty()).then(|| {
            let open = self.crop_menu == Some(CropMenu::Tools);
            // The chosen tool shows on the chevron when it's one of those
            // under it.
            let current = hidden.contains(&self.tool).then_some(self.tool);
            let items: Vec<MenuItem> = hidden
                .iter()
                .map(|&t| (format!("{} ({})", t.label(), t.key().to_uppercase()), t == self.tool, Rc::new(move |this: &mut Editor, _: &mut Window, cx: &mut Context<Editor>| this.set_tool(t, cx)) as MenuPick))
                .collect();
            let icons = hidden.iter().map(|t| Some(t.icon())).collect();
            let native = items.clone();
            let menu = open.then(|| self.menu_list(items, icons, false, true, cx));
            self.menu_anchor(CropMenu::Tools, cx)
                .child(
                    ButtonLike::new("tools-more")
                        .style(if current.is_some() { ButtonStyle::Accent } else if open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                        .size(ButtonSize::Large)
                        .toggle_state(open)
                        .child(
                            h_flex()
                                .gap_0p5()
                                .px_1()
                                .children(current.map(|t| t.icon().size(IconSize::Tool).color(Color::Custom(gpui::white()))))
                                .child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(if current.is_some() { Color::Custom(gpui::white()) } else { Color::Muted })),
                        )
                        .when(!open, |b| b.tip(Tooltip::text(t("editor.more_tools"))))
                        .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                            if Editor::native_menu(&native, e, window, cx) {
                                return;
                            }
                            this.crop_menu = if this.crop_menu == Some(CropMenu::Tools) { None } else { Some(CropMenu::Tools) };
                            cx.notify();
                        })),
                )
                .children(menu)
        });
        let tools = h_flex().gap(px(6.)).children(rows.into_iter().map(|row| h_flex().gap_0p5().children(row.into_iter().map(|t| tool_button(t, cx))))).children(overflow);
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
                        .children(arrow_options)
                        .children(cursor_options)
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
            h_flex().gap_0p5().children(hide_options).children(arrow_options).children(cursor_options).children(text_options).children(fill_toggle).child(palette).child(weights)
        };

        // The window's controls (or room for them), the modes and history at
        // the left; the tools in the middle; their style at the right.
        let left = h_flex()
            .when(centred, |d| d.flex_1())
            .min_w_0()
            .h_full()
            .gap_1()
            .children(kit::window_controls(false, cx))
            .child(modes)
            .child(div().w(px(6.)))
            .child(history)
            .child(kit::drags_window(div().flex_1().h_full()));
        let right = h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .justify_end()
            .child(kit::drags_window(div().flex_1().h_full()))
            .child(h_flex().pr_2().child(style))
            .children(kit::window_controls(true, cx));
        self.top_bar(width, cx).child(left).child(h_flex().flex_none().child(tools)).child(right).into_any_element()
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
        let mut zooms: Vec<MenuItem> = vec![(t("editor.fit").into(), self.zoom == 1., Rc::new(|this: &mut Editor, _: &mut Window, _: &mut Context<Editor>| this.zoom_at(1., (0., 0.))) as MenuPick)];
        zooms.extend([0.25f32, 0.5, 1., 2.].map(|k| (percent(k), false, Rc::new(move |this: &mut Editor, _: &mut Window, _: &mut Context<Editor>| this.zoom_to_real(k)) as MenuPick)));
        self.crop_dropdown(CropMenu::Zoom, percent(self.disp.0 / self.real.0), t("editor.zoom"), zooms, true, cx)
    }

    /// The bottom bar.
    fn render_footer(&self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        if self.cropping.is_some() {
            return self.render_crop_footer(width, cx);
        }
        // Drag the annotated picture out, rendered at the moment the drag starts.
        let drag_state = self.source.clone().map(|src| (src, self.vm.doc.clone(), self.frame.clone(), self.path.clone()));
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
            .tip(Tooltip::text(t("editor.drag_annotated_image_into_chat")))
            .on_drag(DragOut, |_: &DragOut, _, _, cx| cx.new(|_| DragPreview))
            .external_drag_payload(move |_: &DragOut, _, _| {
                let (src, doc, frame, path) = drag_state.as_ref()?;
                let png = kadr_raster::render_png(src, doc, frame)?;
                let dir = util::runtime_dir().join("kadr-drag");
                let _ = std::fs::create_dir_all(&dir);
                let out = dir.join(path.file_name()?);
                std::fs::write(&out, png).ok()?;
                Some(gpui::ExternalDragPayload::Files(gpui::FileDragPaths::new([(out, false)])))
            });

        // What fits, from the sides' widths as last drawn: the drag-out sits
        // in the middle while the wider side leaves it room, else right
        // after the zoom; short of room for everything, the actions fold
        // into a "⋯" menu.
        let rem = kit::REM;
        let (button, g05, g1, g2) = (2. * rem, 0.125 * rem, 0.25 * rem, 0.5 * rem);
        let drag_w = if width >= COMPACT_TOOLBAR { 6. * rem } else { 2.6 * rem };
        let (left_w, right_full) = (self.footer_left.get(), self.footer_right.get());
        let edges = 2. * g2 + 2. * g1;
        let fold = right_full > 0. && left_w + drag_w + right_full + edges > width;
        let right_w = if fold { right_full - 3. * (button + g05) } else { right_full };
        let centred = drag_w <= width - 2. * left_w.max(right_w) - edges;

        let action = |id: &'static str, icon: Icon, tip: String| ButtonLike::new(id).style(ButtonStyle::Subtle).size(ButtonSize::Large).square().child(icon).tip(Tooltip::text(tip));
        let more = fold.then(|| {
            let open = self.crop_menu == Some(CropMenu::Actions);
            let pick = |f: fn(&mut Editor, &mut Window, &mut Context<Editor>)| Rc::new(f) as MenuPick;
            let items: Vec<MenuItem> = vec![
                (t("editor.copy_the_text").into(), false, pick(|this, _, cx| this.read_text(cx))),
                (t("editor.pin").into(), false, pick(|this, _, cx| this.pin(cx))),
                (t("bundle.save_project_tip").into(), false, pick(|this, _, cx| this.save_project(cx))),
                (format!("{} ({})", t("common.copy"), crate::ui::shortcut(kadr_platform::keymap::Command::Copy)), false, pick(|this, window, cx| this.export(Export::Copy, window, cx))),
            ];
            let icons = vec![
                Some(Icon::from_path("icons/kadr/scan_text.svg")),
                Some(Icon::new(IconName::Pin)),
                Some(Icon::from_path("icons/kadr/project.svg")),
                Some(Icon::from_path("icons/copy.svg")),
            ];
            let native = items.clone();
            let menu = open.then(|| self.menu_list(items, icons, true, true, cx));
            self.menu_anchor(CropMenu::Actions, cx)
                .child(
                    ButtonLike::new("actions-more")
                        .style(if open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                        .size(ButtonSize::Large)
                        .square()
                        .toggle_state(open)
                        .child(Icon::from_path("icons/kadr/more.svg").size(IconSize::Medium))
                        .when(!open, |b| b.tip(Tooltip::text(t("editor.more_actions"))))
                        .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                            if Editor::native_menu(&native, e, window, cx) {
                                return;
                            }
                            this.crop_menu = if this.crop_menu == Some(CropMenu::Actions) { None } else { Some(CropMenu::Actions) };
                            cx.notify();
                        })),
                )
                .children(menu)
        });
        let actions = h_flex()
            .gap_0p5()
            .child(action("ocr", Icon::from_path("icons/kadr/scan_text.svg").size(IconSize::Medium), t("editor.copy_text_tip").into()).on_click(cx.listener(|this, _, _, cx| this.read_text(cx))))
            .child(action("pin", Icon::new(IconName::Pin).size(IconSize::Medium), t("editor.pin").into()).on_click(cx.listener(|this, _, _, cx| this.pin(cx))))
            .child(action("save-project", Icon::from_path("icons/kadr/project.svg").size(IconSize::Medium), t("bundle.save_project_tip").into()).on_click(cx.listener(|this, _, _, cx| this.save_project(cx))))
            .child(
                action("copy", Icon::from_path("icons/copy.svg").size(IconSize::Medium), format!("{} ({})", t("common.copy"), crate::ui::shortcut(kadr_platform::keymap::Command::Copy)))
                    .on_click(cx.listener(|this, _, window, cx| this.export(Export::Copy, window, cx))),
            );

        // Save as a quieter button before Save (a project has its own above).
        let save = h_flex()
            .gap_1()
            .child(
                Button::new("save-as", t("editor.save_as"))
                    .style(ButtonStyle::Filled)
                    .size(ButtonSize::Large)
                    .on_click(cx.listener(|this, _, window, cx| this.save_as(window, cx))),
            )
            .child(
                Button::new("save", t("common.save"))
                    .style(ButtonStyle::Accent)
                    .size(ButtonSize::Large)
                    .when(width >= COMPACT_TOOLBAR, |b| b.key_binding(kit::keys("secondary-s")))
                    .on_click(cx.listener(|this, _, window, cx| this.export(Export::Save, window, cx))),
            );

        // Zoom (and undoing a crop or a reopened drawing) at the left, the
        // drag-out in the middle (or after them), the results at the right.
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
                h_flex().when(centred, |d| d.flex_1()).min_w_0().child(
                h_flex()
                    .flex_none()
                    .relative()
                    .gap_1()
                    .child(self.zoom_menu(cx))
                    .when(self.vm.doc.crop.is_some(), |row| {
                        row.child(
                            Button::new("uncrop", t("editor.uncrop")).style(ButtonStyle::Subtle).size(ButtonSize::Large).color(Color::Muted).on_click(cx.listener(|this, _, _, cx| {
                                this.checkpoint();
                                this.vm.doc.crop = None;
                                cx.notify();
                            })),
                        )
                    })
                    // A reopened drawing can be dropped for the clean picture.
                    .when(self.restored && (!self.vm.doc.shapes.is_empty() || self.vm.doc.crop.is_some()), |row| {
                        row.child(
                            Button::new("discard-edits", t("editor.start_over"))
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Large)
                                .color(Color::Muted)
                                .tip(Tooltip::text(t("editor.start_over_tip")))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.discard_edits(cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .child(crate::ui::track_width(&self.footer_left)),
                ),
            )
            .child(drag_out.flex_none())
            .child(
                h_flex().flex_1().min_w_0().justify_end().child(
                    h_flex()
                        .flex_none()
                        .relative()
                        .gap_2()
                        .when(!fold, |d| d.child(actions).child(crate::ui::track_width(&self.footer_right)))
                        .children(more)
                        .child(save),
                ),
            )
            .into_any_element()
    }

    /// A dropdown's open list: the choices, under its button (or over it,
    /// in the footer), from its left edge or its right one.
    fn menu_popup(&self, items: Vec<MenuItem>, up: bool, right: bool, cx: &mut Context<Self>) -> AnyElement {
        self.menu_list(items, Vec::new(), up, right, cx)
    }

    /// `menu_popup` with an icon before each choice that has one; the
    /// current one is marked at the right.
    fn menu_list(&self, items: Vec<MenuItem>, icons: Vec<Option<Icon>>, up: bool, right: bool, cx: &mut Context<Self>) -> AnyElement {
        let hover = cx.theme().colors().element_hover;
        let mut icons = icons.into_iter();
        let rows: Vec<_> = items
            .into_iter()
            .enumerate()
            .map(|(i, (text, on, pick))| {
                let icon = icons.next().flatten();
                let with_icon = icon.is_some();
                let check = || Icon::new(IconName::Check).size(IconSize::XSmall);
                h_flex()
                    .id(SharedString::from(format!("menu-row-{i}")))
                    .w_full()
                    .gap_1p5()
                    .px_2()
                    .h(px(28.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(move |d| d.bg(hover))
                    .when(!with_icon, |row| row.child(div().w(px(14.)).children(on.then(check))))
                    .when(with_icon, |row| row.pr_1())
                    .children(icon.map(|icon| icon.size(IconSize::Small)))
                    .child(Label::new(text).size(LabelSize::Small))
                    .when(with_icon, |row| row.child(div().flex_1()).child(div().w(px(14.)).children(on.then(check))))
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
        let native = items.clone();
        let menu = open.then(|| self.menu_popup(items, up, false, cx));
        self.menu_anchor(which, cx)
            .child(
                ButtonLike::new(match which {
                    CropMenu::Aspect => "dropdown-aspect",
                    CropMenu::Scale => "dropdown-scale",
                    CropMenu::Zoom => "dropdown-zoom",
                    CropMenu::Tools | CropMenu::Actions | CropMenu::Fill => "dropdown",
                })
                    .style(if open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(open)
                    .child(h_flex().gap_1().px_1().child(Label::new(label)).child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted)))
                    .tip(Tooltip::text(tip))
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                        if Editor::native_menu(&native, e, window, cx) {
                            return;
                        }
                        this.crop_menu = if this.crop_menu == Some(which) { None } else { Some(which) };
                        cx.notify();
                    })),
            )
            .children(menu)
            .into_any_element()
    }

    /// A dropdown's choices as the system's menu, where it has one.
    fn native_menu(items: &[MenuItem], e: &gpui::ClickEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let picks: Vec<MenuPick> = items.iter().map(|(_, _, pick)| pick.clone()).collect();
        let entries = items.iter().map(|(text, on, _)| kadr_platform::menu::Item::new(text.clone(), *on)).collect();
        crate::ui::native_menu(window, cx, e.position(), entries, move |this, i, window, cx| {
            if let Some(pick) = picks.get(i) {
                pick(this, window, cx);
            }
        })
    }

    /// The crop's fill: a button with its dot, and swatches under it (see-
    /// through, white, black, then the pen colours).
    fn fill_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let (ring, edge) = (colors.text_accent, colors.text_muted);
        let rgba = |c: Hsla| {
            let c = crate::ui::rgb_of(c);
            [c.r, c.g, c.b, c.a].map(|v| (v * 255.).round() as u8)
        };
        let hsla = |[r, g, b, a]: [u8; 4]| {
            let mut c = crate::ui::from_hex((r as u32) << 16 | (g as u32) << 8 | b as u32);
            c.alpha = a as f32 / 255.;
            c
        };
        // A see-through fill shows as a struck-out circle.
        let dot = move |fill: Option<[u8; 4]>, size: f32| match fill {
            Some(f) => div().size(px(size)).rounded_full().bg(hsla(f)).border_1().border_color(edge).into_any_element(),
            None => div()
                .size(px(size))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(edge)
                .child(Icon::new(IconName::Slash).size(IconSize::XSmall).color(Color::Muted))
                .into_any_element(),
        };
        let mut choices: Vec<Option<[u8; 4]>> = vec![None, Some([255, 255, 255, 255]), Some([0, 0, 0, 255])];
        for &c in &self.colors {
            let c = Some(rgba(c));
            if !choices.contains(&c) {
                choices.push(c);
            }
        }
        let open = self.crop_menu == Some(CropMenu::Fill);
        let swatches = choices.into_iter().enumerate().map(|(i, fill)| {
            div()
                .id(("fill", i))
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_2()
                .border_color(if fill == self.vm.doc.fill { ring } else { gpui::transparent_black() })
                .cursor_pointer()
                .child(dot(fill, 20.))
                .when(fill.is_none(), |d| d.tip(Tooltip::text(t("editor.fill_transparent"))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.vm.doc.fill != fill {
                        this.checkpoint();
                        this.vm.doc.fill = fill;
                    }
                    this.crop_menu = None;
                    cx.notify();
                }))
        }).collect::<Vec<_>>();
        self.menu_anchor(CropMenu::Fill, cx)
            .child(
                ButtonLike::new("crop-fill")
                    .style(if open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(open)
                    .child(h_flex().gap_1().px_1().child(dot(self.vm.doc.fill, 16.)).child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted)))
                    .tip(Tooltip::text(t("editor.fill_tip")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.crop_menu = if this.crop_menu == Some(CropMenu::Fill) { None } else { Some(CropMenu::Fill) };
                        cx.notify();
                    })),
            )
            .when(open, |d| {
                d.child(gpui::deferred(
                    h_flex()
                        .id("fill-menu")
                        .occlude()
                        .absolute()
                        .top(px(38.))
                        .left_0()
                        .gap_1()
                        .p_1p5()
                        .elevation_3(cx)
                        .children(swatches)
                        .child(crate::ui::track_bounds(&self.crop_menu_bounds)),
                ))
            })
            .into_any_element()
    }

    /// The crop mode's top bar: the mode, the shape and
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
            .tip(Tooltip::text(t("editor.leave_crop")))
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
                .tip(Tooltip::text(t(if height { "editor.height_px" } else { "editor.width_px" })))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.apply_size_field();
                    this.size_field = Some((height, String::new()));
                    cx.notify();
                }))
        };
        let icon_button = |id: &'static str, icon: &'static str, tip: &'static str| {
            ButtonLike::new(id).style(ButtonStyle::Subtle).size(ButtonSize::Large).child(div().px_1().child(Icon::from_path(icon).size(IconSize::Small))).tip(Tooltip::text(tip))
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
        // Its layouts, from the fullest: everything; no label and no key
        // hints; no size fields; no spacers and no "px".
        let fit = self.crop_fit.pick(width - 2. * kit::REM * 0.25 - kit::REM * 0.5, 4);
        let (narrow, tight, tiny) = (fit >= 1, fit >= 2, fit >= 3);
        let sized = |k: f32| format!("{}×{}{}", (ow * k).round().max(1.), (oh * k).round().max(1.), if tiny { "" } else { " px" });
        let scales: Vec<MenuItem> = SCALES
            .iter()
            .map(|&k| (format!("{}% · {}", (k * 100.).round(), sized(k)), (self.vm.doc.scale - k).abs() < 1e-3, Rc::new(move |this: &mut Editor, _: &mut Window, _: &mut Context<Editor>| this.pick_scale(k)) as MenuPick))
            .collect();
        let scale = self.crop_dropdown(CropMenu::Scale, sized(self.vm.doc.scale), t("editor.image_size_tip"), scales, false, cx);

        // What fills the crop where it reaches past the picture: a dot and
        // swatches like the pen colour's, see-through first.
        let fill = self.fill_picker(cx);

        let spacer = || (!tiny).then(|| div().w(px(12.)));
        let left = h_flex()
            .flex_none()
            .relative()
            .gap_1()
            .children(kit::window_controls(false, cx))
            .child(leave)
            .child(aspect)
            .when(!tight, |row| row.child(field("crop-width", false, cw, cx)).child(swap).child(field("crop-height", true, ch, cx)))
            .children(spacer())
            .child(rotate)
            .child(flip)
            .children(spacer())
            .when(!narrow, |row| row.child(Label::new(t("editor.image_size")).size(LabelSize::Small).color(Color::Muted)))
            .child(scale)
            .child(fill)
            .child(self.crop_fit.track(fit, 0));
        let right = h_flex()
            .flex_none()
            .relative()
            .gap_1()
            .children(crate::ui::in_system_order(vec![
                Button::new("crop-cancel", t("common.cancel"))
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Large)
                    .when(!narrow, |b| b.key_binding(kit::keys("escape")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_crop(cx);
                        cx.notify();
                    }))
                    .into_any_element(),
                Button::new("crop-apply", t("editor.crop"))
                    .style(ButtonStyle::Accent)
                    .size(ButtonSize::Large)
                    .when(!narrow, |b| b.key_binding(kit::keys("enter")))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.apply_crop();
                        cx.notify();
                    }))
                    .into_any_element(),
            ]))
            .children(kit::window_controls(true, cx))
            .child(self.crop_fit.track(fit, 1));
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
        // Ctrl on Linux and Windows, Cmd on macOS (where Ctrl-click is the
        // right button).
        let held = kadr_platform::keymap::secondary(kadr_platform::keymap::Os::current());
        let hint = if on { tr!("editor.hold_to_move_freely", held) } else { tr!("editor.hold_to_snap", held) };
        let changed = self.vm.doc.crop.is_some() || !self.vm.doc.view.is_upright() || self.vm.doc.scale != 1.;
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

    /// The background tool as a sidebar: presets, then
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
                .tip(Tooltip::text(t("common.your_own_image")))
                .on_click(cx.listener(|this, _, window, cx| this.pick_wallpaper(window, cx)))
                .into_any_element(),
        );

        let blurred = tile("backdrop-blurred".into(), self.frame.backdrop == Backdrop::Blurred)
            .child(swatch().bg(field_bg).children(self.blurred.clone().map(|b| img(b).size_full().object_fit(ObjectFit::Cover))))
            .tip(Tooltip::text(t("editor.blurred_tip")))
            .on_click(pick(Backdrop::Blurred));

        // Plain colours as dots, transparency first.
        let dot = |id: SharedString, on: bool| {
            div().id(id).flex_none().p(px(2.)).rounded_full().border_2().border_color(if on { accent } else { clear }).cursor_pointer()
        };
        let mut plain: Vec<AnyElement> = vec![dot("transparent".into(), self.frame.backdrop == Backdrop::Transparent)
            .child(div().size(px(DOT)).rounded_full().overflow_hidden().border_1().border_color(edge).child(checker_tile(DOT, 5.)))
            .tip(Tooltip::text(t("editor.transparent")))
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
    fn render_canvas(&self, scale_factor: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.scale();
        // Everything is laid out in the work area, the picture `m` in.
        let m = self.margin();
        self.laid_margin.set(m);
        let off = m * s;
        let at = |p: P| (px(p.0 * s + off), px(p.1 * s + off));
        let (pw, ph) = (self.px.0 as f32, self.px.1 as f32);

        // Pixelation stays on the picture (it covers only what's there).
        let blurs = self.vm.doc.shapes.iter().filter_map(|shape| match &shape.kind {
            &Kind::Blur(how) => {
                let patch = self.patches.get(&patch_key(how, shape.a, shape.b, shape.size))?;
                let (l, t, r, b) = rect(shape.a, shape.b);
                let (l, t, r, b) = (l.max(0.).floor(), t.max(0.).floor(), r.min(pw).ceil(), b.min(ph).ceil());
                // The patch comes padded (`hide_png`); the area clips it.
                let pad = PATCH_PAD as f32 * s;
                Some(
                    div()
                        .absolute()
                        .left(px(l * s))
                        .top(px(t * s))
                        .w(px((r - l) * s))
                        .h(px((b - t) * s))
                        .overflow_hidden()
                        .child(img(patch.clone()).absolute().left(px(-pad)).top(px(-pad)).w(px((r - l) * s + 2. * pad)).h(px((b - t) * s + 2. * pad)).object_fit(ObjectFit::Fill)),
                )
            }
            _ => None,
        });
        let pictures = self.vm.doc.shapes.iter().filter_map(|shape| match &shape.kind {
            Kind::Picture(picture) => {
                let patch = self.shown_picture(picture)?;
                let (l, t, r, b) = rect(shape.a, shape.b);
                Some(img(patch.clone()).absolute().left(px(l * s + off)).top(px(t * s + off)).w(px((r - l) * s)).h(px((b - t) * s)).object_fit(ObjectFit::Fill))
            }
            _ => None,
        });

        let picture_size = (pw, ph);
        // The finished strokes come from their picture; only what it lacks
        // (the stroke being drawn, the shape being dragged, edits it hasn't
        // caught up with yet) is outlined here, every frame.
        let shapes = &self.vm.doc.shapes;
        let skip = match self.drag {
            Some(Drag::Move { index, .. } | Drag::Handle { index, .. } | Drag::Scale { index, .. }) => Some(index),
            _ => None,
        };
        let usable = |im: &StrokesImage| im.count <= shapes.len() && strokes_hash(shapes, im.count, im.skip) == im.hash;
        // Sharp at this zoom.
        let density = s * scale_factor;
        let points: usize = shapes.iter().map(|sh| match &sh.kind {
            Kind::Pen(p) | Kind::Marker(p) => p.len(),
            _ => 4,
        }).sum();
        // Too much to outline every frame until the picture catches up
        // (opening a big drawing, undoing on one): the picture comes now.
        if points > HEAVY_STROKES && !self.strokes.borrow().shown.as_ref().is_some_and(usable) {
            let todo: Vec<Shape> = shapes.iter().enumerate().filter(|(i, sh)| sh.kind.is_vector() && Some(*i) != skip).map(|(_, sh)| sh.clone()).collect();
            if let Some((at, size, density, image)) = strokes_image(&todo, density) {
                let new = StrokesImage { count: shapes.len(), skip, hash: strokes_hash(shapes, shapes.len(), skip), density, at, size, image };
                let mut state = self.strokes.borrow_mut();
                if let Some(old) = state.shown.replace(new) {
                    state.retired.push(old.image);
                }
            }
        }
        let state = self.strokes.borrow();
        let shown = state.shown.as_ref().filter(|im| usable(im));
        let uncovered = |i: usize| shown.is_none_or(|im| i >= im.count || Some(i) == im.skip);
        // A shape being moved (or just let go, until the picture has it)
        // comes from a picture of its own, shifted.
        let held = match self.drag {
            Some(Drag::Move { index, .. } | Drag::Scale { index, .. }) => Some(index),
            _ => None,
        };
        let candidate = held.or(state.lifted.as_ref().map(|l| l.index)).filter(|&i| uncovered(i) && shapes.get(i).is_some_and(|sh| sh.kind.is_vector()));
        // Moved: the picture shifts. Resized: it stretches to the new box
        // until a picture of the new shape is drawn.
        let mut stale = false;
        let placing = candidate.and_then(|i| {
            let l = state.lifted.as_ref().filter(|l| l.index == i)?;
            let now = &shapes[i];
            if let Some(d) = moved_by(now, &l.shape) {
                return Some((i, (l.at.0 + d.0, l.at.1 + d.1), l.size));
            }
            stale = true;
            let ((ol, ot, or, ob), (nl, nt, nr, nb)) = (l.shape.bounds(), now.bounds());
            let k = |n: f32, o: f32| if o.abs() > 1e-3 { n / o } else { 1. };
            let (kx, ky) = (k(nr - nl, or - ol), k(nb - nt, ob - ot));
            Some((i, (nl + (l.at.0 - ol) * kx, nt + (l.at.1 - ot) * ky), (l.size.0 * kx, l.size.1 * ky)))
        });
        let lift_busy = state.lift_busy;
        drop(state);
        let lift_box = |sh: &Shape| {
            let (l, t, r, b) = sh.bounds();
            let pad = sh.size + 2.;
            ((l - pad, t - pad), (r - l + 2. * pad, b - t + 2. * pad))
        };
        let lift = match (candidate, placing) {
            (Some(i), Some(place)) => {
                if stale && !lift_busy {
                    self.strokes.borrow_mut().lift_busy = true;
                    let sh = shapes[i].clone();
                    let (at, size) = lift_box(&sh);
                    let task = cx.background_executor().spawn(async move {
                        let made = shapes_image(std::slice::from_ref(&sh), at, size, box_density(size, density));
                        (sh, made)
                    });
                    cx.spawn(async move |this, cx| {
                        let (sh, made) = task.await;
                        let _ = this.update(cx, |this, cx| {
                            let mut state = this.strokes.borrow_mut();
                            state.lift_busy = false;
                            if let Some((_, image)) = made
                                && state.lifted.as_ref().is_some_and(|l| l.index == i)
                                && let Some(old) = state.lifted.replace(Lifted { index: i, shape: sh, at, size, image })
                            {
                                state.retired.push(old.image);
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                }
                Some(place)
            }
            (Some(i), None) if held == Some(i) => {
                let sh = &shapes[i];
                let (at, size) = lift_box(sh);
                let made = shapes_image(std::slice::from_ref(sh), at, size, box_density(size, density));
                let mut state = self.strokes.borrow_mut();
                if let Some(old) = state.lifted.take() {
                    state.retired.push(old.image);
                }
                made.map(|(_, image)| {
                    state.lifted = Some(Lifted { index: i, shape: sh.clone(), at, size, image });
                    (i, at, size)
                })
            }
            _ => {
                let mut state = self.strokes.borrow_mut();
                if let Some(old) = state.lifted.take() {
                    state.retired.push(old.image);
                }
                None
            }
        };
        let state = self.strokes.borrow();
        let shown = state.shown.as_ref().filter(|im| usable(im));
        let uncovered = |i: usize| shown.is_none_or(|im| i >= im.count || Some(i) == im.skip);
        let lifted = lift.and_then(|(i, at, size)| {
            let l = state.lifted.as_ref()?;
            Some((i, l.image.clone(), (at.0 * s + off, at.1 * s + off), (size.0 * s, size.1 * s)))
        });
        let live: Vec<Shape> = shapes
            .iter()
            .enumerate()
            .filter(|(i, sh)| sh.kind.is_vector() && uncovered(*i) && lifted.as_ref().is_none_or(|l| l.0 != *i))
            .map(|(_, sh)| sh.clone())
            .chain(self.draft.iter().cloned())
            .collect();
        let wanted = (shapes.len(), skip);
        let any = shapes.iter().enumerate().any(|(i, sh)| sh.kind.is_vector() && Some(i) != skip);
        let fresh = shown.is_some_and(|im| (im.count, im.skip) == wanted && (im.density / box_density(im.size, density) - 1.).abs() < 0.1);
        let placed = shown.map(|im| (im.image.clone(), (im.at.0 * s + off, im.at.1 * s + off), (im.size.0 * s, im.size.1 * s)));
        drop(state);
        if !fresh && any && !self.strokes.borrow().busy {
            self.strokes.borrow_mut().busy = true;
            let todo: Vec<Shape> = shapes.iter().enumerate().filter(|(i, sh)| sh.kind.is_vector() && Some(*i) != skip).map(|(_, sh)| sh.clone()).collect();
            let hash = strokes_hash(shapes, shapes.len(), skip);
            let task = cx.background_executor().spawn(async move { strokes_image(&todo, density) });
            cx.spawn(async move |this, cx| {
                let made = task.await;
                let _ = this.update(cx, |this, cx| {
                    let mut state = this.strokes.borrow_mut();
                    state.busy = false;
                    if let Some((at, size, density, image)) = made {
                        let new = StrokesImage { count: wanted.0, skip: wanted.1, hash, density, at, size, image };
                        if let Some(old) = state.shown.replace(new) {
                            state.retired.push(old.image);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        } else if !any {
            let mut state = self.strokes.borrow_mut();
            if let Some(old) = state.shown.take() {
                state.retired.push(old.image);
            }
        }
        let spots: Vec<(P, P)> = shapes.iter().chain(self.draft.iter()).filter(|sh| matches!(sh.kind, Kind::Spotlight)).map(|sh| (sh.a, sh.b)).collect();
        let origin = self.origin.clone();
        // The dimming goes under the other shapes.
        let under = canvas(
            move |bounds, _, _| {
                origin.set((f32::from(bounds.origin.x), f32::from(bounds.origin.y)));
                bounds
            },
            move |_, bounds, window, _| {
                let o = bounds.origin;
                let pt = |p: P| point(o.x + px(p.0 * s + off), o.y + px(p.1 * s + off));
                paint_spotlights(window, &spots, picture_size, &pt);
            },
        )
        .absolute()
        .inset_0()
        .size_full();
        let finished = placed.map(|(image, at, size)| img(gpui::ImageSource::Render(image)).absolute().left(px(at.0)).top(px(at.1)).w(px(size.0)).h(px(size.1)).object_fit(ObjectFit::Fill));
        let lifted = lifted.map(|(_, image, at, size)| img(gpui::ImageSource::Render(image)).absolute().left(px(at.0)).top(px(at.1)).w(px(size.0)).h(px(size.1)).object_fit(ObjectFit::Fill));
        let strokes = self.strokes.clone();
        // While a text is typed, the input method writes into it.
        let input = self.editing.is_some().then(|| (self.focus.clone(), cx.entity()));
        let over = canvas(
            |bounds, _, _| bounds,
            move |_, bounds, window, cx| {
                if let Some((focus, entity)) = &input {
                    window.handle_input(focus, gpui::ElementInputHandler::new(bounds, entity.clone()), cx);
                }
                for old in strokes.borrow_mut().retired.drain(..) {
                    let _ = window.drop_image(old);
                }
                let o = bounds.origin;
                let pt = |p: P| point(o.x + px(p.0 * s + off), o.y + px(p.1 * s + off));
                for shape in &live {
                    paint_shape(window, shape, s, &pt);
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        let mut cursors_shown = Vec::new();
        let labels: Vec<AnyElement> = self.vm.doc.shapes.iter().enumerate().filter_map(|(i, shape)| match &shape.kind {
            Kind::Text(text) => {
                let (x, y) = at(shape.a);
                let m = metrics(shape.size);
                let editing = self.editing == Some(i);
                // While typed: the caret as a bar that takes no room, the
                // selection on a tint, the input method's text underlined.
                let (caret, selection, marked) = if editing { (Some(self.field.caret()), self.field.selection(), self.field.marked()) } else { (None, 0..0, None) };
                let mut tint = cx.theme().colors().text_accent;
                tint.alpha = 0.3;
                let lines: Vec<(usize, String)> = text.split('\n').scan(0, |start, line| {
                    let at = *start;
                    *start += line.len() + 1;
                    Some((at, line.to_owned()))
                }).collect();
                let line_h = m.line * s;
                if let Some(c) = caret
                    && let Some(li) = lines.iter().rposition(|(start, _)| *start <= c)
                {
                    // Near enough for the input method's candidates: the
                    // caret's line, at the text's left.
                    self.caret_at.set(Some(Bounds { origin: point(x, y + px(li as f32 * line_h)), size: gpui::size(px(1.), px(line_h)) }));
                }
                let block = |color: Hsla| {
                    div().text_color(color).children(lines.iter().map(|(start, line)| {
                        let row = div().h(px(line_h)).line_height(px(line_h)).flex().flex_row();
                        if caret.is_none() {
                            return row.child(line.clone());
                        }
                        // Cut where the caret, the selection and the marked text start and end.
                        let end = start + line.len();
                        let mut cuts: Vec<usize> = [selection.start, selection.end].into_iter().chain(marked.iter().flat_map(|r| [r.start, r.end])).chain(caret).filter(|&c| c > *start && c < end).collect();
                        cuts.extend([*start, end]);
                        cuts.sort_unstable();
                        cuts.dedup();
                        let mut row = row;
                        for w in cuts.windows(2) {
                            if caret == Some(w[0]) {
                                row = row.child(caret_bar(line_h, color));
                            }
                            let piece = &text[w[0]..w[1]];
                            let selected = w[0] >= selection.start && w[1] <= selection.end && selection.start < selection.end;
                            let composing = marked.as_ref().is_some_and(|r| w[0] >= r.start && w[1] <= r.end);
                            row = row.child(div().when(selected, |d| d.bg(tint)).when(composing, |d| d.underline()).child(piece.to_owned()));
                        }
                        if caret == Some(end) {
                            row = row.child(caret_bar(line_h, color));
                        }
                        row
                    }))
                };
                // The export draws text with the bundled font; the canvas must match it.
                let base = div().absolute().font_family(kit::ANNOTATION_FONT).text_size(px(shape.size * s)).font_weight(gpui::FontWeight::SEMIBOLD).whitespace_nowrap();
                let look = kadr_raster::shapes::text_look(shape.text, shape.color);
                let hsla = crate::ui::from_rgba;
                // GPUI has no text stroke: the outline is the text drawn
                // around itself in the contrasting colour; the shadow is
                // one more copy, down and right.
                let rings = look.ring.into_iter().flat_map(|ring| {
                    let o = TextStyle::outline(shape.size) * s;
                    (0..12).map(move |k| {
                        let th = k as f32 / 12. * std::f32::consts::TAU;
                        div().absolute().left(px(o * th.cos())).top(px(o * th.sin())).child(block(hsla(ring)))
                    })
                });
                let shadow = look.shadow.map(|shadow| {
                    let o = TextStyle::shadow(shape.size) * s;
                    div().absolute().left(px(o)).top(px(o)).child(block(hsla(shadow)))
                });
                let letters = div().relative().children(rings.collect::<Vec<_>>()).children(shadow).child(block(hsla(look.ink)));
                Some(match shape.text.text_box(shape.size) {
                    None => base.left(x).top(y).child(letters).into_any_element(),
                    Some(b) => {
                        let (padx, pady) = b.pad;
                        let (_, t, _, bt) = shape.bounds();
                        let radius = b.radius.min((bt - t) / 2.);
                        // GPUI's border adds to the box; the export's frame
                        // is inside it, so it comes off the margins here.
                        let f = if look.frame.is_some() { TextStyle::frame(shape.size) } else { 0. };
                        base.left(x - px(padx * s))
                            .top(y - px(pady * s))
                            .px(px((padx - f) * s))
                            .py(px((pady - f) * s))
                            .rounded(px(radius * s))
                            .when_some(look.fill, |d, fill| d.bg(hsla(fill)))
                            .when_some(look.frame, |d, frame| d.border(px(TextStyle::frame(shape.size) * s)).border_color(hsla(frame)))
                            .child(letters)
                            .into_any_element()
                    }
                })
            }
            Kind::Cursor(name) => {
                let height = shape.size * s * scale_factor;
                let key = (*name, height.round().max(1.) as u32);
                let cached = self.cursor_images.borrow().get(&key).cloned();
                let image = match cached {
                    Some(image) => image,
                    None => {
                        let image = cursor_image(name, key.1 as f32)?;
                        self.cursor_images.borrow_mut().insert(key, image.clone());
                        image
                    }
                };
                cursors_shown.push(key);
                let ((hx, hy), (w, h)) = cursors::geometry(name, shape.size);
                Some(
                    img(gpui::ImageSource::Render(image))
                        .absolute()
                        .left(px((shape.a.0 - hx) * s + off))
                        .top(px((shape.a.1 - hy) * s + off))
                        .w(px(w * s))
                        .h(px(h * s))
                        .object_fit(ObjectFit::Fill)
                        .into_any_element(),
                )
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
                        .bg(crate::ui::from_rgba(shape.color))
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family(kit::ANNOTATION_FONT)
                        .text_size(px(d * 0.55))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(on_color(crate::ui::from_rgba(shape.color)))
                        .child(n.to_string())
                        .into_any_element(),
                )
            }
            _ => None,
        }).collect();
        // Cursors drawn for another zoom go, for the next frame to free.
        {
            let mut images = self.cursor_images.borrow_mut();
            let gone: Vec<_> = images.keys().filter(|k| !cursors_shown.contains(k)).copied().collect();
            let mut state = self.strokes.borrow_mut();
            for key in gone {
                if let Some(image) = images.remove(&key) {
                    state.retired.push(image);
                }
            }
        }

        let crop = self.vm.doc.crop;
        let cropping = self.cropping.is_some();
        // Thirds while the crop is being dragged, like a camera's grid.
        let thirds = cropping && matches!(self.drag, Some(Drag::Crop { .. } | Drag::CropEdit { .. }));
        // The result's bounds, when drawing has grown it past the picture.
        let (el, et, er, eb) = self.extent();
        let grown = crop.is_none() && (el < 0. || et < 0. || er > pw || eb > ph);
        let marquee = match self.drag {
            Some(Drag::Marquee { start, now }) => Some(rect(start, now)),
            _ => None,
        };
        let marked: Vec<_> = self.marked.iter().filter_map(|&i| self.vm.doc.shapes.get(i)).map(|sh| (sh.bounds(), sh.band_pad(s))).collect();
        let selection = self.selected.filter(|_| self.editing.is_none()).and_then(|i| self.vm.doc.shapes.get(i)).map(|sh| (sh.bounds(), sh.knobs(s), sh.band_pad(s), matches!(sh.kind, Kind::Line | Kind::Arrow(_))));
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
                // A soft, wide band just outside anything
                // boxy with knobs on its corners; lines show their ends.
                // Marked shapes: the selection's band, without knobs.
                let mut band = accent;
                band.lightness = (band.lightness + 0.12).min(0.8);
                band.alpha = 0.8;
                for &((l, t, r, b), pad) in &marked {
                    let half = BAND / 2. / s;
                    let r0 = area(l - pad - half, t - pad - half, r + pad + half, b + pad + half);
                    window.paint_quad(crate::ui::quad(r0, px(2.), gpui::transparent_black(), px(BAND), band, BorderStyle::Solid));
                }
                if let Some((l, t, r, b)) = marquee {
                    let mut tint = accent;
                    tint.alpha = 0.12;
                    window.paint_quad(crate::ui::quad(area(l, t, r, b), px(0.), tint, px(1.), accent, BorderStyle::Solid));
                }
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
            _ if self.space => CursorStyle::OpenHand,
            _ if self.picking => CursorStyle::Crosshair,
            _ if self.drag.is_none() && self.hover.is_some() => self.hover.unwrap_or(CursorStyle::Arrow),
            Tool::Select => CursorStyle::Arrow,
            Tool::Text => CursorStyle::IBeam,
            _ => CursorStyle::Crosshair,
        };
        // No background while cropping: the picture alone, as it's cut.
        let framed = self.frame.on() && self.cropping.is_none();
        // A window shot's see-through margin holds its own shadow; the
        // stage's shadow and edge would show through it as a dark box.
        let clear_edge = matches!(self.window, Some((_, Some(_))));
        let radius = if framed { self.disp.0.max(self.disp.1) * self.frame.radius } else { 6. };
        let picture = div()
            .absolute()
            .left(px(off))
            .top(px(off))
            .w(px(self.disp.0))
            .h(px(self.disp.1))
            .rounded(px(radius))
            .overflow_hidden()
            .when(!framed && !clear_edge, |d| d.shadow(kit::shadow()))
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
            .child(match self.picture.clone() {
                // The padded picture, painted past the bounds by its rim so
                // only the picture itself shows, rounded.
                Some(picture) => {
                    let rim = PICTURE_PAD as f32 * s;
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            let outer = Bounds::new(point(bounds.origin.x - px(rim), bounds.origin.y - px(rim)), gpui::size(bounds.size.width + px(2. * rim), bounds.size.height + px(2. * rim)));
                            let _ = window.paint_image(bounds, outer, px(radius).into(), 0., picture.clone(), 0, false);
                        },
                    )
                    .size_full()
                    .into_any_element()
                }
                // Still being read: the stage shows through for a moment.
                None => div().size_full().into_any_element(),
            })
            .children(blurs)
            // A hairline edge, so a dark screenshot doesn't melt into the
            // dark stage around it.
            .when(!framed && !clear_edge, |d| d.child(div().absolute().inset_0().rounded(px(radius)).border_1().border_color(cx.theme().colors().border)));

        // The background preview mirrors what Save produces: around the
        // result, which may reach past the picture.
        let ((ow, oh), (left, top)) = self.frame.layout((er - el) * s, (eb - et) * s);
        let backdrop = match &self.frame.backdrop {
            _ if self.cropping.is_some() => None,
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
        // A crop past the picture, filled as Save fills it; while cropping,
        // the whole room round the picture, the shade marking what's cut.
        let cropping = self.cropping.is_some();
        let fill = self.vm.doc.fill.map(|[r, g, b, a]| {
            let mut color = crate::ui::from_hex((r as u32) << 16 | (g as u32) << 8 | b as u32);
            color.alpha = a as f32 / 255.;
            div()
                .absolute()
                .map(|d| {
                    if cropping {
                        d.inset_0()
                    } else {
                        d.left(px(el * s + off)).top(px(et * s + off)).w(px((er - el) * s)).h(px((eb - et) * s))
                    }
                })
                .when(framed, |d| d.rounded(px(radius)))
                .bg(color)
        });
        div()
            .id("canvas")
            .relative()
            .flex_none()
            .w(px(self.disp.0 + 2. * off))
            .h(px(self.disp.1 + 2. * off))
            .cursor(pointer)
            .children(backdrop)
            .children(fill)
            .child(picture)
            .map(|d| {
                // What's drawn, laid out over the whole canvas; on a
                // background without padding, cut at the picture's edge
                // (the crop's, if any), as Save cuts it.
                let canvas_size = (self.disp.0 + 2. * off, self.disp.1 + 2. * off);
                let drawn = div().absolute().w(px(canvas_size.0)).h(px(canvas_size.1)).children(pictures).child(under).children(finished).children(lifted).child(over).children(labels);
                if self.frame.clips() && self.cropping.is_none() {
                    let (x, y) = (el * s + off, et * s + off);
                    d.child(div().absolute().left(px(x)).top(px(y)).w(px((er - el) * s)).h(px((eb - et) * s)).overflow_hidden().child(drawn.left(px(-x)).top(px(-y))))
                } else {
                    d.child(drawn.left_0().top_0())
                }
            })
            .child(overlay)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.mouse_down(e, cx)))
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for old in self.stale_pictures.drain(..) {
            let _ = window.drop_image(old);
        }
        self.fit_window();
        // The next screenshot starts with this background.
        let frame = self.frame.to_json();
        if frame != self.remembered {
            remember_frame(&self.frame);
            self.remembered = frame;
        }
        // The window: tools on top, the picture on a stage
        // (with the background panel beside it when open), actions at the
        // bottom. The picture fits the stage, its background included.
        let viewport = window.viewport_size();
        let vw = f32::from(viewport.width);
        let panel = if self.panel { PANEL } else { 0. };
        // The same layout in and out of the crop mode, so the picture keeps
        // its size there.
        let pad_share = self.resting_margin() / self.px.0.max(self.px.1).max(1) as f32;
        let box_of = |frame: &Frame, (w, h): (f32, f32)| {
            if frame.on() {
                frame.layout(w, h).0
            } else {
                let pad = w.max(h) * pad_share;
                (w + 2. * pad, h + 2. * pad)
            }
        };
        let full = box_of(&self.frame, self.natural);
        let room = (vw - 48. - panel, self.stage_h.get() - 48.);
        let k = (room.0 / full.0).min(room.1 / full.1).min(1.);
        let fit = (self.natural.0 * k, self.natural.1 * k);
        self.disp = (fit.0 * self.zoom, fit.1 * self.zoom);
        if self.cropping.is_some() {
            // The stage's free room round the picture, the tighter way,
            // inside its 24px padding.
            let s = self.disp.0 / self.px.0.max(1) as f32;
            let free = (vw - panel - self.disp.0).min(self.stage_h.get() - self.disp.1) / 2. - 24.;
            self.crop_room.set((free / s).max(0.));
        }
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
        // The stage's own bounds, for centring the canvas and zooming at the
        // pointer.
        let stage_box = self.stage_box.clone();
        let stage_measure = canvas(
            move |bounds, window, _| {
                let b = (f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height));
                if b != stage_box.get() {
                    stage_box.set(b);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let canvas_at = {
            let (_, _, w, h) = self.stage_box.get();
            let (s, m) = (self.disp.0 / self.px.0.max(1) as f32, self.margin());
            let off = m * s;
            let at = ((w - self.disp.0) / 2. - off + self.view_pan.0, (h - self.disp.1) / 2. - off + self.view_pan.1);
            // A crop pulled past the stage takes the view along, so its edge
            // stays in sight.
            self.crop_shift = match (&self.drag, self.vm.doc.crop) {
                (Some(Drag::Crop { .. } | Drag::CropEdit { .. }), Some((a, b))) => {
                    let (l, t, r, bt) = rect(a, b);
                    let follow = |lo: f32, hi: f32, base: f32, room: f32| {
                        let (lo, hi) = (base + (lo + m) * s, base + (hi + m) * s);
                        let (min, max) = (CROP_ROOM, room - CROP_ROOM);
                        // Bigger than the stage (zoomed in): no side to keep.
                        if hi - lo > max - min {
                            0.
                        } else if hi > max {
                            max - hi
                        } else if lo < min {
                            min - lo
                        } else {
                            0.
                        }
                    };
                    (follow(l, r, at.0, w), follow(t, bt, at.1, h))
                }
                _ => (0., 0.),
            };
            (at.0 + self.crop_shift.0, at.1 + self.crop_shift.1)
        };
        let stage = h_flex()
            .relative()
            .w_full()
            .flex_1()
            .min_h_0()
            .child(measure)
            // The background sidebar at the left.
            .when(self.panel, |row| row.child(self.render_panel(self.stage_h.get(), cx)))
            .child(
                div()
                    .id("stage")
                    .flex()
                    .items_start()
                    .flex_1()
                    .h_full()
                    .relative()
                    .overflow_hidden()
                    .when(self.space, |d| d.cursor(CursorStyle::OpenHand))
                    // The bars' tone, so the picture sits in one surface;
                    // black while cropping.
                    .bg(if self.cropping.is_some() { gpui::black() } else { cx.theme().colors().title_bar_background })
                    // Ctrl and the wheel zoom at the pointer; the wheel alone pans.
                    .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| {
                        cx.stop_propagation();
                        let d = e.delta.pixel_delta(px(20.));
                        let (dx, dy) = (f32::from(d.x), f32::from(d.y));
                        if e.modifiers.control {
                            if dy != 0. {
                                let (x, y, w, h) = this.stage_box.get();
                                let at = (f32::from(e.position.x) - x - w / 2., f32::from(e.position.y) - y - h / 2.);
                                this.zoom_at(this.zoom * if dy > 0. { 1.1 } else { 1. / 1.1 }, at);
                            }
                        } else {
                            this.view_pan = (this.view_pan.0 + dx, this.view_pan.1 + dy);
                        }
                        cx.notify();
                    }))
                    .child(stage_measure)
                    // In the middle of the stage, moved by the camera.
                    .child(div().absolute().left(px(canvas_at.0)).top(px(canvas_at.1)).child(self.render_canvas(window.scale_factor(), cx))),
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
            .when(self.windowed, |d| d.bg(cx.theme().colors().title_bar_background))
            .when(!self.windowed, |d| d.bg(rgba(0x00000040)).items_center().justify_center())
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .on_key_up(cx.listener(|this, e: &gpui::KeyUpEvent, _, cx| {
                if e.keystroke.key == "space" {
                    this.space = false;
                    this.pan = None;
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                this.drop_files(paths);
                cx.notify();
            }))
            // The pointer stays with this surface while the button is held.
            // `_all`: the plain listeners skip events over occluding children
            // (the bars), and a drag would stall there.
            .on_mouse_move_all({
                let editor = cx.entity().downgrade();
                move |e: &MouseMoveEvent, phase, _, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble {
                        let _ = editor.update(cx, |this, cx| this.mouse_move(e, cx));
                    }
                }
            })
            .on_mouse_up_all({
                let editor = cx.entity().downgrade();
                move |e: &gpui::MouseUpEvent, phase, _, _, cx| {
                    if phase == gpui::DispatchPhase::Bubble && e.button == MouseButton::Left {
                        let _ = editor.update(cx, |this, cx| this.mouse_up(cx));
                    }
                }
            })
            .child(content)
    }
}

/// The crop mode's frame: a thin line, thick white
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
        Kind::Line | Kind::Arrow(_) => {
            let step = std::f32::consts::FRAC_PI_4;
            let angle = (dy.atan2(dx) / step).round() * step;
            let len = dx.hypot(dy);
            (d.a.0 + len * angle.cos(), d.a.1 + len * angle.sin())
        }
        _ => p,
    }
}





/// The box of a picture's opaque pixels, when everything round it is see-
/// through: a window shot's window inside its shadow (which never reaches
/// full opacity), its rounded corners included.
fn opaque_bounds(img: &RgbaImage) -> Option<crop::Rect> {
    let (w, h) = img.dimensions();
    let (mut l, mut t, mut r, mut b) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] == 255 {
            (l, t, r, b) = (l.min(x), t.min(y), r.max(x + 1), b.max(y + 1));
        }
    }
    (l < r && (l, t, r, b) != (0, 0, w, h)).then(|| (l as f32, t as f32, r as f32, b as f32))
}



/// The background looks saved in the editor, by name.
pub fn frame_presets() -> Vec<(String, serde_json::Value)> {
    let saved = std::fs::read_to_string(util::config_dir().join("editor-presets.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    saved.as_ref().and_then(|v| v.as_array()).into_iter().flatten().filter_map(|p| Some((p["name"].as_str()?.to_owned(), p["look"].clone()))).collect()
}

fn save_frame_presets(presets: &[(String, serde_json::Value)]) {
    let v: Vec<_> = presets.iter().map(|(name, look)| serde_json::json!({ "name": name, "look": look })).collect();
    let _ = std::fs::create_dir_all(util::config_dir());
    let _ = std::fs::write(util::config_dir().join("editor-presets.json"), serde_json::to_string_pretty(&v).unwrap_or_default());
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

/// The caret: a bar over the text that takes no room in it.
fn caret_bar(height: f32, color: Hsla) -> gpui::Div {
    div().relative().w(px(0.)).h(px(height)).child(div().absolute().left(px(-0.75)).top(px(height * 0.12)).w(px(1.5)).h(px(height * 0.76)).bg(color))
}

impl Editor {
    /// The text being typed into its shape.
    fn sync_field(&mut self) {
        if let Some(i) = self.editing
            && let Some(Kind::Text(t)) = self.vm.doc.shapes.get_mut(i).map(|s| &mut s.kind)
            && t != self.field.text()
        {
            *t = self.field.text().to_owned();
        }
    }
}

/// The input method's side of typing a text (and plain typing, which GPUI
/// hands here once the keys above let it through). Offsets are UTF-16.
impl gpui::EntityInputHandler for Editor {
    fn text_for_range(&mut self, range: std::ops::Range<usize>, adjusted: &mut Option<std::ops::Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        self.editing?;
        let range = range.start.min(self.field.len_utf16())..range.end.min(self.field.len_utf16());
        *adjusted = Some(range.clone());
        Some(self.field.text_utf16(range))
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<gpui::UTF16Selection> {
        self.editing?;
        Some(gpui::UTF16Selection { range: self.field.selection_utf16(), reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<std::ops::Range<usize>> {
        self.editing?;
        self.field.marked_utf16()
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.field.unmark();
        cx.notify();
    }

    fn replace_text_in_range(&mut self, range: Option<std::ops::Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_none() || text.chars().any(char::is_control) && text != "\n" {
            return;
        }
        self.field.replace(range, text);
        self.sync_field();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(&mut self, range: Option<std::ops::Range<usize>>, text: &str, selected: Option<std::ops::Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_none() {
            return;
        }
        self.field.mark(range, text, selected);
        self.sync_field();
        cx.notify();
    }

    fn bounds_for_range(&mut self, _: std::ops::Range<usize>, element: Bounds<gpui::Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<gpui::Pixels>> {
        let caret = self.caret_at.get()?;
        Some(Bounds { origin: element.origin + caret.origin, size: caret.size })
    }

    fn character_index_for_point(&mut self, _: Point<gpui::Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }

    fn set_selected_text_range(&mut self, range: std::ops::Range<usize>, _: &mut Window, cx: &mut Context<Self>) {
        self.field.select_utf16(range);
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.field.len_utf16())
    }

    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        self.editing.is_some()
    }
}





fn paint_shape(window: &mut Window, shape: &Shape, s: f32, pt: &impl Fn(P) -> Point<gpui::Pixels>) {
    // Outline the stroke the way the export does (round caps and joins) and
    // fill it with the nonzero rule: GPUI's own stroke overlaps itself at sharp
    // turns, which shows as dark knots in translucent marker lines.
    let fill = |window: &mut Window, outline: &tiny_skia::Path, color: Hsla| {
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
    let outline = |points: &[P], closed: bool, cap: tiny_skia::LineCap, width: f32| {
        let (first, rest) = points.split_first()?;
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
        let style = tiny_skia::Stroke { width: width * s, line_cap: cap, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        line.finish().and_then(|path| path.stroke(&style, 1.))
    };
    let stroke = |window: &mut Window, points: &[P], closed: bool, color: Hsla, width: f32| {
        // GPUI counts a path's vertices in 16 bits and drops a path with
        // more, so a long stroke goes in pieces.
        if closed || points.len() <= STROKE_PIECE {
            if let Some(o) = outline(points, closed, tiny_skia::LineCap::Round, width) {
                fill(window, &o, color);
            }
            return;
        }
        let pieces = (points.len() - 1).div_ceil(STROKE_PIECE);
        let translucent = color.alpha < 1.;
        for k in 0..pieces {
            let piece = &points[k * STROKE_PIECE..((k + 1) * STROKE_PIECE + 1).min(points.len())];
            // Overlapping round ends would show darker where translucent
            // pieces meet: those meet square, and the stroke's two ends get
            // their half-round caps on their own.
            let cap = if translucent { tiny_skia::LineCap::Butt } else { tiny_skia::LineCap::Round };
            if let Some(o) = outline(piece, false, cap, width) {
                fill(window, &o, color);
            }
        }
        if translucent {
            for (end, before) in [(points[0], points[1]), (points[points.len() - 1], points[points.len() - 2])] {
                let (dx, dy) = (end.0 - before.0, end.1 - before.1);
                let len = dx.hypot(dy);
                if len < 1e-4 {
                    continue;
                }
                let start = dy.atan2(dx) - std::f32::consts::FRAC_PI_2;
                let r = width / 2.;
                let cap: Vec<P> = (0..=12).map(|i| {
                    let th = start + i as f32 / 12. * std::f32::consts::PI;
                    (end.0 + r * th.cos(), end.1 + r * th.sin())
                }).collect();
                let mut f = PathBuilder::fill();
                f.move_to(pt(cap[0]));
                for p in &cap[1..] {
                    f.line_to(pt(*p));
                }
                f.close();
                if let Ok(path) = f.build() {
                    window.paint_path(path, color);
                }
            }
        }
    };
    let (a, b, c, w) = (shape.a, shape.b, crate::ui::from_rgba(shape.color), shape.size);
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
        Kind::Arrow(style) => {
            for piece in arrow_pieces(&shape.path(), w, *style) {
                match piece {
                    Piece::Stroke(points) => stroke(window, &points, false, c, w),
                    Piece::Fill(points) => {
                        let Some((first, rest)) = points.split_first() else { continue };
                        let mut f = PathBuilder::fill();
                        f.move_to(pt(*first));
                        for p in rest {
                            f.line_to(pt(*p));
                        }
                        f.close();
                        if let Ok(path) = f.build() {
                            window.paint_path(path, c);
                        }
                    }
                }
            }
        }
        // While dragging, outline the area to be hidden.
        Kind::Blur(_) => {
            let mut outline = c;
            outline.alpha = 0.8;
            stroke(window, &[a, (b.0, a.1), b, (a.0, b.1)], true, outline, 1.5 / s);
        }
        Kind::Text(_) | Kind::Counter(_) | Kind::Cursor(_) | Kind::Spotlight | Kind::Picture(_) => {}
    }
}

/// How close to the stage's edges a dragged crop comes before the view
/// follows it, in screen pixels.
const CROP_ROOM: f32 = 24.;

/// Stroke points past which the canvas waits for their picture rather
/// than outlining them all on one frame.
const HEAVY_STROKES: usize = 20_000;

/// Points per piece of a stroke drawn on the canvas (see `paint_shape`).
const STROKE_PIECE: usize = 96;


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






/// The hidden a–b area of `src` as PNG bytes, for the canvas.
fn hide_png(src: &RgbaImage, a: P, b: P, how: Hide, size: f32) -> Option<Vec<u8>> {
    // Its edge pixels repeated round it: drawn scaled, GPUI blends a
    // texture's outer texels with clear black, and the patch would show a
    // grey rim; the canvas clips the repeat off instead.
    let patch = kadr_raster::hide(src, a, b, how, size)?;
    let (w, h) = patch.dimensions();
    let n = PATCH_PAD;
    let patch = RgbaImage::from_fn(w + 2 * n, h + 2 * n, |x, y| *patch.get_pixel(x.saturating_sub(n).min(w - 1), y.saturating_sub(n).min(h - 1)));
    let mut png = Vec::new();
    patch.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// A text style's name, for its button.
fn text_style_name(style: TextStyle) -> &'static str {
    match style {
        TextStyle::Plain => t("editor.plain_text"),
        TextStyle::Plate => t("editor.on_plate"),
        TextStyle::Outline => t("editor.outlined"),
        TextStyle::Shadow => t("editor.text_shadow"),
        TextStyle::Highlight => t("editor.text_highlight"),
        TextStyle::Pill => t("editor.text_pill"),
        TextStyle::Label => t("editor.text_label"),
    }
}

/// "Aa" in a text style and colour, as the canvas draws it, small.
fn text_sample(style: TextStyle, color: kadr_core::color::Rgba) -> Div {
    let look = kadr_raster::shapes::text_look(style, color);
    let hsla = crate::ui::from_rgba;
    let letters = |c| div().child("Aa").text_color(hsla(c));
    let rings = look.ring.into_iter().flat_map(|ring| [(-1., 0.), (1., 0.), (0., -1.), (0., 1.)].map(|(x, y)| letters(ring).absolute().left(px(x)).top(px(y))));
    let shadow = look.shadow.map(|shadow| letters(shadow).absolute().left(px(1.)).top(px(1.)));
    let inner = div().relative().children(rings.collect::<Vec<_>>()).children(shadow).child(letters(look.ink));
    let sample = div().text_size(px(14.)).font_weight(gpui::FontWeight::SEMIBOLD).line_height(px(20.)).px(px(4.)).rounded(px(4.));
    let sample = match style {
        TextStyle::Pill => sample.px(px(7.)).rounded(px(10.)),
        TextStyle::Highlight => sample.rounded(px(2.)),
        _ => sample,
    };
    sample
        .when_some(look.fill, |d, fill| d.bg(hsla(fill)))
        .when_some(look.frame, |d, frame| d.border_1().border_color(hsla(frame)))
        .child(inner)
}

/// White or near-black text on the canvas, whichever reads better on `c`
/// (what `kadr_raster::on_color` picks for the export).
fn on_color(c: Hsla) -> Hsla {
    crate::ui::from_rgba(kadr_raster::on_color(crate::ui::to_rgba(c)))
}

/// A hidden area's patch, by what it depends on: how, where, how strong.
fn patch_key(how: Hide, a: P, b: P, size: f32) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for v in [how as u32, a.0.to_bits(), a.1.to_bits(), b.0.to_bits(), b.1.to_bits(), size.to_bits()] {
        h = (h ^ v as u64).wrapping_mul(0x100000001b3);
    }
    h
}

/// The picture as taken, as the PNG a project keeps under its drawing,
/// with the drawing's black boxes burnt in.
fn base_png(original: Option<&RgbaImage>, density: Option<f64>, doc: &Doc) -> std::io::Result<Vec<u8>> {
    let original = original.ok_or_else(|| std::io::Error::other("the picture isn't read yet"))?;
    let burnt = kadr_raster::burn_redactions(original, doc);
    let original = burnt.as_ref().unwrap_or(original);
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Adaptive);
    original.write_with_encoder(encoder).map_err(std::io::Error::other)?;
    Ok(match density {
        Some(d) => cards::with_density(png, d),
        None => png,
    })
}

/// A dropped, pasted or chosen image file as a picture to lay on top.
fn decode_picture(bytes: &[u8]) -> Option<Arc<RgbaImage>> {
    image::load_from_memory(bytes).ok().map(|i| Arc::new(i.to_rgba8()))
}

/// The repeated rim round the shown picture, in pixels.
const PICTURE_PAD: u32 = 2;

/// The repeated rim round a hidden area's patch on screen, in pixels.
const PATCH_PAD: u32 = 2;











#[cfg(test)]
mod tests {
    use super::*;
    use kadr_raster::{framed, hide, rasterize};

    fn scribbles(n: usize) -> Vec<Shape> {
        (0..n)
            .map(|k| {
                let points: Vec<P> = (0..300).map(|i| {
                    let t = i as f32 / 40.;
                    (400. + (t * 1.3 + k as f32).cos() * (60. + i as f32 * 0.3), 300. + (t * 1.7).sin() * (40. + i as f32 * 0.2))
                }).collect();
                let kind = if k % 5 == 0 { Kind::Marker(points) } else { Kind::Pen(points) };
                Shape { kind, a: (0., 0.), b: (0., 0.), color: Rgba::hsl(k as f32 / n as f32, 0.8, 0.5, 1.), size: 4., filled: false, text: TextStyle::Plain, bend: None }
            })
            .collect()
    }

    /// A moved shape is recognised as the same one, shifted; a reshaped one isn't.
    #[test]
    fn moving_keeps_the_picture() {
        let shape = scribbles(1).remove(0);
        let mut moved = shape.clone();
        moved.translate((12., -5.));
        assert_eq!(moved_by(&moved, &shape), Some((12., -5.)));
        let mut thicker = moved.clone();
        thicker.size += 1.;
        assert_eq!(moved_by(&thicker, &shape), None);
    }

    /// A cursor lands with its tip where it was placed, inside its box.
    #[test]
    fn cursor_tip_on_the_point() {
        /// A stand-in for the system's arrow: dark inside a light rim, its
        /// tip (the hotspot) at the top left.
        fn arrow(_: &str, height: f32) -> Option<(RgbaImage, (f64, f64))> {
            let side = height.round() as u32;
            let image = RgbaImage::from_fn(side, side, |x, y| {
                let edge = y as f32 * 0.6;
                match x as f32 {
                    x if x <= edge - 1.5 => image::Rgba([20, 20, 20, 255]),
                    x if x <= edge => image::Rgba([255, 255, 255, 255]),
                    _ => image::Rgba([0, 0, 0, 0]),
                }
            });
            Some((image, (0., 0.)))
        }
        kadr_raster::cursors::set_theme(arrow);
        let src = RgbaImage::from_pixel(200, 200, image::Rgba([255, 255, 255, 255]));
        let shape = Shape { kind: Kind::Cursor("arrow"), a: (80., 60.), b: (80., 60.), color: Rgba::hsl(0., 0., 0., 1.), size: 44., filled: false, text: TextStyle::Plain, bend: None };
        let out = rasterize(&src, std::slice::from_ref(&shape)).unwrap();
        // The arrow is dark inside a light rim, its tip at the point: ink just below and right of it,
        // none clearly above or left.
        let dark = |x: u32, y: u32| out.get_pixel(x, y).0[0] < 100;
        assert!((81..92).any(|x| (62..76).any(|y| dark(x, y))), "no arrow just inside the tip");
        let inked = |x: u32, y: u32| out.get_pixel(x, y).0[0] < 200;
        assert!(!(60..77).any(|x| (40..57).any(|y| inked(x, y))), "arrow drawn above and left of the tip");
        let (l, t, r, b) = shape.bounds();
        let inked = out.enumerate_pixels().filter(|(_, _, p)| p.0[0] < 250).map(|(x, y, _)| (x as f32, y as f32));
        assert!(inked.clone().all(|(x, y)| x >= l - 1. && x <= r + 1. && y >= t - 1. && y <= b + 1.), "drawn outside its box");
    }

    /// The canvas's picture of the strokes is what Save draws.
    #[test]
    fn stroke_picture_matches_the_export() {
        let shapes = scribbles(12);
        let saved = rasterize(&RgbaImage::new(800, 600), &shapes).unwrap();
        let (_, image) = shapes_image(&shapes, (0., 0.), (800., 600.), 1.).unwrap();
        let shown = image.as_bytes(0).unwrap();
        let worst = saved.as_raw().chunks_exact(4).zip(shown.chunks_exact(4)).map(|(a, b)| {
            // BGRA against RGBA; colour only matters where it shows.
            let rgb = (0..3).map(|i| (a[i] as i32 - b[2 - i] as i32).abs() * a[3] as i32 / 255).max().unwrap();
            rgb.max((a[3] as i32 - b[3] as i32).abs())
        }).max().unwrap();
        assert!(worst <= 2, "off by {worst}");
        let many = scribbles(400);
        let t = std::time::Instant::now();
        strokes_image(&many, 1.).unwrap();
        println!("400 strokes: {:?}", t.elapsed());
    }

    /// Blurring a plain area leaves it plain: no darker rim where the blur
    /// ran out of picture.
    #[test]
    fn blur_has_no_rim() {
        let src = RgbaImage::from_pixel(400, 300, image::Rgba([255, 255, 255, 255]));
        for (a, b) in [((100., 100.), (300., 250.)), ((0., 0.), (400., 300.))] {
            let patch = hide(&src, a, b, Hide::Blur, 1.).unwrap();
            let darkest = patch.pixels().map(|p| p.0[0].min(p.0[3])).min().unwrap();
            assert!(darkest >= 250, "darkest {darkest} for {a:?}-{b:?}");
        }
    }

    /// A window shot's shadow is cut off back to the window, corners and all.
    #[test]
    fn window_inside_its_shadow() {
        let window = RgbaImage::from_pixel(300, 200, image::Rgba([40, 40, 40, 255]));
        let shot = crate::capture::with_shadow(&window, 10., 1.);
        let (l, t, r, b) = opaque_bounds(&shot).expect("a margin");
        assert_eq!((r - l, b - t), (300., 200.));
        assert_eq!(opaque_bounds(&window), None);
    }

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
        let red = Rgba::hsl(0.01, 0.6, 0.54, 1.);
        let mut shapes = Vec::new();
        // Raw polyline (old behaviour) via a chain of lines, then the smoothed pen.
        let raw = trace(60.);
        for w in raw.windows(2) {
            shapes.push(Shape { kind: Kind::Line, a: w[0], b: w[1], color: red, size: 6., filled: false, text: TextStyle::Plain, bend: None });
        }
        shapes.push(Shape { kind: Kind::Pen(trace(160.)), a: (0., 0.), b: (0., 0.), color: red, size: 6., filled: false, text: TextStyle::Plain, bend: None });
        let png = kadr_raster::render_png(&src, &Doc { shapes, crop: None, ..Doc::default() }, &Frame::default()).unwrap();
        assert!(!png.is_empty());
        // KADR_TEST_OUT=/path/out.png to look at the result.
        if let Ok(out) = std::env::var("KADR_TEST_OUT") {
            std::fs::write(out, png).unwrap();
        }
    }

    fn red() -> Rgba {
        Rgba::hsl(0.01, 0.6, 0.54, 1.)
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
        let out = image::load_from_memory(&kadr_raster::render_png(&src, &doc, &Frame::default()).unwrap()).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (217, 100));
        assert!(out.get_pixel(210, 50).0[3] > 200, "the line goes on past the picture");
        assert_eq!(out.get_pixel(210, 10).0[3], 0, "and the new room is clear");
        // A crop reaching past the picture makes it bigger too.
        let doc = Doc { shapes: Vec::new(), crop: Some(((-10., -10.), (210., 100.))), ..Doc::default() };
        let out = image::load_from_memory(&kadr_raster::render_png(&src, &doc, &Frame::default()).unwrap()).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (220, 110));
        assert_eq!(out.get_pixel(5, 5).0[3], 0);
        assert_eq!(out.get_pixel(15, 15).0, [200, 200, 200, 255]);
        // Filled, the new room takes the fill; far past the old margin too.
        let doc = Doc { shapes: Vec::new(), crop: Some(((-60., -10.), (210., 100.))), fill: Some([255, 255, 255, 255]), ..Doc::default() };
        let out = image::load_from_memory(&kadr_raster::render_png(&src, &doc, &Frame::default()).unwrap()).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (270, 110));
        assert_eq!(out.get_pixel(5, 5).0, [255, 255, 255, 255]);
        assert_eq!(out.get_pixel(65, 15).0, [200, 200, 200, 255]);
        // On a backdrop the line still shows past the rounded picture.
        let frame = Frame { backdrop: Backdrop::Gradient(0), ..Frame::default() };
        let doc = Doc { shapes: doc_line(), crop: None, ..Doc::default() };
        let png = kadr_raster::render_png(&src, &doc, &frame).unwrap();
        let framed = image::load_from_memory(&png).unwrap().to_rgba8();
        let ((_, _), (left, top)) = frame.layout(217., 100.);
        let p = framed.get_pixel(left as u32 + 210, top as u32 + 50).0;
        assert!(p[0] > 150 && p[1] < 120, "red line on the backdrop: {p:?}");
    }

    /// On a background without padding, a stroke past the picture is cut
    /// at its edge instead of growing the result.
    #[test]
    fn no_padding_cuts_the_drawing() {
        let src = RgbaImage::from_pixel(200, 100, image::Rgba([200, 200, 200, 255]));
        let doc = Doc { shapes: doc_line(), ..Doc::default() };
        let frame = Frame { backdrop: Backdrop::Gradient(0), padding: 0., ..Frame::default() };
        let out = image::load_from_memory(&kadr_raster::render_png(&src, &doc, &frame).unwrap()).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (200, 100));
        // With padding, it still grows.
        let frame = Frame { padding: 0.1, ..frame };
        let out = image::load_from_memory(&kadr_raster::render_png(&src, &doc, &frame).unwrap()).unwrap().to_rgba8();
        assert!(out.width() > frame.layout(200., 100.).0 .0 as u32);
    }

    fn doc_line() -> Vec<Shape> {
        vec![Shape { kind: Kind::Line, a: (150., 50.), b: (215., 50.), color: red(), size: 4., filled: false, text: TextStyle::Plain, bend: None }]
    }

    #[test]
    fn pictures_go_into_the_export_at_their_box() {
        let src = RgbaImage::from_pixel(100, 80, image::Rgba([255, 255, 255, 255]));
        let mut png = Vec::new();
        RgbaImage::from_pixel(10, 10, image::Rgba([200, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let picture = decode_picture(&png).unwrap();
        let shape = Shape { kind: Kind::Picture(picture), a: (20., 20.), b: (40., 40.), color: Rgba::WHITE, size: 0., filled: false, text: TextStyle::Plain, bend: None };
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
            let patch = hide(&src, (100., 50.), (300., 250.), how, WEIGHT).unwrap();
            assert_eq!(patch.dimensions(), (200, 200));
            let shape = Shape { kind: Kind::Blur(how), a: (100., 50.), b: (300., 250.), color: red(), size: WEIGHT, filled: false, text: TextStyle::Plain, bend: None };
            let out = rasterize(&src, &[shape]).unwrap();
            // The export holds exactly the canvas's patch.
            for (x, y) in [(0, 0), (57, 91), (199, 199)] {
                assert_eq!(out.get_pixel(100 + x, 50 + y), patch.get_pixel(x, y));
            }
            assert_eq!(out.get_pixel(50, 50), src.get_pixel(50, 50), "outside stays sharp");
            assert!(spread(&out, (140, 100, 260, 200)) < 60, "{} hides the stripes", if how == Hide::Blur { "blur" } else { "pixelate" });
        }
        // The blur is smooth: neighbours differ little, unlike blocks.
        let blurred = hide(&src, (100., 50.), (300., 250.), Hide::Blur, WEIGHT).unwrap();
        let jumps = (1..200).map(|x| (blurred.get_pixel(x, 100).0[0] as i32 - blurred.get_pixel(x - 1, 100).0[0] as i32).abs()).max().unwrap();
        assert!(jumps < 8, "blur has no steps: {jumps}");
        // The slider sets the strength: blocks grow toward its end.
        let src = RgbaImage::from_fn(400, 300, |x, y| image::Rgba([(x * 7 % 256) as u8, (y * 5 % 256) as u8, 0, 255]));
        let run = |img: &RgbaImage| (1..200).take_while(|&x| img.get_pixel(x, 0) == img.get_pixel(0, 0)).count();
        let (weak, strong) = (hide(&src, (0., 0.), (200., 200.), Hide::Pixelate, 0.).unwrap(), hide(&src, (0., 0.), (200., 200.), Hide::Pixelate, 1.).unwrap());
        assert!(run(&strong) > run(&weak), "stronger pixelation has bigger blocks: {} vs {}", run(&strong), run(&weak));
    }

    #[test]
    fn hit_handles_and_move() {
        let mut line = Shape { kind: Kind::Arrow(Default::default()), a: (10., 10.), b: (110., 10.), color: red(), size: 4., filled: false, text: TextStyle::Plain, bend: None };
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
        let out = image::load_from_memory(&kadr_raster::render_png(&src, &Doc { shapes, crop: None, ..Doc::default() }, &Frame::default()).unwrap()).unwrap().to_rgba8();
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
        let out = image::load_from_memory(&kadr_raster::render_png(&src, &Doc { shapes, crop: None, ..Doc::default() }, &Frame::default()).unwrap()).unwrap().to_rgba8();
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
        let plain = image::load_from_memory(&kadr_raster::render_png(&src, &doc, &Frame::default()).unwrap()).unwrap();
        assert_eq!((plain.width(), plain.height()), (200, 160));
        let frame = Frame { backdrop: Backdrop::Gradient(0), ..Frame::default() };
        let png = kadr_raster::render_png(&src, &doc, &frame).unwrap();
        let framed = image::load_from_memory(&png).unwrap();
        // 6% of the longer side on each edge.
        assert_eq!((framed.width(), framed.height()), (224, 184));
        // A square output grows the short side.
        let square = Frame { aspect: 3, ..frame.clone() };
        let sq = image::load_from_memory(&kadr_raster::render_png(&src, &doc, &square).unwrap()).unwrap();
        assert_eq!((sq.width(), sq.height()), (224, 224));
        if let Ok(out) = std::env::var("KADR_TEST_FRAME") {
            let big = Doc { shapes: doc.shapes.clone(), crop: None, ..Doc::default() };
            std::fs::write(out, kadr_raster::render_png(&src, &big, &frame).unwrap()).unwrap();
        }
    }
}

impl Editor {
    /// For `kadr dev state`: the drawing as a project keeps it, and what the
    /// editor is doing with it.
    pub(crate) fn dev_state(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path.display().to_string(),
            "tool": format!("{:?}", self.tool).to_lowercase(),
            "color": crate::ui::to_rgba(self.color).to_hex(),
            "picking": self.picking,
            "recent_colors": self.recent.iter().map(|&c| crate::ui::to_rgba(c).to_hex()).collect::<Vec<_>>(),
            "saved_colors": self.saved.iter().map(|&c| crate::ui::to_rgba(c).to_hex()).collect::<Vec<_>>(),
            "selected": self.selected,
            "shapes": self.vm.doc.shapes.iter().map(kadr_core::project::shape_to_json).collect::<Vec<_>>(),
            "crop": self.vm.doc.crop.map(|(a, b)| [[a.0, a.1], [b.0, b.1]]),
            "cropping": self.cropping.is_some(),
            // The picture's size on screen over its real size.
            "zoom": self.disp.0 / self.real.0,
            // The text being typed: its content, the caret and selection
            // (bytes), the input method's marked part.
            "typing": self.editing.map(|_| serde_json::json!({
                "text": self.field.text(),
                "caret": self.field.caret(),
                "selection": [self.field.selection().start, self.field.selection().end],
                "marked": self.field.marked().map(|r| [r.start, r.end]),
            })),
            "frame": self.frame.to_json(),
            "undo": self.vm.history.depth(),
            "redo": self.vm.history.redo_depth(),
            "image": self.source.as_ref().map(|s| [s.width(), s.height()]),
        })
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
