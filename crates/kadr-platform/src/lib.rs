//! Everything that differs between operating systems, behind one set of
//! functions: the clipboard, notifications, files, folders, monitors, the
//! single-instance socket, processes, the pointer and screen capture. The
//! rest of kadr only calls these. No UI framework here: the window options
//! GPUI needs are the app's (`src/platform.rs`). Linux (Wayland, Hyprland, Omarchy) is complete;
//! macOS and Windows share the clipboard, notifications, the trash, monitors
//! and screenshots through cross-platform crates (desktop.rs), and report
//! what isn't ported yet.

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

pub use imp::*;

pub mod dialog;
pub mod keymap;
pub mod menu;
pub mod record;
#[cfg(target_os = "linux")]
mod toplevel;
pub mod style;
pub mod update;

#[cfg(not(target_os = "linux"))]
mod desktop;
#[cfg(not(target_os = "linux"))]
pub use desktop::*;

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Stdio;

/// How to stop or pause a process kadr started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Ask to finish cleanly (a recorder writes out its file).
    Interrupt,
    Terminate,
    Kill,
    /// gpu-screen-recorder's pause toggle.
    Pause,
}

/// Where a window's own controls live, so its top bar can leave room.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub enum WindowControls {
    /// The system draws them at the left (macOS's traffic lights); keep this
    /// much room.
    Left(f32),
    /// Minimise, maximise and close at the right, this wide (Windows); kadr
    /// draws them over the system's hit areas.
    Right(f32),
    /// A floating desktop that leaves kadr's windows bare: kadr draws a close
    /// button at the right.
    Own,
    /// A tiling window manager: no controls, Esc and the manager's keys close.
    None,
}

/// "Not on this system yet", shown once as a notification.
/// Notes a crash in `log`: a Rust panic with where it happened and the
/// stack, and on Windows a crash in native code too (a driver, a codec),
/// which leaves nothing at all otherwise.
pub fn crash_log(log: PathBuf) {
    if let Some(dir) = log.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let previous = std::panic::take_hook();
    let path = log.clone();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("?").to_owned();
        let note = format!("kadr {} panicked on thread {thread}: {info}\n{}\n", env!("CARGO_PKG_VERSION"), std::backtrace::Backtrace::force_capture());
        let _ = std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| std::io::Write::write_all(&mut f, note.as_bytes()));
        previous(info);
    }));
    #[cfg(windows)]
    imp::crash_log(log);
    #[cfg(not(windows))]
    let _ = log;
}

/// On Windows, where kadr is a window app with no console of its own, its
/// output goes to the console it was started from, if any (`kadr --help`
/// in a terminal). Pipes it was given are kept.
pub fn attach_console() {
    #[cfg(windows)]
    imp::attach_console();
}

/// How long ago the system made this process, in ms: with the time kadr
/// itself counts from `main`, what the loader and a virus scan took first.
pub fn process_age_ms() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        // Field 22 of /proc/self/stat, in clock ticks since boot (100 a second).
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let ticks: f64 = stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()?;
        let uptime: f64 = std::fs::read_to_string("/proc/uptime").ok()?.split_whitespace().next()?.parse().ok()?;
        Some(((uptime - ticks / 100.) * 1000.).max(0.) as u64)
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
        let (mut made, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut made, &mut exited, &mut kernel, &mut user) }.ok()?;
        let now = unsafe { GetSystemTimeAsFileTime() };
        Some(ticks(now).saturating_sub(ticks(made)) / 10_000)
    }
    #[cfg(target_os = "macos")]
    None
}

/// Takes the system's frame off an overlay window, where the system gives
/// a borderless window one anyway (Windows: an outline and a shadow).
pub fn bare_window(window: raw_window_handle::RawWindowHandle) {
    #[cfg(windows)]
    if let raw_window_handle::RawWindowHandle::Win32(h) = window {
        imp::bare_window(h.hwnd.get());
    }
    #[cfg(not(windows))]
    let _ = window;
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn unsupported(what: &str) {
    notify("kadr", &kadr_core::tr!("app.linux_only_for_now", what));
}

/// Hand kadr-core the system's folders, before anything reads settings.
pub fn set_dirs() {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")).join("kadr");
    // Omarchy's screenshots and recordings folders where it says.
    let pictures = std::env::var_os("OMARCHY_SCREENSHOT_DIR").map(PathBuf::from).unwrap_or_else(|| user_dir("PICTURES", "Pictures"));
    let videos = std::env::var_os("OMARCHY_SCREENRECORD_DIR").map(PathBuf::from).unwrap_or_else(|| user_dir("VIDEOS", "Videos"));
    kadr_core::dirs::set(kadr_core::dirs::Dirs { config: config_dir(), cache: cache_dir(), data, pictures, videos, runtime: runtime_dir() });
}

/// A folder under the user's home, for fallbacks.
pub fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// The single-instance channel: later `kadr …` calls hand their arguments to
/// the running one. A Unix socket where there is one, localhost elsewhere.
pub mod ipc {
    #[cfg(unix)]
    pub use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};
    #[cfg(windows)]
    pub use std::net::{TcpListener as Listener, TcpStream as Stream};

    #[cfg(unix)]
    fn path() -> std::path::PathBuf {
        super::runtime_dir().join("kadr.sock")
    }

    #[cfg(windows)]
    const ADDRESS: &str = "127.0.0.1:47613";

    pub fn connect() -> Option<Stream> {
        #[cfg(unix)]
        return Stream::connect(path()).ok();
        // Windows retries a refused localhost connection for about two
        // seconds before giving up, which was most of a cold start: only
        // knock where the running kadr's named mutex says one is there, and
        // not for long (it answers well within the limit).
        #[cfg(windows)]
        {
            if !super::imp::instance_running() {
                return None;
            }
            Stream::connect_timeout(&ADDRESS.parse().ok()?, std::time::Duration::from_millis(150)).ok()
        }
    }

    /// Become the running instance; a stale socket from a crash is replaced.
    pub fn bind() -> std::io::Result<Listener> {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(path());
            Listener::bind(path())
        }
        #[cfg(windows)]
        {
            let listener = Listener::bind(ADDRESS)?;
            super::imp::mark_instance();
            Ok(listener)
        }
    }

    /// Stop taking new commands before quitting.
    pub fn close() {
        #[cfg(unix)]
        let _ = std::fs::remove_file(path());
    }
}

/// A plain GET of `url` through the system's curl (every system kadr runs
/// on has one, Windows 10 and later too), so no HTTP stack is built in:
/// the body, or None on any failure.
pub fn fetch(url: &str) -> Option<Vec<u8>> {
    let mut command = std::process::Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    command.args(["-fsSL", "--max-time", "20", "--proto", "=https", url]).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    let out = command.output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// Fire-and-forget process, reaped on a helper thread.
pub fn spawn<S: AsRef<OsStr>>(argv: &[S]) -> Option<u32> {
    let (cmd, args) = argv.split_first()?;
    let mut command = std::process::Command::new(cmd);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::null());
    // No console window flashing up; a kadr started so passes its windowless
    // console on to what it runs.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    match command.spawn() {
        Ok(mut child) => {
            let pid = child.id();
            std::thread::spawn(move || child.wait());
            Some(pid)
        }
        Err(e) => {
            eprintln!("kadr: {:?}: {e}", cmd.as_ref());
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct Mon {
    pub name: String,
    /// Logical layout geometry, as Hyprland and slurp report it.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Physical mode size in pixels.
    pub phys: (i64, i64),
    pub scale: f64,
    /// Refresh rate in Hz.
    pub hz: f64,
    pub focused: bool,
}

impl Mon {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}
