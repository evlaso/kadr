//! Translations. The code keeps its Russian text as the key, so it reads as
//! before: `t("Сохранить")` gives the text in the language from the settings.
//! English comes from `data/i18n/en.json` (Russian key → text), compiled
//! in; a missing entry shows the Russian.

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

/// Russian until `set` runs, so tests see the keys themselves.
static CURRENT: AtomicU8 = AtomicU8::new(1);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        0 => Lang::En,
        _ => Lang::Ru,
    }
}

fn english() -> &'static HashMap<String, String> {
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    EN.get_or_init(|| serde_json::from_str(include_str!("../data/i18n/en.json")).unwrap_or_default())
}

/// `ru` in the current language.
pub fn t(ru: &'static str) -> &'static str {
    match current() {
        Lang::Ru => ru,
        Lang::En => english().get(ru).map(String::as_str).unwrap_or(ru),
    }
}

/// A text with `{}` slots, translated, then filled in order.
pub fn fill(ru: &'static str, args: &[String]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut rest = t(ru);
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

/// `tr!("Освобождено {} ({} файлов)", size, count)`: like `format!` with
/// plain `{}` slots, in the current language.
#[macro_export]
macro_rules! tr {
    ($ru:literal $(, $arg:expr)* $(,)?) => {
        $crate::i18n::fill($ru, &[$(($arg).to_string()),*])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_keeps_every_slot() {
        for (ru, text) in english() {
            assert_eq!(ru.matches("{}").count(), text.matches("{}").count(), "slots differ: {ru} → {text}");
        }
    }
}
