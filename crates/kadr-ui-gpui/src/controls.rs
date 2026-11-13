//! Small controls. Hosts own their choices and drags; these draw their
//! existing looks without turning logical pixels into rems.

use gpui::{App, BorderStyle, Bounds, ClickEvent, Div, ElementId, Hsla, IntoElement, Pixels, Stateful, Window, canvas, div, point, px, size, prelude::*};

use super::{ActiveTheme, ButtonLike, ButtonStyle, Color, Label, LabelSize, StyledExt, h_flex, rems_from_px, tokens};

#[derive(Clone, Copy)]
pub enum ChipLook {
    /// Settings choices, with the wider label inset.
    Settings,
    /// Theme and language choices and trim options.
    Compact,
    /// History's large soft chips, with an optional result count.
    HistoryFilter,
    /// Equal-width, accent-filled studio options.
    Accent,
    /// Cells inside a filled tray in the editor.
    Tray,
}

/// The tray or row around a set of chips.
pub fn segments(look: ChipLook, cx: &App) -> Div {
    let t = tokens();
    match look {
        ChipLook::Settings | ChipLook::Compact | ChipLook::HistoryFilter => h_flex().gap(rems_from_px(t.spacing.step_2)),
        ChipLook::Accent => h_flex().w_full().gap(px(t.small_controls.option_gap)),
        ChipLook::Tray => h_flex().w_full().p(px(t.small_controls.tray_inset)).gap(px(t.small_controls.tray_inset))
            .rounded(px(t.small_controls.tray_outer)).bg(cx.theme().colors.element_background),
    }
}

/// One choice. Soft chips keep ButtonLike's press/focus/propagation policy;
/// accent and tray cells deliberately have no pressed style or key handling.
#[derive(gpui::IntoElement)]
pub struct OptionChip {
    id: ElementId,
    label: gpui::SharedString,
    on: bool,
    look: ChipLook,
    count: usize,
    click: Option<super::ClickHandler>,
}

impl OptionChip {
    pub fn new(id: impl Into<ElementId>, label: impl Into<gpui::SharedString>, on: bool, look: ChipLook) -> Self {
        Self { id: id.into(), label: label.into(), on, look, count: 0, click: None }
    }

    pub fn on_click(mut self, click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.click = Some(Box::new(click));
        self
    }

    /// Matching items, shown only by the history-filter look and only if nonzero.
    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }
}

impl gpui::RenderOnce for OptionChip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = tokens();
        let s = t.small_controls;
        let colors = cx.theme().colors();
        match self.look {
            ChipLook::Settings | ChipLook::Compact | ChipLook::HistoryFilter => {
                let filter = matches!(self.look, ChipLook::HistoryFilter);
                let content = if filter {
                    h_flex().px(rems_from_px(t.spacing.step_8)).gap(rems_from_px(t.spacing.step_6))
                        .child(Label::new(self.label.clone()))
                        .when(self.count > 0, |d| d.child(Label::new(self.count.to_string()).size(LabelSize::Small).color(Color::Muted))).into_any_element()
                } else {
                    div().px(rems_from_px(if matches!(self.look, ChipLook::Settings) { t.spacing.step_8 } else { t.spacing.step_6 }))
                        .child(Label::new(self.label.clone()).size(LabelSize::Small)).into_any_element()
                };
                let mut button = ButtonLike::new(self.id).style(if self.on { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .when(filter, |b| b.size(super::ButtonSize::Large)).toggle_state(self.on).child(content);
                button.dev = ("chip", self.label);
                if let Some(click) = self.click { button = button.on_click(click); }
                button.into_any_element()
            }
            look => {
                let accent = matches!(look, ChipLook::Accent);
                let probe = crate::dev::on().then(|| crate::dev::probe(&self.id, "chip", self.label.to_string(), false, self.on));
                div().id(self.id).relative().flex_grow(1.)
                    .when(accent, |d| d.flex_basis(px(0.)).min_w_0())
                    .px(rems_from_px(t.spacing.step_4)).h(px(if accent { s.option_height } else { s.tray_height }))
                    .flex().items_center().justify_center().rounded(px(if accent { s.option_radius } else { s.tray_radius })).cursor_pointer()
                    .when(accent, |d| d.bg(if self.on { colors.text_accent } else { colors.element_background }))
                    .when(!accent && self.on, |d| d.bg(colors.element_selected))
                    .when(!self.on, |d| d.hover(|d| d.bg(colors.element_hover)))
                    .child(if accent {
                        div().text_size(px(s.option_text)).truncate().text_color(if self.on { gpui::white() } else { colors.text }).child(self.label).into_any_element()
                    } else {
                        Label::new(self.label).size(LabelSize::Small).color(if self.on { Color::Default } else { Color::Muted }).into_any_element()
                    })
                    .children(probe).when_some(self.click, |d, click| d.on_click(click)).into_any_element()
            }
        }
    }
}

#[derive(Clone, Copy)]
pub enum CheckLook { Panel, CropFooter }

pub fn checkbox(id: impl Into<ElementId>, label: impl Into<gpui::SharedString>, on: bool, enabled: bool, look: CheckLook, cx: &App) -> Stateful<Div> {
    let id = id.into();
    let t = tokens();
    let s = t.small_controls;
    let colors = cx.theme().colors();
    let footer = matches!(look, CheckLook::CropFooter);
    let probe = crate::dev::on().then(|| crate::dev::probe(&id, "checkbox", String::new(), !enabled, on));
    h_flex().id(id).relative()
        .gap(rems_from_px(if footer { t.spacing.step_6 } else { t.spacing.step_8 }))
        .when(footer, |d| d.px(rems_from_px(t.spacing.step_4)))
        .h(px(if footer { s.check_footer } else { s.check_row }))
        .when(enabled, |d| d.cursor_pointer()).when(!enabled, |d| d.opacity(t.alpha.disabled))
        .child(div().size(px(s.check_size)).when(!footer, |d| d.flex_none()).flex().items_center().justify_center()
            .rounded(px(s.check_radius)).border_hairline().border_color(if on { colors.text_accent } else { colors.border })
            .when(on, |d| d.bg(colors.text_accent).child(gpui::svg().path("icons/check.svg").size(px(s.check_icon)).text_color(gpui::white()))))
        .child(Label::new(label).size(LabelSize::Small)).children(probe)
}

#[derive(Clone, Copy)]
pub enum SliderLook { Editor, Studio }

/// Track bounds are reported to the host, which keeps its existing drag
/// lifetime and undo checkpoint. Neither copy has wheel or key behavior.
pub fn slider(id: impl Into<ElementId>, value: f32, ticks: usize, look: SliderLook, track_bounds: impl Fn(Bounds<Pixels>) + 'static, cx: &App) -> Stateful<Div> {
    let id = id.into();
    let t = tokens();
    let s = t.small_controls;
    let colors = cx.theme().colors();
    let (track, filled, edge) = (colors.element_active, colors.text_accent, colors.border);
    let tick = super::alpha(colors.text, s.slider_tick_alpha);
    let v = value.clamp(0., 1.);
    let probe = crate::dev::on().then(|| crate::dev::probe(&id, "slider", v.to_string(), false, false));
    let bar = canvas(move |bounds, _, _| { track_bounds(bounds); bounds }, move |_, bounds, window, _| {
        let mid = bounds.origin.y + bounds.size.height / 2. - px(if ticks > 0 { s.slider_tick_shift } else { 0. });
        let (x0, w) = (bounds.origin.x, bounds.size.width);
        for i in 0..=ticks {
            if ticks == 0 { break; }
            let x = px(kadr_ui::controls::position(i as f32 / ticks as f32, x0.into(), w.into()));
            window.paint_quad(super::quad(Bounds::new(point(x - px(s.slider_tick / 2.), mid + px(s.slider_tick_y)), size(px(s.slider_tick), px(s.slider_tick))), px(s.slider_tick / 2.), tick, px(0.), tick, BorderStyle::Solid));
        }
        let r = s.slider_track / 2.;
        window.paint_quad(super::quad(Bounds::new(point(x0, mid - px(r)), size(w, px(s.slider_track))), px(r), track, px(0.), track, BorderStyle::Solid));
        window.paint_quad(super::quad(Bounds::new(point(x0, mid - px(r)), size(w * v, px(s.slider_track))), px(r), filled, px(0.), filled, BorderStyle::Solid));
        let c = x0 + w * v;
        // The sliders' old white knob is not the accent selection handle
        // painted by paint_knob; keep its offset shadow and half-pixel edge.
        window.paint_quad(super::quad(Bounds::new(point(c - px(s.slider_shadow), mid - px(s.slider_knob)), size(px(2. * s.slider_shadow), px(2. * s.slider_shadow))), px(s.slider_shadow), gpui::hsla(0., 0., 0., s.slider_shadow_alpha), px(0.), edge, BorderStyle::Solid));
        window.paint_quad(super::quad(Bounds::new(point(c - px(s.slider_knob), mid - px(s.slider_knob)), size(px(2. * s.slider_knob), px(2. * s.slider_knob))), px(s.slider_knob), gpui::white(), px(s.slider_edge), edge, BorderStyle::Solid));
    }).size_full();
    div().id(id).relative().h(px(if ticks > 0 { s.slider_ticks_height } else { s.slider_height }))
        .map(|d| match look { SliderLook::Editor => d.flex_1().mx(rems_from_px(t.spacing.step_8)), SliderLook::Studio => d.w_full().px(rems_from_px(t.spacing.step_8)) })
        .cursor_pointer().child(bar).children(probe)
}

pub fn slider_value(slider: impl IntoElement, label: String) -> Div {
    let t = tokens();
    h_flex().w_full().gap(rems_from_px(t.spacing.step_12)).child(slider)
        .when(!label.is_empty(), |d| d.child(div().w(px(t.small_controls.slider_label)).flex().justify_end().child(Label::new(label).size(LabelSize::Small).color(Color::Muted))))
}

/// A panel slider below its heading and right-aligned value.
pub fn labeled_slider(name: impl Into<gpui::SharedString>, shown: impl Into<gpui::SharedString>, slider: impl IntoElement) -> Div {
    let t = tokens();
    super::v_flex().w_full().gap(rems_from_px(t.spacing.step_4))
        .child(h_flex().w_full().justify_between().child(Label::new(name).size(LabelSize::Small)).child(Label::new(shown).size(LabelSize::Small).color(Color::Muted)))
        .child(div().mx(px(-t.spacing.step_8)).child(slider))
}

#[derive(Clone, Copy)]
pub enum SwatchLook { Colour, Settings, Tile, Dot }

/// The selection ring and its inset. Content may be a colour, gradient,
/// checker, image or the wallpaper add button; hosts keep their own events.
pub fn swatch(id: impl Into<ElementId>, selected: bool, ring: Hsla, look: SwatchLook) -> Stateful<Div> {
    let id = id.into();
    let s = tokens().small_controls;
    let probe = crate::dev::on().then(|| crate::dev::probe(&id, "swatch", String::new(), false, selected));
    div().id(id).relative()
        .map(|d| match look {
            SwatchLook::Colour => d.size(px(s.swatch_size)).flex().items_center().justify_center().rounded_full(),
            SwatchLook::Settings => d.size(px(s.settings_swatch)).flex().items_center().justify_center().rounded(px(s.settings_outer)),
            SwatchLook::Tile => d.flex_none().p(px(s.tile_inset)).rounded(px(s.tile_radius)),
            SwatchLook::Dot => d.flex_none().p(px(s.tile_inset)).rounded_full(),
        })
        .border(px(s.swatch_ring)).border_color(if selected { ring } else { gpui::transparent_black() }).cursor_pointer().children(probe)
}

pub fn swatch_content(look: SwatchLook, edge: Option<Hsla>) -> Div {
    let s = tokens().small_controls;
    div().map(|d| match look {
        SwatchLook::Colour => d.size(px(s.swatch_dot)).rounded_full(),
        SwatchLook::Settings => d.size(px(s.settings_dot)).rounded(px(s.settings_inner)),
        SwatchLook::Tile => d.size(px(s.tile)).rounded(px(s.tile_radius - s.tile_inset - s.swatch_ring)).overflow_hidden(),
        SwatchLook::Dot => d.size(px(s.tile_dot)).rounded_full(),
    }).when_some(edge, |d, edge| d.border_hairline().border_color(edge))
}

pub fn swatch_grid(look: SwatchLook) -> Div {
    let t = tokens();
    match look {
        SwatchLook::Colour | SwatchLook::Settings => h_flex().gap(rems_from_px(t.spacing.step_4)),
        SwatchLook::Tile => h_flex().w_full().flex_wrap().gap(px(t.small_controls.tile_gap)),
        SwatchLook::Dot => h_flex().w_full().flex_wrap().gap(px(t.small_controls.tile_inset)),
    }
}

/// White controls over pictures. History has no pressed state and stops
/// mouse-down before its thumbnail sees it; the pin alone casts a shadow.
#[derive(Clone, Copy)]
pub enum PillLook { Card, TextCard, History, Pin }

pub fn pill_ink() -> Hsla { gpui::hsla(0., 0., tokens().small_controls.pill_ink, 1.) }

pub fn picture_pill(id: impl Into<ElementId>, look: PillLook) -> Stateful<Div> {
    let id = id.into();
    let s = tokens().small_controls;
    let probe = crate::dev::on().then(|| crate::dev::probe(&id, "picture_pill", String::new(), false, false));
    div().id(id).relative().flex().items_center().justify_center().rounded_full().bg(gpui::hsla(0., 0., 1., s.pill_alpha))
        .when(matches!(look, PillLook::Card | PillLook::History), |d| d.text_color(pill_ink()))
        .when(matches!(look, PillLook::Pin), |d| d.shadow(crate::kit::shadow()))
        .cursor_pointer().hover(|d| d.bg(gpui::white()))
        .when(!matches!(look, PillLook::History), |d| d.active(|d| d.bg(gpui::hsla(0., 0., s.pill_pressed, 1.))))
        .when(matches!(look, PillLook::History), |d| d.on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation()))
        .children(probe)
}

#[derive(Clone, Copy)]
pub enum SectionLook { Settings, Rail }

pub fn section_item(id: impl Into<ElementId>, on: bool, look: SectionLook, cx: &App) -> Stateful<Div> {
    let id = id.into();
    let t = tokens();
    let colors = cx.theme().colors();
    let probe = crate::dev::on().then(|| crate::dev::probe(&id, "section", String::new(), false, on));
    h_flex().id(id).relative().map(|d| match look {
        SectionLook::Settings => d.gap(rems_from_px(t.spacing.step_8)).px(rems_from_px(t.spacing.step_8)).py(rems_from_px(t.spacing.step_6))
            .rounded(rems_from_px(t.radius.button)).rounded_smoothing(t.radius.smoothing),
        SectionLook::Rail => d.size(px(t.small_controls.rail_size)).justify_center().rounded(px(t.small_controls.rail_radius)),
    }).cursor_pointer().when(on, |d| d.bg(colors.text_accent)).when(!on, |d| d.hover(|d| d.bg(colors.ghost_element_hover))).children(probe)
}

/// The two editors share the palette, but keep their different sections,
/// transparent/blurred choices, presets and layout around these parts.
pub struct BackdropParts {
    pub gradients: Div,
    pub wallpapers: Div,
    pub plains: Div,
    pub more: Option<super::Button>,
}

#[derive(Clone)]
pub enum BackdropChoice { Gradient(usize), Wallpaper(std::path::PathBuf), Own, More }

pub struct BackdropOptions<'a> {
    pub colors: &'a [(Hsla, Hsla)],
    pub gradient: Option<usize>,
    pub wallpaper: Option<&'a std::path::Path>,
    pub walls: &'a [(std::path::PathBuf, std::sync::Arc<gpui::Image>)],
    pub expanded: bool,
    /// The editor alone has transparency, before the plain colours.
    pub leading_plain: Vec<gpui::AnyElement>,
}

pub fn backdrop_picker<V: 'static>(options: BackdropOptions<'_>, pick: impl Fn(&mut V, BackdropChoice, &mut Window, &mut gpui::Context<V>) + 'static, cx: &mut gpui::Context<V>) -> BackdropParts {
    use gpui::{ObjectFit, img, linear_color_stop, linear_gradient};
    use super::{Button, Icon, IconName, IconSize, TipExt, Tooltip};
    use crate::i18n::t;
    let s = tokens().small_controls;
    let colors = cx.theme().colors().clone();
    let pick = std::rc::Rc::new(pick);
    let event = |choice: BackdropChoice| {
        let pick = pick.clone();
        cx.listener(move |this, _: &ClickEvent, window, cx| pick(this, choice.clone(), window, cx))
    };
    let gradients: Vec<_> = options.colors.iter().enumerate().filter(|(_, (a, b))| a != b).collect();
    let limit = if options.expanded { gradients.len() } else { gradients.len().min(s.gradient_limit as usize) };
    let gradient_tiles = gradients[..limit].iter().map(|&(i, &(a, b))| {
        swatch(gpui::SharedString::from(format!("gradient-{i}")), options.gradient == Some(i), colors.text_accent, SwatchLook::Tile)
            .child(swatch_content(SwatchLook::Tile, Some(colors.border_variant)).bg(linear_gradient(135., linear_color_stop(a, 0.), linear_color_stop(b, 1.))))
            .on_click(event(BackdropChoice::Gradient(i)))
    }).collect::<Vec<_>>();
    let walls = options.walls.iter().enumerate().map(|(i, (path, thumb))| {
        swatch(gpui::SharedString::from(format!("wall-{i}")), options.wallpaper == Some(path.as_path()), colors.text_accent, SwatchLook::Tile)
            .child(swatch_content(SwatchLook::Tile, Some(colors.border_variant)).child(img(thumb.clone()).size_full().object_fit(ObjectFit::Cover)))
            .on_click(event(BackdropChoice::Wallpaper(path.clone())))
    }).collect::<Vec<_>>();
    let own = options.wallpaper.is_some_and(|p| !options.walls.iter().any(|(w, _)| w == p));
    let own = swatch("own-wallpaper", own, colors.text_accent, SwatchLook::Tile)
        .child(div().size(px(s.tile)).rounded(px(s.tile_radius - s.tile_inset - s.swatch_ring)).border_hairline().border_dashed().border_color(colors.border)
            .flex().items_center().justify_center().child(Icon::new(IconName::Plus).size(IconSize::Small).color(Color::Muted)))
        .tip(Tooltip::text(t("common.your_own_image"))).on_click(event(BackdropChoice::Own));
    let dots = options.colors.iter().enumerate().filter(|(_, (a, b))| a == b).map(|(i, &(a, _))| {
        swatch(gpui::SharedString::from(format!("plain-{i}")), options.gradient == Some(i), colors.text_accent, SwatchLook::Dot)
            .child(swatch_content(SwatchLook::Dot, Some(colors.border_variant)).bg(a)).on_click(event(BackdropChoice::Gradient(i)))
    }).collect::<Vec<_>>();
    BackdropParts {
        gradients: swatch_grid(SwatchLook::Tile).children(gradient_tiles),
        wallpapers: swatch_grid(SwatchLook::Tile).children(walls).child(own),
        plains: swatch_grid(SwatchLook::Dot).children(options.leading_plain).children(dots),
        more: (gradients.len() > s.gradient_limit as usize).then(|| Button::new("more-gradients", t(if options.expanded { "editor.show_less" } else { "editor.show_more" }))
            .style(ButtonStyle::Subtle).label_size(LabelSize::Small).color(Color::Muted).on_click(event(BackdropChoice::More))),
    }
}
