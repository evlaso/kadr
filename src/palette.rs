//! kadr takes its colors from the current Omarchy theme (the user's Sumi
//! Dark / Sumi Light), mapped onto Zed's theme slots so Zed's components
//! pick them up. Read once per run; kadr exits when idle, so a theme switch
//! applies from the next capture.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Hsla, rgb};
use theme::{Appearance, GlobalTheme};

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

fn with_alpha(mut c: Hsla, a: f32) -> Hsla {
    c.a = a;
    c
}

pub fn apply(cx: &mut App) {
    let Some((light, p)) = read_palette() else { return };
    let get = |key: &str, fallback: &str| p.get(key).or_else(|| p.get(fallback)).copied();
    let (Some(bg), Some(fg), Some(accent)) = (get("background", "background"), get("foreground", "foreground"), get("accent", "blue")) else {
        return;
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

    let mut theme = (**GlobalTheme::theme(cx)).clone();
    theme.name = if light { "Omarchy Light".into() } else { "Omarchy Dark".into() };
    theme.appearance = if light { Appearance::Light } else { Appearance::Dark };

    let c = &mut theme.styles.colors;
    for slot in [&mut c.background, &mut c.surface_background, &mut c.panel_background, &mut c.editor_background, &mut c.status_bar_background, &mut c.title_bar_background, &mut c.toolbar_background, &mut c.tab_bar_background] {
        *slot = bg;
    }
    c.elevated_surface_background = surface;
    c.border = border;
    c.border_variant = border;
    c.border_focused = accent;
    c.border_selected = accent;
    c.border_disabled = with_alpha(border, 0.5);
    c.element_background = hover;
    c.element_hover = hover;
    c.element_active = selection;
    c.element_selected = selection;
    c.element_disabled = with_alpha(hover, 0.5);
    c.ghost_element_hover = hover;
    c.ghost_element_active = selection;
    c.ghost_element_selected = selection;
    c.drop_target_background = with_alpha(accent, 0.15);
    c.text = fg;
    c.text_muted = text_muted;
    c.text_placeholder = muted;
    c.text_disabled = muted;
    c.text_accent = accent;
    c.icon = fg;
    c.icon_muted = text_muted;
    c.icon_placeholder = muted;
    c.icon_disabled = muted;
    c.icon_accent = accent;
    c.scrollbar_thumb_background = with_alpha(muted, 0.4);

    let s = &mut theme.styles.status;
    s.error = red;
    s.error_background = with_alpha(red, 0.15);
    s.error_border = with_alpha(red, 0.4);
    s.info = accent;
    s.info_background = with_alpha(accent, 0.18);
    s.info_border = with_alpha(accent, 0.45);
    s.hint = muted;
    if let Some(yellow) = get("yellow", "accent") {
        s.warning = yellow;
        s.warning_background = with_alpha(yellow, 0.15);
    }
    if let Some(green) = get("green", "accent") {
        s.success = green;
    }

    GlobalTheme::update_theme(cx, Arc::new(theme));
}
