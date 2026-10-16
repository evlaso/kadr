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

/// For places without GPUI (the tray): the setting, the desktop's
/// preference, or dark.
pub fn theme_without_gpui() -> Theme {
    built_in(match Settings::load().theme {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => crate::platform::prefers_dark().unwrap_or(true),
    })
}

/// Apply the theme setting to open windows.
pub fn refresh(cx: &mut App) {
    let theme = theme(cx);
    cx.set_global(theme);
    cx.refresh_windows();
}

pub fn built_in(dark: bool) -> Theme {
    if dark {
        soft(false, from_hex(0x100F0F), from_hex(0xCECDC3), from_hex(0x1C1B1A), from_hex(0x4385BE), from_hex(0x575653), from_hex(0x878580), from_hex(0xD14D41), from_hex(0xD0A215))
    } else {
        soft(true, from_hex(0xFFFFFF), from_hex(0x1C1B1A), from_hex(0xFFFFFF), from_hex(0x3B7DC4), from_hex(0xA8A7A1), from_hex(0x6F6E69), from_hex(0xC03A2E), from_hex(0xAD8301))
    }
}

/// A soft, Apple-like set: controls and separators are the text colour at a
/// low opacity over the background, so they stay gentle in light and dark
/// themes alike; the work area sits a shade apart instead of behind lines.
#[allow(clippy::too_many_arguments)]
fn soft(light: bool, bg: Hsla, fg: Hsla, surface: Hsla, accent: Hsla, muted: Hsla, text_muted: Hsla, red: Hsla, yellow: Hsla) -> Theme {
    let tint = |a: f32| {
        let mut c = fg;
        c.alpha = a;
        c
    };
    let mut stage = bg;
    stage.lightness = if light { (bg.lightness - 0.035).max(0.) } else { (bg.lightness - 0.025).max(0.) };
    Theme {
        light,
        colors: Colors {
            panel_background: bg,
            editor_background: stage,
            title_bar_background: bg,
            status_bar_background: bg,
            elevated_surface_background: surface,
            border: tint(if light { 0.12 } else { 0.1 }),
            border_variant: tint(0.07),
            border_focused: accent,
            element_background: tint(0.07),
            element_hover: tint(0.1),
            element_active: tint(0.14),
            element_selected: tint(0.13),
            ghost_element_hover: tint(0.07),
            ghost_element_active: tint(0.11),
            ghost_element_selected: tint(0.11),
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

/// Gradient backdrops shared by the screenshot editor and the studio.
pub fn backdrops() -> Vec<(Hsla, Hsla)> {
    let c = annotation_colors();
    vec![(c[4], c[5]), (c[1], c[0]), (c[3], c[4]), (gpui::hsla(0., 0., 0.22, 1.), gpui::hsla(0., 0., 0.08, 1.))]
}
