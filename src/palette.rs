//! kadr takes its colors from the current Omarchy theme (the user's Sumi
//! Dark / Sumi Light), mapped onto the slots kadr's components draw with. Read once per run; kadr exits when idle, so a theme switch
//! applies from the next capture.

use std::collections::HashMap;

use gpui::{Hsla, hsla};

use crate::ui::{Colors, Status, Theme, from_hex};

fn colors_file() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    home.join(".local/state/omarchy/current/theme/colors.toml")
}

/// `key = "#rrggbb"` lines; other value forms (rgba(), names) are skipped.
fn read_palette() -> Option<(bool, HashMap<String, Hsla>)> {
    let text = std::fs::read_to_string(colors_file()).ok()?;
    let mut light = false;
    let mut colors = HashMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim().trim_matches('"'));
        if key == "mode" {
            light = value == "light";
        } else if let Some(hex) = value.strip_prefix('#').filter(|h| h.len() == 6)
            && let Ok(n) = u32::from_str_radix(hex, 16)
        {
            colors.insert(key.to_owned(), from_hex(n));
        }
    }
    Some((light, colors))
}

/// kadr's theme from the Omarchy palette; a neutral dark one without it.
pub fn theme() -> Theme {
    let Some((light, p)) = read_palette() else { return fallback() };
    let get = |key: &str, fallback: &str| p.get(key).or_else(|| p.get(fallback)).copied();
    let (Some(bg), Some(fg), Some(accent)) = (get("background", "background"), get("foreground", "foreground"), get("accent", "blue")) else {
        return fallback();
    };
    let muted = get("muted", "muted").unwrap_or(fg);
    // Sumi Dark lifts surfaces a step above the background; Sumi Light keeps
    // them white.
    let (surface, text_muted) = if light { (bg, muted) } else { (get("lighter_background", "background").unwrap_or(bg), get("dark_foreground", "muted").unwrap_or(muted)) };
    soft(light, bg, fg, surface, accent, muted, text_muted, get("red", "accent").unwrap_or(accent), get("yellow", "accent").unwrap_or(accent))
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

fn fallback() -> Theme {
    let grey = |l: f32| hsla(0., 0., l, 1.);
    let accent = hsla(0.58, 0.6, 0.62, 1.);
    soft(false, grey(0.1), grey(0.9), grey(0.14), accent, grey(0.45), grey(0.62), hsla(0.01, 0.6, 0.55, 1.), hsla(0.12, 0.7, 0.5, 1.))
}

/// Annotation colors for the editor: the theme's accents when it has them,
/// Flexoki's (Sumi Dark's) otherwise — Sumi Light is monochrome, and grey
/// arrows don't stand out on a screenshot. Plus white and black.
pub fn annotation_colors() -> Vec<Hsla> {
    let theme = read_palette().map(|(_, p)| p).unwrap_or_default();
    let defaults = [("red", 0xD14D41), ("orange", 0xDA702C), ("yellow", 0xD0A215), ("green", 0x879A39), ("blue", 0x4385BE), ("magenta", 0xCE5D97)];
    let mut colors: Vec<Hsla> = defaults
        .iter()
        .map(|&(key, fallback)| theme.get(key).copied().filter(|c| c.saturation > 0.25).unwrap_or_else(|| from_hex(fallback)))
        .collect();
    colors.push(gpui::white());
    colors.push(from_hex(0x100F0F));
    colors
}

/// Gradient backdrops shared by the screenshot editor and the studio.
pub fn backdrops() -> Vec<(Hsla, Hsla)> {
    let c = annotation_colors();
    vec![(c[4], c[5]), (c[1], c[0]), (c[3], c[4]), (gpui::hsla(0., 0., 0.22, 1.), gpui::hsla(0., 0., 0.08, 1.))]
}
