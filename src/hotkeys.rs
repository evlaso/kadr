//! kadr's global hotkeys, set in the settings. They are Hyprland binds that
//! kadr writes to `~/.config/hypr/kadr-keys.lua`, which the hook
//! `data/hypr/kadr.lua` loads. The first save takes kadr's binds over from
//! `bindings.lua`: they are commented out there (after a backup), so nothing
//! is bound twice.

use std::path::PathBuf;

/// Something a hotkey can do: a title for the settings and the command.
pub struct Action {
    pub title: &'static str,
    pub command: &'static str,
}

pub const ACTIONS: [Action; 9] = [
    Action { title: "Все режимы на одной панели", command: "kadr capture" },
    Action { title: "Скриншот области", command: "kadr shot region" },
    Action { title: "Скриншот окна или области", command: "kadr shot smart" },
    Action { title: "Скриншот экрана", command: "kadr shot fullscreen" },
    Action { title: "Скриншот с прокруткой", command: "kadr shot scroll" },
    Action { title: "Запись экрана", command: "kadr record" },
    Action { title: "Распознать текст", command: "kadr text" },
    Action { title: "История", command: "kadr history" },
    Action { title: "Настройки", command: "kadr settings" },
];

/// The Hyprland submap kadr switches to while a shortcut is being recorded:
/// it has no real binds, so the keys pressed reach the settings instead of
/// firing whatever they're bound to.
const QUIET: &str = "kadr-keys";

fn hypr_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config/hypr")
}

fn own_file() -> PathBuf {
    hypr_dir().join("kadr-keys.lua")
}

/// Hyprland key specs ("CTRL + SHIFT + code:10") for each of `ACTIONS`.
pub fn load() -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(own_file()).or_else(|_| std::fs::read_to_string(hypr_dir().join("bindings.lua"))).unwrap_or_default();
    let mut keys = vec![Vec::new(); ACTIONS.len()];
    for (spec, command) in text.lines().filter_map(parse_bind) {
        if let Some(i) = ACTIONS.iter().position(|a| a.command == command)
            && !keys[i].contains(&spec)
        {
            keys[i].push(spec);
        }
    }
    keys
}

/// `o.bind("KEYS", "description", "command")` → (KEYS, command), for lines
/// that aren't commented out.
fn parse_bind(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("o.bind(")?;
    let quoted: Vec<&str> = rest.split('"').skip(1).step_by(2).collect();
    match quoted.as_slice() {
        [keys, _, command, ..] => Some((keys.to_string(), command.to_string())),
        _ => None,
    }
}

/// Write kadr's binds and have Hyprland pick them up.
pub fn save(keys: &[Vec<String>]) -> std::io::Result<()> {
    let first = !own_file().exists();
    let mut out = String::from("-- kadr's hotkeys, written by its settings (Shortcuts). Loaded by hypr/kadr.lua.\n");
    for (action, specs) in ACTIONS.iter().zip(keys) {
        for spec in specs {
            out.push_str(&format!("o.bind(\"{spec}\", \"{} (kadr)\", \"{}\")\n", crate::i18n::t(action.title), action.command));
        }
    }
    std::fs::write(own_file(), out)?;
    if first {
        retire_old_binds()?;
    }
    reload();
    Ok(())
}

/// Comment out kadr's binds in bindings.lua, keeping a backup.
fn retire_old_binds() -> std::io::Result<()> {
    let path = hypr_dir().join("bindings.lua");
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(()) };
    if !text.lines().any(|l| parse_bind(l).is_some_and(|(_, c)| c.starts_with("kadr "))) {
        return Ok(());
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    std::fs::write(hypr_dir().join(format!("bindings.lua.bak-{stamp}")), &text)?;
    let out: Vec<String> = text
        .lines()
        .map(|l| match parse_bind(l) {
            Some((_, c)) if c.starts_with("kadr ") => format!("-- moved to kadr-keys.lua: {l}"),
            _ => l.to_owned(),
        })
        .collect();
    std::fs::write(&path, out.join("\n") + "\n")
}

fn hyprctl(lua: &str) {
    let _ = std::process::Command::new("hyprctl").args(["eval", lua]).output();
}

fn reload() {
    let _ = std::process::Command::new("hyprctl").arg("reload").output();
}

/// Pause every bind while a shortcut is recorded (`false` brings them back).
pub fn quiet(on: bool) {
    if on {
        hyprctl(&format!(
            r#"hl.define_submap("{QUIET}", function() hl.bind("SUPER + CTRL + ALT + SHIFT + F24", hl.dsp.submap("reset")) end); hl.dispatch(hl.dsp.submap("{QUIET}"))"#
        ));
    } else {
        hyprctl(r#"hl.dispatch(hl.dsp.submap("reset"))"#);
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

/// "CTRL + SHIFT + code:10" → "Ctrl+Shift+1".
pub fn label(spec: &str) -> String {
    spec.split('+')
        .map(|part| {
            let part = part.trim();
            if let Some(code) = part.strip_prefix("code:").and_then(|c| c.parse::<u32>().ok()) {
                return POSITIONS.iter().find(|p| p.2 == code).map(|p| p.0.to_uppercase()).unwrap_or_else(|| part.to_owned());
            }
            match part {
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
}
