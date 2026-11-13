//! Hand-editable restyles. Only differences from shipped defaults are kept;
//! bad entries are reported and skipped, never a reason not to start.

use std::path::{Path, PathBuf};
use super::{Tokens, Unit};

pub fn path() -> PathBuf { kadr_core::dirs::config().join("tokens.toml") }

pub fn parse(text: &str) -> (Tokens, Vec<String>) {
    let mut tokens = Tokens::default();
    let mut notes = Vec::new();
    let document = match text.parse::<toml_edit::DocumentMut>() {
        Ok(document) => document,
        Err(e) => return (tokens, vec![format!("ignored tokens.toml: {e}")]),
    };
    for (group, item) in document.iter() {
        let Some(table) = item.as_table_like() else {
            notes.push(format!("ignored {group}: expected a group"));
            continue;
        };
        for (name, item) in table.iter() {
            let path = format!("{group}.{name}");
            let value = item.as_str().map(str::to_owned)
                .or_else(|| item.as_float().map(|v| v.to_string()))
                .or_else(|| item.as_integer().map(|v| v.to_string()));
            let result = value.ok_or_else(|| "expected a number or hex colour".to_owned()).and_then(|v| tokens.edit(&path, &v));
            if let Err(e) = result { notes.push(format!("ignored {path}: {e}")); }
        }
        if table.is_empty() && !tokens.groups().iter().any(|(name, _)| *name == group) {
            notes.push(format!("ignored unknown group: {group}"));
        }
    }
    (tokens, notes)
}

pub fn load() -> Tokens { load_from(&path()) }

fn load_from(path: &Path) -> Tokens {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (tokens, notes) = parse(&text);
            for note in notes { eprintln!("kadr: tokens: {note}"); }
            tokens
        }
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound { eprintln!("kadr: tokens: ignored {}: {e}", path.display()); }
            Tokens::default()
        }
    }
}

/// An empty diff is empty bytes, not an empty configuration file.
pub fn serialize(tokens: &Tokens) -> String {
    let defaults = Tokens::default();
    let mut text = String::new();
    for (group, entries) in tokens.groups() {
        let changed: Vec<_> = entries.into_iter().filter(|e| defaults.entry(&format!("{group}.{}", e.name)).is_some_and(|d| d.value != e.value)).collect();
        if changed.is_empty() { continue; }
        if !text.is_empty() { text.push('\n'); }
        text.push_str(&format!("[{group}]\n"));
        for e in changed {
            let value = if e.unit == Unit::Hex { format!("\"{}\"", e.formatted()) } else { e.formatted() };
            text.push_str(&format!("# {}\n{} = {value}\n", e.doc, e.name));
        }
    }
    text
}

pub fn save(tokens: &Tokens) -> std::io::Result<()> { save_to(tokens, &path()) }

fn save_to(tokens: &Tokens, path: &Path) -> std::io::Result<()> {
    let text = serialize(tokens);
    if text.is_empty() {
        return match std::fs::remove_file(path) { Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()), result => result };
    }
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
    let temp = path.with_extension("toml.tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(temp, path)
}

/// Patch-like text for an agent or a defaults edit, not a second file format.
pub fn copy_defaults(tokens: &Tokens) -> String {
    let defaults = Tokens::default();
    tokens.groups().into_iter().flat_map(|(group, entries)| {
        entries.into_iter().filter_map(|e| {
            let path = format!("{group}.{}", e.name);
            let default = defaults.entry(&path)?;
            (e.value != default.value).then(|| format!("{path}: {} → {}\n", default.formatted(), e.formatted()))
        }).collect::<Vec<_>>()
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restyles_round_trip_with_comments_and_only_differences() {
        let mut tokens = Tokens::default();
        for (path, value) in [("radius.button", "10"), ("tone.accent_pressed", "-0.1"), ("dark.accent", "#12ABEF"), ("shadow_popover.layer_2_blur", "9")] {
            tokens.edit(path, value).unwrap();
        }
        let text = serialize(&tokens);
        assert!(text.contains("# A default-size button.") && text.contains("[dark]\n# Main action") && !text.contains("button_large ="));
        let (restored, notes) = parse(&text);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(restored, tokens);
        assert!(copy_defaults(&tokens).contains("dark.accent: #4385BE → #12ABEF"));
    }

    #[test]
    fn bad_entries_do_not_hide_good_entries() {
        let (tokens, notes) = parse("[control]\nbutton = 0\nbutton_large = 40\nunknown = 2\n[alpha]\ncontrol = 1.1\n[future]\nthing = 5\n[text]\nbody = nan\n");
        assert_eq!(tokens.control.button, 22.);
        assert_eq!(tokens.control.button_large, 40.);
        assert_eq!(tokens.alpha.control, 0.07);
        assert_eq!(notes.len(), 5, "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("out of range")) && notes.iter().any(|n| n.contains("unknown token")));
        let (tokens, notes) = parse("[broken");
        assert_eq!(tokens, Tokens::default());
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("ignored tokens.toml"));
    }

    #[test]
    fn empty_diff_writes_nothing_and_removes_a_saved_restyle() {
        let dir = std::env::temp_dir().join(format!("kadr-token-store-{}", std::process::id()));
        let path = dir.join("restyle-test.toml");
        let mut tokens = Tokens::default();
        assert!(serialize(&tokens).is_empty() && copy_defaults(&tokens).is_empty());
        save_to(&tokens, &path).unwrap();
        assert!(!path.exists());
        tokens.edit("spacing.step_8", "10").unwrap();
        save_to(&tokens, &path).unwrap();
        assert_eq!(load_from(&path), tokens);
        save_to(&Tokens::default(), &path).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn metadata_bounds_are_stable_and_every_default_is_valid() {
        let mut tokens = Tokens::default();
        for (group, entries) in tokens.groups() {
            for e in entries {
                assert!(e.accepts(e.value), "{group}.{}", e.name);
                assert!(e.step > 0. && e.min < e.max && !e.doc.is_empty());
                let path = format!("{group}.{}", e.name);
                assert!(tokens.edit(&path, "NaN").is_err());
                *tokens.get_mut(group, e.name).unwrap() = e.max;
                let changed = tokens.entry(&path).unwrap();
                assert_eq!((e.min, e.max, e.step), (changed.min, changed.max, changed.step));
                assert!(e.accepts(e.at_fraction(0.)) && e.accepts(e.at_fraction(1.)));
            }
        }
        assert!(tokens.edit("dark.accent", "#12345").is_err());
        assert!(tokens.edit("dark.accent", "#GGFFFF").is_err());
        assert!(tokens.edit("control.button", "0").is_err());
        let alpha = Tokens::default().entry("alpha.control").unwrap();
        let tone = Tokens::default().entry("tone.stage_dark").unwrap();
        assert_eq!(alpha.at_fraction(0.18).to_string(), "0.18");
        assert_eq!(tone.at_fraction(0.25).to_string(), "0.05");
        assert!(Tokens::default().entry("tooltip.pad_y").unwrap().min >= 0.);
        assert!(Tokens::default().entry("scrollbar.inset_y").unwrap().min >= 0.);
    }
}
