//! macOS: what isn't shared with Windows (desktop.rs). Screenshots work;
//! recording isn't ported yet.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gpui::{App, WindowOptions};

use super::{Signal, home};
use crate::util::spawn;

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

pub fn open_file(path: &Path) {
    spawn(&[OsStr::new("open"), path.as_os_str()]);
}

pub fn device_holder(_device: &str) -> Option<String> {
    None
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

pub fn cuda() -> bool {
    false
}

pub fn sound_output() -> Option<[&'static str; 3]> {
    None
}

pub fn low_priority(_cmd: &mut std::process::Command) {}

/// The pointer in points, from the top left of the main display.
pub(super) fn pointer() -> Option<(f64, f64)> {
    let event = objc2_core_graphics::CGEvent::new(None)?;
    let p = objc2_core_graphics::CGEvent::location(Some(&*event));
    Some((p.x, p.y))
}

pub(super) fn warp(x: f64, y: f64) {
    let _ = objc2_core_graphics::CGWarpMouseCursorPosition(objc2_core_foundation::CGPoint { x, y });
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
