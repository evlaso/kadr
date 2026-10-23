//! "Hide sensitive": which recognised words to cover. E-mails, addresses
//! with keys or tokens in them, IP addresses, API keys and other long
//! secrets, and long runs of digits (card and phone numbers).

use crate::ocr::Word;

/// Boxes to hide, (left, top, right, bottom) in the picture's pixels, a
/// little bigger than the words so no edge of a letter shows.
pub(super) fn sensitive(words: &[Word]) -> Vec<(f32, f32, f32, f32)> {
    let mut boxes = Vec::new();
    // Lines in reading order, each word with where it sits in the line.
    let mut lines: Vec<((u32, u32, u32), Vec<&Word>)> = Vec::new();
    for word in words {
        match lines.iter_mut().find(|(key, _)| *key == word.line) {
            Some((_, line)) => line.push(word),
            None => lines.push((word.line, vec![word])),
        }
    }
    for (_, line) in lines {
        let mut text = String::new();
        let mut spans = Vec::new();
        for word in &line {
            if !text.is_empty() {
                text.push(' ');
            }
            let start = text.chars().count();
            text.push_str(&word.text);
            spans.push((start, text.chars().count()));
        }
        // Single words that are secrets by themselves.
        let mut hits: Vec<(usize, usize)> = line.iter().zip(&spans).filter(|(w, _)| secret_word(&w.text)).map(|(_, &s)| s).collect();
        hits.extend(digit_runs(&text));
        for (from, to) in hits {
            let covered: Vec<&&Word> = line.iter().zip(&spans).filter(|(_, s)| s.0 < to && s.1 > from).map(|(w, _)| w).collect();
            if let Some(b) = union(&covered) {
                boxes.push(b);
            }
        }
    }
    boxes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    boxes.dedup();
    boxes
}

fn union(words: &[&&Word]) -> Option<(f32, f32, f32, f32)> {
    let first = words.first()?;
    let (mut l, mut t, mut r, mut b) = (first.left, first.top, first.left + first.width, first.top + first.height);
    for w in &words[1..] {
        (l, t, r, b) = (l.min(w.left), t.min(w.top), r.max(w.left + w.width), b.max(w.top + w.height));
    }
    let pad = ((b - t) * 0.25).max(2.);
    Some((l - pad, t - pad, r + pad, b + pad))
}

/// Punctuation a word may be wrapped in: quotes, brackets, a full stop.
fn trimmed(word: &str) -> &str {
    word.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';' | '.' | '!' | '?' | '«' | '»'))
}

pub(super) fn secret_word(word: &str) -> bool {
    let w = trimmed(word);
    email(w) || ip(w) || secret_url(w) || key(w) || assignment(w)
}

fn email(w: &str) -> bool {
    let w = w.strip_prefix("mailto:").unwrap_or(w);
    let Some((user, domain)) = w.split_once('@') else { return false };
    let label = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '-');
    let parts: Vec<&str> = domain.split('.').collect();
    !user.is_empty()
        && user.chars().all(|c| c.is_alphanumeric() || "._%+-".contains(c))
        && parts.len() >= 2
        && parts.iter().all(|p| label(p))
        && parts.last().is_some_and(|tld| tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic))
}

fn ip(w: &str) -> bool {
    let host = w.rsplit_once(':').filter(|(_, port)| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit())).map_or(w, |(h, _)| h);
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| (1..=3).contains(&p.len()) && p.chars().all(|c| c.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n <= 255))
}

/// Names of things that are secret when given a value.
const SECRET_NAMES: [&str; 12] = ["token", "key", "secret", "password", "passwd", "pwd", "sig", "signature", "auth", "session", "access", "code"];

fn secret_name(name: &str) -> bool {
    let name = name.to_lowercase();
    SECRET_NAMES.iter().any(|s| name == *s || name.ends_with(&format!("_{s}")) || name.ends_with(&format!("-{s}")) || name.ends_with(s) && name.len() > s.len() + 2)
}

/// A web address that carries a key: a secret query parameter, or a path
/// part that looks like one.
fn secret_url(w: &str) -> bool {
    let lower = w.to_lowercase();
    if !(lower.contains("://") || lower.starts_with("www.")) {
        return false;
    }
    let (path, query) = w.split_once('?').unwrap_or((w, ""));
    let in_query = query.split('&').any(|pair| pair.split_once('=').is_some_and(|(name, value)| secret_name(name) && !value.is_empty()));
    let after_host = path.split_once("://").map_or(path, |(_, rest)| rest);
    let in_path = after_host.split('/').skip(1).any(key);
    in_query || in_path
}

/// `NAME=value` or `name: value` with a secret-sounding name.
fn assignment(w: &str) -> bool {
    w.split_once(['=', ':']).is_some_and(|(name, value)| !name.contains("//") && secret_name(name.trim_start_matches('-')) && value.len() >= 6)
}

/// Prefixes of well-known API keys and tokens.
const KEY_PREFIXES: [&str; 20] = [
    "sk-", "sk_live_", "sk_test_", "pk_live_", "pk_test_", "rk_live_", "ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "glpat-", "xoxb-", "xoxp-", "xoxa-", "xoxs-", "AKIA", "AIza", "eyJ",
];

/// Looks like a key: a known prefix, a long hex string, or a long mix of
/// letters and digits.
fn key(w: &str) -> bool {
    let token = |c: char| c.is_ascii_alphanumeric() || "+/=_-.".contains(c);
    if w.len() < 16 || !w.chars().all(token) {
        return false;
    }
    if KEY_PREFIXES.iter().any(|p| w.starts_with(p) && w.len() >= p.len() + 12) {
        return true;
    }
    let hex = w.chars().all(|c| c.is_ascii_hexdigit());
    let digits = w.chars().filter(char::is_ascii_digit).count();
    let letters = w.chars().filter(char::is_ascii_alphabetic).count();
    let upper = w.chars().any(|c| c.is_ascii_uppercase());
    let lower = w.chars().any(|c| c.is_ascii_lowercase());
    (hex && w.len() >= 32 && digits > 0 && letters > 0) || (w.len() >= 24 && !w.contains('.') && digits >= 3 && letters >= 3 && (upper && lower || digits >= 6))
}

/// Character ranges of digit runs (spaces, dashes, brackets and plus signs
/// between them) with ten digits or more: card and phone numbers. A date
/// with a time after it isn't one.
fn digit_runs(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !(chars[i].is_ascii_digit() || chars[i] == '+') {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        let mut digits = 0;
        while i < chars.len() && (chars[i].is_ascii_digit() || " -()+".contains(chars[i])) {
            if chars[i].is_ascii_digit() {
                digits += 1;
                end = i + 1;
            }
            i += 1;
        }
        let run: String = chars[start..end].iter().collect();
        // Glued to letters on either side, it's part of a word.
        let glued = start > 0 && chars[start - 1].is_alphabetic() || end < chars.len() && chars[end].is_alphabetic();
        if digits >= 10 && !glued && !iso_date(&run) {
            runs.push((start, end));
        }
        i = i.max(start + 1);
    }
    runs
}

fn iso_date(run: &str) -> bool {
    let b = run.as_bytes();
    b.len() >= 10 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-' && b[5..7].iter().all(u8::is_ascii_digit) && b[7] == b'-' && b[8..10].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(texts: &[&str]) -> Vec<Word> {
        let mut x = 0.;
        texts
            .iter()
            .map(|t| {
                let w = Word { text: t.to_string(), line: (1, 1, 1), left: x, top: 10., width: t.len() as f32 * 8., height: 20. };
                x += w.width + 8.;
                w
            })
            .collect()
    }

    #[test]
    fn finds_secrets_in_single_words() {
        for secret in [
            "me@example.com",
            "(ivan.petrov+news@mail.co.uk),",
            "192.168.1.20",
            "10.0.0.1:8080",
            "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8",
            "sk-proj-4Fh2kLm9Qx7Rt1Zp8Wv3",
            "AKIAIOSFODNN7EXAMPLE",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abc123def456",
            "https://api.example.com/v1/items?id=4&token=abc123xyz",
            "https://hooks.slack.com/services/T000/B000/XXXXxxxx1234YYYYzzzz5678",
            "API_KEY=4f9a8b7c6d5e",
            "password:hunter2!",
            "3f786850e387550fdab836ed7e6dc881de23001b",
        ] {
            assert!(secret_word(secret), "{secret} should be hidden");
        }
    }

    #[test]
    fn leaves_ordinary_words_alone() {
        for plain in [
            "hello",
            "internationalization",
            "https://example.com/docs/getting-started",
            "v1.2.3",
            "1.2",
            "@username",
            "kadr-editor",
            "screenshot-2026-10-17_23-47-08.png",
            "keyboard",
            "Cargo.toml",
            "a@b",
        ] {
            assert!(!secret_word(plain), "{plain} should stay");
        }
    }

    #[test]
    fn digit_runs_span_words() {
        let words = line(&["Card:", "4111", "1111", "1111", "1111", "exp", "12/28"]);
        let boxes = sensitive(&words);
        assert_eq!(boxes.len(), 1);
        let (l, _, r, _) = boxes[0];
        assert!(l < words[1].left && l > words[0].left, "starts at the first group");
        assert!(r > words[4].left + words[4].width && r < words[5].left + 8., "ends at the last");

        assert_eq!(sensitive(&line(&["Call", "+7", "(999)", "123-45-67"])).len(), 1, "a phone number");
        assert!(sensitive(&line(&["Saved", "2026-10-17", "23:47:08"])).is_empty(), "a date and time");
        assert!(sensitive(&line(&["Saved", "2026-10-17", "23", "47"])).is_empty(), "a date and time read as words");
        assert!(sensitive(&line(&["Order", "#12345"])).is_empty(), "a short number");
    }

    #[test]
    fn words_on_other_lines_dont_join() {
        let mut words = line(&["4111", "1111"]);
        let mut next = line(&["1111", "1111"]);
        next.iter_mut().for_each(|w| w.line = (1, 1, 2));
        words.append(&mut next);
        assert!(sensitive(&words).is_empty());
    }
}
