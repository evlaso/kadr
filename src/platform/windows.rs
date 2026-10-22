//! Windows: what isn't shared with macOS (desktop.rs). Screenshots work;
//! recording isn't ported yet.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gpui::{App, WindowOptions};

use super::{Signal, home};
use crate::util::spawn;

pub fn signal(pid: u32, signal: Signal) {
    // No signals here: stopping means ending the process; pausing isn't possible.
    if signal != Signal::Pause {
        spawn(&["taskkill", "/PID", &pid.to_string(), "/T", "/F"]);
    }
}

pub fn open_file(path: &Path) {
    spawn(&[OsStr::new("explorer"), path.as_os_str()]);
}

pub fn device_holder(_device: &str) -> Option<String> {
    None
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

pub fn cuda() -> bool {
    false
}

pub fn sound_output() -> Option<[&'static str; 3]> {
    None
}

pub fn low_priority(_cmd: &mut std::process::Command) {}

/// The pointer in physical pixels.
pub(super) fn pointer() -> Option<(f64, f64)> {
    let mut p = windows::Win32::Foundation::POINT::default();
    // SAFETY: GetCursorPos writes into the point we pass.
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p) }.ok()?;
    Some((p.x as f64, p.y as f64))
}

pub(super) fn warp(x: f64, y: f64) {
    // SAFETY: plain SetCursorPos.
    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::SetCursorPos(x.round() as i32, y.round() as i32) };
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

/// `kadr tray` holds the hotkeys: the Run key starts it at login, through a
/// headless console so no window shows. Written again only when kadr moved.
pub fn ensure_desktop() {
    let Ok(exe) = std::env::current_exe() else { return };
    let command = format!("conhost.exe --headless \"{}\" tray", exe.display());
    let marker = config_dir().join("autostart");
    if std::fs::read_to_string(&marker).ok().as_deref() == Some(command.as_str()) {
        return;
    }
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    if spawn(&["reg", "add", key, "/v", "kadr", "/t", "REG_SZ", "/d", &command, "/f"]).is_some() {
        let _ = std::fs::create_dir_all(config_dir());
        let _ = std::fs::write(marker, command);
    }
}

/// Three caption buttons at the right, 46 px each, as Windows has them.
pub fn window_controls() -> super::WindowControls {
    super::WindowControls::Right(138.)
}

/// A transparent title bar over kadr's own top bar, or none at all.
pub fn titlebar() -> Option<gpui::TitlebarOptions> {
    None
}
