//! kadr takes its colors from the current Omarchy theme (the user's Sumi
//! Dark / Sumi Light), mapped onto the slots kadr's components draw with. Read once per run; kadr exits when idle, so a theme switch
//! applies from the next capture.

use std::collections::HashMap;

use gpui::{Hsla, hsla, rgb};

use crate::ui::{Colors, Status, Theme};

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
            colors.insert(key.to_owned(), rgb(n).into());
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
    let selection = get("selection", "muted").unwrap_or(muted);
    // Sumi Dark lifts panels a step above the background; Sumi Light keeps
    // them white and draws structure with light greys.
    let (surface, border, hover, text_muted) = if light {
        (bg, get("darker_background", "selection").unwrap_or(selection), get("dark_background", "selection").unwrap_or(selection), muted)
    } else {
        (get("lighter_background", "background").unwrap_or(bg), selection, selection, get("dark_foreground", "muted").unwrap_or(muted))
    };
    let red = get("red", "accent").unwrap_or(accent);
    Theme {
        light,
        colors: Colors {
            panel_background: bg,
            editor_background: bg,
            title_bar_background: bg,
            status_bar_background: bg,
            elevated_surface_background: surface,
            border,
            border_variant: border,
            border_focused: accent,
            element_background: hover,
            element_hover: hover,
            element_active: selection,
            element_selected: selection,
            ghost_element_hover: hover,
            ghost_element_active: selection,
            ghost_element_selected: selection,
            text: fg,
            text_muted,
            text_placeholder: muted,
            text_disabled: muted,
            text_accent: accent,
        },
        status: Status {
            error: red,
            warning: get("yellow", "accent").unwrap_or(accent),
            info: accent,
        },
    }
}

fn fallback() -> Theme {
    let grey = |l: f32| hsla(0., 0., l, 1.);
    let accent = hsla(0.58, 0.6, 0.62, 1.);
    Theme {
        light: false,
        colors: Colors {
            panel_background: grey(0.1),
            editor_background: grey(0.1),
            title_bar_background: grey(0.1),
            status_bar_background: grey(0.1),
            elevated_surface_background: grey(0.14),
            border: grey(0.22),
            border_variant: grey(0.22),
            border_focused: accent,
            element_background: grey(0.2),
            element_hover: grey(0.22),
            element_active: grey(0.27),
            element_selected: grey(0.27),
            ghost_element_hover: grey(0.2),
            ghost_element_active: grey(0.25),
            ghost_element_selected: grey(0.25),
            text: grey(0.9),
            text_muted: grey(0.62),
            text_placeholder: grey(0.45),
            text_disabled: grey(0.45),
            text_accent: accent,
        },
        status: Status { error: hsla(0.01, 0.6, 0.55, 1.), warning: hsla(0.12, 0.7, 0.5, 1.), info: accent },
    }
}

/// Annotation colors for the editor: the theme's accents when it has them,
/// Flexoki's (Sumi Dark's) otherwise — Sumi Light is monochrome, and grey
/// arrows don't stand out on a screenshot. Plus white and black.
pub fn annotation_colors() -> Vec<Hsla> {
    let theme = read_palette().map(|(_, p)| p).unwrap_or_default();
    let defaults = [("red", 0xD14D41), ("orange", 0xDA702C), ("yellow", 0xD0A215), ("green", 0x879A39), ("blue", 0x4385BE), ("magenta", 0xCE5D97)];
    let mut colors: Vec<Hsla> = defaults
        .iter()
        .map(|&(key, fallback)| theme.get(key).copied().filter(|c| c.s > 0.25).unwrap_or_else(|| rgb(fallback).into()))
        .collect();
    colors.push(gpui::white());
    colors.push(rgb(0x100F0F).into());
    colors
}

/// Gradient backdrops shared by the screenshot editor and the studio.
pub fn backdrops() -> Vec<(Hsla, Hsla)> {
    let c = annotation_colors();
    vec![(c[4], c[5]), (c[1], c[0]), (c[3], c[4]), (gpui::hsla(0., 0., 0.22, 1.), gpui::hsla(0., 0., 0.08, 1.))]
}
