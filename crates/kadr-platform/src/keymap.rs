//! Keyboard conventions per system (SPEC §3.10): the keys of the common
//! commands, and how a shortcut is written for the user (⌘⇧Z on macOS,
//! Ctrl+Shift+Z elsewhere). Plain data for every system, so each one's
//! table is tested on any of them; `Os::current()` picks the one in use.

/// A desktop system, for the tables here and in `style`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

impl Os {
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// Commands whose keys differ from one system to another.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Settings,
    CloseWindow,
    Quit,
    Save,
    SaveAs,
    Copy,
    Paste,
    Undo,
    Redo,
    SelectAll,
    ZoomIn,
    ZoomOut,
    LineStart,
    LineEnd,
    WordLeft,
    WordRight,
    DeleteWordBack,
}

/// The keys for `command` as GPUI keystrokes ("cmd-,"), the usual one
/// first; empty when the system has none of its own.
pub fn keys(os: Os, command: Command) -> &'static [&'static str] {
    use Command::*;
    use Os::*;
    match (os, command) {
        (MacOs, Settings) => &["cmd-,"],
        (_, Settings) => &["ctrl-,"],
        (MacOs, CloseWindow) => &["cmd-w"],
        (Windows, CloseWindow) => &["alt-f4", "ctrl-w"],
        (Linux, CloseWindow) => &["ctrl-w"],
        (MacOs, Quit) => &["cmd-q"],
        // Windows apps quit by closing their last window.
        (Windows, Quit) => &[],
        (Linux, Quit) => &["ctrl-q"],
        (MacOs, Save) => &["cmd-s"],
        (_, Save) => &["ctrl-s"],
        (MacOs, SaveAs) => &["cmd-shift-s"],
        (_, SaveAs) => &["ctrl-shift-s"],
        (MacOs, Copy) => &["cmd-c"],
        (_, Copy) => &["ctrl-c"],
        (MacOs, Paste) => &["cmd-v"],
        (_, Paste) => &["ctrl-v"],
        (MacOs, Undo) => &["cmd-z"],
        (_, Undo) => &["ctrl-z"],
        (MacOs, Redo) => &["cmd-shift-z"],
        (Windows, Redo) => &["ctrl-y", "ctrl-shift-z"],
        (Linux, Redo) => &["ctrl-shift-z", "ctrl-y"],
        (MacOs, SelectAll) => &["cmd-a"],
        (_, SelectAll) => &["ctrl-a"],
        (MacOs, ZoomIn) => &["cmd-="],
        (_, ZoomIn) => &["ctrl-="],
        (MacOs, ZoomOut) => &["cmd--"],
        (_, ZoomOut) => &["ctrl--"],
        (MacOs, LineStart) => &["cmd-left", "home"],
        (_, LineStart) => &["home"],
        (MacOs, LineEnd) => &["cmd-right", "end"],
        (_, LineEnd) => &["end"],
        (MacOs, WordLeft) => &["alt-left"],
        (_, WordLeft) => &["ctrl-left"],
        (MacOs, WordRight) => &["alt-right"],
        (_, WordRight) => &["ctrl-right"],
        (MacOs, DeleteWordBack) => &["alt-backspace"],
        (_, DeleteWordBack) => &["ctrl-backspace"],
    }
}

/// The usual key for `command` on this system, if it has one.
pub fn key(command: Command) -> Option<&'static str> {
    keys(Os::current(), command).first().copied()
}

/// A keystroke ("ctrl-shift-z"; "secondary" is Cmd on macOS and Ctrl
/// elsewhere) as the system writes it: its keycaps, modifiers first in the
/// system's order. macOS draws one cap of symbols ("⇧⌘Z"), the others one
/// per key ("Ctrl", "Shift", "Z").
pub fn caps(os: Os, keystroke: &str) -> Vec<String> {
    let mut parts: Vec<&str> = keystroke.split('-').collect();
    // "ctrl--": the key itself is a minus.
    if keystroke.ends_with("--") {
        parts.truncate(parts.len() - 2);
        parts.push("-");
    }
    let Some((key, mods)) = parts.split_last() else { return Vec::new() };
    let has = |m: &[&str]| mods.iter().any(|p| m.contains(p));
    let (ctrl, alt, shift) = (has(&["ctrl"]), has(&["alt", "option"]), has(&["shift"]));
    let logo = has(&["cmd", "super", "platform", "win"]);
    let secondary = has(&["secondary"]);
    match os {
        Os::MacOs => {
            // ⌃⌥⇧⌘, as in macOS's menus.
            let mut cap = String::new();
            for (on, sym) in [(ctrl, "⌃"), (alt, "⌥"), (shift, "⇧"), (logo || secondary, "⌘")] {
                if on {
                    cap.push_str(sym);
                }
            }
            cap.push_str(&key_name(os, key));
            vec![cap]
        }
        _ => {
            let mut out = Vec::new();
            for (on, name) in [(ctrl || secondary, "Ctrl"), (alt, "Alt"), (shift, "Shift"), (logo, if os == Os::Windows { "Win" } else { "Super" })] {
                if on {
                    out.push(name.to_owned());
                }
            }
            out.push(key_name(os, key));
            out
        }
    }
}

/// The shortcut modifier by name: "⌘" on macOS, "Ctrl" elsewhere.
pub fn secondary(os: Os) -> &'static str {
    if os == Os::MacOs { "⌘" } else { "Ctrl" }
}

/// A shortcut for running text, such as a tooltip's "(Ctrl+C)": "⌘C" on
/// macOS, "Ctrl+C" elsewhere.
pub fn text(os: Os, keystroke: &str) -> String {
    caps(os, keystroke).join(if os == Os::MacOs { "" } else { "+" })
}

fn key_name(os: Os, key: &str) -> String {
    let mac = os == Os::MacOs;
    match key {
        "enter" => if mac { "↩" } else { "Enter" }.into(),
        "escape" => if mac { "⎋" } else { "Esc" }.into(),
        "backspace" => if mac { "⌫" } else { "Backspace" }.into(),
        "delete" => if mac { "⌦" } else { "Del" }.into(),
        "tab" => if mac { "⇥" } else { "Tab" }.into(),
        "space" => if mac { "Space" } else { "Space" }.into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "home" => if mac { "↖" } else { "Home" }.into(),
        "end" => if mac { "↘" } else { "End" }.into(),
        "-" => "−".into(),
        k if k.len() == 1 => k.to_uppercase(),
        k => {
            let mut c = k.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_written_the_systems_way() {
        assert_eq!(caps(Os::MacOs, "ctrl-shift-z"), ["⌃⇧Z"]);
        assert_eq!(caps(Os::MacOs, "secondary-shift-z"), ["⇧⌘Z"]);
        assert_eq!(caps(Os::MacOs, "cmd-,"), ["⌘,"]);
        assert_eq!(caps(Os::Windows, "secondary-shift-z"), ["Ctrl", "Shift", "Z"]);
        assert_eq!(caps(Os::Linux, "super-shift-4"), ["Shift", "Super", "4"]);
        assert_eq!(caps(Os::Windows, "super-e"), ["Win", "E"]);
        assert_eq!(text(Os::Linux, "ctrl-c"), "Ctrl+C");
        assert_eq!(text(Os::MacOs, "cmd-c"), "⌘C");
        assert_eq!(text(Os::Windows, "ctrl--"), "Ctrl+−");
        assert_eq!(text(Os::MacOs, "escape"), "⎋");
        assert_eq!(text(Os::Windows, "escape"), "Esc");
    }

    #[test]
    fn each_system_has_its_own_keys() {
        assert_eq!(keys(Os::MacOs, Command::Settings), ["cmd-,"]);
        assert_eq!(keys(Os::Windows, Command::Settings), ["ctrl-,"]);
        assert_eq!(keys(Os::MacOs, Command::Quit), ["cmd-q"]);
        assert!(keys(Os::Windows, Command::Quit).is_empty());
        assert_eq!(keys(Os::Windows, Command::CloseWindow)[0], "alt-f4");
        assert_eq!(keys(Os::MacOs, Command::WordLeft), ["alt-left"]);
        assert_eq!(keys(Os::Linux, Command::WordLeft), ["ctrl-left"]);
        assert_eq!(keys(Os::MacOs, Command::LineStart)[0], "cmd-left");
        assert_eq!(keys(Os::Windows, Command::Redo)[0], "ctrl-y");
    }

    #[test]
    fn every_command_has_keys_somewhere() {
        use Command::*;
        for c in [Settings, CloseWindow, Quit, Save, SaveAs, Copy, Paste, Undo, Redo, SelectAll, ZoomIn, ZoomOut, LineStart, LineEnd, WordLeft, WordRight, DeleteWordBack] {
            for os in [Os::MacOs, Os::Linux] {
                assert!(!keys(os, c).is_empty(), "{os:?} {c:?}");
            }
        }
    }
}
