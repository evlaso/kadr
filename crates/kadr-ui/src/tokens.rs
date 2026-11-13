//! The interface's shared values in one place, with no UI framework: radii,
//! spacing, control sizes, type, icon sizes, borders, the alphas and tones
//! the theme is mixed from, shadows, and the few metrics of the shared
//! components (tooltip, scrollbar, knob). The app's components read them
//! through `get()`, while drawing; `set` replaces the whole set for the
//! process, so the look can change while kadr runs.
//!
//! Units: most sizes are design units, pixels at a 16 px base that the app
//! draws as rems (so at kadr's rem of 19 a 16 is 19 px). Some are plain
//! logical pixels, which don't grow with the rem: each value's `Unit` says
//! which (`Tokens::groups` lists them all). Values marked "normalisation
//! candidate" are today's, kept as they are, where a smaller common scale
//! would merge them later.
//!
//! What follows the system (the window's corner radius, which button of a
//! pair confirms) stays with the platform; the colours themselves stay in the
//! app's palette, which mixes its controls from `Alpha` and `Tone`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// What a number means, so it's drawn and shown right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Pixels at a 16 px base, drawn as rems: they grow with the rem size.
    Design,
    /// Logical pixels as they are.
    Px,
    /// An opacity, 0 to 1.
    Alpha,
    /// A change of lightness, -1 to 1, or a lightness.
    Lightness,
    /// A plain factor.
    Ratio,
    /// A font weight, 100 to 900.
    Weight,
}

/// One value of a group, for listing and editing them by name.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub name: &'static str,
    pub value: f32,
    pub unit: Unit,
    /// Its doc comment: what the role is for.
    pub doc: &'static str,
}

/// A group of named values: the struct, its defaults (today's look), and
/// the listing and by-name access the gallery and a live editor use.
macro_rules! group {
    (
        $(#[doc = $gdoc:literal])*
        $name:ident {
            $(
                $(#[doc = $doc:literal])*
                $field:ident: $value:expr, $unit:ident;
            )*
        }
    ) => {
        $(#[doc = $gdoc])*
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct $name {
            $(
                $(#[doc = $doc])*
                pub $field: f32,
            )*
        }

        impl Default for $name {
            fn default() -> Self {
                $name { $($field: $value),* }
            }
        }

        impl $name {
            /// Every value, in order, with its name, unit and role.
            pub fn entries(&self) -> Vec<Entry> {
                vec![$(Entry { name: stringify!($field), value: self.$field, unit: Unit::$unit, doc: concat!($($doc),*).trim_ascii() }),*]
            }

            /// A value by its name, to change it.
            pub fn get_mut(&mut self, name: &str) -> Option<&mut f32> {
                match name {
                    $(stringify!($field) => Some(&mut self.$field),)*
                    _ => None,
                }
            }
        }
    };
}

group! {
    /// Corner radii.
    Radius {
        /// Tooltips and other hints on the dark plate. Normalisation candidate (8).
        tooltip: 6., Design;
        /// A default-size button. Normalisation candidate (a scale of 4, 8, 12, 16 would make it 8).
        button: 7., Design;
        /// A large button; concentric inside the Linux window's 13 px corner at a 4 px inset. Normalisation candidate (8).
        button_large: 9., Design;
        /// Raised surfaces: popovers, floating bars.
        surface: 12., Design;
        /// Pinned pictures. Logical pixels, unlike the other radii. Normalisation candidate (about 17 design units).
        picture: 20., Px;
        /// Corner smoothing of rounded rectangles: 0 is circular, 1 the softest squircle.
        smoothing: kadr_core::color::SMOOTHING, Ratio;
    }
}

group! {
    /// The spacing steps used by the UI's rem-based gap, padding and margin helpers.
    Spacing {
        /// The tightest inset, also the space around key hints (`_0p5`).
        step_2: 2., Design;
        /// Tight gaps, including a button's children (`_1`).
        step_4: 4., Design;
        /// Compact row gaps and insets (`_1p5`).
        step_6: 6., Design;
        /// Ordinary gaps and small padding (`_2`).
        step_8: 8., Design;
        /// Intermediate padding (`_2p5`). Normalisation candidate (8 or 12).
        step_10: 10., Design;
        /// Space between groups within a panel (`_3`).
        step_12: 12., Design;
        /// Inset cards and sections (`_4`).
        step_16: 16., Design;
        /// Generous group spacing (`_5`). Normalisation candidate (24).
        step_20: 20., Design;
        /// Section gaps and wide row insets (`_6`).
        step_24: 24., Design;
        /// Large page insets (`_10`).
        step_40: 40., Design;
    }
}

group! {
    /// Space inside and between the shared components.
    Space {
        /// Each side of a key hint.
        key_hint: 2., Design;
        /// Between a button's icon, label and key hint.
        button_gap: 4., Design;
        /// Each side of a default-size button.
        button_pad: 4., Design;
        /// Each side of a large button.
        button_pad_large: 8., Design;
    }
}

group! {
    /// The shared controls' sizes.
    Control {
        /// A default-size button's height (and width, square).
        button: 22., Design;
        /// A large button's height: toolbars, the editor's tools.
        button_large: 32., Design;
        /// A switch's track, wide.
        switch_width: 30., Design;
        /// A switch's track, high.
        switch_height: 18., Design;
        /// A switch's white knob.
        switch_knob: 14., Design;
        /// Between a switch's track and its knob. Logical pixels inside a design-unit track: normalisation candidate.
        switch_inset: 2., Px;
        /// A progress bar's height. Normalisation candidate (off any scale).
        progress: 5., Design;
    }
}

group! {
    /// Text sizes (`LabelSize`).
    Text {
        /// Body text: labels, buttons, the windows' own text.
        body: 14., Design;
        /// Secondary text, key hints.
        small: 12., Design;
        /// Captions, tooltips.
        caption: 10., Design;
    }
}

group! {
    /// Font weights in use. The screens still name the framework's own today.
    Weight {
        /// Ordinary text.
        regular: 400., Weight;
        /// A rare emphasis: the studio's zoom chips.
        medium: 500., Weight;
        /// Headings and titles.
        semibold: 600., Weight;
    }
}

group! {
    /// Icon sizes (`IconSize`).
    Icon {
        /// The smallest: inside small controls, chevrons.
        xsmall: 12., Design;
        /// The default.
        small: 14., Design;
        /// Toolbar icons.
        medium: 16., Design;
        /// The editor's drawing tools, a step up from the other toolbars.
        tool: 18., Design;
        /// The capture bar's big mode icons.
        large: 22., Design;
    }
}

group! {
    /// Border widths.
    Border {
        /// Every hairline: a raised surface's edge, the tooltip's, an outlined button's, a divider.
        hairline: 1., Px;
    }
}

group! {
    /// Opacities: the text colour at these over the background makes the
    /// theme's controls and lines (`palette::soft`); the rest are the
    /// components' own.
    Alpha {
        /// A raised surface's hairline edge in the dark theme.
        edge_dark: 0.1, Alpha;
        /// A raised surface's hairline edge in the light theme.
        edge_light: 0.12, Alpha;
        /// Separators, an outlined button's border.
        separator: 0.07, Alpha;
        /// A filled control at rest: buttons, fields, tracks.
        control: 0.07, Alpha;
        /// A filled control under the pointer, menu rows.
        control_hover: 0.1, Alpha;
        /// A filled control pressed; a switch that's off.
        control_pressed: 0.14, Alpha;
        /// A filled control toggled on. Normalisation candidate (merge with pressed, 0.14).
        control_selected: 0.13, Alpha;
        /// A subtle control under the pointer.
        ghost_hover: 0.07, Alpha;
        /// A subtle control pressed. Normalisation candidate (0.14).
        ghost_pressed: 0.11, Alpha;
        /// A subtle control toggled on. Normalisation candidate (0.14).
        ghost_selected: 0.11, Alpha;
        /// A disabled switch, all of it.
        disabled: 0.45, Alpha;
        /// A scrollbar's thumb.
        scrollbar: 0.28, Alpha;
        /// A scrollbar's thumb while dragged.
        scrollbar_grabbed: 0.5, Alpha;
        /// The white hairline round the tooltip's black plate, so it reads on a dark screen.
        tip_edge: 0.14, Alpha;
        /// Dimmer words on the tooltip's plate: shortcuts.
        tip_dim: 0.5, Alpha;
        /// A key hint on an accent button, in the text colour on the accent.
        hint_on_accent: 0.7, Alpha;
    }
}

group! {
    /// Lightness: the accent's states, the work area's shade, and which text reads on a colour.
    Tone {
        /// An accent button under the pointer, lighter by this.
        accent_hover: 0.05, Lightness;
        /// An accent button pressed (negative: darker).
        accent_pressed: -0.05, Lightness;
        /// The work area a shade darker than the panels, in the dark theme.
        stage_dark: 0.025, Lightness;
        /// The same in the light theme.
        stage_light: 0.035, Lightness;
        /// Above this lightness a colour takes dark text, below it white.
        on_light_above: 0.62, Lightness;
        /// The lightness of that dark text.
        ink: 0.1, Lightness;
    }
}

group! {
    /// The tooltip, in logical pixels: it's measured and placed by hand.
    Tooltip {
        /// The widest its words get before they're cut.
        max_width: 240., Px;
        /// Each side of the words.
        pad_x: 6., Px;
        /// Above its words' line height, in all.
        pad_y: 4., Px;
        /// The line height, times the text size.
        line_height: 1.6, Ratio;
        /// Between the tooltip and its element.
        gap: 6., Px;
        /// Kept clear of the window's edges.
        margin: 4., Px;
        /// The room measured for the gap before a shortcut. Wider than the gap drawn (`keys_gap`): normalisation candidate.
        keys_room: 8., Px;
        /// The gap drawn before a shortcut.
        keys_gap: 4., Design;
        /// With no element found under the pointer: the tooltip's top this far below the pointer's tip.
        below_pointer: 14., Px;
        /// Or, above it, its bottom this far above.
        above_pointer: 4., Px;
    }
}

group! {
    /// The thin scrollbar, in logical pixels.
    Scrollbar {
        /// Its thumb's width.
        width: 5., Px;
        /// The thumb's corners.
        radius: 2.5, Px;
        /// From the thumb's left to the pane's right edge.
        inset: 8., Px;
        /// Kept clear of the pane's top and bottom.
        inset_y: 3., Px;
        /// The width along the right edge that takes clicks and drags.
        reach: 12., Px;
        /// The shortest thumb.
        min_thumb: 32., Px;
        /// No scrollbar on a pane lower than this.
        min_pane: 40., Px;
    }
}

group! {
    /// Single-line text editing, matching the canvas text's bar proportions.
    TextField {
        /// The caret's width, in logical pixels.
        caret_width: 1.5, Px;
        /// The caret's height as a fraction of the line box.
        caret_height: 0.76, Ratio;
        /// The caret's top inset as a fraction of the line box.
        caret_inset: 0.12, Ratio;
        /// Selected text's accent wash.
        selection: 0.25, Alpha;
        /// Under the input method's composing text, in logical pixels.
        composition_line: 1., Px;
    }
}

group! {
    /// The selection knob (`ui::paint_knob`): an accent dot in a white ring, in logical pixels.
    Knob {
        /// Its radius, ring included.
        radius: 9.5, Px;
        /// The white ring's width.
        ring: 2.5, Px;
        /// The soft shadow's outer disc: wider than the knob by this.
        outer_grow: 2., Px;
        /// The outer disc, lower than the knob by this.
        outer_y: 1.5, Px;
        /// The outer disc's opacity (black).
        outer_alpha: 0.1, Alpha;
        /// The shadow's inner disc: wider than the knob by this.
        inner_grow: 1., Px;
        /// The inner disc, lower by this.
        inner_y: 1., Px;
        /// The inner disc's opacity (black).
        inner_alpha: 0.18, Alpha;
    }
}

/// One layer of a drop shadow, black, straight down, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowLayer {
    pub y: f32,
    pub blur: f32,
    /// Its opacity in the dark theme and in the light one.
    pub dark: f32,
    pub light: f32,
}

impl ShadowLayer {
    const fn new(y: f32, blur: f32, dark: f32, light: f32) -> Self {
        ShadowLayer { y, blur, dark, light }
    }

    /// The same in both themes.
    const fn fixed(y: f32, blur: f32, alpha: f32) -> Self {
        ShadowLayer { y, blur, dark: alpha, light: alpha }
    }
}

/// Shadow recipes, each a wide soft layer and maybe a tight one.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadows {
    /// Popovers and floating bars (`elevation_2`).
    pub popover: Vec<ShadowLayer>,
    /// The highest surfaces: toasts, panels over the screen (`elevation_3`).
    pub floating: Vec<ShadowLayer>,
    /// Floating pictures: cards, pins, the camera (`kit::shadow`).
    pub picture: Vec<ShadowLayer>,
    /// A switch's knob.
    pub switch_knob: Vec<ShadowLayer>,
}

impl Default for Shadows {
    fn default() -> Self {
        Shadows {
            popover: vec![ShadowLayer::new(4., 14., 0.28, 0.10), ShadowLayer::new(1., 3., 0.18, 0.06)],
            floating: vec![ShadowLayer::new(10., 30., 0.36, 0.14), ShadowLayer::new(2., 6., 0.22, 0.08)],
            picture: vec![ShadowLayer::fixed(8., 22., 0.38), ShadowLayer::fixed(2., 6., 0.22)],
            switch_knob: vec![ShadowLayer::fixed(1., 2., 0.25)],
        }
    }
}

impl Shadows {
    /// Every recipe with its name and role.
    pub fn recipes(&self) -> [(&'static str, &[ShadowLayer], &'static str); 4] {
        [
            ("popover", &self.popover, "popovers, floating bars"),
            ("floating", &self.floating, "toasts, panels over the screen"),
            ("picture", &self.picture, "floating pictures"),
            ("switch_knob", &self.switch_knob, "a switch's knob"),
        ]
    }
}

/// The whole set. `Default` is today's look.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tokens {
    pub radius: Radius,
    pub spacing: Spacing,
    pub space: Space,
    pub control: Control,
    pub text: Text,
    pub weight: Weight,
    pub icon: Icon,
    pub border: Border,
    pub alpha: Alpha,
    pub tone: Tone,
    pub tooltip: Tooltip,
    pub scrollbar: Scrollbar,
    pub knob: Knob,
    pub text_field: TextField,
    pub shadow: Shadows,
}

impl Tokens {
    /// The groups of plain values, by name (shadows aside: `shadow.recipes`).
    pub fn groups(&self) -> Vec<(&'static str, Vec<Entry>)> {
        vec![
            ("radius", self.radius.entries()),
            ("spacing", self.spacing.entries()),
            ("space", self.space.entries()),
            ("control", self.control.entries()),
            ("text", self.text.entries()),
            ("weight", self.weight.entries()),
            ("icon", self.icon.entries()),
            ("border", self.border.entries()),
            ("alpha", self.alpha.entries()),
            ("tone", self.tone.entries()),
            ("tooltip", self.tooltip.entries()),
            ("scrollbar", self.scrollbar.entries()),
            ("knob", self.knob.entries()),
            ("text_field", self.text_field.entries()),
        ]
    }

    /// A plain value by its group and name ("radius", "button"), to change it.
    pub fn get_mut(&mut self, group: &str, name: &str) -> Option<&mut f32> {
        match group {
            "radius" => self.radius.get_mut(name),
            "spacing" => self.spacing.get_mut(name),
            "space" => self.space.get_mut(name),
            "control" => self.control.get_mut(name),
            "text" => self.text.get_mut(name),
            "weight" => self.weight.get_mut(name),
            "icon" => self.icon.get_mut(name),
            "border" => self.border.get_mut(name),
            "alpha" => self.alpha.get_mut(name),
            "tone" => self.tone.get_mut(name),
            "tooltip" => self.tooltip.get_mut(name),
            "scrollbar" => self.scrollbar.get_mut(name),
            "knob" => self.knob.get_mut(name),
            "text_field" => self.text_field.get_mut(name),
            _ => None,
        }
    }
}

// ------------------------------------------------------------ the current set

/// The set every thread reads, and a count of how often it was replaced:
/// a thread keeps its own copy of the `Arc` and takes the lock only when the
/// count has moved. Steady-state reads only load the version and clone an Arc.
static SHARED: Mutex<Option<Arc<Tokens>>> = Mutex::new(None);
static VERSION: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static SEEN: RefCell<Option<(u64, Arc<Tokens>)>> = const { RefCell::new(None) };
}

/// The current set. Cheap: call it while drawing.
pub fn get() -> Arc<Tokens> {
    let version = VERSION.load(Ordering::Acquire);
    SEEN.with_borrow_mut(|seen| match seen {
        Some((v, tokens)) if *v == version => tokens.clone(),
        _ => {
            let tokens = SHARED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(|| Arc::new(Tokens::default())).clone();
            // If a replacement came after the version load, the next read
            // takes the lock again rather than keeping this cache.
            *seen = Some((version, tokens.clone()));
            tokens
        }
    })
}

/// Replace the set for the whole process. The caller must redraw the app's
/// windows and rebuild its theme for changes to `Alpha` and `Tone`.
pub fn set(tokens: Tokens) {
    let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    *shared = Some(Arc::new(tokens));
    VERSION.fetch_add(1, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Today's look, written out once: what the components drew before they
    /// read tokens. A change here is a change of the look.
    #[test]
    fn the_defaults_are_todays_look() {
        let t = Tokens::default();
        let r = t.radius;
        assert_eq!((r.tooltip, r.button, r.button_large, r.surface, r.picture, r.smoothing), (6., 7., 9., 12., 20., 0.6));
        let s = t.spacing;
        assert_eq!((s.step_2, s.step_4, s.step_6, s.step_8, s.step_10, s.step_12, s.step_16, s.step_20, s.step_24, s.step_40), (2., 4., 6., 8., 10., 12., 16., 20., 24., 40.));
        let s = t.space;
        assert_eq!((s.key_hint, s.button_gap, s.button_pad, s.button_pad_large), (2., 4., 4., 8.));
        let c = t.control;
        assert_eq!((c.button, c.button_large, c.switch_width, c.switch_height, c.switch_knob, c.switch_inset, c.progress), (22., 32., 30., 18., 14., 2., 5.));
        assert_eq!((t.text.body, t.text.small, t.text.caption), (14., 12., 10.));
        assert_eq!((t.weight.regular, t.weight.medium, t.weight.semibold), (400., 500., 600.));
        let i = t.icon;
        assert_eq!((i.xsmall, i.small, i.medium, i.tool, i.large), (12., 14., 16., 18., 22.));
        assert_eq!(t.border.hairline, 1.);
        let a = t.alpha;
        assert_eq!((a.edge_dark, a.edge_light, a.separator), (0.1, 0.12, 0.07));
        assert_eq!((a.control, a.control_hover, a.control_pressed, a.control_selected), (0.07, 0.1, 0.14, 0.13));
        assert_eq!((a.ghost_hover, a.ghost_pressed, a.ghost_selected), (0.07, 0.11, 0.11));
        assert_eq!((a.disabled, a.scrollbar, a.scrollbar_grabbed, a.tip_edge, a.tip_dim, a.hint_on_accent), (0.45, 0.28, 0.5, 0.14, 0.5, 0.7));
        let o = t.tone;
        assert_eq!((o.accent_hover, o.accent_pressed, o.stage_dark, o.stage_light, o.on_light_above, o.ink), (0.05, -0.05, 0.025, 0.035, 0.62, 0.1));
        let p = t.tooltip;
        assert_eq!((p.max_width, p.pad_x, p.pad_y, p.line_height, p.gap, p.margin), (240., 6., 4., 1.6, 6., 4.));
        assert_eq!((p.keys_room, p.keys_gap, p.below_pointer, p.above_pointer), (8., 4., 14., 4.));
        let b = t.scrollbar;
        assert_eq!((b.width, b.radius, b.inset, b.inset_y, b.reach, b.min_thumb, b.min_pane), (5., 2.5, 8., 3., 12., 32., 40.));
        let k = t.knob;
        assert_eq!((k.radius, k.ring, k.outer_grow, k.outer_y, k.outer_alpha, k.inner_grow, k.inner_y, k.inner_alpha), (9.5, 2.5, 2., 1.5, 0.1, 1., 1., 0.18));
        let layer = |y, blur, dark, light| ShadowLayer { y, blur, dark, light };
        assert_eq!(t.shadow.popover, [layer(4., 14., 0.28, 0.10), layer(1., 3., 0.18, 0.06)]);
        assert_eq!(t.shadow.floating, [layer(10., 30., 0.36, 0.14), layer(2., 6., 0.22, 0.08)]);
        assert_eq!(t.shadow.picture, [layer(8., 22., 0.38, 0.38), layer(2., 6., 0.22, 0.22)]);
        assert_eq!(t.shadow.switch_knob, [layer(1., 2., 0.25, 0.25)]);
    }

    #[test]
    fn every_value_is_listed_and_reachable_by_name() {
        let mut t = Tokens::default();
        let groups = t.groups();
        assert_eq!(groups.iter().map(|(_, e)| e.len()).sum::<usize>(), 91);
        for (group, entries) in groups {
            for e in entries {
                assert!(!e.doc.is_empty(), "{group}.{} has no role", e.name);
                assert_eq!(t.get_mut(group, e.name).copied(), Some(e.value), "{group}.{}", e.name);
            }
        }
        assert!(t.get_mut("radius", "nothing").is_none() && t.get_mut("nothing", "button").is_none());
    }

    /// A new set reaches readers, on this thread and on others.
    #[test]
    fn a_set_replaces_what_get_returns() {
        let before = get();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let cached = get();
            ready_tx.send(cached).unwrap();
            read_rx.recv().unwrap();
            get()
        });
        assert_eq!(*ready_rx.recv().unwrap(), *before);
        let mut changed = Tokens::default();
        *changed.get_mut("radius", "button").unwrap() = 8.;
        changed.shadow.picture.pop();
        set(changed.clone());
        assert_eq!(*get(), changed);
        read_tx.send(()).unwrap();
        assert_eq!(*reader.join().unwrap(), changed);
        assert_eq!(*std::thread::spawn(get).join().unwrap(), changed);
        set((*before).clone());
        assert_eq!(*get(), *before);
    }
}
