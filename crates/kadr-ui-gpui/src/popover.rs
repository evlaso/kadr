//! Shared surfaces, deferred placement and choice rows. Screens own the
//! open state and key ordering; closing a menu must not consume a host's
//! Escape (text editing and selections may handle it first).

use gpui::{App, ClickEvent, Div, ElementId, IntoElement, MouseDownEvent, ParentElement, Stateful, Styled, Window, div, px, prelude::*};
pub use kadr_ui::popover::{Align, Outside, Placement, Side};
use crate::ui::{self, ActiveTheme, ButtonLike, ButtonSize, ButtonStyle, Color, Icon, IconName, IconSize, Label, LabelSize, MenuBounds, StyledExt, h_flex, rems_from_px, v_flex};

#[derive(Clone, Copy)]
pub enum Elevation { Popover, Floating }

/// An elevated panel, also used inside the microphone's OS popup. Padding,
/// radius overrides and content layout remain ordinary style refinements.
pub fn panel(cx: &App, elevation: Elevation) -> Div {
    match elevation { Elevation::Popover => v_flex().elevation_2(cx), Elevation::Floating => v_flex().elevation_3(cx) }
}

/// A panel hanging outside its anchor. Keep the original origin offsets:
/// these are logical pixels, not rems. `Stretch` follows the anchor's width.
pub fn surface(id: impl Into<ElementId>, bounds: &MenuBounds, at: Placement, elevation: Elevation, cx: &App) -> Stateful<Div> {
    panel(cx, elevation).id(id).occlude().absolute()
        .when(at.side == Side::Below && !at.edge, |d| d.top(px(at.offset)))
        .when(at.side == Side::Above && !at.edge, |d| d.bottom(px(at.offset)))
        .when(at.side == Side::Below && at.edge, |d| d.top(gpui::relative(1.)).mt(px(at.offset)))
        .when(at.side == Side::Above && at.edge, |d| d.bottom(gpui::relative(1.)).mb(px(at.offset)))
        .when(at.align != Align::End, |d| d.left_0())
        .when(at.align != Align::Start, |d| d.right_0())
        .child(ui::track_bounds(bounds))
}

/// Deferring stays outside the styled panel so it keeps exactly the same
/// layout parent as the old menus. Presets use priority 1; the rest use 0.
pub fn show(panel: impl IntoElement, priority: usize) -> gpui::Deferred {
    gpui::deferred(panel).priority(priority)
}

/// Install on the original anchor, not the deferred panel. The latter is
/// outside that anchor's box, so bounds must be checked before dismissing.
pub fn anchor(base: Div, open: bool, bounds: &MenuBounds, policy: Outside, close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Div {
    let bounds = bounds.clone();
    base.relative().on_mouse_down_out(move |e, window, cx| {
        if kadr_ui::popover::dismiss_outside(open, bounds.get().contains(&e.position), policy) { close(e, window, cx); }
    })
}

/// The dropdown composition: its original anchor, a current-value button,
/// and an optional deferred list. Rich buttons (presets with +, or a swatch)
/// use the same composition without moving their state into a child view.
pub fn dropdown_menu(anchor: Div, button: impl IntoElement, menu: Option<impl IntoElement>) -> Div {
    anchor.child(button).children(menu)
}

#[derive(Clone, Copy)]
pub enum RowLook { Compact, Settings, Recording, Preset }

/// The differences between today's rows, not a normalized style scale.
pub struct RowOptions {
    pub look: RowLook,
    pub full_width: bool,
    /// A logical-pixel override; absent, the row look reads its token.
    pub height: Option<f32>,
    pub leading: Option<Icon>,
    pub disabled: bool,
    pub highlighted: bool,
    /// Preset rename rows install their own hover and click handlers.
    pub interactive: bool,
}

impl Default for RowOptions {
    fn default() -> Self { Self { look: RowLook::Compact, full_width: false, height: None, leading: None, disabled: false, highlighted: false, interactive: true } }
}

/// The row shell can also hold rich content (preset rename and actions).
pub fn row_shell(id: impl Into<ElementId>, text: &str, on: bool, options: &RowOptions, cx: &App) -> Stateful<Div> {
    let id = id.into();
    let colors = cx.theme().colors();
    let t = ui::tokens();
    let (m, s) = (t.menu, t.spacing);
    let probe = crate::dev::on().then(|| crate::dev::probe(&id, "menu-item", text.to_owned(), options.disabled, on));
    let row = h_flex().id(id).relative()
        .when(options.full_width, |d| d.w_full())
        .when(options.interactive && !options.disabled, |d| d.cursor_pointer().hover(|d| d.bg(colors.element_hover)))
        .when(options.disabled, |d| d.cursor_not_allowed().text_color(colors.text_disabled))
        .when(options.highlighted, |d| d.bg(colors.element_hover));
    let row = match options.look {
        RowLook::Compact => row.gap(rems_from_px(s.step_6)).px(rems_from_px(s.step_8)).h(px(options.height.unwrap_or(m.compact_row))).rounded(px(m.compact_radius)),
        RowLook::Settings => row.gap(rems_from_px(s.step_8)).px(rems_from_px(s.step_8)).py(rems_from_px(s.step_4)).rounded(rems_from_px(m.settings_radius)),
        RowLook::Recording => row.h(px(options.height.unwrap_or(m.recording_row * crate::kit::SCALE))).flex_none().px(rems_from_px(s.step_8)).gap(rems_from_px(s.step_8)).rounded(rems_from_px(m.recording_radius)).when(on, |d| d.bg(colors.element_selected)),
        RowLook::Preset => row.h(px(options.height.unwrap_or(m.preset_row))).gap(rems_from_px(s.step_8)).pl(rems_from_px(s.step_8)).pr(rems_from_px(s.step_4)).rounded(px(m.preset_radius)),
    };
    row.children(probe)
}

/// A disabled row can still be laid out and inspected, but cannot pick.
#[derive(gpui::IntoElement)]
pub struct MenuRow {
    base: Stateful<Div>,
    disabled: bool,
}

impl MenuRow {
    pub fn on_click(mut self, pick: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        if !self.disabled { self.base = self.base.on_click(pick); }
        self
    }
}

impl Styled for MenuRow {
    fn style(&mut self) -> &mut gpui::StyleRefinement { self.base.style() }
}

impl ParentElement for MenuRow {
    fn extend(&mut self, children: impl IntoIterator<Item = gpui::AnyElement>) { self.base.extend(children); }
}

impl gpui::RenderOnce for MenuRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement { self.base }
}

/// A choice with its optional icon and the chosen check. Empty check slots
/// keep the labels aligned. Icon rows put the check at the far end.
pub fn row(id: impl Into<ElementId>, text: impl Into<gpui::SharedString>, on: bool, options: RowOptions, cx: &App) -> MenuRow {
    let text = text.into();
    let row = row_shell(id, &text, on, &options, cx);
    let compact = matches!(options.look, RowLook::Compact);
    let label = Label::new(text).size(if compact { LabelSize::Small } else { LabelSize::Default }).color(if options.disabled { Color::Disabled } else { Color::Default });
    let check = || div().w(px(ui::tokens().menu.check_column)).children(on.then(|| Icon::new(IconName::Check).size(IconSize::XSmall)));
    let base = if compact {
        match options.leading {
            Some(icon) => row.pr(rems_from_px(ui::tokens().spacing.step_4)).child(icon.size(IconSize::Small)).child(label).child(div().flex_1()).child(check()),
            None => row.child(check()).child(label),
        }
    } else {
        row.child(Icon::new(IconName::Check).size(IconSize::Small).color(if on { Color::Accent } else { Color::Custom(gpui::transparent_black()) })).children(options.leading).child(label)
    };
    MenuRow { base, disabled: options.disabled }
}

#[derive(Clone, Copy)]
pub enum DropdownLook { Settings, Editor, History, Presets, Recording }

/// The button half of a dropdown. The anchor and `show(surface(..))` form
/// its menu half; callbacks stay with the screen, including native dispatch.
pub fn dropdown(id: impl Into<ElementId>, label: impl Into<gpui::SharedString>, open: bool, look: DropdownLook) -> ButtonLike {
    dropdown_label(id, Label::new(label), open, look)
}

pub fn dropdown_label(id: impl Into<ElementId>, label: Label, open: bool, look: DropdownLook) -> ButtonLike {
    let s = ui::tokens().spacing;
    let small = matches!(look, DropdownLook::Settings);
    if matches!(look, DropdownLook::Presets) {
        return ButtonLike::new(id).style(ButtonStyle::Filled).size(ButtonSize::Large).full_width().toggle_state(open)
            .child(h_flex().flex_1().min_w_0().gap(rems_from_px(s.step_8)).child(div().flex_1().min_w_0().child(label.truncate()))
                .child(Icon::new(IconName::ChevronDown).size(IconSize::Small).color(Color::Muted)));
    }
    let content = h_flex().gap(rems_from_px(if small { s.step_6 } else { s.step_4 })).px(rems_from_px(if matches!(look, DropdownLook::Editor | DropdownLook::Recording) { s.step_4 } else { s.step_8 }))
        .child(if matches!(look, DropdownLook::Recording) {
            div().font_family(crate::kit::MONO_FONT).text_sm().child(label).into_any_element()
        } else { label.size(if small { LabelSize::Small } else { LabelSize::Default }).into_any_element() })
        .child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted));
    ButtonLike::new(id).style(if small || open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
        .size(if small { ButtonSize::Default } else { ButtonSize::Large }).toggle_state(open).child(content)
}

/// Place a measured-size specimen from a tracked anchor in window pixels.
/// Existing screens use origin offsets without clamping; changing their
/// overflow would be normalization, not extraction.
pub fn constrained(anchor: gpui::Bounds<gpui::Pixels>, size: (f32, f32), at: Placement, window: &Window, child: impl IntoElement) -> impl IntoElement {
    let rect = kadr_ui::popover::Rect { x: anchor.left().into(), y: anchor.top().into(), w: anchor.size.width.into(), h: anchor.size.height.into() };
    let room = window.viewport_size();
    let (x, y) = at.position(rect, size, (room.width.into(), room.height.into()), true);
    gpui::anchored().snap_to_window().position(gpui::point(px(x), px(y))).child(child)
}

/// The studio's full-width field differs from the toolbar buttons.
pub fn field_dropdown(id: impl Into<ElementId>, label: impl Into<gpui::SharedString>, open: bool, cx: &App, click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Stateful<Div> {
    let id = id.into();
    let text = label.into();
    let colors = cx.theme().colors();
    let t = ui::tokens();
    let probe = crate::dev::on().then(|| crate::dev::probe(&id, "dropdown", text.to_string(), false, open));
    h_flex().id(id).relative().w_full().h(px(t.menu.field_height)).px(rems_from_px(t.spacing.step_10)).justify_between().rounded(px(t.menu.field_radius)).bg(colors.element_background)
        .cursor_pointer().hover(|d| d.bg(colors.element_hover))
        .child(Label::new(text).size(LabelSize::Small))
        .child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted)).children(probe).on_click(click)
}
