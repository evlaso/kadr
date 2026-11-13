//! The interface's shared values in one place, with no UI framework: radii,
//! spacing, control sizes, type, icon sizes, borders, the alphas and tones
//! the theme is mixed from, shadows, and the few metrics of the shared
//! components (tooltip, scrollbar, knob, text field, menu, small controls). The app reads
//! them through `get()`, while drawing; `set` replaces the whole set for the
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
//! pair confirms) stays with the platform. The palette mixes its controls
//! from the base colour, alpha and tone tokens.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[path = "token_store.rs"]
pub mod store;

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
    /// An opaque sRGB colour, written as #RRGGBB (24 bits fit exactly in f32).
    Hex,
}

/// One value of a group, for listing and editing them by name.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub name: &'static str,
    pub value: f32,
    pub unit: Unit,
    /// Its doc comment: what the role is for.
    pub doc: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

impl Entry {
    fn new(group: &str, name: &'static str, value: f32, unit: Unit, doc: &'static str) -> Self {
        // Ranges follow roles, not the raw unit alone: a radius can be zero,
        // a hit target or a font cannot. Defaults always remain in range.
        let (min, max, step) = match unit {
            Unit::Hex => (0., 0xFFFFFF as f32, 1.),
            Unit::Alpha => (0., 1., 0.01),
            Unit::Lightness => match name {
                "accent_pressed" | "accent_hover" => (-0.3, 0.3, 0.01),
                "stage_dark" | "stage_light" => (0., 0.2, 0.005),
                _ => (0., 1., 0.01),
            },
            Unit::Weight => (100., 900., 100.),
            Unit::Ratio => match name {
                "line_height" => (1., 3., 0.05),
                "gradient_limit" => (1., 32., 1.),
                _ => (0., 1., 0.01),
            },
            _ if group == "Radius" || name.contains("radius") => (0., (value * 3.).max(24.), 0.5),
            _ if group == "Spacing" || group == "Space" => (0., value * 3., 0.5),
            _ if group == "Text" => (8., 32., 0.5),
            _ if group == "Knob" && name.ends_with("_y") => (-32., 64., 0.5),
            _ if name == "blur" => (0., 96., 0.5),
            _ => ((value * 0.5).min(8.), value * 2., 0.5),
        };
        Self { name, value, unit, doc, min, max, step }
    }

    pub fn accepts(self, value: f32) -> bool {
        value.is_finite() && value >= self.min && value <= self.max && (self.unit != Unit::Hex || value.fract() == 0.)
    }

    /// Exact input needn't snap; only dragging uses the suggested step.
    pub fn at_fraction(self, fraction: f32) -> f32 {
        let value = self.min + fraction.clamp(0., 1.) * (self.max - self.min);
        // Remove multiplication noise from decimal steps (0.18, not
        // 0.17999999), so both the exact field and saved file read naturally.
        let scale = 10_f64.powi((-self.step.log10().floor()).max(0.) as i32);
        let stepped = (f64::from(value) / f64::from(self.step)).round() * f64::from(self.step);
        (((stepped * scale).round() / scale) as f32).clamp(self.min, self.max)
    }

    pub fn formatted(self) -> String {
        if self.unit == Unit::Hex { format!("#{:06X}", self.value as u32) } else { self.value.to_string() }
    }

    pub fn parse(self, text: &str) -> Result<f32, String> {
        let value = if self.unit == Unit::Hex {
            let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
            if hex.len() != 6 { return Err("expected #RRGGBB".into()); }
            u32::from_str_radix(hex, 16).map(|v| v as f32).map_err(|_| "expected #RRGGBB".to_owned())?
        } else {
            text.trim().parse::<f32>().map_err(|_| "expected a number".to_owned())?
        };
        if self.accepts(value) { Ok(value) } else { Err(format!("out of range ({}..{})", self.min, self.max)) }
    }
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
                vec![$(Entry { value: self.$field, ..Entry::new(stringify!($name), stringify!($field), $value, Unit::$unit, concat!($($doc),*).trim_ascii()) }),*]
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
    /// Dark theme's base colours; derived control fills use Alpha and Tone.
    Dark {
        /// Panel and top bar background.
        background: 0x100F0F as f32, Hex;
        /// Main text and the tint used for controls.
        foreground: 0xCECDC3 as f32, Hex;
        /// Raised surfaces, menus and popovers.
        surface: 0x1C1B1A as f32, Hex;
        /// Main action and selection accent.
        accent: 0x4385BE as f32, Hex;
        /// Placeholder and disabled text.
        muted: 0x575653 as f32, Hex;
        /// Secondary text.
        text_muted: 0x878580 as f32, Hex;
        /// Error status.
        error: 0xD14D41 as f32, Hex;
        /// Warning status.
        warning: 0xD0A215 as f32, Hex;
    }
}

group! {
    /// Light theme's base colours.
    Light {
        /// Panel and top bar background.
        background: 0xFFFFFF as f32, Hex;
        /// Main text and the tint used for controls.
        foreground: 0x1C1B1A as f32, Hex;
        /// Raised surfaces, menus and popovers.
        surface: 0xFFFFFF as f32, Hex;
        /// Main action and selection accent.
        accent: 0x3B7DC4 as f32, Hex;
        /// Placeholder and disabled text.
        muted: 0xA8A7A1 as f32, Hex;
        /// Secondary text.
        text_muted: 0x6F6E69 as f32, Hex;
        /// Error status.
        error: 0xC03A2E as f32, Hex;
        /// Warning status.
        warning: 0xAD8301 as f32, Hex;
    }
}

group! {
    /// Small controls, kept at their existing logical-pixel sizes.
    SmallControls {
        /// Accent option height.
        option_height: 28., Px;
        /// Accent option corners; normalisation candidate.
        option_radius: 7., Px;
        /// Accent option text (not rem-based); normalisation candidate.
        option_text: 13., Px;
        /// Accent option gap.
        option_gap: 4., Px;
        /// Tray option height.
        tray_height: 24., Px;
        /// Tray option corners.
        tray_radius: 6., Px;
        /// Tray corners.
        tray_outer: 8., Px;
        /// Tray cell inset and gap.
        tray_inset: 2., Px;
        /// Checkbox box.
        check_size: 16., Px;
        /// Checkbox corners.
        check_radius: 4., Px;
        /// Checkbox check icon (not rem-based).
        check_icon: 12., Px;
        /// Checkbox row in a panel.
        check_row: 26., Px;
        /// Checkbox row in the crop footer.
        check_footer: 28., Px;
        /// Slider hit height.
        slider_height: 22., Px;
        /// Slider hit height with ticks.
        slider_ticks_height: 28., Px;
        /// Slider ticked track's vertical shift.
        slider_tick_shift: 3., Px;
        /// Slider track thickness.
        slider_track: 4., Px;
        /// Slider ticks' size.
        slider_tick: 2., Px;
        /// Slider ticks' distance below the track centre.
        slider_tick_y: 9., Px;
        /// Slider ticks' alpha.
        slider_tick_alpha: 0.3, Alpha;
        /// White slider knob radius; distinct from the selection knob.
        slider_knob: 8., Px;
        /// Slider knob shadow radius.
        slider_shadow: 9., Px;
        /// Slider knob shadow alpha.
        slider_shadow_alpha: 0.18, Alpha;
        /// Slider knob edge width.
        slider_edge: 0.5, Px;
        /// Slider value column.
        slider_label: 44., Px;
        /// Colour menu's ring diameter.
        swatch_size: 28., Px;
        /// Colour menu's dot diameter.
        swatch_dot: 20., Px;
        /// Chosen colour ring width.
        swatch_ring: 2., Px;
        /// Settings backdrop ring diameter.
        settings_swatch: 26., Px;
        /// Settings backdrop dot size.
        settings_dot: 18., Px;
        /// Settings backdrop ring corners (not concentric); normalisation candidate.
        settings_outer: 7., Px;
        /// Settings backdrop inner corners; normalisation candidate.
        settings_inner: 5., Px;
        /// Backdrop square content.
        tile: 36., Px;
        /// Backdrop square outer corners.
        tile_radius: 10., Px;
        /// Backdrop ring gap.
        tile_inset: 2., Px;
        /// Backdrop squares' gap.
        tile_gap: 4., Px;
        /// Backdrop plain-colour content.
        tile_dot: 22., Px;
        /// Initial number of gradients.
        gradient_limit: 10., Ratio;
        /// Over-image pill ink lightness.
        pill_ink: 0.12, Lightness;
        /// Over-image pill rest opacity.
        pill_alpha: 0.92, Alpha;
        /// Over-image pill pressed lightness.
        pill_pressed: 0.88, Lightness;
        /// Studio section icon hit size.
        rail_size: 36., Px;
        /// Studio section icon corners.
        rail_radius: 9., Px;
    }
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
    /// Menu rows and the studio's dropdown field. Distinct units and radii
    /// preserve the screens' old looks until normalization.
    Menu {
        /// Compact choice rows in the editor, history and studio.
        compact_row: 28., Px;
        /// Compact rows' corners (not rems).
        compact_radius: 6., Px;
        /// Natural-height settings rows' corners. Normalisation candidate.
        settings_radius: 6., Design;
        /// Recording menu rows, scaled with the UI.
        recording_row: 32., Design;
        /// Recording rows' corners. Normalisation candidate.
        recording_radius: 7., Design;
        /// Saved looks' rows, including their inline actions.
        preset_row: 30., Px;
        /// Saved looks' row corners. Normalisation candidate.
        preset_radius: 7., Px;
        /// Reserved space for a compact row's check (not the icon size).
        check_column: 14., Px;
        /// The studio's full-width aspect field.
        field_height: 30., Px;
        /// That field's corners. Normalisation candidate.
        field_radius: 8., Px;
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
    fn entries(layers: &[ShadowLayer]) -> Vec<Entry> {
        let names = [["layer_1_y", "layer_1_blur", "layer_1_dark", "layer_1_light"], ["layer_2_y", "layer_2_blur", "layer_2_dark", "layer_2_light"]];
        layers.iter().zip(names).flat_map(|(l, names)| {
            [(l.y, Unit::Px, "Shadow layer's vertical offset, in px.", -32., 64., 0.5),
             (l.blur, Unit::Px, "Shadow layer's blur, in px.", 0., 96., 0.5),
             (l.dark, Unit::Alpha, "Shadow layer's opacity in the dark theme.", 0., 1., 0.01),
             (l.light, Unit::Alpha, "Shadow layer's opacity in the light theme.", 0., 1., 0.01)]
                .into_iter().zip(names).map(|((value, unit, doc, min, max, step), name)| Entry { name, value, unit, doc, min, max, step })
        }).collect()
    }

    fn get_mut(&mut self, recipe: &str, name: &str) -> Option<&mut f32> {
        let layers = match recipe { "shadow_popover" => &mut self.popover, "shadow_floating" => &mut self.floating, "shadow_picture" => &mut self.picture, "shadow_switch_knob" => &mut self.switch_knob, _ => return None };
        let rest = name.strip_prefix("layer_")?;
        let (index, field) = rest.split_once('_')?;
        let layer = layers.get_mut(index.parse::<usize>().ok()?.checked_sub(1)?)?;
        match field { "y" => Some(&mut layer.y), "blur" => Some(&mut layer.blur), "dark" => Some(&mut layer.dark), "light" => Some(&mut layer.light), _ => None }
    }

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
    pub dark: Dark,
    pub light: Light,
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
    pub menu: Menu,
    pub small_controls: SmallControls,
    pub shadow: Shadows,
}

impl Tokens {
    /// Every editable value, including the existing shadow layers.
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
            ("menu", self.menu.entries()),
            ("small_controls", self.small_controls.entries()),
            ("dark", self.dark.entries()),
            ("light", self.light.entries()),
            ("shadow_popover", Shadows::entries(&self.shadow.popover)),
            ("shadow_floating", Shadows::entries(&self.shadow.floating)),
            ("shadow_picture", Shadows::entries(&self.shadow.picture)),
            ("shadow_switch_knob", Shadows::entries(&self.shadow.switch_knob)),
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
            "menu" => self.menu.get_mut(name),
            "small_controls" => self.small_controls.get_mut(name),
            "dark" => self.dark.get_mut(name),
            "light" => self.light.get_mut(name),
            _ if group.starts_with("shadow_") => self.shadow.get_mut(group, name),
            _ => None,
        }
    }

    pub fn entry(&self, path: &str) -> Option<Entry> {
        let (group, name) = path.split_once('.')?;
        self.groups().into_iter().find(|(g, _)| *g == group)?.1.into_iter().find(|e| e.name == name)
    }

    /// By-name edits share validation between the file, gallery and dev hook.
    pub fn edit(&mut self, path: &str, text: &str) -> Result<(), String> {
        let entry = self.entry(path).ok_or_else(|| format!("unknown token: {path}"))?;
        let value = entry.parse(text)?;
        let (group, name) = path.split_once('.').unwrap();
        *self.get_mut(group, name).unwrap() = value;
        Ok(())
    }
}

// ------------------------------------------------------------ the current set

/// The set every thread reads, and a count of how often it was replaced:
/// a thread keeps its own copy of the `Arc` and takes the lock only when the
/// count has moved. Steady-state reads only load the version and clone an Arc.
static SHARED: Mutex<Option<Arc<Tokens>>> = Mutex::new(None);
static VERSION: AtomicU64 = AtomicU64::new(0);

/// Replacements so far, for the dev hook's frame-coalescing checks.
pub fn revision() -> u64 { VERSION.load(Ordering::Acquire) }

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
        let d = t.dark;
        assert_eq!([d.background, d.foreground, d.surface, d.accent, d.muted, d.text_muted, d.error, d.warning].map(|v| v as u32),
            [0x100F0F, 0xCECDC3, 0x1C1B1A, 0x4385BE, 0x575653, 0x878580, 0xD14D41, 0xD0A215]);
        let l = t.light;
        assert_eq!([l.background, l.foreground, l.surface, l.accent, l.muted, l.text_muted, l.error, l.warning].map(|v| v as u32),
            [0xFFFFFF, 0x1C1B1A, 0xFFFFFF, 0x3B7DC4, 0xA8A7A1, 0x6F6E69, 0xC03A2E, 0xAD8301]);
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
        assert_eq!(groups.iter().map(|(_, e)| e.len()).sum::<usize>(), 188);
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
