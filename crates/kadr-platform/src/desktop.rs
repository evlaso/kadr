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

/// The windows showing on `mon`, topmost first, kadr's own ones with a
/// title too (its overlay and cards aside): each with its id (for
/// `grab_window`) and its frame in the monitor's logical pixels.
pub fn windows_on(mon: &Mon) -> Vec<(u64, (f32, f32, f32, f32))> {
    // Windows: in the z-order, from the system itself.
    #[cfg(windows)]
    {
        let s = mon.scale;
        let (ox, oy) = ((mon.x as f64 * s).round(), (mon.y as f64 * s).round());
        return super::imp::top_windows()
            .into_iter()
            .map(|(id, (x, y, w, h))| (id, (((x as f64 - ox) / s) as f32, ((y as f64 - oy) / s) as f32, (w as f64 / s) as f32, (h as f64 / s) as f32)))
            .filter(|(_, (x, y, w, h))| *x < mon.w as f32 && *y < mon.h as f32 && x + w > 0. && y + h > 0.)
            .collect();
    }
    #[cfg(not(windows))]
    macos_windows_on(mon)
}

/// `windows_on` on macOS: xcap's list (CoreGraphics', topmost first).
#[cfg(not(windows))]
fn macos_windows_on(mon: &Mon) -> Vec<(u64, (f32, f32, f32, f32))> {
    // The menu bar, the Dock and such are windows too on macOS.
    const SYSTEM: [&str; 6] = ["Window Server", "Dock", "Control Center", "Notification Center", "SystemUIServer", "Spotlight"];
    let own = std::process::id();
    let Ok(all) = xcap::Window::all() else { return Vec::new() };
    all.into_iter()
        .filter(|w| !w.is_minimized().unwrap_or(true))
        .filter(|w| {
            let title = w.title().unwrap_or_default();
            title != "kadr-picker" && !(title.is_empty() && w.pid().ok() == Some(own))
        })
        .filter(|w| !w.app_name().is_ok_and(|a| SYSTEM.contains(&a.as_str())))
        .filter_map(|w| {
            // In points, as the monitors are.
            let (x, y, width, height) = (w.x().ok()? as f64 - mon.x as f64, w.y().ok()? as f64 - mon.y as f64, w.width().ok()? as f64, w.height().ok()? as f64);
            let on = x < mon.w as f64 && y < mon.h as f64 && x + width > 0. && y + height > 0.;
            let id = w.id().ok()? as u64;
            (on && width >= 40. && height >= 40.).then_some((id, (x as f32, y as f32, width as f32, height as f32)))
        })
        .collect()
}

/// The window at a point of `mon` (its logical pixels): on Windows the
/// system's answer for that point (WindowFromPoint, past kadr's overlay);
/// on macOS the topmost listed window there.
pub fn window_at(mon: &Mon, (x, y): (f32, f32)) -> Option<u64> {
    #[cfg(windows)]
    {
        let s = mon.scale;
        return super::imp::window_under(((mon.x as f64 + x as f64) * s).round() as i32, ((mon.y as f64 + y as f64) * s).round() as i32);
    }
    #[cfg(not(windows))]
    windows_on(mon).into_iter().find(|(_, (wx, wy, w, h))| (*wx..wx + w).contains(&x) && (*wy..wy + h).contains(&y)).map(|(id, _)| id)
}

/// A window by itself, whatever covers it, in physical pixels: macOS draws
/// it alone (its corners transparent), Windows through
/// Windows.Graphics.Capture.
pub fn grab_window(id: u64) -> Option<image::RgbaImage> {
    // Windows: kadr's own Windows.Graphics.Capture frame (xcap's came back
    // empty, or hung with kadr's main thread busy).
    #[cfg(windows)]
    return crate::record::imp::window_shot(id as isize);
    #[cfg(not(windows))]
    let window = xcap::Window::all().ok()?.into_iter().find(|w| w.id().ok() == Some(id as u32))?;
    #[cfg(not(windows))]
    window.capture_image().map_err(|e| eprintln!("kadr: window shot: {e}")).ok()
}

/// One of kadr's own windows as it is on screen, for `kadr dev shot`: by its
/// title, or kadr's only titled one.
pub fn own_window_shot(title: &str, _kind: &str) -> Option<image::RgbaImage> {
    let pid = std::process::id();
    let mine: Vec<xcap::Window> = xcap::Window::all().ok()?.into_iter().filter(|w| w.pid().ok() == Some(pid)).collect();
    let window = mine.iter().find(|w| w.title().ok().as_deref() == Some(title)).or(mine.first())?;
    window.capture_image().ok()
}

/// The system's own cursors as pictures, for the studio and the editor's
/// cursor stamps (macOS and Windows): each shape once at a large size, and
/// its hotspot; drawn at any size by scaling from there.
pub(crate) struct CursorPictures {
    shapes: Vec<(&'static str, image::RgbaImage, (f64, f64))>,
    /// How tall the arrow's visible part is in its picture, in pixels.
    arrow: f32,
}

impl CursorPictures {
    pub(crate) fn new(shapes: Vec<(&'static str, image::RgbaImage, (f64, f64))>) -> Option<CursorPictures> {
        let (_, arrow, _) = shapes.iter().find(|(n, _, _)| *n == "arrow")?;
        // Rows with anything in them: the arrow's own height, its margin aside.
        let rows: Vec<u32> = (0..arrow.height()).filter(|&y| (0..arrow.width()).any(|x| arrow.get_pixel(x, y)[3] > 0)).collect();
        let arrow = (rows.last()? - rows.first()? + 1) as f32;
        Some(CursorPictures { shapes, arrow })
    }

    /// `shape` (the arrow where the system has none like it) with the
    /// arrow `arrow_height` pixels tall: premultiplied RGBA, and its hotspot.
    pub(crate) fn picture(&self, shape: &str, arrow_height: f32) -> Option<(image::RgbaImage, (f64, f64))> {
        let (_, image, (hx, hy)) = self.shapes.iter().find(|(n, _, _)| *n == shape).or_else(|| self.shapes.iter().find(|(n, _, _)| *n == "arrow"))?;
        let k = arrow_height / self.arrow;
        let (w, h) = (((image.width() as f32 * k).round() as u32).max(1), ((image.height() as f32 * k).round() as u32).max(1));
        let picture = image::imageops::resize(image, w, h, image::imageops::FilterType::CatmullRom);
        Some((picture, (hx * k as f64, hy * k as f64)))
    }
}
