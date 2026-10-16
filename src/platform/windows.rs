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

pub fn notify(title: &str, body: &str) {
    eprintln!("kadr: {title}: {body}");
}

pub fn open_file(path: &Path) {
    spawn(&[OsStr::new("explorer"), path.as_os_str()]);
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
    super::unsupported("Снимок экрана");
    None
}
