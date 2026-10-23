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

/// `f` on a thread of its own, waited for (without holding up the UI) at
/// most `limit`: None when it took longer. A system call that hangs (a
/// capture waiting on something) is left to finish there, or not.
pub async fn off_main<T: Send + 'static>(cx: &gpui::AsyncApp, limit: std::time::Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    use futures::FutureExt;
    let (tx, rx) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    let timer = cx.background_executor().timer(limit);
    futures::select! {
        got = rx.fuse() => got.ok(),
        _ = timer.fuse() => None,
    }
}

/// A capture that didn't finish in time: a card saying so, not a hang.
pub fn capture_timed_out(what: &str, cx: &mut gpui::AsyncApp) {
    eprintln!("kadr: {what} timed out");
    let _ = cx.update(|cx| crate::cards::show_error(crate::i18n::t("capture.didnt_finish"), crate::i18n::t("capture.system_took_too_long"), cx));
}

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
