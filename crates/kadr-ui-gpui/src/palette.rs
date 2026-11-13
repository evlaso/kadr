//! kadr's own colours, the same on every system: a dark and a light set
//! (from Sumi Dark and a clean white one with the same blue). Settings pick
//! one or follow the system's dark or light appearance.

use gpui::{App, Hsla, WindowAppearance};

use crate::settings::{Settings, ThemeMode};
use crate::ui::{Colors, Status, Theme, from_hex};

/// The theme the settings ask for; "system" follows the appearance GPUI
/// reports (the portal's colour scheme on Linux, the OS setting elsewhere).
pub fn theme(cx: &App) -> Theme {
    let dark = match Settings::load().theme {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => crate::platform::prefers_dark().unwrap_or_else(|| matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)),
    };
    built_in(dark)
}

/// Apply the theme setting to open windows.
pub fn refresh(cx: &mut App) {
    let theme = theme(cx);
    cx.set_global(theme);
    cx.refresh_windows();
}

pub fn built_in(dark: bool) -> Theme {
    let t = crate::ui::tokens();
    if dark {
        let c = t.dark;
        soft(false, from_hex(c.background as u32), from_hex(c.foreground as u32), from_hex(c.surface as u32), from_hex(c.accent as u32), from_hex(c.muted as u32), from_hex(c.text_muted as u32), from_hex(c.error as u32), from_hex(c.warning as u32))
    } else {
        let c = t.light;
        soft(true, from_hex(c.background as u32), from_hex(c.foreground as u32), from_hex(c.surface as u32), from_hex(c.accent as u32), from_hex(c.muted as u32), from_hex(c.text_muted as u32), from_hex(c.error as u32), from_hex(c.warning as u32))
    }
}

/// Replace the live set without losing the gallery's temporary theme choice.
pub fn restyle(tokens: kadr_ui::tokens::Tokens, cx: &mut App) {
    use crate::ui::ActiveTheme;
    if *crate::ui::tokens() == tokens { return; }
    let dark = !cx.theme().light;
    kadr_ui::tokens::set(tokens);
    cx.set_global(built_in(dark));
    cx.refresh_windows();
}

/// A soft, Apple-like set: controls and separators are the text colour at a
/// low opacity over the background, so they stay gentle in light and dark
/// themes alike; the work area sits a shade apart instead of behind lines.
/// The opacities and the shade are tokens (`Alpha`, `Tone`): after a new
/// set, `refresh` mixes the theme again.
#[allow(clippy::too_many_arguments)]
fn soft(light: bool, bg: Hsla, fg: Hsla, surface: Hsla, accent: Hsla, muted: Hsla, text_muted: Hsla, red: Hsla, yellow: Hsla) -> Theme {
    let tokens = crate::ui::tokens();
    let (a, tone) = (tokens.alpha, tokens.tone);
    let tint = |opacity: f32| crate::ui::alpha(fg, opacity);
    let mut stage = bg;
    stage.lightness = if light { (bg.lightness - tone.stage_light).max(0.) } else { (bg.lightness - tone.stage_dark).max(0.) };
    Theme {
        light,
        colors: Colors {
            panel_background: bg,
            editor_background: stage,
            title_bar_background: bg,
            status_bar_background: bg,
            elevated_surface_background: surface,
            border: tint(if light { a.edge_light } else { a.edge_dark }),
            border_variant: tint(a.separator),
            element_background: tint(a.control),
            element_hover: tint(a.control_hover),
            element_active: tint(a.control_pressed),
            element_selected: tint(a.control_selected),
            ghost_element_hover: tint(a.ghost_hover),
            ghost_element_active: tint(a.ghost_pressed),
            ghost_element_selected: tint(a.ghost_selected),
            text: fg,
            text_muted,
            text_placeholder: muted,
            text_disabled: muted,
            text_accent: accent,
        },
        status: Status { error: red, warning: yellow, info: accent },
    }
}

/// Annotation colors for the editor: Flexoki's accents (they stand out on
/// any screenshot), plus white and black.
pub fn annotation_colors() -> Vec<Hsla> {
    let mut colors: Vec<Hsla> = [0xD14D41, 0xDA702C, 0xD0A215, 0x879A39, 0x4385BE, 0xCE5D97].into_iter().map(from_hex).collect();
    colors.push(gpui::white());
    colors.push(from_hex(0x100F0F));
    colors
}

/// Gradient backdrops shared by the screenshot editor and the studio, as
/// `kadr_raster` draws them.
pub fn backdrops() -> Vec<(Hsla, Hsla)> {
    kadr_raster::backdrops().into_iter().map(|(a, b)| (crate::ui::from_rgba(a), crate::ui::from_rgba(b))).collect()
}
