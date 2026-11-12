//! The user manual (`docs/manual/{en,ru}`) against the code and against
//! itself: both languages have the same pages with the same shape, the
//! command-line page carries `kadr --help` word for word, the settings page
//! names every key of `settings.toml`, and no page links to one that isn't
//! there. The rules are in CLAUDE.md ("Manual").

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn manual() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/manual")
}

fn pages(lang: &str) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(manual().join(lang))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".md") && n != "SUMMARY.md")
        .collect();
    names.sort();
    names
}

fn read(lang: &str, page: &str) -> String {
    fs::read_to_string(manual().join(lang).join(page)).unwrap_or_else(|e| panic!("{lang}/{page}: {e}"))
}

/// A page's shape: its heading levels in order, how many fenced code
/// blocks and tables it has, and the inline code it shares with the other
/// language (commands and settings keys), which a translation keeps.
#[derive(Debug, PartialEq)]
struct Shape {
    headings: Vec<usize>,
    code_blocks: usize,
    tables: usize,
    shared_code: BTreeSet<String>,
}

fn shape(text: &str) -> Shape {
    let keys: BTreeSet<&str> = kadr_core::settings::KEYS
        .iter()
        .map(|(k, _, _)| *k)
        .chain(kadr_core::settings::AFTER.iter().map(|(k, _)| *k))
        .chain(kadr_core::settings::AFTER_KEYS.iter().map(|(k, _)| *k))
        .filter(|k| !k.is_empty())
        .collect();
    let mut s = Shape { headings: vec![], code_blocks: 0, tables: 0, shared_code: BTreeSet::new() };
    let mut in_code = false;
    let mut in_table = false;
    for line in text.lines() {
        if line.starts_with("```") {
            if !in_code {
                s.code_blocks += 1;
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if line.starts_with('#') {
            s.headings.push(line.chars().take_while(|c| *c == '#').count());
        }
        let table_row = line.starts_with('|');
        if table_row && !in_table {
            s.tables += 1;
        }
        in_table = table_row;
        for span in line.split('`').skip(1).step_by(2) {
            if span.starts_with("kadr ") || keys.contains(span) {
                s.shared_code.insert(span.to_owned());
            }
        }
    }
    s
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
fn both_languages_have_the_same_pages() {
    assert_eq!(pages("en"), pages("ru"));
    for lang in ["en", "ru"] {
        let summary = read(lang, "SUMMARY.md");
        for page in pages(lang) {
            assert!(summary.contains(&format!("({page})")), "{lang}/SUMMARY.md doesn't list {page}");
        }
    }
    let listed = |lang: &str| -> Vec<String> {
        read(lang, "SUMMARY.md")
            .lines()
            .filter_map(|l| l.split("](").nth(1).map(|r| r.trim_end_matches(')').to_owned()))
            .collect()
    };
    assert_eq!(listed("en"), listed("ru"), "the two SUMMARY.md list pages in a different order");
}

#[test]
fn a_page_and_its_translation_have_the_same_shape() {
    for page in pages("en") {
        let (en, ru) = (shape(&read("en", &page)), shape(&read("ru", &page)));
        assert_eq!(en, ru, "en/{page} and ru/{page} differ in shape (headings, code blocks, tables or shared code)");
        assert!(!en.headings.is_empty(), "en/{page} has no headings");
    }
}

#[test]
fn the_command_line_page_carries_the_usage_text() {
    for lang in ["en", "ru"] {
        let text = read(lang, "cli.md");
        let block: Vec<&str> = text.split("```").nth(1).unwrap_or("").lines().skip(1).collect();
        let block: Vec<&str> = block.iter().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect();
        let usage: Vec<&str> = usage_commands().iter().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect();
        assert_eq!(block, usage, "{lang}/cli.md's first code block isn't `kadr --help`'s command list");
    }
}

#[test]
fn the_settings_page_names_every_key() {
    let keys = kadr_core::settings::KEYS
        .iter()
        .map(|(k, _, _)| *k)
        .chain(kadr_core::settings::AFTER.iter().map(|(k, _)| *k))
        .chain(kadr_core::settings::AFTER_KEYS.iter().map(|(k, _)| *k))
        .filter(|k| !k.is_empty());
    for lang in ["en", "ru"] {
        let text = read(lang, "settings.md");
        for key in keys.clone() {
            assert!(text.contains(&format!("`{key}`")), "{lang}/settings.md doesn't name `{key}`");
        }
    }
}

#[test]
fn links_between_pages_lead_somewhere() {
    for lang in ["en", "ru"] {
        let pages = pages(lang);
        for page in &pages {
            let text = read(lang, page);
            for target in text.split("](").skip(1).map(|r| r.split(')').next().unwrap()) {
                if target.contains("://") || target.starts_with('#') {
                    continue;
                }
                let file = target.split('#').next().unwrap();
                assert!(pages.iter().any(|p| p == file), "{lang}/{page} links to {target}, which isn't a page");
            }
        }
    }
}
