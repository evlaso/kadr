//! Single-line editing over a screen's own `Field`. The binding owns no
//! text: input methods, keys and mouse hits all edit the host's state.
//! The caret is steady, so even a focused field has no idle frame timer.

use std::{cell::RefCell, ops::Range, rc::Rc};
use gpui::{App, Bounds, Context, Div, ElementId, Entity, FocusHandle, InputHandler, KeyDownEvent, Pixels, Point, SharedString, ShapedText, Stateful, Window, canvas, div, point, px, size};
use kadr_ui::text::{Field, Move};
use super::{ActiveTheme, Color, Icon, IconName, IconSize, prelude::*};

#[derive(Clone, Copy)]
pub enum Event { Focus, Changed, Enter, Escape, Tab, Blur }

pub trait Host: Sized + 'static {
    fn field(&mut self, slot: usize) -> Option<&mut Field>;
    fn field_event(&mut self, slot: usize, event: Event, window: &mut Window, cx: &mut Context<Self>);
}

/// The UTF-16 bridge is also used by the multiline canvas text handler.
pub fn text_for_range(field: &Field, range: Range<usize>, adjusted: &mut Option<Range<usize>>) -> String {
    let range = range.start.min(field.len_utf16())..range.end.min(field.len_utf16());
    *adjusted = Some(range.clone());
    field.text_utf16(range)
}

pub fn selection(field: &Field) -> gpui::UTF16Selection {
    gpui::UTF16Selection { range: field.selection_utf16(), reversed: !field.selection().is_empty() && field.caret() == field.selection().start }
}

#[derive(Default)]
pub struct Options {
    pub placeholder: SharedString,
    pub icon: Option<(IconName, IconSize)>,
    pub unit: Option<SharedString>,
    pub disabled: bool,
    pub centered: bool,
    /// History reserves this space even when its caret isn't showing.
    pub caret_room: f32,
    pub placeholder_muted: bool,
    /// The old search/rename drew a glyph caret in the text run. Keep its
    /// line metrics, but not its ink, until the looks are normalised.
    pub legacy_caret_metrics: bool,
}

/// The keyboard side, also used by the editor's multiline text tool.
/// Printable keys are deliberately left to GPUI's input handler.
pub fn edit_key(field: &mut Field, k: &gpui::Keystroke, cx: &mut App) -> bool {
    use kadr_platform::keymap::Command as C;
    let is = |c| super::is_command(k, c);
    let moves = [(C::WordLeft, Move::WordLeft), (C::WordRight, Move::WordRight), (C::LineStart, Move::LineStart), (C::LineEnd, Move::LineEnd)];
    if let Some(&(_, to)) = moves.iter().find(|(c, _)| is(*c)) {
        field.go(to, k.modifiers.shift);
    } else if is(C::DeleteWordBack) {
        field.delete_word_back();
    } else if is(C::SelectAll) {
        field.select_all();
    } else if is(C::Copy) || is(C::Cut) {
        let selection = field.selection();
        if !selection.is_empty() {
            kadr_platform::copy_text(&field.text()[selection]);
            if is(C::Cut) { field.backspace(); }
        }
    } else if is(C::Paste) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) { field.insert(&text); }
    } else {
        match k.key.as_str() {
            "backspace" => field.backspace(),
            "delete" => field.delete(),
            "left" => field.go(Move::Left, k.modifiers.shift),
            "right" => field.go(Move::Right, k.modifiers.shift),
            "home" => field.go(Move::Start, k.modifiers.shift),
            "end" => field.go(Move::End, k.modifiers.shift),
            _ => return false,
        }
    }
    true
}

pub fn key<T: Host>(host: &mut T, slot: usize, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<T>) {
    let event = match e.keystroke.key.as_str() {
        "enter" => Event::Enter,
        "escape" => Event::Escape,
        "tab" => Event::Tab,
        _ => {
            let Some(field) = host.field(slot) else { return };
            let before = field.text().to_owned();
            if !edit_key(field, &e.keystroke, cx) { return; }
            if field.text() == before {
                cx.stop_propagation();
                cx.notify();
                return;
            }
            Event::Changed
        }
    };
    cx.stop_propagation();
    host.field_event(slot, event, window, cx);
    cx.notify();
}

#[derive(Default)]
struct Geometry {
    bounds: Bounds<Pixels>,
    line: Option<ShapedText>,
}

impl Geometry {
    fn byte(&self, position: Point<Pixels>) -> usize {
        self.line.as_ref().map_or(0, |l| l.closest_byte_index_for_pixel_point(point(position.x - self.bounds.left(), px(0.)), self.bounds.size.height).unwrap_or_else(|i| i))
    }

    fn x(&self, byte: usize) -> Pixels {
        self.bounds.left() + self.line.as_ref().and_then(|l| l.visual_position_for_byte_index(byte, self.bounds.size.height)).map_or(px(0.), |p| p.x)
    }
}

struct Binding<T> {
    host: Entity<T>,
    slot: usize,
    geometry: Rc<RefCell<Geometry>>,
}

impl<T: Host> Binding<T> {
    fn read<R>(&self, cx: &mut App, f: impl FnOnce(&Field) -> R) -> Option<R> {
        self.host.update(cx, |host, _| host.field(self.slot).map(|field| f(field)))
    }

    fn edit(&self, window: &mut Window, cx: &mut App, f: impl FnOnce(&mut Field)) {
        self.host.update(cx, |host, cx| {
            if let Some(field) = host.field(self.slot) {
                let before = field.text().to_owned();
                f(field);
                if field.text() != before { host.field_event(self.slot, Event::Changed, window, cx); }
                cx.notify();
            }
        });
    }
}

impl<T: Host> InputHandler for Binding<T> {
    fn selected_text_range(&mut self, _: bool, _: &mut Window, cx: &mut App) -> Option<gpui::UTF16Selection> {
        self.read(cx, selection)
    }
    fn marked_text_range(&mut self, _: &mut Window, cx: &mut App) -> Option<Range<usize>> { self.read(cx, Field::marked_utf16).flatten() }
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, _: &mut Window, cx: &mut App) -> Option<String> {
        self.read(cx, |f| text_for_range(f, range, adjusted))
    }
    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut App) { self.edit(window, cx, |f| f.replace(range, text)); }
    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>, window: &mut Window, cx: &mut App) { self.edit(window, cx, |f| f.mark(range, text, selected)); }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) { self.edit(window, cx, Field::unmark); }
    fn set_selected_text_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut App) { self.edit(window, cx, |f| f.select_utf16(range)); }
    fn text_length_utf16(&mut self, _: &mut Window, cx: &mut App) -> Option<usize> { self.read(cx, Field::len_utf16) }
    fn bounds_for_range(&mut self, range: Range<usize>, _: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        let byte = self.read(cx, |f| f.text_utf16(0..range.start).len())?;
        let g = self.geometry.borrow();
        Some(Bounds::new(point(g.x(byte), g.bounds.top()), size(px(super::tokens().text_field.caret_width), g.bounds.size.height)))
    }
    fn character_index_for_point(&mut self, position: Point<Pixels>, _: &mut Window, cx: &mut App) -> Option<usize> {
        let byte = self.geometry.borrow().byte(position);
        self.read(cx, |f| f.utf16_at(byte))
    }
}

/// A normal styled row: callers keep their current dimensions and surfaces.
/// `slot` routes back to the host; neither the element nor the input handler
/// keeps a second copy of its editing state.
pub fn render<T: Host>(id: impl Into<ElementId>, slot: usize, field: &Field, focus: &FocusHandle, active: bool, options: Options, cx: &mut Context<T>) -> Stateful<Div> {
    let id = id.into();
    let colors = cx.theme().colors().clone();
    let metrics = super::tokens().text_field;
    let geometry = Rc::new(RefCell::new(Geometry::default()));
    let binding = Binding { host: cx.entity(), slot, geometry: geometry.clone() };
    let (focus, input_focus) = (focus.clone(), focus.clone());
    let snapshot = field.clone();
    let text = field.text().to_owned();
    let empty = text.is_empty();
    let shown: SharedString = if empty { options.placeholder.clone() } else { text.clone().into() };
    // An empty, placeholder-less field still needs a line box for its caret.
    let shown = if shown.is_empty() { SharedString::from(" ") } else { shown };
    let shown_len = shown.len();
    let shown = if options.legacy_caret_metrics {
        gpui::StyledText::new(format!("{shown}▏")).with_highlights([(shown_len..shown_len + "▏".len(), gpui::HighlightStyle { color: Some(gpui::transparent_black()), ..Default::default() })]).into_any_element()
    } else { shown.into_any_element() };
    let caret_room = options.caret_room;
    let disabled = options.disabled;
    let color = if disabled { colors.text_disabled } else if empty { if options.placeholder_muted { colors.text_muted } else { colors.text_placeholder } } else { colors.text };
    let paint_geometry = geometry.clone();
    let hit_geometry = geometry.clone();
    let drag_geometry = geometry.clone();
    let accent = colors.text_accent;
    let muted = colors.text_muted;
    let content = div()
        .relative()
        .flex_none()
        .whitespace_nowrap()
        .text_color(color)
        .when(empty && caret_room > 0., |d| d.pl(px(caret_room)))
        .when(!empty && caret_room > 0., |d| d.pr(px(caret_room)))
        .child(shown)
        .child(canvas(
            move |bounds, window, _| {
                let style = window.text_style();
                let run = gpui::TextRun { len: text.len(), font: style.font(), color: style.color, ..Default::default() };
                let line = window.text_system().shape_text(text.clone(), style.font_size.to_pixels(window.rem_size()), &[run], None, None).ok();
                *paint_geometry.borrow_mut() = Geometry { bounds, line };
            },
            move |bounds, _, window, cx| {
                if active && !disabled {
                    window.handle_input(&input_focus, binding, cx);
                }
                if !active || disabled || !input_focus.is_focused(window) || !window.is_window_active() { return; }
                let g = geometry.borrow();
                let selection = snapshot.selection();
                if !selection.is_empty() {
                    window.paint_quad(super::fill(Bounds::new(point(g.x(selection.start), bounds.top()), size(g.x(selection.end) - g.x(selection.start), bounds.size.height)), super::alpha(accent, metrics.selection)));
                }
                if let Some(marked) = snapshot.marked() {
                    window.paint_quad(super::fill(Bounds::new(point(g.x(marked.start), bounds.bottom() - px(metrics.composition_line)), size(g.x(marked.end) - g.x(marked.start), px(metrics.composition_line))), accent));
                }
                let height = bounds.size.height * metrics.caret_height;
                window.paint_quad(super::fill(Bounds::new(point(g.x(snapshot.caret()), bounds.top() + bounds.size.height * metrics.caret_inset), size(px(metrics.caret_width), height)), accent));
            },
        ).absolute().inset_0());
    let probe = crate::dev::on().then(|| crate::dev::probe(&id, "text_field", field.text().to_owned(), disabled, active));
    super::h_flex().id(id).relative()
        .when(disabled, |d| d.cursor_not_allowed())
        .when(!disabled, |d| d.cursor_text())
        .children(options.icon.map(|(icon, size)| Icon::new(icon).size(size).color(Color::Muted)))
        .child(div().flex_1().min_w_0().overflow_hidden().flex().items_center().when(options.centered, |d| d.justify_center()).child(content))
        .children(options.unit.map(|unit| div().flex_none().text_color(muted).child(unit)))
        .children(probe)
        .when(!disabled, |d| d.on_mouse_down(gpui::MouseButton::Left, cx.listener(move |host, e: &gpui::MouseDownEvent, window, cx| {
            window.focus(&focus, cx);
            host.field_event(slot, Event::Focus, window, cx);
            if let Some(field) = host.field(slot) {
                let byte = hit_geometry.borrow().byte(e.position);
                if e.click_count >= 2 { field.select_word_at(byte); } else { field.place(byte, e.modifiers.shift); }
            }
            cx.stop_propagation();
            cx.notify();
        })).on_mouse_move(cx.listener(move |host, e: &gpui::MouseMoveEvent, _, cx| {
            if active && e.dragging() {
                if let Some(field) = host.field(slot) { field.place(drag_geometry.borrow().byte(e.position), true); }
                cx.stop_propagation();
                cx.notify();
            }
        })).on_mouse_down_out(cx.listener(move |host, _, window, cx| {
            if active { host.field_event(slot, Event::Blur, window, cx); }
        })))
}
