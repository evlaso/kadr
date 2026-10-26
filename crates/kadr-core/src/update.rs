//! Looking for a newer release (SPEC 12.2–12.3), without the network: what
//! `latest.json` says, whether its version is newer than this one, when the
//! tray looks again, and what it remembers of the last look
//! (`cache()/update.json`). The tray fetches; kadr shows the card.

use std::cmp::Ordering;
use std::path::PathBuf;

use serde_json::{Value, json};

/// Where the newest release is described (kadr.sh's `releases/latest.json`
/// once the domain is up; the gh-pages branch until then).
pub const LATEST_URL: &str = "https://raw.githubusercontent.com/evlaso/kadr/gh-pages/releases/latest.json";

/// This build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// At start the tray looks no more often than this…
pub const AT_START: u64 = 60 * 60;
/// …and then once a day.
pub const DAILY: u64 = 24 * 60 * 60;

/// What `latest.json` says of the newest release.
#[derive(Clone, PartialEq, Debug)]
pub struct Latest {
    pub version: String,
    pub date: String,
    /// The release's page, for the card's link.
    pub notes_url: String,
}

impl Latest {
    pub fn parse(json: &[u8]) -> Option<Latest> {
        let v: Value = serde_json::from_slice(json).ok()?;
        let s = |k: &str| v[k].as_str().map(str::to_owned);
        let version = s("version")?;
        parse_version(&version)?;
        Some(Latest { version, date: s("date").unwrap_or_default(), notes_url: s("notes_url")? })
    }
}

/// A version as SemVer: its numbers and its pre-release parts (build
/// metadata dropped).
fn parse_version(v: &str) -> Option<([u64; 3], Vec<&str>)> {
    let v = v.trim().trim_start_matches('v');
    let v = v.split('+').next()?;
    let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p)));
    let mut n = core.split('.').map(|p| p.parse::<u64>().ok());
    let numbers = [n.next()??, n.next()??, n.next()??];
    if n.next().is_some() {
        return None;
    }
    Some((numbers, pre.map(|p| p.split('.').collect()).unwrap_or_default()))
}

/// SemVer's order: a pre-release comes before its release, its parts
/// compared numerically where they're numbers.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (an, ap) = parse_version(a)?;
    let (bn, bp) = parse_version(b)?;
    Some(an.cmp(&bn).then_with(|| match (ap.is_empty(), bp.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            for (x, y) in ap.iter().zip(&bp) {
                let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    _ => x.cmp(y),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
            ap.len().cmp(&bp.len())
        }
    }))
}

/// `candidate` is a later release than `current`.
pub fn newer(candidate: &str, current: &str) -> bool {
    compare(candidate, current) == Some(Ordering::Greater)
}

/// What the tray remembers between looks.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct State {
    /// When it last looked, in seconds since the epoch.
    pub checked: u64,
    /// The newest version it found, and the one it showed a card for (once
    /// per version).
    pub latest: Option<String>,
    pub shown: Option<String>,
}

impl State {
    fn path() -> PathBuf {
        crate::dirs::cache().join("update.json")
    }

    pub fn load() -> State {
        let Some(v) = std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return State::default() };
        let s = |k: &str| v[k].as_str().map(str::to_owned);
        State { checked: v["checked"].as_u64().unwrap_or(0), latest: s("latest"), shown: s("shown") }
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, json!({ "checked": self.checked, "latest": self.latest, "shown": self.shown }).to_string());
    }

    /// Whether to look now (`now` in seconds since the epoch): at start,
    /// when the last look is over an hour old; later, once a day.
    pub fn due(&self, now: u64, at_start: bool) -> bool {
        now.saturating_sub(self.checked) >= if at_start { AT_START } else { DAILY }
    }

    /// Takes what was found at `now`: the release to show a card for, if
    /// it's newer than `current` and its card hasn't been shown.
    pub fn found(&mut self, latest: &Latest, now: u64, current: &str) -> bool {
        self.checked = now;
        self.latest = Some(latest.version.clone());
        let show = newer(&latest.version, current) && self.shown.as_deref() != Some(latest.version.as_str());
        if show {
            self.shown = Some(latest.version.clone());
        }
        show
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_go_in_semver_order() {
        let order = ["0.2.0", "0.3.0-alpha.1", "0.3.0-alpha.2", "0.3.0-alpha.10", "0.3.0-beta", "0.3.0-rc.1", "0.3.0", "0.3.1", "1.0.0"];
        for w in order.windows(2) {
            assert!(newer(w[1], w[0]), "{} after {}", w[1], w[0]);
            assert!(!newer(w[0], w[1]));
        }
        assert!(!newer("0.3.0", "v0.3.0+build.5"));
        assert_eq!(compare("0.3", "0.3.0"), None);
        assert!(parse_version(CURRENT).is_some());
    }

    #[test]
    fn latest_json_is_read() {
        // As the release job writes it (SPEC 12.2).
        let json = br#"{ "version": "0.3.0-alpha.1", "date": "2026-10-24", "notes_url": "https://example.org/r",
            "platforms": { "linux-x86_64": { "url": "https://example.org/a.tar.xz", "sha256": "ab12", "sig": "" } } }"#;
        let latest = Latest::parse(json).unwrap();
        assert_eq!(latest, Latest { version: "0.3.0-alpha.1".into(), date: "2026-10-24".into(), notes_url: "https://example.org/r".into() });
        assert!(Latest::parse(br#"{"version": "soon", "notes_url": "x"}"#).is_none());
    }

    #[test]
    fn a_release_gets_its_card_once_and_looks_are_spaced() {
        let mut s = State::default();
        assert!(s.due(10_000, true));
        let latest = Latest { version: "0.4.0".into(), date: String::new(), notes_url: "u".into() };
        assert!(s.found(&latest, 10_000, "0.3.0"));
        assert!(!s.found(&latest, 20_000, "0.3.0"));
        assert!(!s.due(20_000 + AT_START - 1, true) && s.due(20_000 + AT_START, true));
        assert!(!s.due(20_000 + DAILY - 1, false) && s.due(20_000 + DAILY, false));
        let same = Latest { version: "0.3.0".into(), ..latest };
        assert!(!State::default().found(&same, 0, "0.3.0"));
    }
}
