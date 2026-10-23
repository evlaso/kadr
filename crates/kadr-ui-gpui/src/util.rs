use std::ffi::OsStr;
use std::path::Path;
use std::process::Stdio;

use gpui::{App, DisplayId};


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

pub use crate::platform::{Mon, Signal, spawn, cache_dir, config_dir, copy_file, copy_files, copy_image, cursor_pos, monitors, notify, open_file, runtime_dir, signal, trash};

pub fn open_folder(path: &Path) {
    if let Some(dir) = path.parent() {
        open_file(dir);
    }
}

pub fn format_duration(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}


/// The GPUI display for a monitor, matched by its position in the layout.
pub fn display_of(mon: &Mon, cx: &App) -> Option<DisplayId> {
    let near = |a: f32, b: i32| (a - b as f32).abs() < 2.0;
    cx.displays()
        .into_iter()
        .find(|d| {
            let o = d.bounds().origin;
            near(o.x.into(), mon.x) && near(o.y.into(), mon.y)
        })
        .map(|d| d.id())
}

pub fn focused_monitor() -> Option<Mon> {
    monitors().into_iter().find(|m| m.focused)
}
