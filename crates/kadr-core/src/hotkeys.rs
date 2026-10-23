//! kadr's global hotkeys as data: what each can do (`ACTIONS`), and key
//! specs in Hyprland's form ("CTRL + SHIFT + code:10"), which kadr keeps on
//! every system: turned into global-hotkey's form (`global`), shown to people
//! (`label`), made from a pressed key (`spec`). Where they're stored and how
//! the system gets them is the app's (`hotkeys.rs`).

/// Something a hotkey can do: a title for the settings and the command.
pub struct Action {
    pub title: &'static str,
    pub command: &'static str,
}

pub const ACTIONS: [Action; 11] = [
    Action { title: "shortcuts.all_modes_on_one_bar", command: "kadr capture" },
    Action { title: "shortcuts.screenshot_area", command: "kadr shot region" },
    Action { title: "shortcuts.screenshot_previous_area", command: "kadr shot previous" },
    Action { title: "shortcuts.screenshot_window_area", command: "kadr shot smart" },
    Action { title: "shortcuts.screenshot_screen", command: "kadr shot fullscreen" },
    Action { title: "common.scrolling_screenshot", command: "kadr shot scroll" },
    Action { title: "common.record_screen", command: "kadr record" },
    Action { title: "shortcuts.record_previous_area", command: "kadr record previous" },
    Action { title: "common.recognize_text", command: "kadr text" },
    Action { title: "shortcuts.history", command: "kadr history" },
    Action { title: "common.settings", command: "kadr settings" },
];

/// `o.bind("KEYS", "description", "command")` → (KEYS, command), for lines
/// that aren't commented out.
pub fn parse_bind(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("o.bind(")?;
    let quoted: Vec<&str> = rest.split('"').skip(1).step_by(2).collect();
    match quoted.as_slice() {
        [keys, _, command, ..] => Some((keys.to_string(), command.to_string())),
        _ => None,
    }
}

/// Keys on the main block by position (xkb keycodes), so a hotkey works in
/// any layout: Latin and Russian letters both map to their key.
const POSITIONS: [(&str, &str, u32); 36] = [
    ("1", "1", 10), ("2", "2", 11), ("3", "3", 12), ("4", "4", 13), ("5", "5", 14),
    ("6", "6", 15), ("7", "7", 16), ("8", "8", 17), ("9", "9", 18), ("0", "0", 19),
    ("q", "й", 24), ("w", "ц", 25), ("e", "у", 26), ("r", "к", 27), ("t", "е", 28),
    ("y", "н", 29), ("u", "г", 30), ("i", "ш", 31), ("o", "щ", 32), ("p", "з", 33),
    ("a", "ф", 38), ("s", "ы", 39), ("d", "в", 40), ("f", "а", 41), ("g", "п", 42),
    ("h", "р", 43), ("j", "о", 44), ("k", "л", 45), ("l", "д", 46),
    ("z", "я", 52), ("x", "ч", 53), ("c", "с", 54), ("v", "м", 55), ("b", "и", 56),
    ("n", "т", 57), ("m", "ь", 58),
];

/// Named keys: GPUI's name, Hyprland's, and how the settings show it.
const NAMED: [(&str, &str, &str); 9] = [
    ("print", "PRINT", "Print"),
    ("space", "SPACE", "Space"),
    ("enter", "RETURN", "Enter"),
    ("insert", "INSERT", "Insert"),
    ("delete", "DELETE", "Delete"),
    ("home", "HOME", "Home"),
    ("end", "END", "End"),
    ("pageup", "PRIOR", "PageUp"),
    ("pagedown", "NEXT", "PageDown"),
];

/// A pressed shortcut as a Hyprland key spec. None for a modifier alone, or
/// a plain key without modifiers (it would stop typing it everywhere),
/// except F-keys and Print.
pub fn spec(key: &str, ctrl: bool, alt: bool, shift: bool, logo: bool) -> Option<String> {
    let key = key.to_lowercase();
    let (name, bare_ok) = if let Some(&(_, _, code)) = POSITIONS.iter().find(|(l, r, _)| *l == key || *r == key) {
        (format!("code:{code}"), false)
    } else if let Some(&(_, hypr, _)) = NAMED.iter().find(|(g, _, _)| *g == key) {
        (hypr.to_owned(), hypr == "PRINT")
    } else if key.len() > 1 && key.starts_with('f') && key[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) {
        (key.to_uppercase(), true)
    } else {
        return None;
    };
    let mods: Vec<&str> = [(logo, "SUPER"), (ctrl, "CTRL"), (alt, "ALT"), (shift, "SHIFT")].into_iter().filter_map(|(on, m)| on.then_some(m)).collect();
    if mods.is_empty() && !bare_ok {
        return None;
    }
    Some(mods.into_iter().chain([name.as_str()]).collect::<Vec<_>>().join(" + "))
}

/// A key spec as global-hotkey reads it: "CTRL + SHIFT + code:10" → "CTRL+SHIFT+1".
pub fn global(spec: &str) -> String {
    spec.split('+')
        .map(|part| {
            let part = part.trim();
            if let Some(code) = part.strip_prefix("code:").and_then(|c| c.parse::<u32>().ok()) {
                return POSITIONS.iter().find(|p| p.2 == code).map(|p| p.0.to_uppercase()).unwrap_or_else(|| part.to_owned());
            }
            match part {
                "PRINT" => "PRINTSCREEN".into(),
                "RETURN" => "ENTER".into(),
                "PRIOR" => "PAGEUP".into(),
                "NEXT" => "PAGEDOWN".into(),
                _ => part.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// "CTRL + SHIFT + code:10" → "Ctrl+Shift+1".
pub fn label(spec: &str) -> String {
    spec.split('+')
        .map(|part| {
            let part = part.trim();
            if let Some(code) = part.strip_prefix("code:").and_then(|c| c.parse::<u32>().ok()) {
                return POSITIONS.iter().find(|p| p.2 == code).map(|p| p.0.to_uppercase()).unwrap_or_else(|| part.to_owned());
            }
            match part {
                "SUPER" if cfg!(target_os = "macos") => "Cmd".into(),
                "SUPER" if cfg!(windows) => "Win".into(),
                "SUPER" => "Super".into(),
                "CTRL" => "Ctrl".into(),
                "ALT" => "Alt".into(),
                "SHIFT" => "Shift".into(),
                _ => NAMED.iter().find(|n| n.1 == part).map(|n| n.2.to_owned()).unwrap_or_else(|| part.to_owned()),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_turn_into_hyprland_binds_and_back() {
        assert_eq!(spec("1", true, false, true, false).as_deref(), Some("CTRL + SHIFT + code:10"));
        // The same key in the Russian layout.
        assert_eq!(spec("ф", true, false, false, true).as_deref(), Some("SUPER + CTRL + code:38"));
        assert_eq!(spec("print", false, false, false, false).as_deref(), Some("PRINT"));
        assert_eq!(spec("a", false, false, false, false), None, "a bare letter would stop typing it");
        assert_eq!(spec("shift", false, false, true, false), None);
        assert_eq!(label("CTRL + SHIFT + code:10"), "Ctrl+Shift+1");
        assert_eq!(label("ALT + PRINT"), "Alt+Print");
        assert_eq!(parse_bind(r#"o.bind("CTRL + SHIFT + code:11", "Screen recording (kadr)", "kadr record")"#), Some(("CTRL + SHIFT + code:11".into(), "kadr record".into())));
        assert_eq!(parse_bind(r#"-- o.bind("X", "y", "kadr record")"#), None);
    }

    #[test]
    fn shortcuts_turn_into_global_hotkeys() {
        assert_eq!(global("CTRL + SHIFT + code:10"), "CTRL+SHIFT+1");
        assert_eq!(global("SUPER + ALT + code:38"), "SUPER+ALT+A");
        assert_eq!(global("ALT + PRINT"), "ALT+PRINTSCREEN");
        assert_eq!(global("SHIFT + PRIOR"), "SHIFT+PAGEUP");
        assert_eq!(global("F5"), "F5");
    }
}
