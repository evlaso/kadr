//! The gallery's live token editor. The same view fits the full page or a
//! side pane; pointer events replace a pending set, applied once next frame.

use std::{cell::Cell, collections::BTreeSet, rc::Rc};
use gpui::{Context, FocusHandle, ScrollHandle, Window, px};
use kadr_ui::{text::Field, tokens::{Entry, Tokens, Unit, store}};
use crate::{i18n::t, palette, ui::{self, controls, prelude::*, text_field}};

enum Status { Saved, Copied, Error(String) }

pub struct TokenPanel {
    focus: FocusHandle,
    fields: Vec<Field>,
    active: Option<usize>,
    errors: Vec<bool>,
    expanded: BTreeSet<&'static str>,
    scroll: ScrollHandle,
    grab: Rc<Cell<Option<f32>>>,
    bounds: Vec<ui::MenuBounds>,
    dragging: Option<usize>,
    pending: Option<Tokens>,
    saved: Tokens,
    status: Option<Status>,
}

impl TokenPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let entries = entries(&ui::tokens());
        Self {
            focus: cx.focus_handle(), fields: entries.iter().map(|(_, e)| Field::single_line(&e.formatted())).collect(),
            active: None, errors: vec![false; entries.len()], expanded: BTreeSet::from(["radius"]),
            scroll: ScrollHandle::new(), grab: Default::default(), bounds: entries.iter().map(|_| Default::default()).collect(),
            dragging: None, pending: None, saved: store::load(), status: None,
        }
    }

    fn queue(&mut self, next: Tokens, window: &mut Window, cx: &mut Context<Self>) {
        let schedule = self.pending.is_none();
        self.pending = Some(next);
        if schedule {
            let weak = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| { let _ = weak.update(cx, |this, cx| this.flush(cx)); });
        }
    }

    fn flush(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.pending.take() {
            palette::restyle(next, cx);
            self.status = None;
            cx.notify();
        }
    }

    fn replace(&mut self, next: Tokens, cx: &mut Context<Self>) {
        self.pending = None;
        self.active = None;
        self.errors.fill(false);
        self.status = None;
        palette::restyle(next, cx);
        cx.notify();
    }

    fn slide(&mut self, slot: usize, x: f32, window: &mut Window, cx: &mut Context<Self>) {
        let mut next = self.pending.clone().unwrap_or_else(|| (*ui::tokens()).clone());
        let (path, entry) = entries(&next)[slot].clone();
        let b = self.bounds[slot].get();
        let fraction = kadr_ui::controls::fraction(x, b.origin.x.into(), b.size.width.into());
        let value = entry.at_fraction(fraction);
        if value == entry.value { return; }
        let (group, name) = path.split_once('.').unwrap();
        *next.get_mut(group, name).unwrap() = value;
        self.active = None;
        self.errors[slot] = false;
        self.queue(next, window, cx);
    }

    fn row(&self, slot: usize, group: &str, entry: Entry, default: Entry, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors().clone();
        let path = format!("{group}.{}", entry.name);
        let active = self.active == Some(slot);
        let changed = entry.value != default.value;
        let field = text_field::render(SharedString::from(format!("token-field-{path}")), slot, &self.fields[slot], &self.focus, active, Default::default(), cx)
            .w(px(if entry.unit == Unit::Hex { 100. } else { 78. })).h(px(28.)).px_2().text_size(px(12.)).rounded(px(6.))
            .bg(colors.element_background).border_1().border_color(if self.errors[slot] { cx.theme().status.error } else if active { colors.text_accent } else { gpui::transparent_black() });
        let reset = ui::IconButton::new(SharedString::from(format!("token-reset-{path}")), ui::IconName::Undo)
            .disabled(!changed && !self.errors[slot]).tip(ui::Tooltip::text(t("gallery.reset_token")))
            .on_click(cx.listener(move |this, _, _, cx| {
                let mut next = this.pending.take().unwrap_or_else(|| (*ui::tokens()).clone());
                let (group, name) = path.split_once('.').unwrap();
                *next.get_mut(group, name).unwrap() = default.value;
                this.replace(next, cx);
            }));
        let value = h_flex().gap_2().child(field).child(Label::new(unit_name(entry.unit)).size(LabelSize::XSmall).color(Color::Muted)).child(reset);
        let mut row = v_flex().w_full().gap_1().py_2()
            .child(h_flex().gap_2().child(Label::new(entry.name).size(LabelSize::Small)).when(changed, |d| d.child(Label::new("*").color(Color::Accent))))
            .child(Label::new(entry.doc).size(LabelSize::XSmall).color(Color::Muted))
            .when(entry.unit != Unit::Hex && !group.starts_with("shadow_"), |d| d.child(specimen(group, entry, cx)));
        if entry.unit == Unit::Hex {
            let look = controls::SwatchLook::Colour;
            row = row.child(h_flex().gap_2().child(controls::swatch(SharedString::from(format!("token-swatch-{group}.{}", entry.name)), false, colors.text_accent, look)
                .child(controls::swatch_content(look, Some(colors.border_variant)).bg(ui::from_hex(entry.value as u32)))).child(value));
        } else {
            let bounds = self.bounds[slot].clone();
            let slider = controls::slider(SharedString::from(format!("token-slider-{group}.{}", entry.name)), (entry.value - entry.min) / (entry.max - entry.min), 0,
                controls::SliderLook::Studio, move |b| bounds.set(b), cx)
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                    this.dragging = Some(slot);
                    this.slide(slot, e.position.x.into(), window, cx);
                }));
            row = row.child(h_flex().gap_2().child(div().flex_1().min_w_0().child(slider)).child(value));
        }
        row.children(self.errors[slot].then(|| Label::new(if entry.unit == Unit::Hex {
            t("gallery.invalid_hex").to_owned()
        } else { tr!("gallery.invalid_number", entry.min, entry.max) }).size(LabelSize::XSmall).color(Color::Error))).into_any_element()
    }

    /// Keep the old Sizes page's shadow and corner specimens beside the
    /// editable data, rather than losing them when the table becomes fields.
    fn samples(&self, cx: &mut Context<Self>) -> AnyElement {
        let tokens = ui::tokens();
        let colors = cx.theme().colors().clone();
        let light = cx.theme().light;
        let open = self.expanded.contains("reference");
        let heading = Button::new("tokens-reference", format!("{} {}", if open { "−" } else { "+" }, t("gallery.reference_specimens")))
            .on_click(cx.listener(|this, _, _, cx| { if !this.expanded.remove("reference") { this.expanded.insert("reference"); } cx.notify(); }));
        let mut body = v_flex().w_full().gap_4().child(heading);
        if open {
            for (name, layers, role) in tokens.shadow.recipes() {
                let sample = div().w(px(150.)).h(px(70.)).flex().items_center().justify_center();
                let sample = match name {
                    "popover" => sample.elevation_2(cx), "floating" => sample.elevation_3(cx),
                    "picture" => sample.rounded(px(crate::kit::picture_radius())).bg(colors.elevated_surface_background).shadow(crate::kit::shadow()),
                    _ => sample.child(div().size(ui::rems_from_px(tokens.control.switch_knob)).rounded_full().bg(gpui::white()).shadow(ui::shadow(layers, light))),
                };
                body = body.child(v_flex().gap_2().child(sample).child(Label::new(format!("{name}: {role}")).size(LabelSize::XSmall)));
            }
            let (outer, inset) = (crate::kit::WINDOW_RADIUS, 4.);
            body = body.child(Label::new(format!("System corner {outer} − inset {inset} = {} px", outer - inset)).size(LabelSize::XSmall))
                .child(div().w(px(120.)).h(px(46.)).p(px(inset)).rounded(px(outer)).rounded_smoothing(ui::smoothing()).bg(colors.element_active)
                    .child(div().size_full().rounded(px(outer - inset)).rounded_smoothing(ui::smoothing()).bg(colors.text_accent)))
                .child(div().p(px(inset)).rounded(px(outer)).rounded_smoothing(ui::smoothing()).bg(colors.element_active)
                    .child(Button::new("concentric-button", t("common.save")).style(ButtonStyle::Accent).size(ButtonSize::Large)));
        }
        body.into_any_element()
    }
}

fn entries(tokens: &Tokens) -> Vec<(String, Entry)> {
    tokens.groups().into_iter().flat_map(|(group, entries)| entries.into_iter().map(move |e| (format!("{group}.{}", e.name), e))).collect()
}

fn unit_name(unit: Unit) -> &'static str {
    match unit { Unit::Design => "rem-base px", Unit::Px => "px", Unit::Alpha => "alpha", Unit::Lightness => "lightness", Unit::Ratio => "ratio", Unit::Weight => "weight", Unit::Hex => "#RRGGBB" }
}

/// The old token table's visual samples still follow each value. Some roles
/// (notably medium/semibold weights) aren't read by screens yet; their sample
/// makes that distinction visible without migrating those screens here.
fn specimen(group: &str, entry: Entry, cx: &Context<TokenPanel>) -> AnyElement {
    let t = ui::tokens();
    let colors = cx.theme().colors();
    let v = entry.value;
    let length = |v: f32| -> gpui::AbsoluteLength { if entry.unit == Unit::Px { px(v).into() } else { ui::rems_from_px(v).into() } };
    let tile = || div().flex_none().bg(colors.element_active);
    let sample = match (group, entry.unit) {
        ("radius", Unit::Ratio) => tile().w(px(64.)).h(px(44.)).rounded(ui::rems_from_px(t.radius.surface)).rounded_smoothing(v).into_any_element(),
        ("radius", _) => tile().w(px(64.)).h(px(44.)).rounded(length(v)).rounded_smoothing(ui::smoothing()).into_any_element(),
        ("text", _) => v_flex().text_size(ui::rems_from_px(v)).child("Screenshots").child("Снимки экрана").into_any_element(),
        ("weight", _) => div().font_weight(ui::weight(v)).child("Screenshots · Снимки").into_any_element(),
        ("icon", _) => Icon::new(IconName::Folder).size(match entry.name {
            "xsmall" => IconSize::XSmall, "medium" => IconSize::Medium, "tool" => IconSize::Tool, "large" => IconSize::Large, _ => IconSize::Small,
        }).into_any_element(),
        ("border", _) => tile().w(px(64.)).h(px(28.)).bg(gpui::transparent_black()).border_color(colors.text).border(length(v)).into_any_element(),
        (_, Unit::Alpha) => tile().w(px(44.)).h(px(28.)).rounded(px(7.)).bg(ui::alpha(colors.text, v)).into_any_element(),
        (_, Unit::Lightness) => {
            let mut color = match entry.name { "accent_hover" | "accent_pressed" => colors.text_accent, "stage_dark" | "stage_light" => colors.panel_background, _ => gpui::white() };
            color.lightness = match entry.name { "accent_hover" | "accent_pressed" => (color.lightness + v).clamp(0., 1.), "stage_dark" | "stage_light" => (color.lightness - v).max(0.), _ => v };
            tile().w(px(44.)).h(px(28.)).rounded(px(7.)).bg(color).into_any_element()
        }
        ("control", Unit::Design | Unit::Px) if !entry.name.ends_with("width") => tile().w(px(48.)).h(length(v)).rounded(px(3.)).into_any_element(),
        (_, Unit::Design | Unit::Px) => tile().w(length(v.abs())).h(px(12.)).bg(colors.text_accent).into_any_element(),
        _ => div().into_any_element(),
    };
    h_flex().min_h(px(28.)).child(sample).into_any_element()
}

impl text_field::Host for TokenPanel {
    fn field(&mut self, slot: usize) -> Option<&mut Field> { self.fields.get_mut(slot) }

    fn field_event(&mut self, slot: usize, event: text_field::Event, window: &mut Window, cx: &mut Context<Self>) {
        use text_field::Event;
        match event {
            Event::Focus => self.active = Some(slot),
            Event::Changed | Event::Enter => {
                let mut next = self.pending.clone().unwrap_or_else(|| (*ui::tokens()).clone());
                let (path, _) = entries(&next)[slot].clone();
                match next.edit(&path, self.fields[slot].text()) {
                    Ok(()) => { self.errors[slot] = false; self.queue(next, window, cx); }
                    Err(_) => self.errors[slot] = true,
                }
                if matches!(event, Event::Enter) && !self.errors[slot] { self.active = None; }
            }
            Event::Escape => { self.active = None; self.errors[slot] = false; }
            Event::Tab | Event::Blur => self.active = None,
        }
        cx.notify();
    }
}

impl Render for TokenPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = ui::tokens();
        let defaults = Tokens::default();
        let dirty = *tokens != self.saved;
        for (slot, (_, e)) in entries(&tokens).into_iter().enumerate() {
            if self.active != Some(slot) && !self.errors[slot] && self.fields[slot].text() != e.formatted() { self.fields[slot] = Field::single_line(&e.formatted()); }
        }
        let toolbar = div().flex().flex_wrap().gap_2()
            .child(Button::new("tokens-save", t("common.save")).style(ButtonStyle::Accent).on_click(cx.listener(|this, _, _, cx| {
                this.flush(cx);
                match store::save(&ui::tokens()) {
                    Ok(()) => { this.saved = (*ui::tokens()).clone(); this.status = Some(Status::Saved); }
                    Err(e) => this.status = Some(Status::Error(e.to_string())),
                }
                cx.notify();
            })))
            .child(Button::new("tokens-revert", t("gallery.revert")).on_click(cx.listener(|this, _, _, cx| {
                this.saved = store::load();
                this.replace(this.saved.clone(), cx);
            })))
            .child(Button::new("tokens-reset-all", t("gallery.reset_all")).on_click(cx.listener(|this, _, _, cx| this.replace(Tokens::default(), cx))))
            .child(Button::new("tokens-copy", t("gallery.copy_defaults")).on_click(cx.listener(|this, _, _, cx| {
                this.flush(cx);
                kadr_platform::copy_text(&store::copy_defaults(&ui::tokens()));
                this.status = Some(Status::Copied);
                cx.notify();
            })));
        let mut slot = 0;
        let groups: Vec<_> = tokens.groups().into_iter().map(|(group, entries)| {
            let open = self.expanded.contains(group);
            let heading = Button::new(SharedString::from(format!("tokens-group-{group}")), format!("{} {group}", if open { "−" } else { "+" })).full_width()
                .on_click(cx.listener(move |this, _, _, cx| { if !this.expanded.remove(group) { this.expanded.insert(group); } cx.notify(); }));
            let rows: Vec<_> = entries.into_iter().map(|entry| {
                let current = slot;
                slot += 1;
                (current, entry)
            }).filter(|_| open).map(|(slot, entry)| self.row(slot, group, entry, defaults.entry(&format!("{group}.{}", entry.name)).unwrap(), cx)).collect();
            v_flex().w_full().child(h_flex().child(heading)).children(rows).into_any_element()
        }).collect();
        v_flex().flex_1().min_w_0().h_full().track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e, window, cx| { if let Some(slot) = this.active { text_field::key(this, slot, e, window, cx); } }))
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, window, cx| {
                if let Some(slot) = this.dragging { if e.dragging() { this.slide(slot, e.position.x.into(), window, cx); } else { this.dragging = None; } }
            }))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(|this, _, _, _| this.dragging = None))
            .child(v_flex().flex_none().p_3().gap_2().child(toolbar)
                .child(Label::new(if dirty { t("gallery.tokens_unsaved") } else { t("gallery.tokens_clean") }).size(LabelSize::XSmall).color(Color::Muted))
                .children(self.status.as_ref().map(|status| Label::new(match status {
                    Status::Saved => t("gallery.tokens_saved").to_owned(), Status::Copied => t("gallery.tokens_copied").to_owned(), Status::Error(error) => tr!("gallery.tokens_save_failed", error),
                }).size(LabelSize::XSmall))))
            .child(div().relative().flex_1().min_h_0()
                .child(div().id("tokens-body").size_full().overflow_y_scroll().track_scroll(&self.scroll).child(v_flex().p_3().gap_2().children(groups).child(self.samples(cx))))
                .child(ui::scrollbar(&self.scroll, &self.grab, cx)))
    }
}
