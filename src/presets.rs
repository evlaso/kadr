//! Saved looks (the studio's, the editor's background) in one dropdown:
//! "+" beside it keeps the current look as a new one; in the menu each can
//! be picked, renamed or deleted.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, Bounds, Context, Pixels, canvas, FocusHandle, KeyDownEvent, SharedString, Window, div, px};
use serde_json::Value;

use crate::i18n::t;

use crate::ui::{ButtonLike, Tooltip, prelude::*};

pub struct Presets {
    pub list: Vec<(String, Value)>,
    open: bool,
    /// The preset being renamed and its name so far.
    renaming: Option<(usize, String)>,
    focus: FocusHandle,
    /// Where typing went before a rename took it.
    back: Option<FocusHandle>,
    save: fn(&[(String, Value)]),
    /// Where the open menu is drawn: it hangs outside the button's box, so
    /// a click in it mustn't count as a click outside.
    menu_bounds: Rc<Cell<Bounds<Pixels>>>,
}

/// A view with presets: where they are, its current look and how to take one.
pub trait HasPresets: Sized + 'static {
    fn presets(&mut self) -> &mut Presets;
    fn current_look(&self) -> Value;
    fn apply_look(&mut self, look: &Value, cx: &mut Context<Self>);
}

impl Presets {
    pub fn new(list: Vec<(String, Value)>, save: fn(&[(String, Value)]), cx: &mut App) -> Self {
        Presets { list, open: false, renaming: None, focus: cx.focus_handle(), back: None, save, menu_bounds: Rc::default() }
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
        self.renaming = Some((i, String::new()));
        if self.back.is_none() {
            self.back = window.focused(cx);
        }
        window.focus(&self.focus, cx);
    }

    /// Finish a rename; an empty name keeps the old one.
    fn commit(&mut self, window: &mut Window, cx: &mut App) {
        if let Some((i, name)) = self.renaming.take() {
            let name = name.trim();
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
}

/// The dropdown and the "+" beside it, full width.
pub fn render<T: HasPresets>(p: &Presets, current: &Value, cx: &mut Context<T>) -> AnyElement {
    let colors = cx.theme().colors();
    let active = p.list.iter().position(|(_, look)| crate::studio::look_matches(look, current));
    let title = match active {
        Some(i) => Label::new(p.list[i].0.clone()),
        None => Label::new(if p.list.is_empty() { t("presets.none_yet") } else { t("presets.custom") }).color(Color::Muted),
    };

    let menu = p.open.then(|| {
        let rows = p.list.iter().enumerate().map(|(i, (name, look))| {
            let group = SharedString::from(format!("preset-row-{i}"));
            let editing = p.renaming.as_ref().filter(|r| r.0 == i).map(|r| r.1.clone());
            let row = h_flex()
                .id(SharedString::from(format!("preset-{i}")))
                .group(group.clone())
                .h(px(30.))
                .gap_2()
                .pl_2()
                .pr_1()
                .rounded(px(7.))
                .child(Icon::new(IconName::Check).size(IconSize::Small).color(if active == Some(i) { Color::Accent } else { Color::Custom(gpui::transparent_black()) }));
            if let Some(text) = editing {
                // Typing goes here; the old name shows until the first key.
                let shown = if text.is_empty() { div().text_color(colors.text_placeholder).child(format!("{name}▏")) } else { div().child(format!("{text}▏")) };
                return row.child(div().flex_1().min_w_0().h(px(24.)).px_1p5().flex().items_center().rounded(px(5.)).border_1().border_color(colors.text_accent).child(shown)).into_any_element();
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
                                .tooltip(Tooltip::text(t("presets.rename")))
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
                                .tooltip(Tooltip::text(t("common.delete_preset")))
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
        });
        let empty = p.list.is_empty().then(|| div().px_2().py_1p5().child(Label::new(t("presets.empty_hint")).size(LabelSize::Small).color(Color::Muted)));
        gpui::deferred(
            v_flex()
                .id("preset-menu")
                .occlude()
                .absolute()
                .top(px(38.))
                .left_0()
                .right_0()
                .p_1()
                .gap_0p5()
                .elevation_3(cx)
                .children(rows)
                .children(empty)
                .child({
                    let cell = p.menu_bounds.clone();
                    canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {}).absolute().inset_0()
                }),
        )
        .priority(1)
    });

    div()
        .relative()
        .w_full()
        .track_focus(&p.focus)
        .on_key_down(cx.listener(|this: &mut T, e: &KeyDownEvent, window, cx| {
            let p = this.presets();
            let Some((_, text)) = p.renaming.as_mut() else { return };
            cx.stop_propagation();
            match e.keystroke.key.as_str() {
                "enter" => p.commit(window, cx),
                "escape" => {
                    p.renaming = None;
                    p.commit(window, cx);
                }
                "backspace" => {
                    text.pop();
                }
                _ => {
                    if !(e.keystroke.modifiers.control || e.keystroke.modifiers.platform)
                        && let Some(ch) = e.keystroke.key_char.as_deref().filter(|c| !c.chars().any(char::is_control))
                    {
                        text.push_str(ch);
                    }
                }
            }
            cx.notify();
        }))
        .on_mouse_down_out(cx.listener(|this: &mut T, e: &gpui::MouseDownEvent, window, cx| {
            let p = this.presets();
            if p.open && !p.menu_bounds.get().contains(&e.position) {
                p.close(window, cx);
                cx.notify();
            }
        }))
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .child(
                    ButtonLike::new("presets")
                        .style(ButtonStyle::Filled)
                        .size(ButtonSize::Large)
                        .full_width()
                        .toggle_state(p.open)
                        .child(h_flex().flex_1().min_w_0().gap_2().child(div().flex_1().min_w_0().child(title.truncate())).child(Icon::new(IconName::ChevronDown).size(IconSize::Small).color(Color::Muted)))
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
                        .tooltip(Tooltip::text(t("common.save_as_preset")))
                        .on_click(cx.listener(|this: &mut T, _, window, cx| {
                            let look = this.current_look();
                            let p = this.presets();
                            p.commit(window, cx);
                            p.add(look, window, cx);
                            cx.notify();
                        })),
                ),
        )
        .children(menu)
        .into_any_element()
}
