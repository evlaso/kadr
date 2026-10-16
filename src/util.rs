use std::ffi::OsStr;
use std::path::Path;
use std::process::Stdio;

use gpui::{App, DisplayId};

/// Fire-and-forget process, reaped on a helper thread.
pub fn spawn<S: AsRef<OsStr>>(argv: &[S]) -> Option<u32> {
    let (cmd, args) = argv.split_first()?;
    match std::process::Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).spawn() {
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

/// Run a command to completion without blocking the UI; returns (success, stdout).
pub async fn output<S: AsRef<OsStr>>(argv: &[S]) -> Option<(bool, String)> {
    let (cmd, args) = argv.split_first()?;
    let out = async_process::Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    Some((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned()))
}

pub use crate::platform::{Signal, cache_dir, config_dir, copy_file, copy_image, cursor_pos, hypr, monitors, notify, open_file, runtime_dir, signal, trash, user_dir};

pub fn open_folder(path: &Path) {
    if let Some(dir) = path.parent() {
        open_file(dir);
    }
}

pub fn format_duration(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
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
    pub focused: bool,
}

impl Mon {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// The GPUI display for this monitor, matched by its position in the layout.
    pub fn display(&self, cx: &App) -> Option<DisplayId> {
        let near = |a: f32, b: i32| (a - b as f32).abs() < 2.0;
        cx.displays()
            .into_iter()
            .find(|d| {
                let o = d.bounds().origin;
                near(o.x.into(), self.x) && near(o.y.into(), self.y)
            })
            .map(|d| d.id())
    }
}

pub fn focused_monitor() -> Option<Mon> {
    monitors().into_iter().find(|m| m.focused)
}
