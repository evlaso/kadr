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
pub mod style;

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
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn unsupported(what: &str) {
    notify("kadr", &kadr_core::tr!("app.linux_only_for_now", what));
}

/// Hand kadr-core the system's folders, before anything reads settings.
pub fn set_dirs() {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")).join("kadr");
    // Omarchy's screenshots folder where it says.
    let pictures = std::env::var_os("OMARCHY_SCREENSHOT_DIR").map(PathBuf::from).unwrap_or_else(|| user_dir("PICTURES", "Pictures"));
    kadr_core::dirs::set(kadr_core::dirs::Dirs { config: config_dir(), cache: cache_dir(), data, pictures, videos: user_dir("VIDEOS", "Videos"), runtime: runtime_dir() });
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
        #[cfg(windows)]
        return Stream::connect(ADDRESS).ok();
    }

    /// Become the running instance; a stale socket from a crash is replaced.
    pub fn bind() -> std::io::Result<Listener> {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(path());
            Listener::bind(path())
        }
        #[cfg(windows)]
        Listener::bind(ADDRESS)
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
