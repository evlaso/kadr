//! Looking for a newer release and taking it (SPEC 12.2–12.3), without the
//! network or the files: what `latest.json` says for each channel, whether
//! a version is newer than this one, when the tray looks again and what it
//! remembers of the last look (`cache()/update.json`), a download's
//! signature, and what an update left to undo (`cache()/updates/pending.json`).
//! The tray fetches; kadr shows the card; `kadr_platform::update` puts the
//! files in place.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};

/// Where the newest releases are described (kadr.sh's `releases/latest.json`
/// once the domain is up; the gh-pages branch until then).
pub const LATEST_URL: &str = "https://raw.githubusercontent.com/evlaso/kadr/gh-pages/releases/latest.json";

/// This build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// The minisign key the release job signs every file with.
pub const PUBLIC_KEY: &str = "RWS5ts/3nAnfXq73NHWM1xQE63brvVBUDQgCnRAQLRQi71oKzHs0S6Nn";

/// At start the tray looks no more often than this…
pub const AT_START: u64 = 60 * 60;
/// …and then once a day.
pub const DAILY: u64 = 24 * 60 * 60;

/// Which releases to take: Stable has only versions without a pre-release
/// part; Beta has the alpha, beta and rc ones as well, and a stable one
/// when it's the newest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    Stable,
    Beta,
}

impl Channel {
    pub const ALL: [Channel; 2] = [Channel::Stable, Channel::Beta];

    /// The channel a version comes from.
    pub fn of(version: &str) -> Channel {
        match parse_version(version) {
            Some((_, pre)) if !pre.is_empty() => Channel::Beta,
            _ => Channel::Stable,
        }
    }

    /// The one picked in the settings, else this build's.
    pub fn chosen(setting: Option<Channel>) -> Channel {
        setting.unwrap_or(Channel::of(CURRENT))
    }

    pub fn key(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
        }
    }

    pub fn from_key(key: &str) -> Option<Channel> {
        Channel::ALL.into_iter().find(|c| c.key() == key)
    }
}

/// One file of a release, for one system.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
    /// Its minisign signature (the `.minisig` file's text); empty when the
    /// release wasn't signed, and then it's only shown, not installed.
    pub sig: String,
    pub size: Option<u64>,
}

/// A release as `latest.json` describes it.
#[derive(Clone, PartialEq, Debug)]
pub struct Release {
    pub version: String,
    pub date: String,
    /// The release's page, for the card's link.
    pub notes_url: String,
    /// Its files by system: `linux-x86_64`, `macos-universal`,
    /// `windows-x86_64` (the installer), `windows-x86_64-portable`.
    pub platforms: BTreeMap<String, Asset>,
}

impl Release {
    fn parse(v: &Value) -> Option<Release> {
        let s = |k: &str| v[k].as_str().map(str::to_owned);
        let version = s("version")?;
        parse_version(&version)?;
        let platforms = v["platforms"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, a)| {
                        let s = |key: &str| a[key].as_str().map(str::to_owned);
                        Some((k.clone(), Asset { url: s("url")?, sha256: s("sha256")?, sig: s("sig").unwrap_or_default(), size: a["size"].as_u64() }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Release { version, date: s("date").unwrap_or_default(), notes_url: s("notes_url")?, platforms })
    }
}

/// What `latest.json` says: the newest release on each channel. Its top
/// level is the newest of all (what the trays before channels read);
/// `channels` has each one's.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Latest {
    pub stable: Option<Release>,
    pub beta: Option<Release>,
}

impl Latest {
    pub fn parse(json: &[u8]) -> Option<Latest> {
        let v: Value = serde_json::from_slice(json).ok()?;
        if v["channels"].is_object() {
            let latest = Latest { stable: Release::parse(&v["channels"]["stable"]), beta: Release::parse(&v["channels"]["beta"]) };
            return (latest.stable.is_some() || latest.beta.is_some()).then_some(latest);
        }
        // One release, as written before channels.
        let release = Release::parse(&v)?;
        Some(match Channel::of(&release.version) {
            Channel::Stable => Latest { stable: Some(release.clone()), beta: Some(release) },
            Channel::Beta => Latest { stable: None, beta: Some(release) },
        })
    }

    /// The newest release on `channel`: Beta takes a stable one that's
    /// newer than its last pre-release.
    pub fn on(&self, channel: Channel) -> Option<&Release> {
        match channel {
            Channel::Stable => self.stable.as_ref(),
            Channel::Beta => match (&self.stable, &self.beta) {
                (Some(s), Some(b)) => Some(if newer(&s.version, &b.version) { s } else { b }),
                (s, b) => b.as_ref().or(s.as_ref()),
            },
        }
    }
}

impl Asset {
    /// Whether `data` is this file: its sha256, and the release key's
    /// signature (an unsigned release isn't installed).
    pub fn holds(&self, data: &[u8]) -> bool {
        use sha2::Digest;
        let sum: String = sha2::Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect();
        sum.eq_ignore_ascii_case(self.sha256.trim()) && signed(data, &self.sig)
    }
}

/// Whether `sig` (a `.minisig` file's text) is the release key's signature
/// of `data`.
pub fn signed(data: &[u8], sig: &str) -> bool {
    signed_by(PUBLIC_KEY, data, sig)
}

fn signed_by(key: &str, data: &[u8], sig: &str) -> bool {
    let (Ok(key), Ok(sig)) = (minisign_verify::PublicKey::from_base64(key), minisign_verify::Signature::decode(sig)) else { return false };
    key.verify(data, &sig, false).is_ok()
}

/// What an update left behind until the new version proves it starts:
/// each replaced file and where the old one waits, and how often the new
/// one started without settling (kadr counts at start and settles after a
/// few seconds up; at three it puts the old files back).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Pending {
    pub version: String,
    pub starts: u32,
    /// (the file in use, the previous one kept beside it)
    pub files: Vec<(PathBuf, PathBuf)>,
}

/// What kadr does with a pending update when it starts.
#[derive(PartialEq, Debug)]
pub enum Start {
    /// Nothing pending, or for another version (this one is older: it was
    /// put back, or the update never ran): leave it.
    Nothing,
    /// The new version, starting: counted, settles if it stays up.
    Counted,
    /// The new version failed to settle too often: put the old files back.
    RollBack,
}

/// Starts without settling after which the old files come back.
pub const TRIES: u32 = 3;

impl Pending {
    fn path() -> PathBuf {
        crate::dirs::cache().join("updates").join("pending.json")
    }

    pub fn load() -> Option<Pending> {
        let v: Value = serde_json::from_slice(&std::fs::read(Self::path()).ok()?).ok()?;
        let files = v["files"]
            .as_array()?
            .iter()
            .filter_map(|p| Some((PathBuf::from(p[0].as_str()?), PathBuf::from(p[1].as_str()?))))
            .collect();
        Some(Pending { version: v["version"].as_str()?.to_owned(), starts: v["starts"].as_u64().unwrap_or(0) as u32, files })
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let files: Vec<_> = self.files.iter().map(|(a, b)| json!([a, b])).collect();
        let _ = std::fs::write(path, json!({ "version": self.version, "starts": self.starts, "files": files }).to_string());
    }

    pub fn clear() {
        let _ = std::fs::remove_file(Self::path());
    }

    /// Counts a start of `current`.
    pub fn start(&mut self, current: &str) -> Start {
        if compare(&self.version, current) != Some(Ordering::Equal) {
            return Start::Nothing;
        }
        self.starts += 1;
        if self.starts >= TRIES { Start::RollBack } else { Start::Counted }
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
    pub fn found(&mut self, latest: &Release, now: u64, current: &str) -> bool {
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
        assert!(latest.stable.is_none());
        let beta = latest.on(Channel::Beta).unwrap();
        assert_eq!((beta.version.as_str(), beta.date.as_str(), beta.notes_url.as_str()), ("0.3.0-alpha.1", "2026-10-24", "https://example.org/r"));
        assert_eq!(beta.platforms["linux-x86_64"].sha256, "ab12");
        assert!(Latest::parse(br#"{"version": "soon", "notes_url": "x"}"#).is_none());
    }

    #[test]
    fn a_release_gets_its_card_once_and_looks_are_spaced() {
        let mut s = State::default();
        assert!(s.due(10_000, true));
        let latest = release("0.4.0");
        assert!(s.found(&latest, 10_000, "0.3.0"));
        assert!(!s.found(&latest, 20_000, "0.3.0"));
        assert!(!s.due(20_000 + AT_START - 1, true) && s.due(20_000 + AT_START, true));
        assert!(!s.due(20_000 + DAILY - 1, false) && s.due(20_000 + DAILY, false));
        let same = release("0.3.0");
        assert!(!State::default().found(&same, 0, "0.3.0"));
    }

    fn release(version: &str) -> Release {
        Release { version: version.into(), date: String::new(), notes_url: "u".into(), platforms: BTreeMap::new() }
    }

    #[test]
    fn channels_take_their_releases() {
        let json = br#"{ "version": "0.5.0-beta.1", "notes_url": "b",
            "channels": {
                "stable": { "version": "0.4.0", "notes_url": "s", "platforms": {} },
                "beta": { "version": "0.5.0-beta.1", "notes_url": "b", "platforms": {
                    "windows-x86_64-portable": { "url": "u", "sha256": "ff", "sig": "x", "size": 7 } } } } }"#;
        let latest = Latest::parse(json).unwrap();
        assert_eq!(latest.on(Channel::Stable).unwrap().version, "0.4.0");
        assert_eq!(latest.on(Channel::Beta).unwrap().platforms["windows-x86_64-portable"].size, Some(7));
        // Beta gets a stable release newer than its last pre-release.
        let later = Latest { stable: Some(release("0.5.0")), beta: Some(release("0.5.0-rc.2")) };
        assert_eq!(later.on(Channel::Beta).unwrap().version, "0.5.0");
        assert_eq!(Latest { stable: Some(release("1.0.0")), beta: None }.on(Channel::Beta).unwrap().version, "1.0.0");
        // A stable release written before channels is on both.
        let old = Latest::parse(br#"{"version": "0.4.0", "notes_url": "s"}"#).unwrap();
        assert_eq!(old.on(Channel::Stable).unwrap().version, "0.4.0");
        assert_eq!(old.on(Channel::Beta).unwrap().version, "0.4.0");
    }

    #[test]
    fn a_build_follows_its_own_channel() {
        assert_eq!(Channel::of("0.3.0-alpha.2"), Channel::Beta);
        assert_eq!(Channel::of("0.4.0"), Channel::Stable);
        assert_eq!(Channel::chosen(Some(Channel::Stable)), Channel::Stable);
        assert_eq!(Channel::chosen(None), Channel::of(CURRENT));
        assert_eq!(Channel::from_key(Channel::Beta.key()), Some(Channel::Beta));
    }

    #[test]
    fn only_the_key_s_signature_counts() {
        // A file signed by a key made for this test (rsign, prehashed).
        const KEY: &str = "RWQ7bEEMMYZm83vRqTq9Cp8FmvnMBCER1GnnV/07eYJjBWop9MNBsedl";
        const SIG: &str = "untrusted comment: signature from rsign secret key
RUQ7bEEMMYZm84ba/DtRF4Gowy3DwTVa14fjbpbk/ZD2+fAy2umryFR2Pt3vTINd4yqCGcqiemI/gHkXV+HlvsZhyIZMA5nVxQM=
trusted comment: kadr test
56+eH1WcbcE+i8LdsQq3XdyeCyjmy0YRq04yc/fbir/MJNGFrfKHfnwjiuY9V+p6IxF8fsDpgvBkXIiXwF44AA==
";
        assert!(signed_by(KEY, b"kadr\n", SIG));
        assert!(!signed_by(KEY, b"kadr!\n", SIG));
        assert!(!signed_by(PUBLIC_KEY, b"kadr\n", SIG));
        assert!(!signed_by(KEY, b"kadr\n", ""));
        // The release key didn't sign it: not installed, whatever its sum.
        let asset = Asset { sha256: "2b8a7a6a1e0c0c33b0a5ee6a28d7c2cab4a5b0e9f42c5c4f0f9df6f4c5c4b0a1".into(), sig: SIG.into(), ..Asset::default() };
        assert!(!asset.holds(b"kadr\n"));
    }

    #[test]
    fn a_new_version_that_keeps_failing_is_rolled_back() {
        let mut p = Pending { version: "0.4.0".into(), starts: 0, files: vec![] };
        assert_eq!(p.start("0.3.0"), Start::Nothing);
        assert_eq!(p.start("0.4.0"), Start::Counted);
        assert_eq!(p.start("0.4.0"), Start::Counted);
        assert_eq!(p.start("0.4.0"), Start::RollBack);
    }
}
