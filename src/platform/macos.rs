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

pub fn copy_file(path: &Path) {
    let script = format!("set the clipboard to (POSIX file \"{}\")", path.display());
    spawn(&["osascript", "-e", &script]);
}

pub fn notify(title: &str, body: &str) {
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    spawn(&["osascript", "-e", &format!("display notification \"{}\" with title \"{}\"", quote(body), quote(title))]);
}

pub fn open_file(path: &Path) {
    spawn(&[OsStr::new("open"), path.as_os_str()]);
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

pub async fn screenshot(_mode: &str, _dir: &Path) -> Option<PathBuf> {
    super::unsupported("Скриншоты");
    None
}

pub fn link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
