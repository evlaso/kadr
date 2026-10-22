//! Translations. The code names each text by a key, `t("editor.save")`; the
//! text for each language lives in `data/i18n/<lang>.json` (key → text),
//! compiled in. A key missing in a language falls back to English, then to
//! the key itself.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ru];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    pub fn from_code(code: &str) -> Lang {
        match code {
            "ru" => Lang::Ru,
            _ => Lang::En,
        }
    }

    /// The language's own name, for the settings.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        0 => Lang::En,
        _ => Lang::Ru,
    }
}

fn table(lang: Lang) -> &'static HashMap<String, String> {
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static RU: OnceLock<HashMap<String, String>> = OnceLock::new();
    let (cell, source) = match lang {
        Lang::En => (&EN, include_str!("../../../data/i18n/en.json")),
        Lang::Ru => (&RU, include_str!("../../../data/i18n/ru.json")),
    };
    cell.get_or_init(|| serde_json::from_str(source).unwrap_or_default())
}

/// The text for `key` in the current language.
pub fn t(key: &'static str) -> &'static str {
    table(current()).get(key).or_else(|| table(Lang::En).get(key)).map(String::as_str).unwrap_or(key)
}

/// A text with `{}` slots, in the current language, filled in order.
pub fn fill(key: &'static str, args: &[String]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut rest = t(key);
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        out.push_str(args.next().map(String::as_str).unwrap_or(""));
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// A number as the language writes it: "1,5" in Russian, "1.5" otherwise.
pub fn decimal(number: String) -> String {
    if current() == Lang::Ru { number.replace('.', ",") } else { number }
}

/// `tr!("settings.freed_files", size, count)`: the key's text with its `{}`
/// slots filled, like `format!`.
#[macro_export]
macro_rules! tr {
    ($key:literal $(, $arg:expr)* $(,)?) => {
        $crate::i18n::fill($key, &[$(($arg).to_string()),*])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_every_key_with_the_same_slots() {
        let (en, ru) = (table(Lang::En), table(Lang::Ru));
        let only: Vec<&String> = en.keys().filter(|k| !ru.contains_key(*k)).chain(ru.keys().filter(|k| !en.contains_key(*k))).collect();
        assert!(only.is_empty(), "keys in one language only: {only:?}");
        for (key, text) in en {
            assert_eq!(text.matches("{}").count(), ru[key].matches("{}").count(), "slots differ for {key}");
        }
    }

    #[test]
    fn missing_keys_fall_back() {
        set(Lang::Ru);
        assert_eq!(t("common.save"), "Сохранить");
        assert_eq!(t("no.such.key"), "no.such.key");
        set(Lang::En);
        assert_eq!(t("common.save"), "Save");
    }
}
