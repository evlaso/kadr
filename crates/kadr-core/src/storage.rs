//! How much room captures take, and clearing out old ones. Only kadr's own
//! files are counted or removed (screenshot-…, screenrecording-… and what's
//! exported from them), never anything else in those folders.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::dirs;
use crate::settings::Settings;

/// Choices for automatic clearing, in days; 0 keeps everything.
pub const CLEANUP_DAYS: [u32; 4] = [0, 7, 30, 90];

/// Bytes and file count.
#[derive(Clone, Copy, Default)]
pub struct Size {
    pub bytes: u64,
    pub files: usize,
}

#[derive(Clone, Copy, Default)]
pub struct Usage {
    pub screenshots: Size,
    pub recordings: Size,
    /// Click logs, studio projects and previews kadr keeps for recordings,
    /// and what keeps edited screenshots editable.
    pub cache: Size,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.screenshots.bytes + self.recordings.bytes + self.cache.bytes
    }
}

fn is_ours(path: &Path, template: &str) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    [".png", ".mp4", ".gif", ".webm", ".jpg"].iter().any(|ext| name.ends_with(ext)) && is_capture(path, template)
}

/// What a capture is, for its name ({type}) and what happens after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    Screenshot,
    Recording,
}

impl Capture {
    fn word(self) -> &'static str {
        match self {
            Capture::Screenshot => "screenshot",
            Capture::Recording => "screenrecording",
        }
    }
}

/// The names kadr (and Omarchy) have always used: screenshot-2026-10-16_10-00-00.
pub const DEFAULT_NAME: &str = "{type}-{date}_{time}";

/// A name template's pieces: text, or one of the tokens.
enum Part {
    Text(String),
    Date,
    Time,
    Type,
    Number,
}

fn parts(template: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        let token = [("{date}", Part::Date), ("{time}", Part::Time), ("{type}", Part::Type), ("{n}", Part::Number)].into_iter().find(|(t, _)| rest.starts_with(t));
        if let Some((t, part)) = token {
            if !text.is_empty() {
                parts.push(Part::Text(std::mem::take(&mut text)));
            }
            parts.push(part);
            rest = &rest[t.len()..];
        } else {
            text.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    parts
}

/// The template filled in, without the extension. Slashes can't be in a
/// file name; an empty result falls back to the default.
pub fn render_name(template: &str, kind: Capture, now: chrono::DateTime<chrono::Local>, n: u32) -> String {
    let name: String = parts(template)
        .into_iter()
        .map(|p| match p {
            Part::Text(t) => t,
            Part::Date => now.format("%Y-%m-%d").to_string(),
            Part::Time => now.format("%H-%M-%S").to_string(),
            Part::Type => kind.word().to_owned(),
            Part::Number => n.to_string(),
        })
        .collect::<String>()
        .replace(['/', '\\', '\0'], "-");
    let name = name.trim();
    if name.is_empty() { render_name(DEFAULT_NAME, kind, now, n) } else { name.to_owned() }
}

/// Where a new capture goes: named by the settings' template and unique in
/// `dir` ({n} counts up; without it, a number is added on a clash).
pub fn capture_path(dir: &Path, kind: Capture, ext: &str) -> PathBuf {
    let template = Settings::load().name_template;
    let now = chrono::Local::now();
    let counts = template.contains("{n}");
    for n in 1..10_000 {
        let name = if counts || n == 1 { render_name(&template, kind, now, n) } else { format!("{}-{n}", render_name(&template, kind, now, 1)) };
        let path = dir.join(format!("{name}.{ext}"));
        if !path.exists() {
            return path;
        }
    }
    dir.join(format!("{}.{ext}", render_name(DEFAULT_NAME, kind, now, 1)))
}

/// Whether a file is a capture: named by the default template or by
/// `template`, maybe with a suffix kadr adds (-studio, -2…).
pub fn is_capture(path: &Path, template: &str) -> bool {
    let Some(stem) = path.file_stem().and_then(|n| n.to_str()) else { return false };
    let fits = |template: &str| !template.trim().is_empty() && matches(&parts(template), stem);
    fits(DEFAULT_NAME) || fits(template)
}

fn matches(parts: &[Part], s: &str) -> bool {
    // '9' stands for a digit.
    let shaped = |s: &str, shape: &str| s.len() >= shape.len() && s.bytes().zip(shape.bytes()).all(|(c, want)| if want == b'9' { c.is_ascii_digit() } else { c == want });
    match parts.split_first() {
        None => s.is_empty() || s.starts_with(['-', '_', ' ', '(', '.']),
        Some((Part::Text(t), rest)) => s.strip_prefix(t.as_str()).is_some_and(|s| matches(rest, s)),
        Some((Part::Date, rest)) => shaped(s, "9999-99-99") && matches(rest, &s[10..]),
        Some((Part::Time, rest)) => shaped(s, "99-99-99") && matches(rest, &s[8..]),
        Some((Part::Type, rest)) => ["screenshot", "screenrecording"].iter().any(|w| s.strip_prefix(w).is_some_and(|s| matches(rest, s))),
        Some((Part::Number, rest)) => {
            let digits = s.bytes().take_while(u8::is_ascii_digit).count();
            (1..=digits).rev().any(|n| matches(rest, &s[n..]))
        }
    }
}

/// kadr's files in a folder (not its subfolders).
fn ours(dir: &Path) -> Vec<(PathBuf, std::fs::Metadata)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let template = Settings::load().name_template;
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_ours(p, &template))
        .filter_map(|p| Some((p.clone(), std::fs::metadata(&p).ok().filter(|m| m.is_file())?)))
        .collect()
}

fn add(size: &mut Size, meta: &std::fs::Metadata) {
    size.bytes += meta.len();
    size.files += 1;
}

fn walk(dir: &Path, size: &mut Size) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            walk(&entry.path(), size);
        } else {
            add(size, &meta);
        }
    }
}

pub fn usage(settings: &Settings) -> Usage {
    let mut u = Usage::default();
    let mut dirs = vec![settings.screenshot_dir(), settings.video_dir()];
    dirs.dedup();
    for dir in dirs {
        for (path, meta) in ours(&dir) {
            let picture = path.extension().is_some_and(|e| e == "png" || e == "jpg");
            add(if picture { &mut u.screenshots } else { &mut u.recordings }, &meta);
        }
    }
    walk(&dirs::cache(), &mut u.cache);
    walk(&crate::project::root(), &mut u.cache);
    u
}

/// Delete kadr's captures older than `days` (for good: the point is to free
/// space), and cached data of recordings that are gone. Returns what was freed.
pub fn clean(settings: &Settings, days: u32) -> Size {
    let mut freed = Size::default();
    if days > 0 {
        let cutoff = SystemTime::now() - Duration::from_secs(days as u64 * 24 * 3600);
        let mut dirs = vec![settings.screenshot_dir(), settings.video_dir()];
        dirs.dedup();
        for dir in dirs {
            for (path, meta) in ours(&dir) {
                if meta.modified().is_ok_and(|t| t < cutoff) && std::fs::remove_file(&path).is_ok() {
                    add(&mut freed, &meta);
                }
            }
        }
    }
    // Editable drawings of screenshots that were deleted.
    let (bytes, files) = crate::project::prune();
    freed.bytes += bytes;
    freed.files += files;
    // Click logs and studio projects whose recording was deleted.
    let tracks = dirs::cache().join("tracks");
    if let Ok(entries) = std::fs::read_dir(&tracks) {
        let videos = settings.video_dir();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let video = name.split(".mp4").next().unwrap_or_default().to_owned() + ".mp4";
            if !videos.join(&video).exists()
                && let Ok(meta) = entry.metadata()
                && std::fs::remove_file(entry.path()).is_ok()
            {
                add(&mut freed, &meta);
            }
        }
    }
    freed
}

fn stamp() -> PathBuf {
    dirs::cache().join("last-cleanup")
}

/// Clear old captures in the background, at most once a day, when turned on.
pub fn auto() {
    let settings = Settings::load();
    if settings.cleanup_days == 0 {
        return;
    }
    let fresh = std::fs::metadata(stamp()).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|e| e < Duration::from_secs(24 * 3600)));
    if fresh {
        return;
    }
    std::thread::spawn(move || {
        let _ = std::fs::create_dir_all(dirs::cache());
        let _ = std::fs::write(stamp(), "");
        clean(&settings, settings.cleanup_days);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_kadr_files_count() {
        let ours = |p: &str| is_ours(Path::new(p), DEFAULT_NAME);
        assert!(ours("/x/screenshot-2026-10-16_10-00-00.png"));
        assert!(ours("/x/screenrecording-2026-10-16_10-00-00-studio.mp4"));
        assert!(!ours("/x/holiday.png"));
        assert!(!ours("/x/screenshot-notes.txt"));
        assert!(!ours("/x/screenshot-2026-10-16_10-00-00.txt"));
    }

    #[test]
    fn names_follow_the_template() {
        use chrono::TimeZone;
        let now = chrono::Local.with_ymd_and_hms(2026, 10, 17, 9, 5, 3).unwrap();
        assert_eq!(render_name(DEFAULT_NAME, Capture::Screenshot, now, 1), "screenshot-2026-10-17_09-05-03");
        assert_eq!(render_name("kadr {date} {time} #{n}", Capture::Recording, now, 4), "kadr 2026-10-17 09-05-03 #4");
        assert_eq!(render_name("a/b {type}", Capture::Recording, now, 1), "a-b screenrecording");
        assert_eq!(render_name("  ", Capture::Screenshot, now, 1), "screenshot-2026-10-17_09-05-03");
    }

    #[test]
    fn captures_are_recognised_by_their_template() {
        let t = "kadr {date} {time}";
        assert!(is_capture(Path::new("/x/kadr 2026-10-17 09-05-03.png"), t));
        assert!(is_capture(Path::new("/x/kadr 2026-10-17 09-05-03-studio.mp4"), t));
        assert!(is_capture(Path::new("/x/screenshot-2026-10-17_09-05-03.png"), t));
        assert!(!is_capture(Path::new("/x/kadr notes.png"), t));
        assert!(!is_capture(Path::new("/x/holiday.png"), t));
        assert!(is_capture(Path::new("/x/shot 12.png"), "shot {n}"));
        assert!(!is_capture(Path::new("/x/shot x.png"), "shot {n}"));
    }
}
