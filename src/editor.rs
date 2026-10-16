//! Annotation editor in Zed's look with CleanShot's tools: draw on a
//! screenshot, then copy or save. The view lives on a full-monitor layer
//! (GPUI can't move or resize layers, and a modal overlay suits it anyway).
//! Saving re-draws every shape at full resolution with tiny-skia, so the file
//! matches what was on screen without the screen's scaling.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use gpui::{
    App, Context, CursorStyle, FocusHandle, Hsla, Image, ImageFormat, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, ObjectFit, PathBuilder, Point, Window, canvas, div, img,
    layer_shell::*, point, px, rgba,
};
use image::RgbaImage;
use ui::{ButtonLike, Divider, IconButtonShape, Tooltip, prelude::*};

use crate::kit::{self, LayerSpec};
use crate::{cards, palette, util};

/// A point in image pixels.
type P = (f32, f32);

/// Stroke widths, font sizes and counter sizes, in screen pixels.
const WIDTHS: [f32; 3] = [3., 5., 9.];
const FONT_SIZES: [f32; 3] = [16., 22., 32.];
const COUNTER_SIZES: [f32; 3] = [22., 28., 36.];
/// Opacity of the highlighter.
const MARKER_ALPHA: f32 = 0.4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Arrow,
    Rect,
    Ellipse,
    Line,
    Pen,
    Marker,
    Text,
    Counter,
    Blur,
}

impl Tool {
    const ALL: [Tool; 9] = [Tool::Arrow, Tool::Rect, Tool::Ellipse, Tool::Line, Tool::Pen, Tool::Marker, Tool::Text, Tool::Counter, Tool::Blur];

    fn icon(self) -> Icon {
        match self {
            Tool::Arrow => Icon::new(IconName::ArrowUpRight),
            Tool::Rect => Icon::from_path("icons/kadr/rect.svg"),
            Tool::Ellipse => Icon::from_path("icons/kadr/ellipse.svg"),
            Tool::Line => Icon::new(IconName::Slash),
            Tool::Pen => Icon::new(IconName::Pencil),
            Tool::Marker => Icon::from_path("icons/kadr/marker.svg"),
            Tool::Text => Icon::new(IconName::CaseSensitive),
            Tool::Counter => Icon::from_path("icons/kadr/counter.svg"),
            Tool::Blur => Icon::from_path("icons/kadr/pixelate.svg"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tool::Arrow => "Стрелка",
            Tool::Rect => "Прямоугольник",
            Tool::Ellipse => "Эллипс",
            Tool::Line => "Линия",
            Tool::Pen => "Карандаш",
            Tool::Marker => "Маркер",
            Tool::Text => "Текст",
            Tool::Counter => "Нумерация шагов",
            Tool::Blur => "Скрыть (пикселизация)",
        }
    }

    /// Single-key shortcut, CleanShot-style.
    fn key(self) -> &'static str {
        match self {
            Tool::Arrow => "a",
            Tool::Rect => "r",
            Tool::Ellipse => "o",
            Tool::Line => "l",
            Tool::Pen => "p",
            Tool::Marker => "h",
            Tool::Text => "t",
            Tool::Counter => "n",
            Tool::Blur => "b",
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
    /// The pixelated patch, once computed.
    Blur(Option<Arc<Image>>),
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
    shapes: Vec<Shape>,
    redo: Vec<Shape>,
    draft: Option<Shape>,
    /// Index of the text shape being typed.
    editing: Option<usize>,
    tool: Tool,
    colors: Vec<Hsla>,
    color: usize,
    weight: usize,
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
    // leave room for the toolbars.
    let natural = (w as f32 / mon.scale as f32, h as f32 / mon.scale as f32);
    let fit = ((mw - 160.) / natural.0).min((mh - 240.) / natural.1).min(1.0);
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
            })
            .detach();
            Editor {
                path,
                image,
                px: (w, h),
                source: None,
                disp,
                shapes: Vec::new(),
                redo: Vec::new(),
                draft: None,
                editing: None,
                tool: Tool::Arrow,
                colors: palette::annotation_colors(),
                color: 0,
                weight: 1,
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

    fn finish_text(&mut self) {
        if let Some(i) = self.editing.take()
            && matches!(&self.shapes[i].kind, Kind::Text(t) if t.trim().is_empty())
        {
            self.shapes.remove(i);
        }
    }

    fn push(&mut self, shape: Shape) {
        self.shapes.push(shape);
        self.redo.clear();
    }

    fn undo(&mut self) {
        self.finish_text();
        if let Some(s) = self.shapes.pop() {
            self.redo.push(s);
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.shapes.push(s);
        }
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        self.finish_text();
        let Some(p) = self.to_image(e.position, false) else { return };
        let s = self.scale();
        let color = self.current_color();
        let kind = match self.tool {
            Tool::Text => {
                self.push(Shape { kind: Kind::Text(String::new()), a: p, b: p, color, size: FONT_SIZES[self.weight] / s });
                self.editing = Some(self.shapes.len() - 1);
                return cx.notify();
            }
            Tool::Counter => {
                let n = self.shapes.iter().filter(|s| matches!(s.kind, Kind::Counter(_))).count() as u32 + 1;
                self.push(Shape { kind: Kind::Counter(n), a: p, b: p, color, size: COUNTER_SIZES[self.weight] / s });
                return cx.notify();
            }
            Tool::Arrow => Kind::Arrow,
            Tool::Rect => Kind::Rect,
            Tool::Ellipse => Kind::Ellipse,
            Tool::Line => Kind::Line,
            Tool::Pen => Kind::Pen(vec![p]),
            Tool::Marker => Kind::Marker(vec![p]),
            Tool::Blur => Kind::Blur(None),
        };
        let width = if self.tool == Tool::Marker { WIDTHS[self.weight] * 3. } else { WIDTHS[self.weight] };
        self.draft = Some(Shape { kind, a: p, b: p, color, size: width / s });
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        if e.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some(p) = self.to_image(e.position, true) else { return };
        let Some(d) = self.draft.as_mut() else { return };
        let mut b = p;
        // Shift: squares, circles and 45° lines.
        if e.modifiers.shift {
            let (dx, dy) = (p.0 - d.a.0, p.1 - d.a.1);
            match d.kind {
                Kind::Rect | Kind::Ellipse | Kind::Blur(_) => {
                    let m = dx.abs().max(dy.abs());
                    b = (d.a.0 + m * dx.signum(), d.a.1 + m * dy.signum());
                }
                Kind::Line | Kind::Arrow => {
                    let angle = dy.atan2(dx);
                    let snapped = (angle / std::f32::consts::FRAC_PI_4).round() * std::f32::consts::FRAC_PI_4;
                    let len = dx.hypot(dy);
                    b = (d.a.0 + len * snapped.cos(), d.a.1 + len * snapped.sin());
                }
                _ => {}
            }
        }
        d.b = b;
        if let Kind::Pen(points) | Kind::Marker(points) = &mut d.kind
            && points.last().is_none_or(|l| (l.0 - p.0).hypot(l.1 - p.1) > 1.5)
        {
            points.push(p);
        }
        cx.notify();
    }

    fn mouse_up(&mut self, cx: &mut Context<Self>) {
        let Some(shape) = self.draft.take() else { return };
        let tiny = (shape.a.0 - shape.b.0).abs() < 3. && (shape.a.1 - shape.b.1).abs() < 3.;
        let freehand = matches!(shape.kind, Kind::Pen(ref p) | Kind::Marker(ref p) if p.len() > 1);
        if tiny && !freehand {
            return cx.notify();
        }
        if matches!(shape.kind, Kind::Blur(_)) {
            self.push(shape);
            let index = self.shapes.len() - 1;
            if let Some(src) = self.source.clone() {
                let (a, b) = (self.shapes[index].a, self.shapes[index].b);
                cx.spawn(async move |this, cx| {
                    let patch = cx.background_executor().spawn(async move { pixelate_png(&src, a, b) }).await;
                    let _ = this.update(cx, |editor, cx| {
                        if let Some(Shape { kind: Kind::Blur(slot), .. }) = editor.shapes.get_mut(index) {
                            *slot = patch.map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        } else {
            self.push(shape);
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
                "enter" => push_text(&mut self.shapes[i], "\n"),
                "backspace" => {
                    if let Kind::Text(t) = &mut self.shapes[i].kind {
                        t.pop();
                    }
                }
                _ if !ctrl => {
                    if let Some(ch) = &k.key_char {
                        push_text(&mut self.shapes[i], ch);
                    }
                }
                _ => {}
            }
            return cx.notify();
        }

        match (k.key.as_str(), ctrl, shift) {
            ("escape", _, _) => window.remove_window(),
            ("z", true, false) => self.undo(),
            ("z", true, true) | ("y", true, _) => self.redo(),
            ("c", true, _) => self.export(Export::Copy, window, cx),
            ("s", true, _) => self.export(Export::Save, window, cx),
            (key, false, false) => {
                if let Some(tool) = Tool::ALL.into_iter().find(|t| t.key() == key) {
                    self.tool = tool;
                } else if let Some(n) = key.parse::<usize>().ok().filter(|n| (1..=self.colors.len()).contains(n)) {
                    self.color = n - 1;
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn export(&mut self, how: Export, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_text();
        let Some(src) = self.source.clone() else {
            return util::notify("kadr", "Картинка ещё загружается, попробуй через секунду");
        };
        let shapes = self.shapes.clone();
        let path = self.path.clone();
        window.remove_window();
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let png = cx.background_executor().spawn(async move { rasterize(&src, &shapes) }).await;
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

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tools = Tool::ALL.into_iter().map(|tool| {
            ButtonLike::new(SharedString::from(format!("tool-{}", tool.key())))
                .style(ButtonStyle::Subtle)
                .size(ButtonSize::Large)
                .toggle_state(self.tool == tool)
                .child(div().px_1().child(tool.icon().size(IconSize::Small)))
                .tooltip(Tooltip::text(format!("{} ({})", tool.label(), tool.key().to_uppercase())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.finish_text();
                    this.tool = tool;
                    cx.notify();
                }))
        });
        let ring = cx.theme().colors().text;
        let swatches = self.colors.iter().enumerate().map(|(i, &c)| {
            let selected = i == self.color;
            div()
                .id(("color", i))
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_2()
                .border_color(if selected { ring } else { gpui::transparent_black() })
                .cursor_pointer()
                .child(div().size(px(14.)).rounded_full().bg(c))
                .tooltip(Tooltip::text(format!("Цвет ({})", i + 1)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.color = i;
                    cx.notify();
                }))
        });
        let dot = cx.theme().colors().text;
        let weights = WIDTHS.iter().enumerate().map(|(i, w)| {
            ButtonLike::new(("weight", i))
                .style(ButtonStyle::Subtle)
                .size(ButtonSize::Large)
                .toggle_state(i == self.weight)
                .child(div().w(px(18.)).flex().justify_center().child(div().size(px(w + 1.)).rounded_full().bg(dot)))
                .tooltip(Tooltip::text(["Тонко", "Средне", "Толсто"][i]))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.weight = i;
                    cx.notify();
                }))
        });

        h_flex()
            .gap_0p5()
            .p_1()
            .elevation_3(cx)
            .children(tools)
            .child(Divider::vertical().h_5().mx_1())
            .child(h_flex().gap_0p5().children(swatches))
            .child(Divider::vertical().h_5().mx_1())
            .children(weights)
            .child(Divider::vertical().h_5().mx_1())
            .child(
                IconButton::new("undo", IconName::Undo)
                    .shape(IconButtonShape::Square)
                    .size(ButtonSize::Large)
                    .disabled(self.shapes.is_empty())
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
                    .disabled(self.redo.is_empty())
                    .child(div().px_1().child(Icon::from_path("icons/kadr/redo.svg").size(IconSize::Small).color(if self.redo.is_empty() { Color::Disabled } else { Color::Default })))
                    .tooltip(Tooltip::text("Повторить (Ctrl+Shift+Z)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.redo();
                        cx.notify();
                    })),
            )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        h_flex()
            .w(px(self.disp.0.max(560.)))
            .justify_between()
            .gap_2()
            .pl_2p5()
            .pr_1()
            .py_1()
            .elevation_3(cx)
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .child(Icon::new(IconName::Image).size(IconSize::Small).color(Color::Muted))
                    .child(div().min_w_0().child(Label::new(name).size(LabelSize::Small).truncate()))
                    .child(Label::new(format!("{}×{}", self.px.0, self.px.1)).size(LabelSize::Small).color(Color::Muted)),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
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

    /// Picture with blur patches, vector shapes, and text/counters on top.
    fn render_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.scale();
        let at = |p: P| (px(p.0 * s), px(p.1 * s));

        let patches = self.shapes.iter().filter_map(|shape| match &shape.kind {
            Kind::Blur(Some(patch)) => {
                let (l, t, r, b) = rect(shape.a, shape.b);
                Some(img(patch.clone()).absolute().left(px(l * s)).top(px(t * s)).w(px((r - l) * s)).h(px((b - t) * s)).object_fit(ObjectFit::Fill))
            }
            _ => None,
        });

        let vector: Vec<Shape> = self.shapes.iter().chain(self.draft.iter()).cloned().collect();
        let origin = self.origin.clone();
        let strokes = canvas(
            move |bounds, _, _| {
                origin.set((f32::from(bounds.origin.x), f32::from(bounds.origin.y)));
                bounds
            },
            move |_, bounds, window, _| {
                let o = bounds.origin;
                let pt = |p: P| point(o.x + px(p.0 * s), o.y + px(p.1 * s));
                for shape in &vector {
                    paint_shape(window, shape, s, &pt);
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        let labels = self.shapes.iter().enumerate().filter_map(|(i, shape)| match &shape.kind {
            Kind::Text(text) => {
                let (x, y) = at(shape.a);
                let m = metrics(shape.size);
                let editing = self.editing == Some(i);
                let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
                let count = lines.len();
                Some(
                    div()
                        .absolute()
                        .left(x)
                        .top(y)
                        .text_size(px(shape.size * s))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(shape.color)
                        .whitespace_nowrap()
                        .children(lines.into_iter().enumerate().map(move |(li, line)| {
                            let caret = editing && li + 1 == count;
                            div().h(px(m.line * s)).line_height(px(m.line * s)).child(if caret { format!("{line}▏") } else { line })
                        }))
                        .into_any_element(),
                )
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

        div()
            .id("canvas")
            .relative()
            .w(px(self.disp.0))
            .h(px(self.disp.1))
            .rounded_md()
            .overflow_hidden()
            .shadow(kit::shadow())
            .cursor(if self.tool == Tool::Text { CursorStyle::IBeam } else { CursorStyle::Crosshair })
            .child(img(self.image.clone()).size_full().object_fit(ObjectFit::Fill))
            .children(patches)
            .child(strokes)
            .children(labels)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.mouse_down(e, cx)))
    }
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        kit::root(cx)
            .bg(rgba(0x00000099))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            // The pointer stays with this surface while the button is held.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.mouse_up(cx)))
            .child(self.render_toolbar(cx))
            .child(self.render_canvas(cx))
            .child(self.render_footer(cx))
    }
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
    let stroke = |window: &mut Window, points: &[P], closed: bool, color: Hsla, width: f32| {
        let Some((first, rest)) = points.split_first() else { return };
        let mut b = PathBuilder::stroke(px(width * s));
        b.move_to(pt(*first));
        for p in rest {
            b.line_to(pt(*p));
        }
        if closed {
            b.close();
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
    };
    let (a, b, c, w) = (shape.a, shape.b, shape.color, shape.size);
    match &shape.kind {
        Kind::Rect => stroke(window, &[a, (b.0, a.1), b, (a.0, b.1)], true, c, w),
        Kind::Ellipse => stroke(window, &ellipse_points(a, b), true, c, w),
        Kind::Line => stroke(window, &[a, b], false, c, w),
        Kind::Pen(points) => stroke(window, points, false, c, w),
        Kind::Marker(points) => {
            let mut faded = c;
            faded.a = MARKER_ALPHA;
            stroke(window, points, false, faded, w)
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
        Kind::Blur(Some(_)) | Kind::Text(_) | Kind::Counter(_) => {}
    }
}

/// White or near-black text, whichever reads better on `c`.
fn on_color(c: Hsla) -> Hsla {
    if c.l > 0.65 { gpui::hsla(0., 0., 0.08, 1.) } else { gpui::white() }
}

fn font() -> &'static FontRef<'static> {
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| FontRef::try_from_slice(include_bytes!("../data/zed/fonts/IBMPlexSans-SemiBold.ttf")).expect("bundled font"))
}

struct Metrics {
    ascent: f32,
    descent: f32,
    /// Line box height; text sits centered in it, as in GPUI.
    line: f32,
}

fn metrics(size: f32) -> Metrics {
    let f = font().as_scaled(PxScale::from(size));
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

/// Draw everything onto the original at full resolution; returns PNG bytes.
fn rasterize(src: &RgbaImage, shapes: &[Shape]) -> Option<Vec<u8>> {
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
                    pm.stroke_path(&path, &solid, &stroke_for(size), Transform::identity(), None);
                }
            }
            Kind::Ellipse => {
                let (l, t, r, bt) = rect(a, b);
                if let Some(path) = Rect::from_ltrb(l, t, r, bt).and_then(PathBuilder::from_oval) {
                    pm.stroke_path(&path, &solid, &stroke_for(size), Transform::identity(), None);
                }
            }
            Kind::Line => {
                if let Some(path) = polyline(&[a, b], false) {
                    pm.stroke_path(&path, &solid, &stroke_for(size), Transform::identity(), None);
                }
            }
            Kind::Pen(points) => {
                if let Some(path) = polyline(points, false) {
                    pm.stroke_path(&path, &solid, &stroke_for(size), Transform::identity(), None);
                }
            }
            Kind::Marker(points) => {
                if let Some(path) = polyline(points, false) {
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
                for (i, line) in text.split('\n').enumerate() {
                    let top = a.1 + i as f32 * m.line;
                    let baseline = top + (m.line - (m.ascent - m.descent)) / 2. + m.ascent;
                    draw_line(&mut pm, line, (a.0, baseline), size, c);
                }
            }
            Kind::Blur(_) => {}
        }
    }
    pm.encode_png().ok()
}

fn text_width(text: &str, size: f32) -> f32 {
    let f = font().as_scaled(PxScale::from(size));
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
fn draw_line(pm: &mut tiny_skia::Pixmap, text: &str, origin: P, size: f32, color: Hsla) {
    let font = font();
    let scale = PxScale::from(size);
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

