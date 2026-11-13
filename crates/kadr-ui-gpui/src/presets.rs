//! Saved looks (the studio's, the editor's background) in one dropdown:
//! "+" beside it keeps the current look as a new one; in the menu each can
//! be picked, renamed or deleted.

use gpui::{App, Context, FocusHandle, KeyDownEvent, SharedString, Window, div, px};
use serde_json::Value;

use crate::i18n::t;

use crate::ui::{Tooltip, prelude::*};
use crate::ui::text_field::{self, Event, Options};
use kadr_ui::text::Field;
use crate::ui::popover::{self, Align, DropdownLook, Elevation, Outside, Placement, RowLook, RowOptions};

pub const FIELD: usize = 100;

pub struct Presets {
    pub list: Vec<(String, Value)>,
    open: bool,
    /// The preset being renamed and its name so far.
    renaming: Option<(usize, Field)>,
    focus: FocusHandle,
    /// Where typing went before a rename took it.
    back: Option<FocusHandle>,
    save: fn(&[(String, Value)]),
    /// Where the open menu is drawn: it hangs outside the button's box, so
    /// a click in it mustn't count as a click outside.
    menu_bounds: crate::ui::MenuBounds,
}

/// A view with presets: where they are, its current look and how to take one.
pub trait HasPresets: text_field::Host {
    fn presets(&mut self) -> &mut Presets;
    fn current_look(&self) -> Value;
    fn apply_look(&mut self, look: &Value, cx: &mut Context<Self>);
}

impl Presets {
    pub fn new(list: Vec<(String, Value)>, save: fn(&[(String, Value)]), cx: &mut App) -> Self {
        Presets { list, open: false, renaming: None, focus: cx.focus_handle(), back: None, save, menu_bounds: Default::default() }
    }

    fn store(&self) {
        (self.save)(&self.list);
    }

    /// Keep `look` as "Preset N" and start naming it.
    fn add(&mut self, look: Value, window: &mut Window, cx: &mut App) {
        let next = self.list.iter().filter_map(|(name, _)| name.strip_prefix(t("common.preset_prefix"))?.parse::<usize>().ok()).max().unwrap_or(0) + 1;
        self.list.push((tr!("common.preset_n", next), look));
        self.store();
        self.open = true;
        self.rename(self.list.len() - 1, window, cx);
    }

    fn rename(&mut self, i: usize, window: &mut Window, cx: &mut App) {
        self.renaming = Some((i, Field::single_line("")));
        if self.back.is_none() {
            self.back = window.focused(cx);
        }
        window.focus(&self.focus, cx);
    }

    /// Finish a rename; an empty name keeps the old one.
    fn commit(&mut self, window: &mut Window, cx: &mut App) {
        if let Some((i, name)) = self.renaming.take() {
            let name = name.text().trim();
            if !name.is_empty()
                && let Some(p) = self.list.get_mut(i)
            {
                p.0 = name.to_owned();
                self.store();
            }
        }
        if let Some(back) = self.back.take() {
            window.focus(&back, cx);
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut App) {
        self.commit(window, cx);
        self.open = false;
    }

    pub fn field(&mut self) -> Option<&mut Field> {
        self.renaming.as_mut().map(|(_, field)| field)
    }

    pub fn field_event(&mut self, event: Event, window: &mut Window, cx: &mut App) {
        match event {
            Event::Enter | Event::Blur => self.commit(window, cx),
            Event::Escape => { self.renaming = None; self.commit(window, cx); }
            _ => {}
        }
    }
}

/// The dropdown and the "+" beside it, full width.
pub fn render<T: HasPresets>(p: &Presets, current: &Value, cx: &mut Context<T>) -> AnyElement {
    let colors = cx.theme().colors().clone();
    let active = p.list.iter().position(|(_, look)| crate::studio::look_matches(look, current));
    let title = match active {
        Some(i) => Label::new(p.list[i].0.clone()),
        None => Label::new(if p.list.is_empty() { t("presets.none_yet") } else { t("presets.custom") }).color(Color::Muted),
    };

    let menu = p.open.then(|| {
        let rows: Vec<_> = p.list.iter().enumerate().map(|(i, (name, look))| {
            let group = SharedString::from(format!("preset-row-{i}"));
            let editing = p.renaming.as_ref().filter(|r| r.0 == i).map(|r| &r.1);
            let row = popover::row_shell(SharedString::from(format!("preset-{i}")), name, active == Some(i), &RowOptions { look: RowLook::Preset, interactive: false, ..Default::default() }, cx)
                .group(group.clone())
                .child(Icon::new(IconName::Check).size(IconSize::Small).color(if active == Some(i) { Color::Accent } else { Color::Custom(gpui::transparent_black()) }));
            if let Some(text) = editing {
                // Typing goes here; the old name shows until the first key.
                return row.child(text_field::render("preset-name", FIELD, text, &p.focus, true, Options { placeholder: name.clone().into(), legacy_caret_metrics: true, ..Default::default() }, cx)
                    .flex_1().min_w_0().h(px(24.)).px_1p5().rounded(px(5.)).border_1().border_color(colors.text_accent)).into_any_element();
            }
            let look = look.clone();
            row.cursor_pointer()
                .hover(|d| d.bg(colors.element_hover))
                .child(div().flex_1().min_w_0().child(Label::new(name.clone()).truncate()))
                .child(
                    h_flex()
                        .gap_0p5()
                        .opacity(0.)
                        .group_hover(group, |d| d.opacity(1.))
                        .child(
                            IconButton::new(SharedString::from(format!("preset-rename-{i}")), IconName::Pencil)
                                .icon_size(IconSize::XSmall)
                                .icon_color(Color::Muted)
                                .tip(Tooltip::text(t("presets.rename")))
                                .on_click(cx.listener(move |this: &mut T, _, window, cx| {
                                    let p = this.presets();
                                    p.commit(window, cx);
                                    p.rename(i, window, cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            IconButton::new(SharedString::from(format!("preset-delete-{i}")), IconName::Trash)
                                .icon_size(IconSize::XSmall)
                                .icon_color(Color::Muted)
                                .tip(Tooltip::text(t("common.delete_preset")))
                                .on_click(cx.listener(move |this: &mut T, _, window, cx| {
                                    let p = this.presets();
                                    p.commit(window, cx);
                                    p.list.remove(i);
                                    p.store();
                                    cx.notify();
                                })),
                        ),
                )
                .on_click(cx.listener(move |this: &mut T, _, window, cx| {
                    this.presets().close(window, cx);
                    this.apply_look(&look, cx);
                    cx.notify();
                }))
                .into_any_element()
        }).collect();
        let empty = p.list.is_empty().then(|| div().px_2().py_1p5().child(Label::new(t("presets.empty_hint")).size(LabelSize::Small).color(Color::Muted)));
        popover::show(
            popover::surface("preset-menu", &p.menu_bounds, Placement::below(38., Align::Stretch), Elevation::Floating, cx)
                .p_1()
                .gap_0p5()
                .children(rows)
                .children(empty),
            1,
        )
    });

    popover::dropdown_menu(popover::anchor(div(), p.open, &p.menu_bounds, Outside::AnchorAndMenu, cx.listener(|this: &mut T, _, window, cx| {
            this.presets().close(window, cx);
            cx.notify();
        }))
        .w_full()
        .track_focus(&p.focus)
        .on_key_down(cx.listener(|this: &mut T, e: &KeyDownEvent, window, cx| {
            if this.presets().renaming.is_some() { text_field::key(this, FIELD, e, window, cx); }
        })),
            h_flex()
                .w_full()
                .gap_1()
                .child(
                    popover::dropdown_label("presets", title, p.open, DropdownLook::Presets)
                        .on_click(cx.listener(|this: &mut T, _, window, cx| {
                            let p = this.presets();
                            if p.open {
                                p.close(window, cx);
                            } else {
                                p.open = true;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    IconButton::new("preset-add", IconName::Plus)
                        .style(ButtonStyle::Filled)
                        .size(ButtonSize::Large)
                        .tip(Tooltip::text(t("common.save_as_preset")))
                        .on_click(cx.listener(|this: &mut T, _, window, cx| {
                            let look = this.current_look();
                            let p = this.presets();
                            p.commit(window, cx);
                            p.add(look, window, cx);
                            cx.notify();
                        })),
                ),
        menu)
        .into_any_element()
}
