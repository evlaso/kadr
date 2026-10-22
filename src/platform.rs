//! Everything that differs between operating systems, behind one set of
//! functions: the clipboard, notifications, files, folders, monitors, the
//! single-instance socket, overlay windows and screen capture. The rest of
//! kadr only calls these. Linux (Wayland, Hyprland, Omarchy) is complete;
//! macOS and Windows share the clipboard, notifications, the trash, monitors
//! and screenshots through cross-platform crates (desktop.rs), and report
//! what isn't ported yet.

#[cfg(target_os = "linux")]
#[path = "platform/linux.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "platform/macos.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "platform/windows.rs"]
mod imp;

pub use imp::*;

#[cfg(not(target_os = "linux"))]
#[path = "platform/desktop.rs"]
mod desktop;
#[cfg(not(target_os = "linux"))]
pub use desktop::*;

use std::path::PathBuf;

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
    notify("kadr", &tr!("app.linux_only_for_now", what));
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
