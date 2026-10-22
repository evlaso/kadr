//! What macOS and Windows share, through cross-platform crates: the
//! clipboard (arboard), notifications (notify-rust), the trash, monitors and
//! screenshots (xcap). Only the pointer is asked of each system directly.
//!
//! Monitors are in logical pixels like on Linux: macOS reports points
//! already, Windows reports physical pixels, divided here by the monitor's
//! scale.

use std::path::{Path, PathBuf};

use crate::Mon;

/// Clipboard work runs off the UI thread: decoding a large PNG takes a while.
fn clipboard(set: impl FnOnce(arboard::Set) -> Result<(), arboard::Error> + Send + 'static) {
    std::thread::spawn(move || {
        if let Err(e) = arboard::Clipboard::new().and_then(|mut c| set(c.set())) {
            eprintln!("kadr: clipboard: {e}");
        }
    });
}

pub fn copy_image(path: &Path) {
    let path = path.to_path_buf();
    clipboard(move |set| {
        let img = image::open(&path).map_err(|_| arboard::Error::ConversionFailure)?.into_rgba8();
        let (width, height) = (img.width() as usize, img.height() as usize);
        set.image(arboard::ImageData { width, height, bytes: img.into_raw().into() })
    });
}

pub fn copy_text(text: &str) {
    let text = text.to_owned();
    clipboard(move |set| set.text(text));
}

/// Put a file (not its contents) on the clipboard, for pasting into chats and file managers.
pub fn copy_file(path: &Path) {
    copy_files(&[path]);
}

pub fn copy_files(paths: &[&Path]) {
    let paths: Vec<PathBuf> = paths.iter().map(|p| p.to_path_buf()).collect();
    clipboard(move |set| set.file_list(&paths));
}

pub fn notify(title: &str, body: &str) {
    if let Err(e) = notify_rust::Notification::new().summary(title).body(body).show() {
        eprintln!("kadr: {title}: {body} ({e})");
    }
}

/// To the system's trash; deleted outright if there is none.
pub fn trash(path: &Path) {
    if trash::delete(path).is_err() {
        let _ = std::fs::remove_file(path);
    }
}

/// xcap's monitor with its scale, so logical and physical pixels convert.
fn screens() -> Vec<(xcap::Monitor, Mon)> {
    let pointer = super::imp::pointer();
    let mut found: Vec<(xcap::Monitor, Mon)> = xcap::Monitor::all()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| {
            let scale = m.scale_factor().ok()? as f64;
            let (x, y, w, h) = (m.x().ok()?, m.y().ok()?, m.width().ok()? as i32, m.height().ok()? as i32);
            // Windows gives physical pixels, macOS points.
            let to_logical = if cfg!(windows) { scale } else { 1. };
            let logical = |v: i32| (v as f64 / to_logical).round() as i32;
            let phys = |v: i32| (v as f64 * scale / to_logical).round() as i64;
            let mon = Mon {
                name: m.name().ok()?,
                x: logical(x),
                y: logical(y),
                w: logical(w),
                h: logical(h),
                phys: (phys(w), phys(h)),
                scale,
                hz: m.frequency().unwrap_or(60.) as f64,
                focused: m.is_primary().unwrap_or(false),
            };
            Some((m, mon))
        })
        .collect();
    // The monitor under the pointer has focus, the primary one otherwise.
    let under = pointer.and_then(|(px, py)| {
        found.iter().position(|(_, mon)| {
            let s = if cfg!(windows) { mon.scale } else { 1. };
            mon.contains((px / s).floor() as i32, (py / s).floor() as i32)
        })
    });
    if let Some(i) = under {
        for (j, (_, mon)) in found.iter_mut().enumerate() {
            mon.focused = i == j;
        }
    }
    found
}

pub fn monitors() -> Vec<Mon> {
    screens().into_iter().map(|(_, mon)| mon).collect()
}

/// The pointer in the monitors' logical layout.
pub fn cursor_pos() -> Option<(f64, f64)> {
    let (x, y) = super::imp::pointer()?;
    if cfg!(windows) {
        // Physical pixels: scale by the monitor they're on.
        let mon = screens().into_iter().map(|(_, mon)| mon).find(|m| {
            let (l, t) = ((m.x as f64 * m.scale) as i32, (m.y as f64 * m.scale) as i32);
            let (r, b) = (l + m.phys.0 as i32, t + m.phys.1 as i32);
            (l..r).contains(&(x as i32)) && (t..b).contains(&(y as i32))
        })?;
        return Some((x / mon.scale, y / mon.scale));
    }
    Some((x, y))
}

pub fn move_cursor(x: f64, y: f64) {
    if cfg!(windows) {
        let Some(mon) = monitors().into_iter().find(|m| m.contains(x.floor() as i32, y.floor() as i32)) else { return };
        return super::imp::warp(x * mon.scale, y * mon.scale);
    }
    super::imp::warp(x, y);
}

pub fn grab_monitor(mon: &Mon) -> Option<image::RgbaImage> {
    // macOS asks for the Screen Recording permission on the first one.
    let (screen, _) = screens().into_iter().find(|(_, m)| m.name == mon.name)?;
    screen.capture_image().map_err(|e| eprintln!("kadr: screenshot: {e}")).ok()
}

/// One of kadr's own windows as it is on screen, for `kadr dev shot`: by its
/// title, or kadr's only titled one.
pub fn own_window_shot(title: &str, _kind: &str) -> Option<image::RgbaImage> {
    let pid = std::process::id();
    let mine: Vec<xcap::Window> = xcap::Window::all().ok()?.into_iter().filter(|w| w.pid().ok() == Some(pid)).collect();
    let window = mine.iter().find(|w| w.title().ok().as_deref() == Some(title)).or(mine.first())?;
    window.capture_image().ok()
}
