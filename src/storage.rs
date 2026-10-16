//! How much room captures take, and clearing out old ones. Only kadr's own
//! files are counted or removed (screenshot-…, screenrecording-… and what's
//! exported from them), never anything else in those folders.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::settings::Settings;
use crate::util;

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
    /// Click logs, studio projects and previews kadr keeps for recordings.
    pub cache: Size,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.screenshots.bytes + self.recordings.bytes + self.cache.bytes
    }
}

fn is_ours(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    (name.starts_with("screenshot-") || name.starts_with("screenrecording-"))
        && [".png", ".mp4", ".gif", ".webm", ".jpg"].iter().any(|ext| name.ends_with(ext))
}

/// kadr's files in a folder (not its subfolders).
fn ours(dir: &Path) -> Vec<(PathBuf, std::fs::Metadata)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_ours(p))
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
    walk(&util::cache_dir(), &mut u.cache);
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
    // Click logs and studio projects whose recording was deleted.
    let tracks = util::cache_dir().join("tracks");
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
    util::cache_dir().join("last-cleanup")
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
        let _ = std::fs::create_dir_all(util::cache_dir());
        let _ = std::fs::write(stamp(), "");
        clean(&settings, settings.cleanup_days);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_kadr_files_count() {
        assert!(is_ours(Path::new("/x/screenshot-2026-10-16_10-00-00.png")));
        assert!(is_ours(Path::new("/x/screenrecording-2026-10-16_10-00-00-studio.mp4")));
        assert!(!is_ours(Path::new("/x/holiday.png")));
        assert!(!is_ours(Path::new("/x/screenshot-notes.txt")));
    }
}
