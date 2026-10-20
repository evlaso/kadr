//! macOS: the interface runs; capture and recording aren't ported yet.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gpui::{App, WindowOptions};

use super::{Signal, home};
use crate::util::{Mon, spawn};

pub fn signal(pid: u32, signal: Signal) {
    let number = match signal {
        Signal::Interrupt => libc::SIGINT,
        Signal::Terminate => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
        Signal::Pause => libc::SIGUSR2,
    };
    // SAFETY: plain kill(2) on a pid we spawned.
    unsafe {
        libc::kill(pid as i32, number);
    }
}

pub fn copy_image(path: &Path) {
    let script = format!("set the clipboard to (read (POSIX file \"{}\") as «class PNGf»)", path.display());
    spawn(&["osascript", "-e", &script]);
}

pub fn copy_text(text: &str) {
    let script = format!("set the clipboard to \"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
    spawn(&["osascript", "-e", &script]);
}

pub fn copy_file(path: &Path) {
    let script = format!("set the clipboard to (POSIX file \"{}\")", path.display());
    spawn(&["osascript", "-e", &script]);
}

/// Several files: only the first for now.
pub fn copy_files(paths: &[&Path]) {
    if let Some(path) = paths.first() {
        copy_file(path);
    }
}

pub fn notify(title: &str, body: &str) {
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    spawn(&["osascript", "-e", &format!("display notification \"{}\" with title \"{}\"", quote(body), quote(title))]);
}

pub fn open_file(path: &Path) {
    spawn(&[OsStr::new("open"), path.as_os_str()]);
}

pub fn device_holder(_device: &str) -> Option<String> {
    None
}

pub fn trash(path: &Path) {
    let script = format!("tell application \"Finder\" to delete (POSIX file \"{}\")", path.display());
    if spawn(&["osascript", "-e", &script]).is_none() {
        let _ = std::fs::remove_file(path);
    }
}

pub fn runtime_dir() -> PathBuf {
    std::env::temp_dir()
}

pub fn cache_dir() -> PathBuf {
    home().join("Library/Caches/kadr")
}

pub fn config_dir() -> PathBuf {
    home().join("Library/Application Support/kadr")
}

pub fn user_dir(_key: &str, fallback: &str) -> PathBuf {
    home().join(match fallback {
        "Videos" => "Movies",
        other => other,
    })
}

pub fn hypr(_request: &str) -> Option<String> {
    None
}

pub fn move_cursor(_x: f64, _y: f64) {}

pub fn cursor_pos() -> Option<(f64, f64)> {
    None
}

pub fn monitors() -> Vec<Mon> {
    Vec::new()
}

pub fn transparent(options: &mut WindowOptions) {
    options.macos_window_background = gpui::MacosWindowBackground::Transparent;
}

/// Overlays are borderless windows above the others, placed like the
/// Wayland layer would be.
pub fn overlay_kind(spec: &crate::kit::LayerSpec, cx: &App) -> (gpui::WindowKind, gpui::Bounds<gpui::Pixels>) {
    (gpui::WindowKind::PopUp, crate::kit::overlay_bounds(spec, cx))
}


pub fn link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Capture leaves the cursor out on its own here; nothing to switch.
pub struct CursorOffCapture;

impl CursorOffCapture {
    pub fn start() -> Option<Self> {
        None
    }
}

pub fn grab_monitor(_mon: &Mon) -> Option<image::RgbaImage> {
    super::unsupported(crate::i18n::t("app.screenshot"));
    None
}

/// GPUI's own appearance is right here.
pub fn prefers_dark() -> Option<bool> {
    None
}

/// Which shape the pointer has while recording; not ported yet, so the
/// studio draws the arrow throughout.
pub struct CursorShapes;

impl CursorShapes {
    pub fn start() -> Option<Self> {
        None
    }

    pub fn current(&self) -> Option<(&'static str, f64)> {
        None
    }
}

/// The user's own cursor for a shape; not ported yet, so kadr's own are drawn.
pub fn theme_cursor(_shape: &str, _arrow_height: f32) -> Option<(image::RgbaImage, (f64, f64))> {
    None
}

/// Windows' corner radius in logical pixels, for window shots.
pub fn window_rounding() -> f32 {
    10.
}

/// File types are registered by the app bundle / installer there, not yet.
pub fn install_desktop() -> std::io::Result<()> {
    Err(std::io::Error::other("not on this system yet"))
}

pub fn ensure_desktop() {}

/// The traffic lights over the top bar's left end.
pub fn window_controls() -> super::WindowControls {
    super::WindowControls::Left(76.)
}

/// A transparent title bar over kadr's own top bar, or none at all.
pub fn titlebar() -> Option<gpui::TitlebarOptions> {
    Some(gpui::TitlebarOptions { title: None, appears_transparent: true, traffic_light_position: Some(gpui::point(gpui::px(14.), gpui::px(14.))) })
}
