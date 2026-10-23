//! macOS: what isn't shared with Windows (desktop.rs). Screenshots work;
//! recording isn't ported yet.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::{Signal, home};
use crate::spawn;

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

/// Which shape the pointer has while recording: the system's cursor
/// (NSCursor's current system cursor), told by its hotspot and size,
/// watched on a thread.
pub struct CursorShapes {
    current: std::sync::Arc<std::sync::Mutex<Option<(&'static str, std::time::Instant)>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl CursorShapes {
    pub fn start() -> Option<Self> {
        cursors::snapshot();
        let current = std::sync::Arc::new(std::sync::Mutex::new(None));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (shared, stopped) = (current.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(shape) = cursors::shown() {
                    *shared.lock().unwrap() = Some((shape, std::time::Instant::now()));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
        Some(CursorShapes { current, stop })
    }

    /// The last shape seen, as a `cursors` name, and how many seconds ago.
    pub fn current(&self) -> Option<(&'static str, f64)> {
        self.current.lock().unwrap().map(|(shape, at)| (shape, at.elapsed().as_secs_f64()))
    }
}

impl Drop for CursorShapes {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The system's cursor for a shape, so the drawn one is the real one, only
/// bigger: premultiplied RGBA with the arrow `arrow_height` pixels tall,
/// and its hotspot.
pub fn theme_cursor(shape: &str, arrow_height: f32) -> Option<(image::RgbaImage, (f64, f64))> {
    cursors::snapshot().as_ref()?.0.picture(shape, arrow_height)
}

/// The shape the pointer has now, as a `cursors` name.
pub fn pointer_shape() -> Option<&'static str> {
    cursors::shown()
}

/// Takes the system's cursors once, best on the main thread at start.
pub fn snapshot_cursors() {
    cursors::snapshot();
}

mod cursors {
    use objc2::rc::Retained;
    use objc2_app_kit::NSCursor;
    use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    /// Each shape's hotspot and size in points: how a cursor on screen is
    /// told apart.
    type Known = Vec<(&'static str, (f64, f64), (f64, f64))>;

    /// kadr's shape names and the system's cursors that draw them; macOS has
    /// no wait cursor of its own, nor diagonal ones before macOS 15. (The
    /// edge cursors' newer names need macOS 15.)
    #[allow(deprecated)]
    fn system() -> Vec<(&'static str, Retained<NSCursor>)> {
        vec![
            ("arrow", NSCursor::arrowCursor()),
            ("pointer", NSCursor::pointingHandCursor()),
            ("text", NSCursor::IBeamCursor()),
            ("grab", NSCursor::openHandCursor()),
            ("grabbing", NSCursor::closedHandCursor()),
            ("crosshair", NSCursor::crosshairCursor()),
            ("not-allowed", NSCursor::operationNotAllowedCursor()),
            ("ew-resize", NSCursor::resizeLeftRightCursor()),
            ("ns-resize", NSCursor::resizeUpDownCursor()),
        ]
    }

    pub(super) fn snapshot() -> &'static Option<(crate::desktop::CursorPictures, Known)> {
        static TAKEN: std::sync::OnceLock<Option<(crate::desktop::CursorPictures, Known)>> = std::sync::OnceLock::new();
        TAKEN.get_or_init(|| {
            let mut shapes = Vec::new();
            let mut known = Vec::new();
            for (name, cursor) in system() {
                let size = cursor.image().size();
                let hot = cursor.hotSpot();
                known.push((name, (hot.x, hot.y), (size.width, size.height)));
                if let Some((image, hot)) = picture(&cursor, 4.) {
                    shapes.push((name, image, hot));
                }
            }
            Some((crate::desktop::CursorPictures::new(shapes)?, known))
        })
    }

    /// The cursor on screen now, by kadr's name for it. (Deprecated, but
    /// the one call that sees other apps' cursors without a screen stream.)
    #[allow(deprecated)]
    pub(super) fn shown() -> Option<&'static str> {
        let (_, known) = snapshot().as_ref()?;
        let cursor = NSCursor::currentSystemCursor()?;
        let (size, hot) = (cursor.image().size(), cursor.hotSpot());
        let near = |a: f64, b: f64| (a - b).abs() < 0.01;
        known.iter().find(|(_, (hx, hy), (w, h))| near(hot.x, *hx) && near(hot.y, *hy) && near(size.width, *w) && near(size.height, *h)).map(|(n, _, _)| *n)
    }

    /// A cursor drawn `k` times its size in points, premultiplied RGBA, and
    /// its hotspot there.
    fn picture(cursor: &NSCursor, k: f64) -> Option<(image::RgbaImage, (f64, f64))> {
        let image = cursor.image();
        let size = image.size();
        let (w, h) = ((size.width * k).ceil() as usize, (size.height * k).ceil() as usize);
        if w == 0 || h == 0 {
            return None;
        }
        let mut rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(w as f64, h as f64));
        // SAFETY: a rect of our own; no context or hints.
        let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
        let space = CGColorSpace::new_device_rgb()?;
        let mut data = vec![0u8; w * h * 4];
        // SAFETY: `data` holds w × h four-byte pixels and outlives the context.
        let ctx = unsafe { CGBitmapContextCreate(data.as_mut_ptr().cast(), w, h, 8, w * 4, Some(&space), CGImageAlphaInfo::PremultipliedLast.0) }?;
        CGContext::draw_image(Some(&ctx), NSRect::new(NSPoint::new(0., 0.), NSSize::new(w as f64, h as f64)), Some(&cg));
        drop(ctx);
        let hot = cursor.hotSpot();
        Some((image::RgbaImage::from_raw(w as u32, h as u32, data)?, (hot.x * k, hot.y * k)))
    }
}

/// Windows' corner radius in logical pixels, for window shots.
pub fn window_rounding() -> f32 {
    10.
}

/// File types are registered by the app bundle / installer there, not yet.
pub fn install_desktop() -> std::io::Result<()> {
    Err(std::io::Error::other("not on this system yet"))
}

/// `kadr tray` holds the hotkeys: a LaunchAgent starts it at login.
pub fn ensure_desktop() {
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.display().to_string().replace('&', "&amp;").replace('<', "&lt;");
    let agent = home().join("Library/LaunchAgents/kadr.tray.plist");
    let text = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>kadr.tray</string>
  <key>ProgramArguments</key><array><string>{exe}</string><string>tray</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#
    );
    if std::fs::read_to_string(&agent).ok().as_deref() != Some(text.as_str()) {
        let _ = agent.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(&agent, text);
    }
}

/// The traffic lights over the top bar's left end.
pub fn window_controls() -> super::WindowControls {
    super::WindowControls::Left(76.)
}

