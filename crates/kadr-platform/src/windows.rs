//! Windows: what isn't shared with macOS (desktop.rs).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::{Signal, home};
use crate::spawn;

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

pub fn sound_output() -> Option<&'static [&'static str]> {
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

/// Which shape the pointer has while recording: the system's cursor
/// (GetCursorInfo), told by its handle, watched on a thread.
pub struct CursorShapes {
    current: std::sync::Arc<std::sync::Mutex<Option<(&'static str, std::time::Instant)>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl CursorShapes {
    pub fn start() -> Option<Self> {
        let current = std::sync::Arc::new(std::sync::Mutex::new(None));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (shared, stopped) = (current.clone(), stop.clone());
        std::thread::spawn(move || {
            let known = cursors::handles();
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(shape) = cursors::shown().and_then(|h| known.iter().find(|(_, k)| *k == h).map(|(n, _)| *n)) {
                    *shared.lock().unwrap() = Some((shape, std::time::Instant::now()));
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
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

/// The running kadr's mark: a named mutex it holds for as long as it runs.
const INSTANCE: windows::core::PCWSTR = windows::core::w!("Local\\kadr-instance");

/// Marks this process as the running kadr (after binding its socket).
pub(crate) fn mark_instance() {
    use windows::Win32::System::Threading::CreateMutexW;
    // SAFETY: a named mutex; its handle is never closed, so it lasts as
    // long as the process.
    let _ = unsafe { CreateMutexW(None, false, INSTANCE) };
}

/// Whether a kadr marked itself running.
pub(crate) fn instance_running() -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    // SAFETY: opening a named mutex only to see that it's there.
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, INSTANCE) } {
        Ok(handle) => {
            let _ = unsafe { CloseHandle(handle) };
            true
        }
        Err(_) => false,
    }
}

/// The windows on screen, topmost first (the order EnumWindows walks the
/// z-order in): visible, not minimized or cloaked, not kadr's overlay or
/// its untitled cards; each with its handle and its frame (the DWM's,
/// without the invisible resize border) in physical pixels.
pub(crate) fn top_windows() -> Vec<(u64, (i32, i32, i32, i32))> {
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetAncestor, GA_ROOT};
    let mut found: Vec<HWND> = Vec::new();
    unsafe extern "system" fn each(hwnd: HWND, list: LPARAM) -> windows::core::BOOL {
        // SAFETY: `list` is the Vec handed to EnumWindows below.
        unsafe { (*(list.0 as *mut Vec<HWND>)).push(hwnd) };
        true.into()
    }
    // SAFETY: the callback only pushes into `found`, alive for the call.
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut found as *mut _ as isize)) };
    found
        .into_iter()
        // SAFETY: plain queries on handles EnumWindows just gave.
        .filter(|&h| unsafe { GetAncestor(h, GA_ROOT) } == h && takeable(h))
        .filter_map(|h| {
            let r: RECT = frame(h)?;
            (r.right - r.left >= 40 && r.bottom - r.top >= 40).then_some((h.0 as usize as u64, (r.left, r.top, r.right - r.left, r.bottom - r.top)))
        })
        .collect()
}

/// The window under a point (physical pixels): the topmost root window
/// there, through WindowFromPoint and GetAncestor(GA_ROOT), past kadr's
/// own overlay and untitled cards, which it looks under.
pub(crate) fn window_under(x: i32, y: i32) -> Option<u64> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GW_HWNDNEXT, GetAncestor, GetWindow, WindowFromPoint};
    // SAFETY: plain window queries.
    unsafe {
        let mut h = GetAncestor(WindowFromPoint(POINT { x, y }), GA_ROOT);
        // kadr's own: the next ones down the z-order until one has the point.
        for _ in 0..256 {
            if h.is_invalid() {
                return None;
            }
            let inside = frame(h).is_some_and(|r| (r.left..r.right).contains(&x) && (r.top..r.bottom).contains(&y));
            if inside && takeable(h) {
                return Some(h.0 as usize as u64);
            }
            h = GetWindow(h, GW_HWNDNEXT).unwrap_or_default();
        }
        None
    }
}

/// A window to snap to and take: shown, not minimized or cloaked (on another
/// desktop), and not one of kadr's own that aren't windows to a person
/// (the overlay, titled kadr-picker, and kadr's untitled cards).
fn takeable(h: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible};
    // SAFETY: plain queries on a window handle, into buffers of ours.
    unsafe {
        if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
            return false;
        }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4).is_ok() && cloaked != 0 {
            return false;
        }
        let mut text = [0u16; 64];
        let n = GetWindowTextW(h, &mut text).max(0) as usize;
        let title = String::from_utf16_lossy(&text[..n]);
        let mut pid = 0;
        GetWindowThreadProcessId(h, Some(&mut pid));
        title != "kadr-picker" && !(title.is_empty() && pid == std::process::id())
    }
}

/// A window's frame as the DWM draws it, in physical pixels.
fn frame(h: windows::Win32::Foundation::HWND) -> Option<windows::Win32::Foundation::RECT> {
    use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    let mut r = windows::Win32::Foundation::RECT::default();
    // SAFETY: a RECT-sized buffer for the attribute that fills one.
    unsafe { DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut _, std::mem::size_of_val(&r) as u32) }.ok()?;
    Some(r)
}

/// The shape the pointer has now, as a `cursors` name.
pub fn pointer_shape() -> Option<&'static str> {
    let shown = cursors::shown()?;
    cursors::handles().into_iter().find(|(_, h)| *h == shown).map(|(name, _)| name)
}

/// The system's cursor for a shape, so the drawn one is the real one, only
/// bigger: premultiplied RGBA with the arrow `arrow_height` pixels tall,
/// and its hotspot.
pub fn theme_cursor(shape: &str, arrow_height: f32) -> Option<(image::RgbaImage, (f64, f64))> {
    static PICTURES: std::sync::OnceLock<Option<crate::desktop::CursorPictures>> = std::sync::OnceLock::new();
    PICTURES.get_or_init(cursors::pictures).as_ref()?.picture(shape, arrow_height)
}

/// Nothing to take ahead: the system's cursors load on any thread here.
pub fn snapshot_cursors() {}

mod cursors {
    use windows::Win32::Graphics::Gdi::{BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, SelectObject};
    use windows::Win32::UI::WindowsAndMessaging::{
        CURSORINFO, CURSOR_SHOWING, DI_NORMAL, DestroyCursor, DrawIconEx, GetCursorInfo, GetIconInfo, HCURSOR, HICON, ICONINFO, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM,
        IDC_NO, IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, IDC_WAIT, IMAGE_CURSOR, LR_DEFAULTCOLOR, LoadCursorW, LoadImageW,
    };
    use windows::core::PCWSTR;

    /// kadr's shape names and the system cursors that draw them; Windows
    /// has one hand for pointing and grabbing.
    const SHAPES: [(&str, PCWSTR); 13] = [
        ("arrow", IDC_ARROW),
        ("pointer", IDC_HAND),
        ("text", IDC_IBEAM),
        ("grab", IDC_HAND),
        ("grabbing", IDC_HAND),
        ("crosshair", IDC_CROSS),
        ("move", IDC_SIZEALL),
        ("not-allowed", IDC_NO),
        ("wait", IDC_WAIT),
        ("ew-resize", IDC_SIZEWE),
        ("ns-resize", IDC_SIZENS),
        ("nwse-resize", IDC_SIZENWSE),
        ("nesw-resize", IDC_SIZENESW),
    ];

    /// The system cursors' shared handles, by shape name (as numbers, to go
    /// to the watching thread).
    pub(super) fn handles() -> Vec<(&'static str, isize)> {
        // SAFETY: loading the system's own shared cursors.
        SHAPES.iter().filter_map(|&(name, id)| unsafe { LoadCursorW(None, id) }.ok().map(|h| (name, h.0 as isize))).collect()
    }

    /// The cursor on screen now, if it's showing.
    pub(super) fn shown() -> Option<isize> {
        let mut info = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        // SAFETY: a CURSORINFO with its size set, as the call wants.
        unsafe { GetCursorInfo(&mut info) }.ok()?;
        (info.flags.0 & CURSOR_SHOWING.0 != 0).then_some(info.hCursor.0 as isize)
    }

    /// Every shape's picture at a large size.
    pub(super) fn pictures() -> Option<crate::desktop::CursorPictures> {
        let shapes = SHAPES.iter().filter_map(|&(name, id)| picture(id, 128).map(|(image, hot)| (name, image, hot))).collect();
        crate::desktop::CursorPictures::new(shapes)
    }

    /// A system cursor drawn `side` pixels square, premultiplied RGBA, and
    /// its hotspot. Drawn once on black and once on white: what differs is
    /// how see-through each pixel is, which also brings out the cursors
    /// that invert what's under them (the I-beam).
    fn picture(id: PCWSTR, side: i32) -> Option<(image::RgbaImage, (f64, f64))> {
        // SAFETY: GDI calls on objects made here and freed before returning.
        unsafe {
            let cursor = HCURSOR(LoadImageW(None, id, IMAGE_CURSOR, side, side, LR_DEFAULTCOLOR).ok()?.0);
            let icon = HICON(cursor.0);
            let mut info = ICONINFO::default();
            let hot = if GetIconInfo(icon, &mut info).is_ok() {
                if !info.hbmColor.is_invalid() {
                    let _ = DeleteObject(info.hbmColor.into());
                }
                if !info.hbmMask.is_invalid() {
                    let _ = DeleteObject(info.hbmMask.into());
                }
                (info.xHotspot as f64, info.yHotspot as f64)
            } else {
                (0., 0.)
            };
            let dc = CreateCompatibleDC(None);
            let header = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side,
                biHeight: -side,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let bmi = BITMAPINFO { bmiHeader: header, ..Default::default() };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let old = SelectObject(dc, bitmap.into());
            let n = (side * side * 4) as usize;
            let on = |background: u8| {
                let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, n);
                pixels.fill(background);
                let _ = DrawIconEx(dc, 0, 0, icon, side, side, 0, None, DI_NORMAL);
                pixels.to_vec()
            };
            let (black, white) = (on(0), on(255));
            SelectObject(dc, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            let _ = DestroyCursor(cursor);
            let mut rgba = Vec::with_capacity(n);
            for (b, w) in black.chunks_exact(4).zip(white.chunks_exact(4)) {
                // How much of the white came through, per channel, on average.
                let through = ((w[0] as u32 + w[1] as u32 + w[2] as u32).saturating_sub(b[0] as u32 + b[1] as u32 + b[2] as u32)) / 3;
                let alpha = 255u32.saturating_sub(through) as u8;
                // Over black, what's left is the colour times its alpha:
                // premultiplied already. B G R in, R G B out.
                rgba.extend_from_slice(&[b[2].min(alpha), b[1].min(alpha), b[0].min(alpha), alpha]);
            }
            Some((image::RgbaImage::from_raw(side as u32, side as u32, rgba)?, hot))
        }
    }
}

pub(crate) fn attach_console() {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    // SAFETY: plain calls; a handle given by the parent (a pipe) is left be.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|h| !h.is_invalid() && !h.0.is_null()) {
            return;
        }
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Where a crash outside Rust's panics leaves its note (`crash_log`).
static CRASH_LOG: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Notes a crash in native code (a driver, the encoder) in `log`: the
/// exception's code, its address and the module it's in.
pub(crate) fn crash_log(log: PathBuf) {
    use windows::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter;
    let _ = CRASH_LOG.set(log);
    unsafe { SetUnhandledExceptionFilter(Some(crashed)) };
}

unsafe extern "system" fn crashed(info: *const windows::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW};
    // SAFETY: the system hands the exception's record; only read here.
    unsafe {
        if let (Some(log), Some(record)) = (CRASH_LOG.get(), info.as_ref().and_then(|i| i.ExceptionRecord.as_ref())) {
            let at = record.ExceptionAddress;
            let mut module = HMODULE::default();
            let mut name = [0u16; 260];
            let found = GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, windows::core::PCWSTR(at as *const u16), &mut module).is_ok();
            let len = if found { GetModuleFileNameW(Some(module), &mut name) as usize } else { 0 };
            let path = String::from_utf16_lossy(&name[..len]);
            let file = path.rsplit('\\').next().unwrap_or("?");
            let note = format!("kadr {} crashed: exception {:#010x} at {:?} in {} (+{:#x})\n", env!("CARGO_PKG_VERSION"), record.ExceptionCode.0 as u32, at, file, at as usize - module.0 as usize);
            let _ = std::fs::OpenOptions::new().create(true).append(true).open(log).and_then(|mut f| std::io::Write::write_all(&mut f, note.as_bytes()));
        }
    }
    // EXCEPTION_CONTINUE_SEARCH: the system ends the process as before.
    0
}

/// Takes the system's frame off a borderless overlay. GPUI makes its pop-ups
/// plain overlapped windows, which get a caption and a border, so DWM drew an
/// outline and a shadow round the whole window, mostly transparent (the
/// cards' window is as tall as their stack).
pub(crate) fn bare_window(hwnd: isize, (w, h): (f64, f64)) {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongPtrW, GetWindowRect, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_CAPTION, WS_POPUP, WS_THICKFRAME,
    };
    let hw = HWND(hwnd as *mut _);
    unsafe {
        // GPUI made it an ordinary window, with room for a frame added
        // evenly around the size asked for, which stays in it once the
        // frame is off (a 2 px overlay came out 20 or 30 thick): the window
        // keeps its middle and takes the size asked for.
        let mut r = RECT::default();
        let placed = GetWindowRect(hw, &mut r).is_ok();
        let style = GetWindowLongPtrW(hw, GWL_STYLE) as u32;
        SetWindowLongPtrW(hw, GWL_STYLE, ((style & !(WS_CAPTION.0 | WS_THICKFRAME.0)) | WS_POPUP.0) as isize);
        let (w, h) = (w.round() as i32, h.round() as i32);
        let (x, y) = ((r.left + r.right - w) / 2, (r.top + r.bottom - h) / 2);
        let _ = if placed {
            SetWindowPos(hw, None, x, y, w, h, SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOACTIVATE)
        } else {
            use windows::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOSIZE};
            SetWindowPos(hw, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)
        };
        let none = DWMWA_COLOR_NONE;
        let _ = DwmSetWindowAttribute(hw, DWMWA_BORDER_COLOR, &none as *const u32 as *const _, 4);
        let square = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(hw, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as *const _, 4);
    }
}

/// Windows' corner radius in logical pixels, for window shots (Windows 11).
pub fn window_rounding() -> f32 {
    8.
}

/// File types are registered by the app bundle / installer there, not yet.
pub fn install_desktop() -> std::io::Result<()> {
    Err(std::io::Error::other("not on this system yet"))
}

/// The tray holds the hotkeys: the Run key starts it at login (window apps
/// both, so no console shows). Written again only when kadr moved.
pub fn ensure_desktop() {
    let Ok(exe) = std::env::current_exe() else { return };
    // kadr-tray where it's installed beside kadr: a lighter process.
    let tray = exe.with_file_name("kadr-tray.exe");
    let command = if tray.exists() { format!("\"{}\"", tray.display()) } else { format!("\"{}\" tray", exe.display()) };
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

