//! The user manual (`docs/manual`) against the code and against itself:
//! SUMMARY.md lists every page, every screen's page has the fixed sections,
//! the command-line page carries `kadr --help` word for word, the settings
//! page names every key of `settings.toml`, and no page links to one that
//! isn't there. Pictures exist in docs/images, with both theme variants.
//! Also the roadmap (`docs/ROADMAP.md`) keeps the shape the
//! site's parser needs. The rules are in CLAUDE.md ("Manual").

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manual() -> PathBuf {
    root().join("docs/manual")
}

fn pages() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(manual())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".md") && n != "SUMMARY.md")
        .collect();
    names.sort();
    names
}

fn read(page: &str) -> String {
    fs::read_to_string(manual().join(page)).unwrap_or_else(|e| panic!("{page}: {e}"))
}

/// A page's headings outside code blocks: (level, text).
fn headings(text: &str) -> Vec<(usize, String)> {
    let mut in_code = false;
    let mut out = vec![];
    for line in text.lines() {
        if line.starts_with("```") {
            in_code = !in_code;
        } else if !in_code && line.starts_with('#') {
            let level = line.chars().take_while(|c| *c == '#').count();
            out.push((level, line[level..].trim().to_owned()));
        }
    }
    out
}

/// The command lines of `USAGE`: from the first `kadr` line to the last.
fn usage_commands() -> Vec<&'static str> {
    let lines: Vec<&str> = kadr_core::cli::USAGE.lines().collect();
    // The commands are indented; the title line above them isn't.
    let first = lines.iter().position(|l| l.starts_with("  kadr ")).unwrap();
    let last = lines.iter().rposition(|l| l.starts_with("  kadr ")).unwrap();
    lines[first..=last].to_vec()
}

#[test]
fn the_summary_lists_every_page_and_only_pages() {
    let pages = pages();
    let listed: Vec<String> = read("SUMMARY.md")
        .lines()
        .filter_map(|l| l.split("](").nth(1).map(|r| r.trim_end_matches(')').to_owned()))
        .collect();
    for page in &pages {
        assert!(listed.contains(page), "SUMMARY.md doesn't list {page}");
    }
    for entry in &listed {
        assert!(pages.contains(entry), "SUMMARY.md lists {entry}, which isn't a page");
    }
}

#[test]
fn every_page_starts_with_its_title() {
    for page in pages() {
        let h = headings(&read(&page));
        assert!(h.first().is_some_and(|(level, _)| *level == 1), "{page} doesn't start with a title");
    }
}

/// A screen's page opens with "How to open" and ends with "Keys", then
/// "Settings" where the screen has any, then "Commands".
#[test]
fn a_screen_page_has_the_fixed_sections() {
    const SCREENS: [&str; 7] =
        ["capture.md", "editor.md", "record.md", "studio.md", "history.md", "settings.md", "projects.md"];
    for page in pages() {
        let sections: Vec<String> =
            headings(&read(&page)).into_iter().filter(|(l, _)| *l == 2).map(|(_, t)| t).collect();
        let screen = sections.iter().any(|s| s == "How to open");
        assert_eq!(screen, SCREENS.contains(&page.as_str()), "{page}: a screen's page, and only one, has \"How to open\"");
        if !screen {
            continue;
        }
        assert_eq!(sections.first().map(String::as_str), Some("How to open"), "{page}: \"How to open\" comes first");
        let at = |name: &str| sections.iter().position(|s| s == name);
        let keys = at("Keys").unwrap_or_else(|| panic!("{page} has no \"Keys\""));
        let commands = at("Commands").unwrap_or_else(|| panic!("{page} has no \"Commands\""));
        assert_eq!(commands, sections.len() - 1, "{page}: \"Commands\" comes last");
        match at("Settings") {
            Some(settings) => assert!(keys + 1 == settings && settings + 1 == commands, "{page}: Keys, Settings, Commands go last, in that order"),
            None => assert_eq!(keys + 1, commands, "{page}: Keys, Commands go last, in that order"),
        }
    }
}

#[test]
fn the_command_line_page_carries_the_usage_text() {
    let text = read("cli.md");
    let block: Vec<&str> = text.split("```").nth(1).unwrap_or("").lines().skip(1).collect();
    let block: Vec<&str> = block.iter().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect();
    let usage: Vec<&str> = usage_commands().iter().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect();
    assert_eq!(block, usage, "cli.md's first code block isn't `kadr --help`'s command list");
}

#[test]
fn the_settings_page_names_every_key() {
    let keys = kadr_core::settings::KEYS
        .iter()
        .map(|(k, _, _)| *k)
        .chain(kadr_core::settings::AFTER.iter().map(|(k, _)| *k))
        .chain(kadr_core::settings::AFTER_KEYS.iter().map(|(k, _)| *k))
        .filter(|k| !k.is_empty());
    let text = read("settings.md");
    for key in keys {
        assert!(text.contains(&format!("`{key}`")), "settings.md doesn't name `{key}`");
    }
}

#[test]
fn links_between_pages_lead_somewhere() {
    let pages = pages();
    for page in &pages {
        let text = read(page);
        for (at, _) in text.match_indices("](") {
            let target = text[at + 2..].split(')').next().unwrap();
            let image = text[..at].rfind('[').is_some_and(|start| text[..start].ends_with('!'));
            if image {
                let name = target.strip_prefix("../images/").unwrap_or_else(|| panic!("{page}: image {target} must live in docs/images"));
                assert!(!name.contains('/') && !name.contains('\\'), "{page}: image {target} must be directly in docs/images");
                let image = root().join("docs/images").join(name);
                assert!(image.is_file(), "{page}: image {target} doesn't exist");
                if let Some((stem, ext)) = name.rsplit_once('.')
                    && let Some(screen) = stem.strip_suffix("-light")
                {
                    let dark = image.with_file_name(format!("{screen}-dark.{ext}"));
                    assert!(dark.is_file(), "{page}: image {target} has no dark sibling {}", dark.display());
                }
                continue;
            }
            if target.contains("://") || target.starts_with('#') {
                continue;
            }
            let file = target.split('#').next().unwrap();
            assert!(pages.iter().any(|p| p == file), "{page} links to {target}, which isn't a page");
        }
    }
}

/// The roadmap's `##` sections: (heading, tables), each table its rows
/// below the header and the separator, as trimmed cells.
fn roadmap_sections() -> Vec<(String, Vec<Vec<Vec<String>>>)> {
    let text = fs::read_to_string(root().join("docs/ROADMAP.md")).unwrap();
    let mut sections: Vec<(String, Vec<Vec<Vec<String>>>)> = vec![];
    let mut in_table = false;
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            sections.push((title.trim().to_owned(), vec![]));
            in_table = false;
            continue;
        }
        let Some((_, tables)) = sections.last_mut() else { continue };
        if !line.starts_with('|') {
            in_table = false;
            continue;
        }
        let row = line.replace("\\|", "\u{0}");
        let cells: Vec<String> = row
            .trim()
            .trim_start_matches('|')
            .trim_end_matches('|')
            .split('|')
            .map(|c| c.replace('\u{0}', "|").trim().to_owned())
            .collect();
        if !in_table {
            tables.push(vec![cells]);
            in_table = true;
        } else {
            tables.last_mut().unwrap().push(cells);
        }
    }
    // Drop the separator row; keep the header as row 0.
    for (_, tables) in &mut sections {
        for table in tables.iter_mut() {
            if table.len() > 1 && table[1].iter().all(|c| c.chars().all(|ch| matches!(ch, '-' | ':'))) {
                table.remove(1);
            }
        }
    }
    sections
}

#[test]
fn the_roadmap_keeps_the_shape_the_site_reads() {
    const STATUS: [&str; 5] = ["✅", "🟡", "⬜", "❌", "—"];
    let sections = roadmap_sections();
    let (first, tables) = sections.first().expect("ROADMAP.md has no sections");
    assert!(!first.starts_with(char::is_numeric), "ROADMAP.md: the milestones come before section 1");
    let milestones = tables.first().unwrap_or_else(|| panic!("ROADMAP.md: \"{first}\" has no table"));
    for row in milestones {
        assert_eq!(row.len(), 5, "ROADMAP.md: a milestone row hasn't 5 columns: {row:?}");
    }
    for row in &milestones[1..] {
        assert!(["✅", "🟡", "⬜"].contains(&row[3].as_str()), "ROADMAP.md: milestone {} has no state in column four", row[0]);
    }
    for n in 1..=9 {
        let prefix = format!("{n}. ");
        let (title, tables) = sections
            .iter()
            .find(|(t, _)| t.starts_with(&prefix))
            .unwrap_or_else(|| panic!("ROADMAP.md has no section \"## {n}.\""));
        assert_eq!(tables.len(), 1, "ROADMAP.md: \"{title}\" needs exactly one table");
        let columns = if n <= 7 { 1..4 } else { 1..2 };
        for row in &tables[0][1..] {
            for i in columns.clone() {
                let cell = row.get(i).map(String::as_str).unwrap_or("");
                assert!(STATUS.contains(&cell), "ROADMAP.md: \"{title}\", row \"{}\": column {} is {cell:?}, not a status", row[0], i + 1);
            }
        }
    }
}

/// The languages page lists every file of `data/i18n` with how much of the
/// English it has: "source" for English, a percentage for the rest.
#[test]
fn the_languages_page_says_how_much_is_translated() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/i18n");
    let strings = |name: &str| -> std::collections::BTreeMap<String, String> {
        serde_json::from_str(&fs::read_to_string(dir.join(name)).unwrap()).unwrap()
    };
    let english = strings("en.json");
    let page = fs::read_to_string(manual().join("languages.md")).unwrap();
    for entry in fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let row = page
            .lines()
            .find(|l| l.starts_with('|') && l.contains(&format!("/data/i18n/{name})")))
            .unwrap_or_else(|| panic!("languages.md has no row for {name}"));
        let wanted = if name == "en.json" {
            "source".to_owned()
        } else {
            let theirs = strings(&name);
            let done = english.keys().filter(|k| theirs.get(*k).is_some_and(|t| !t.is_empty())).count();
            format!("{}%", done * 100 / english.len())
        };
        assert!(row.contains(&format!("| {wanted} |")), "languages.md's row for {name} should say \"{wanted}\": {row}");
    }
}
