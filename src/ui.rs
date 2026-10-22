//! kadr's own components in Zed's look: buttons, icons, labels, a switch, a
//! tooltip, key hints and a progress bar, styled from the Omarchy theme.
//! They depend on GPUI alone, so kadr can follow GPUI (or a fork of it)
//! without Zed's editor crates, and every detail can be tuned for kadr.
//!
//! Sizes are in rems built from Zed's pixel values, so they grow with the
//! window's rem size (`kit::REM`) the same way Zed's components did.

use gpui::{
    AnyElement, AnyView, App, Background, BorderStyle, Bounds, BoxShadow, ClickEvent, Div, PaintQuad, Pixels, ElementId, Global, Hsla, IntoElement,
    ParentElement, Point, Rems, RenderOnce, SharedString, Stateful, Styled, StyleRefinement, Window, canvas, div, hsla, point,
    prelude::*, px, relative, rems, svg,
};
use palette::IntoColor;
use smallvec::SmallVec;

use crate::kit;

pub mod prelude {
    pub use gpui::prelude::*;
    pub use gpui::{AnyElement, Div, InteractiveElement, ParentElement, SharedString, Styled, div, px};

    pub use super::{
        ActiveTheme, Button, ButtonSize, ButtonStyle, Color, Icon, IconButton, IconName, IconSize, Label, LabelSize, StyledExt, TipExt, h_flex, v_flex,
    };
}

/// A Zed pixel size as rems at Zed's 16px base.
pub fn rems_from_px(px: f32) -> Rems {
    rems(px / 16.)
}

/// A colour as plain 0..1 RGBA, for kadr's own rasterising.
#[derive(Clone, Copy, Debug)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

pub fn rgb_of(color: Hsla) -> Rgb {
    let c = gpui::hsla_to_rgba(color);
    Rgb { r: c.red, g: c.green, b: c.blue, a: c.alpha }
}

/// `0xRRGGBB` as a colour.
pub fn from_hex(hex: u32) -> Hsla {
    gpui::rgb(hex).into_color()
}

/// GPUI CE paints quads through `PaintQuad`; these keep the familiar calls.
pub fn fill(bounds: Bounds<Pixels>, color: impl Into<Background>) -> PaintQuad {
    PaintQuad::new(bounds).background(color)
}

pub fn outline(bounds: Bounds<Pixels>, color: impl Into<Background>, style: BorderStyle) -> PaintQuad {
    PaintQuad::new(bounds).border_widths(px(1.)).border_color(color).border_style(style)
}

pub fn quad(bounds: Bounds<Pixels>, radius: Pixels, background: impl Into<Background>, border: Pixels, border_color: impl Into<Background>, style: BorderStyle) -> PaintQuad {
    PaintQuad::new(bounds).corner_radii(radius).background(background).border_widths(border).border_color(border_color).border_style(style)
}

/// A selection knob like CleanShot's: an accent dot in a white ring with
/// a soft shadow under it, centred on `c`.
pub fn paint_knob(window: &mut gpui::Window, c: gpui::Point<Pixels>, accent: Hsla) {
    let disc = |r: f32, dy: f32| Bounds::new(gpui::point(c.x - px(r), c.y - px(r) + px(dy)), gpui::size(px(2. * r), px(2. * r)));
    window.paint_quad(fill(disc(11.5, 1.5), hsla(0., 0., 0., 0.1)).corner_radii(px(11.5)));
    window.paint_quad(fill(disc(10.5, 1.), hsla(0., 0., 0., 0.18)).corner_radii(px(10.5)));
    window.paint_quad(quad(disc(9.5, 0.), px(9.5), accent, px(2.5), gpui::white(), BorderStyle::Solid));
}

/// White or near-black, whichever reads on `background`.
pub fn on(background: Hsla) -> Hsla {
    if background.lightness > 0.62 { hsla(0., 0., 0.1, 1.) } else { hsla(0., 0., 1., 1.) }
}

/// Corner smoothing for panels and windows (0 is circular).
pub const SMOOTHING: f32 = 0.6;

pub fn h_flex() -> Div {
    div().flex().flex_row().items_center()
}

pub fn v_flex() -> Div {
    div().flex().flex_col()
}

// ---------------------------------------------------------------- theme

#[derive(Clone, Debug)]
pub struct Colors {
    pub panel_background: Hsla,
    pub editor_background: Hsla,
    pub title_bar_background: Hsla,
    pub status_bar_background: Hsla,
    pub elevated_surface_background: Hsla,
    pub border: Hsla,
    pub border_variant: Hsla,
    pub element_background: Hsla,
    pub element_hover: Hsla,
    pub element_active: Hsla,
    pub element_selected: Hsla,
    pub ghost_element_hover: Hsla,
    pub ghost_element_active: Hsla,
    pub ghost_element_selected: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_placeholder: Hsla,
    pub text_disabled: Hsla,
    pub text_accent: Hsla,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub error: Hsla,
    pub warning: Hsla,
    pub info: Hsla,
}

/// The colors everything is drawn with, from the Omarchy theme
/// (`palette::theme`).
#[derive(Clone, Debug)]
pub struct Theme {
    pub light: bool,
    pub colors: Colors,
    pub status: Status,
}

impl Global for Theme {}

impl Theme {
    pub fn colors(&self) -> &Colors {
        &self.colors
    }

    pub fn status(&self) -> &Status {
        &self.status
    }
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}

// ---------------------------------------------------------------- colors

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Color {
    #[default]
    Default,
    Muted,
    Accent,
    Error,
    Disabled,
    Placeholder,
    Custom(Hsla),
}

impl Color {
    pub fn color(self, cx: &App) -> Hsla {
        let theme = cx.theme();
        match self {
            Color::Default => theme.colors.text,
            Color::Muted => theme.colors.text_muted,
            Color::Accent => theme.colors.text_accent,
            Color::Error => theme.status.error,
            Color::Disabled => theme.colors.text_disabled,
            Color::Placeholder => theme.colors.text_placeholder,
            Color::Custom(color) => color,
        }
    }
}

pub trait StyledExt: Styled + Sized {
    /// A raised surface: popovers and floating bars.
    fn elevation_2(self, cx: &App) -> Self {
        let light = cx.theme().light;
        self.elevated(cx, vec![
            BoxShadow { color: hsla(0., 0., 0., if light { 0.10 } else { 0.28 }).into(), offset: point(px(0.), px(4.)), blur_radius: px(14.), spread_radius: px(0.), inset: false },
            BoxShadow { color: hsla(0., 0., 0., if light { 0.06 } else { 0.18 }).into(), offset: point(px(0.), px(1.)), blur_radius: px(3.), spread_radius: px(0.), inset: false },
        ])
    }

    /// The highest surface: toasts, panels over the screen.
    fn elevation_3(self, cx: &App) -> Self {
        let light = cx.theme().light;
        self.elevated(cx, vec![
            BoxShadow { color: hsla(0., 0., 0., if light { 0.14 } else { 0.36 }).into(), offset: point(px(0.), px(10.)), blur_radius: px(30.), spread_radius: px(0.), inset: false },
            BoxShadow { color: hsla(0., 0., 0., if light { 0.08 } else { 0.22 }).into(), offset: point(px(0.), px(2.)), blur_radius: px(6.), spread_radius: px(0.), inset: false },
        ])
    }

    fn elevated(self, cx: &App, shadow: Vec<BoxShadow>) -> Self {
        let colors = cx.theme().colors();
        // Soft squircle surfaces with a hairline edge, macOS-style.
        self.bg(colors.elevated_surface_background).rounded(rems_from_px(12.)).rounded_smoothing(SMOOTHING).border_1().border_color(colors.border).shadow(shadow)
    }
}

impl<E: Styled> StyledExt for E {}

// ---------------------------------------------------------------- icons

/// Icons from Zed's set, bundled under `icons/<name>.svg` (`kit::Assets`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconName {
    ArrowUpRight,
    AudioOn,
    CaseSensitive,
    Check,
    ChevronDown,
    Circle,
    Close,
    Crosshair,
    Dash,
    DebugPause,
    Folder,
    Image,
    Keyboard,
    MagnifyingGlass,
    Mic,
    Pencil,
    Pin,
    PlayFilled,
    Plus,
    Scissors,
    Screen,
    Settings,
    Slash,
    Sparkle,
    Stop,
    Trash,
    Undo,
    Restart,
}

impl IconName {
    pub fn path(self) -> &'static str {
        match self {
            IconName::ArrowUpRight => "icons/arrow_up_right.svg",
            IconName::AudioOn => "icons/audio_on.svg",
            IconName::CaseSensitive => "icons/case_sensitive.svg",
            IconName::Check => "icons/check.svg",
            IconName::ChevronDown => "icons/chevron_down.svg",
            IconName::Circle => "icons/circle.svg",
            IconName::Close => "icons/close.svg",
            IconName::Crosshair => "icons/crosshair.svg",
            IconName::Dash => "icons/dash.svg",
            IconName::DebugPause => "icons/debug_pause.svg",
            IconName::Folder => "icons/folder.svg",
            IconName::Image => "icons/image.svg",
            IconName::Keyboard => "icons/keyboard.svg",
            IconName::MagnifyingGlass => "icons/magnifying_glass.svg",
            IconName::Mic => "icons/mic.svg",
            IconName::Pencil => "icons/pencil.svg",
            IconName::Pin => "icons/pin.svg",
            IconName::PlayFilled => "icons/play_filled.svg",
            IconName::Plus => "icons/plus.svg",
            IconName::Scissors => "icons/scissors.svg",
            IconName::Screen => "icons/screen.svg",
            IconName::Settings => "icons/settings.svg",
            IconName::Slash => "icons/slash.svg",
            IconName::Sparkle => "icons/sparkle.svg",
            IconName::Stop => "icons/stop.svg",
            IconName::Trash => "icons/trash.svg",
            IconName::Undo => "icons/undo.svg",
            IconName::Restart => "icons/kadr/restart.svg",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum IconSize {
    XSmall,
    #[default]
    Small,
    /// Toolbar icons.
    Medium,
    /// The editor's drawing tools, a step up from the other toolbars.
    Tool,
    /// The capture bar's big mode icons.
    Large,
}

impl IconSize {
    pub fn rems(self) -> Rems {
        rems_from_px(match self {
            IconSize::XSmall => 12.,
            IconSize::Small => 14.,
            IconSize::Medium => 16.,
            IconSize::Tool => 18.,
            IconSize::Large => 22.,
        })
    }
}

#[derive(IntoElement)]
pub struct Icon {
    path: SharedString,
    size: IconSize,
    color: Color,
}

impl Icon {
    pub fn new(name: IconName) -> Self {
        Icon::from_path(name.path())
    }

    pub fn from_path(path: impl Into<SharedString>) -> Self {
        Icon { path: path.into(), size: IconSize::default(), color: Color::Default }
    }

    pub fn size(mut self, size: IconSize) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        svg().flex_none().path(self.path).size(self.size.rems()).text_color(self.color.color(cx))
    }
}

// ---------------------------------------------------------------- labels

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LabelSize {
    #[default]
    Default,
    Small,
    XSmall,
}

impl LabelSize {
    pub fn rems(self) -> Rems {
        rems_from_px(match self {
            LabelSize::Default => 14.,
            LabelSize::Small => 12.,
            LabelSize::XSmall => 10.,
        })
    }
}

#[derive(IntoElement)]
pub struct Label {
    text: SharedString,
    size: LabelSize,
    color: Color,
    truncate: bool,
}

impl Label {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Label { text: text.into(), size: LabelSize::Default, color: Color::Default, truncate: false }
    }

    pub fn size(mut self, size: LabelSize) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// One line, cut with an ellipsis when it doesn't fit.
    pub fn truncate(mut self) -> Self {
        self.truncate = true;
        self
    }
}

impl RenderOnce for Label {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .text_size(self.size.rems())
            .text_color(self.color.color(cx))
            .when(self.truncate, |label| label.overflow_hidden().whitespace_nowrap().text_ellipsis())
            .child(self.text)
    }
}

// ---------------------------------------------------------------- divider

/// A hairline between groups; style it like any element (`.h_5().mx_1()`).
#[derive(IntoElement)]
pub struct Divider {
    vertical: bool,
    style: StyleRefinement,
}

impl Divider {
    pub fn vertical() -> Self {
        Divider { vertical: true, style: StyleRefinement::default() }
    }

}

impl Styled for Divider {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Divider {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // Groups are told apart by space, not lines: a vertical divider is
        // just air, a horizontal one a barely-there hairline.
        let mut line = div().flex_none();
        line = if self.vertical { line.w(px(1.)).h_full() } else { line.h(px(1.)).w_full().bg(cx.theme().colors().border_variant) };
        line.style().refine(&self.style);
        line
    }
}

// ---------------------------------------------------------------- key hints

/// A shortcut as Zed shows it on Linux: "Ctrl-S", "Escape".
#[derive(Clone, IntoElement)]
pub struct KeyBinding {
    text: SharedString,
    color: Option<Hsla>,
}

impl KeyBinding {
    /// From a GPUI keystroke source like "ctrl-shift-z" or "escape".
    pub fn new(source: &str) -> Self {
        let words = source.split_whitespace().map(|stroke| {
            stroke
                .split('-')
                .map(|part| match part {
                    "ctrl" => "Ctrl".to_string(),
                    "shift" => "Shift".to_string(),
                    "alt" => "Alt".to_string(),
                    "super" | "cmd" | "platform" => "Super".to_string(),
                    key if key.chars().count() == 1 => key.to_uppercase(),
                    key => {
                        let mut chars = key.chars();
                        chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
                    }
                })
                .collect::<Vec<_>>()
                .join("-")
        });
        KeyBinding { text: words.collect::<Vec<_>>().join(" ").into(), color: None }
    }
}

impl RenderOnce for KeyBinding {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // Smaller than the label it follows, on the same line.
        div().flex_none().px_0p5().text_size(LabelSize::Small.rems()).text_color(self.color.unwrap_or_else(|| Color::Muted.color(cx))).child(self.text)
    }
}

// ---------------------------------------------------------------- tooltip

thread_local! {
    /// The bounds of the last element with a tooltip found under the pointer
    /// while painting, and its window: where the tooltip lines up.
    static TIP_ANCHOR: std::cell::Cell<Option<(gpui::WindowId, Bounds<Pixels>)>> = const { std::cell::Cell::new(None) };
}

/// Notes the element it's inside (which must be positioned) as a tooltip's
/// anchor while the pointer is over it. GPUI only tells a tooltip where the
/// pointer is, not what it belongs to.
pub fn tip_anchor() -> AnyElement {
    canvas(
        |bounds, window, _| {
            if bounds.contains(&window.mouse_position()) {
                TIP_ANCHOR.set(Some((window.window_handle().window_id(), bounds)));
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

/// `.tip(...)`: a tooltip centred under the element instead of at the
/// pointer.
pub trait TipExt: gpui::StatefulInteractiveElement + ParentElement + Styled + Sized {
    fn tip(self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.relative().tooltip(tooltip).child(tip_anchor())
    }
}

impl<E: gpui::StatefulInteractiveElement + ParentElement + Styled + Sized> TipExt for E {}

/// The tooltips' dark plate with small white text, also for other hints
/// (the picker's keys); dimmer parts take `TIP_DIM`.
pub fn tip_plate() -> Div {
    h_flex()
        .rounded(rems_from_px(6.))
        .rounded_smoothing(SMOOTHING)
        .bg(gpui::black())
        // A hairline, like the desktop's window borders, so the black plate
        // still reads on a dark screen.
        .border_1()
        .border_color(gpui::hsla(0., 0., 1., 0.14))
        .font_family(kit::UI_FONT)
        .text_size(LabelSize::XSmall.rems())
        .text_color(gpui::white())
        .px(px(TOOLTIP_PAD))
}

/// Shortcuts and other secondary words on a `tip_plate`.
pub const TIP_DIM: Hsla = gpui::hsla(0., 0., 1., 0.5);

/// A floating hint: a few words, maybe a shortcut.
pub struct Tooltip {
    title: SharedString,
    key_binding: Option<KeyBinding>,
    /// Where the pointer was and what was under it when it appeared; GPUI
    /// places the tooltip from that pointer position.
    placed: Option<(Point<Pixels>, Option<Bounds<Pixels>>)>,
}

impl Tooltip {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Tooltip { title: title.into(), key_binding: None, placed: None }
    }

    /// For `.tooltip(...)`: a plain text hint.
    pub fn text(title: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let title = title.into();
        move |_, cx| cx.new(|_| Tooltip::new(title.clone())).into()
    }

    pub fn key_binding(mut self, key_binding: impl Into<Option<KeyBinding>>) -> Self {
        self.key_binding = key_binding.into();
        self
    }
}

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        // A few white words on a dark plate, centred under the element it
        // belongs to (above it near the bottom of the window), like macOS's.
        let size = LabelSize::XSmall.rems().to_pixels(window.rem_size());
        let font = gpui::font(kit::UI_FONT);
        let width_of = |text: &str| {
            let run = gpui::TextRun { len: text.len(), font: font.clone(), ..Default::default() };
            f32::from(window.text_system().layout_line(text, size, &[run]).width)
        };
        let keys_text = self.key_binding.as_ref().map(|k| k.text.clone());
        let keys = keys_text.as_ref().map_or(0., |k| width_of(k) + 8.);
        let width = (width_of(&self.title) + keys).min(TOOLTIP_MAX) + 2. * TOOLTIP_PAD + 2.;
        let height = (f32::from(size) * 1.6).round() + 4.;
        let window_id = window.window_handle().window_id();
        let (mouse, anchor) = *self.placed.get_or_insert_with(|| {
            let mouse = window.mouse_position();
            let anchor = TIP_ANCHOR.get().filter(|(id, b)| *id == window_id && b.contains(&mouse)).map(|(_, b)| b);
            (mouse, anchor)
        });
        let (mx, my) = (f32::from(mouse.x), f32::from(mouse.y));
        let room = window.viewport_size();
        let (rw, rh) = (f32::from(room.width), f32::from(room.height));
        // Where GPUI puts a box this size: a pixel off the pointer, flipped
        // to the other side at the window's edges.
        let origin_x = if mx + 1. + width <= rw { mx + 1. } else { (mx - width - 1.).max(0.) };
        let origin_y = if my + 1. + height <= rh { my + 1. } else { (my - height - 1.).max(0.) };
        // Where it should be: the element's middle, clear of it.
        let (centre, top, bottom) = match anchor {
            Some(b) => (f32::from(b.center().x), f32::from(b.top()), f32::from(b.bottom())),
            None => (mx, my - 4., my + 14.),
        };
        let x = (centre - width / 2.).clamp(4., (rw - width - 4.).max(4.));
        let y = if bottom + TOOLTIP_GAP + height <= rh - 4. { bottom + TOOLTIP_GAP } else { (top - TOOLTIP_GAP - height).max(4.) };
        div().w(px(width)).h(px(height)).child(
            tip_plate()
                .relative()
                .left(px(x - origin_x))
                .top(px(y - origin_y))
                .w(px(width))
                .h(px(height))
                .justify_center()
                .gap_1()
                .child(div().max_w(px(TOOLTIP_MAX)).truncate().child(self.title.clone()))
                // The shortcut, dimmer, in the same small size.
                .children(keys_text.map(|keys| div().flex_none().text_color(TIP_DIM).child(keys))),
        )
    }
}

/// The widest a tooltip's text gets before it wraps, and its side padding.
const TOOLTIP_MAX: f32 = 240.;
const TOOLTIP_PAD: f32 = 6.;

/// The gap between a tooltip and its element.
const TOOLTIP_GAP: f32 = 6.;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type TooltipBuilder = Box<dyn Fn(&mut Window, &mut App) -> AnyView + 'static>;

// ---------------------------------------------------------------- buttons

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonStyle {
    /// The one main action: the accent colour.
    Accent,
    /// A soft grey button.
    Filled,
    /// Background only on hover.
    #[default]
    Subtle,
    /// A border, background on hover.
    Outlined,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    Large,
    #[default]
    Default,
}

impl ButtonSize {
    fn height(self) -> Rems {
        rems_from_px(match self {
            ButtonSize::Large => 32.,
            ButtonSize::Default => 22.,
        })
    }

    fn padding(self) -> Rems {
        rems_from_px(match self {
            ButtonSize::Large => 8.,
            ButtonSize::Default => 4.,
        })
    }
}

/// ToggleState for switches; buttons take a bool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToggleState {
    Unselected,
    Selected,
}

impl From<bool> for ToggleState {
    fn from(on: bool) -> Self {
        if on { ToggleState::Selected } else { ToggleState::Unselected }
    }
}

/// The base of every button: any children, a style, a size, selected and
/// disabled states, a tooltip and a click handler.
#[derive(IntoElement)]
pub struct ButtonLike {
    id: ElementId,
    style: ButtonStyle,
    size: ButtonSize,
    selected: bool,
    disabled: bool,
    square: bool,
    full_width: bool,
    children: SmallVec<[AnyElement; 2]>,
    tooltip: Option<TooltipBuilder>,
    on_click: Option<ClickHandler>,
    /// What `dev dump` reports it as: the kind and its text or icon.
    dev: (&'static str, SharedString),
}

impl ButtonLike {
    pub fn new(id: impl Into<ElementId>) -> Self {
        ButtonLike {
            id: id.into(),
            style: ButtonStyle::default(),
            size: ButtonSize::default(),
            selected: false,
            disabled: false,
            square: false,
            full_width: false,
            children: SmallVec::new(),
            tooltip: None,
            on_click: None,
            dev: ("button_like", SharedString::default()),
        }
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn toggle_state(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Take the row's spare width.
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// As wide as it is tall, like an `IconButton` (for icons by path).
    pub fn square(mut self) -> Self {
        self.square = true;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip = Some(Box::new(tooltip));
        self
    }

    /// The same as `tooltip`, named like `TipExt::tip` for plain elements.
    pub fn tip(self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip(tooltip)
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl ParentElement for ButtonLike {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements)
    }
}

impl RenderOnce for ButtonLike {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let clear = gpui::transparent_black();
        let accent = |a: f32| {
            let mut c = colors.text_accent;
            c.lightness = (c.lightness + a).clamp(0., 1.);
            c
        };
        let (rest, hover, active) = match self.style {
            ButtonStyle::Accent => (accent(0.), accent(0.05), accent(-0.05)),
            ButtonStyle::Filled => (if self.selected { colors.element_selected } else { colors.element_background }, colors.element_hover, colors.element_active),
            ButtonStyle::Subtle | ButtonStyle::Outlined => {
                (if self.selected { colors.ghost_element_selected } else { clear }, colors.ghost_element_hover, colors.ghost_element_active)
            }
        };
        let height = self.size.height();
        let probe = crate::dev::on().then(|| crate::dev::probe(&self.id, self.dev.0, self.dev.1.to_string(), self.disabled, self.selected));
        let button: Stateful<Div> = h_flex()
            .id(self.id)
            .relative()
            .flex_none()
            .justify_center()
            .gap(rems_from_px(4.))
            .h(height)
            .rounded(rems_from_px(if self.size == ButtonSize::Large { 9. } else { 7. }))
            .rounded_smoothing(SMOOTHING)
            .bg(rest)
            .when(self.style == ButtonStyle::Outlined, |b| b.border_1().border_color(colors.border_variant))
            .map(|b| if self.square { b.w(height) } else { b.px(self.size.padding()) })
            .when(self.full_width, |b| b.flex_1().min_w_0())
            .when(!self.disabled, |b| {
                b.cursor_pointer().hover(|s| s.bg(hover)).active(|s| s.bg(active))
            })
            .when(self.disabled, |b| b.cursor_not_allowed())
            .children(self.children)
            .children(probe);
        button
            .when_some(self.tooltip, |b, tooltip| b.tooltip(tooltip).child(tip_anchor()))
            .when_some(self.on_click.filter(|_| !self.disabled), |b, on_click| {
                b.on_mouse_down(gpui::MouseButton::Left, |_, window, _| window.prevent_default()).on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    on_click(event, window, cx)
                })
            })
    }
}

/// A text button, maybe with an icon before and a shortcut after.
#[derive(IntoElement)]
pub struct Button {
    base: ButtonLike,
    label: SharedString,
    label_size: LabelSize,
    color: Color,
    start_icon: Option<Icon>,
    key_binding: Option<KeyBinding>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        let label: SharedString = label.into();
        let mut base = ButtonLike::new(id);
        base.dev = ("button", label.clone());
        Button { base, label, label_size: LabelSize::Default, color: Color::Default, start_icon: None, key_binding: None }
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.base = self.base.style(style);
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.base = self.base.size(size);
        self
    }


    pub fn disabled(mut self, disabled: bool) -> Self {
        self.base = self.base.disabled(disabled);
        self
    }

    pub fn full_width(mut self) -> Self {
        self.base = self.base.full_width();
        self
    }


    pub fn tooltip(mut self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.base = self.base.tooltip(tooltip);
        self
    }

    /// The same as `tooltip`, named like `TipExt::tip` for plain elements.
    pub fn tip(self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip(tooltip)
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.base = self.base.on_click(handler);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn label_size(mut self, size: LabelSize) -> Self {
        self.label_size = size;
        self
    }

    pub fn start_icon(mut self, icon: Icon) -> Self {
        self.start_icon = Some(icon);
        self
    }

    pub fn key_binding(mut self, key_binding: impl Into<Option<KeyBinding>>) -> Self {
        self.key_binding = key_binding.into();
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let accent = self.base.style == ButtonStyle::Accent;
        let on_accent = on(cx.theme().colors.text_accent);
        let color = match () {
            _ if self.base.disabled => Color::Disabled,
            _ if accent => Color::Custom(on_accent),
            _ => self.color,
        };
        let size = self.label_size;
        let hint = |mut c: Hsla| {
            c.alpha = 0.7;
            c
        };
        self.base
            .children(self.start_icon.map(|icon| if accent { icon.color(Color::Custom(on_accent)) } else { icon }))
            .child(Label::new(self.label).size(size).color(color))
            .children(self.key_binding.map(|keys| if accent { KeyBinding { color: Some(hint(on_accent)), ..keys } } else { keys }))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconButtonShape {
    #[default]
    Square,
}

/// A button that is just an icon; square by default.
#[derive(IntoElement)]
pub struct IconButton {
    base: ButtonLike,
    icon: IconName,
    icon_size: IconSize,
    icon_color: Color,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName) -> Self {
        let mut base = ButtonLike::new(id);
        base.square = true;
        base.dev = ("icon_button", format!("{icon:?}").into());
        IconButton { base, icon, icon_size: IconSize::Small, icon_color: Color::Default }
    }

    pub fn shape(self, _: IconButtonShape) -> Self {
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.base = self.base.style(style);
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.base = self.base.size(size);
        self
    }


    pub fn disabled(mut self, disabled: bool) -> Self {
        self.base = self.base.disabled(disabled);
        self
    }

    pub fn tooltip(mut self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.base = self.base.tooltip(tooltip);
        self
    }

    /// The same as `tooltip`, named like `TipExt::tip` for plain elements.
    pub fn tip(self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.tooltip(tooltip)
    }

    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.base = self.base.on_click(handler);
        self
    }

    pub fn icon_size(mut self, size: IconSize) -> Self {
        self.icon_size = size;
        self
    }

    pub fn icon_color(mut self, color: Color) -> Self {
        self.icon_color = color;
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let color = if self.base.disabled { Color::Disabled } else { self.icon_color };
        self.base.child(Icon::new(self.icon).size(self.icon_size).color(color))
    }
}

// ---------------------------------------------------------------- switch

type ToggleHandler = Box<dyn Fn(&ToggleState, &mut Window, &mut App) + 'static>;

/// An on/off switch; `on_click` gets the new state.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    state: ToggleState,
    disabled: bool,
    on_click: Option<ToggleHandler>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, state: ToggleState) -> Self {
        Switch { id: id.into(), state, disabled: false, on_click: None }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&ToggleState, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors();
        let on = self.state == ToggleState::Selected;
        let track = if on { colors.text_accent } else { colors.element_active };
        let next = if on { ToggleState::Unselected } else { ToggleState::Selected };
        let probe = crate::dev::on().then(|| crate::dev::probe(&self.id, "switch", String::new(), self.disabled, on));
        // A macOS switch: a soft track and a white knob with a small shadow.
        h_flex()
            .id(self.id)
            .flex_none()
            .w(rems_from_px(30.))
            .h(rems_from_px(18.))
            .p(px(2.))
            .rounded_full()
            .bg(track)
            .when(on, |s| s.justify_end())
            .when(self.disabled, |s| s.opacity(0.45).cursor_not_allowed())
            .when(!self.disabled, |s| s.cursor_pointer())
            .child(div().size(rems_from_px(14.)).rounded_full().bg(gpui::white()).shadow(vec![BoxShadow {
                color: hsla(0., 0., 0., 0.25).into(),
                offset: point(px(0.), px(1.)),
                blur_radius: px(2.),
                spread_radius: px(0.),
                inset: false,
            }]))
            .children(probe)
            .when_some(self.on_click.filter(|_| !self.disabled), |s, on_click| {
                s.on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    on_click(&next, window, cx)
                })
            })
    }
}

// ---------------------------------------------------------------- progress

#[derive(IntoElement)]
pub struct ProgressBar {
    fraction: f32,
    track: Hsla,
    fill: Hsla,
}

impl ProgressBar {
    pub fn new(_id: impl Into<ElementId>, value: f32, max: f32, cx: &App) -> Self {
        let theme = cx.theme();
        ProgressBar { fraction: (value / max.max(f32::EPSILON)).clamp(0., 1.), track: theme.colors.element_background, fill: theme.status.info }
    }
}

impl RenderOnce for ProgressBar {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .w_full()
            .h(rems_from_px(5.))
            .rounded_full()
            .bg(self.track)
            .child(div().h_full().w(relative(self.fraction)).rounded_full().bg(self.fill))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_hints_read_like_zed() {
        assert_eq!(KeyBinding::new("ctrl-s").text.as_ref(), "Ctrl-S");
        assert_eq!(KeyBinding::new("ctrl-shift-z").text.as_ref(), "Ctrl-Shift-Z");
        assert_eq!(KeyBinding::new("escape").text.as_ref(), "Escape");
        assert_eq!(KeyBinding::new("x").text.as_ref(), "X");
    }
}

// ---------------------------------------------------------------- scrollbar

/// A thin scrollbar for a container that tracks `handle`: put it after that
/// container, in a shared `relative` parent. The thumb can be dragged, and
/// a click on the track jumps there. `grab` keeps the drag between frames.
pub fn scrollbar(handle: &gpui::ScrollHandle, grab: &std::rc::Rc<std::cell::Cell<Option<f32>>>, cx: &App) -> impl IntoElement {
    let (handle, grab) = (handle.clone(), grab.clone());
    let mut color = cx.theme().colors.text;
    gpui::canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let view = handle.bounds();
            let max = f32::from(handle.max_offset().y);
            let h = f32::from(view.size.height);
            if max < 1. || h < 40. {
                return;
            }
            let thumb_h = (h * h / (h + max)).max(32.);
            let travel = h - thumb_h;
            let top = f32::from(view.top()) + travel * (-f32::from(handle.offset().y) / max).clamp(0., 1.);
            let track_x = f32::from(view.right()) - 12.;
            color.alpha = if grab.get().is_some() { 0.5 } else { 0.28 };
            let thumb = Bounds::new(point(px(f32::from(view.right()) - 8.), px(top + 3.)), gpui::size(px(5.), px(thumb_h - 6.)));
            window.paint_quad(fill(thumb, color).corner_radii(px(2.5)));

            let scroll_to = {
                let handle = handle.clone();
                move |thumb_top: f32| {
                    let at = ((thumb_top - f32::from(view.top())) / travel.max(1.)).clamp(0., 1.) * max;
                    handle.set_offset(point(handle.offset().x, px(-at)));
                }
            };
            let on_track = move |p: gpui::Point<Pixels>| f32::from(p.x) >= track_x && f32::from(p.x) <= f32::from(view.right()) && p.y >= view.top() && p.y <= view.bottom();
            let g = grab.clone();
            let jump = scroll_to.clone();
            window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Capture || e.button != gpui::MouseButton::Left || !on_track(e.position) {
                    return;
                }
                let y = f32::from(e.position.y);
                if y < top || y > top + thumb_h {
                    // The thumb's middle comes to the click.
                    jump(y - thumb_h / 2.);
                    g.set(Some(thumb_h / 2.));
                } else {
                    g.set(Some(y - top));
                }
                cx.stop_propagation();
                window.refresh();
            });
            let g = grab.clone();
            window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, window, _| {
                if phase != gpui::DispatchPhase::Capture {
                    return;
                }
                if let Some(at) = g.get() {
                    if !e.dragging() {
                        g.set(None);
                    } else {
                        scroll_to(f32::from(e.position.y) - at);
                    }
                    window.refresh();
                }
            });
            let g = grab.clone();
            window.on_mouse_event(move |_: &gpui::MouseUpEvent, _, window, _| {
                if g.take().is_some() {
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

/// Where a dropdown menu was last drawn. Menus hang outside their button's
/// box, so its `on_mouse_down_out` would close the menu before a click in
/// it lands; check `contains` first.
pub type MenuBounds = std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>;

/// Put inside a menu to keep `bounds` up to date.
pub fn track_bounds(bounds: &MenuBounds) -> impl IntoElement {
    let cell = bounds.clone();
    gpui::canvas(move |b, _, _| cell.set(b), |_, _, _, _| {}).absolute().inset_0()
}
