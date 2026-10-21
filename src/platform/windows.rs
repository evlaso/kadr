//! Windows: the interface runs; capture and recording aren't ported yet.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gpui::{App, WindowOptions};

use super::{Signal, home};
use crate::util::{Mon, spawn};

pub fn signal(pid: u32, signal: Signal) {
    // No signals here: stopping means ending the process; pausing isn't possible.
    if signal != Signal::Pause {
        spawn(&["taskkill", "/PID", &pid.to_string(), "/T", "/F"]);
    }
}

fn powershell(script: &str) {
    spawn(&["powershell", "-NoProfile", "-WindowStyle", "Hidden", "-Command", script]);
}

pub fn copy_image(path: &Path) {
    powershell(&format!(
        "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard]::SetImage([System.Drawing.Image]::FromFile('{}'))",
        path.display()
    ));
}

pub fn copy_text(text: &str) {
    powershell(&format!("Set-Clipboard -Value '{}'", text.replace('\'', "''")));
}

pub fn copy_file(path: &Path) {
    powershell(&format!("Set-Clipboard -Path '{}'", path.display()));
}

pub fn copy_files(paths: &[&Path]) {
    let list: Vec<String> = paths.iter().map(|p| format!("'{}'", p.display().to_string().replace('\'', "''"))).collect();
    powershell(&format!("Set-Clipboard -Path {}", list.join(",")));
}

pub fn notify(title: &str, body: &str) {
    eprintln!("kadr: {title}: {body}");
}

pub fn open_file(path: &Path) {
    spawn(&[OsStr::new("explorer"), path.as_os_str()]);
}

pub fn device_holder(_device: &str) -> Option<String> {
    None
}

pub fn trash(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData/Local"))
}

pub fn runtime_dir() -> PathBuf {
    std::env::temp_dir()
}

pub fn cache_dir() -> PathBuf {
    local_app_data().join("kadr/cache")
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData/Roaming")).join("kadr")
}

pub fn user_dir(_key: &str, fallback: &str) -> PathBuf {
    home().join(fallback)
}

pub fn hypr(_request: &str) -> Option<String> {
    None
}

pub fn move_cursor(_x: f64, _y: f64) {}

pub fn cuda() -> bool {
    false
}

pub fn low_priority(_cmd: &mut std::process::Command) {}

pub fn cursor_pos() -> Option<(f64, f64)> {
    None
}

pub fn monitors() -> Vec<Mon> {
    Vec::new()
}

pub fn transparent(options: &mut WindowOptions) {
    options.windows_window_background = gpui::WindowsWindowBackground::Transparent;
}

pub fn overlay_kind(spec: &crate::kit::LayerSpec, cx: &App) -> (gpui::WindowKind, gpui::Bounds<gpui::Pixels>) {
    (gpui::WindowKind::PopUp, crate::kit::overlay_bounds(spec, cx))
}


pub fn link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::fs::copy(target, link).map(|_| ())
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

/// Windows' corner radius in logical pixels, for window shots (Windows 11).
pub fn window_rounding() -> f32 {
    8.
}

/// File types are registered by the app bundle / installer there, not yet.
pub fn install_desktop() -> std::io::Result<()> {
    Err(std::io::Error::other("not on this system yet"))
}

pub fn ensure_desktop() {}

/// Three caption buttons at the right, 46 px each, as Windows has them.
pub fn window_controls() -> super::WindowControls {
    super::WindowControls::Right(138.)
}

/// A transparent title bar over kadr's own top bar, or none at all.
pub fn titlebar() -> Option<gpui::TitlebarOptions> {
    None
}
