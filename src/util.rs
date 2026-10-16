use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
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

pub fn signal(pid: u32, sig: i32) {
    // SAFETY: plain kill(2) on a pid we spawned.
    unsafe {
        libc::kill(pid as i32, sig);
    }
}

pub fn copy_image(path: &Path) {
    // wl-copy keeps serving the clipboard after kadr exits.
    let script = r#"exec wl-copy --type image/png < "$1""#;
    spawn(&[OsStr::new("sh"), "-c".as_ref(), script.as_ref(), "sh".as_ref(), path.as_os_str()]);
}

/// Put a file (not its contents) on the clipboard, for pasting into chats and file managers.
pub fn copy_file(path: &Path) {
    spawn(&["wl-copy", "--type", "text/uri-list", &format!("{}\r\n", file_uri(path))]);
}

pub fn notify(title: &str, body: &str) {
    spawn(&["notify-send", "-a", "kadr", title, body]);
}

pub fn open_file(path: &Path) {
    spawn(&[OsStr::new("xdg-open"), path.as_os_str()]);
}

pub fn open_folder(path: &Path) {
    if let Some(dir) = path.parent() {
        open_file(dir);
    }
}

pub fn trash(path: &Path) {
    if spawn(&[OsStr::new("gio"), "trash".as_ref(), path.as_os_str()]).is_none() {
        let _ = std::fs::remove_file(path);
    }
}

fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for b in path.as_os_str().as_encoded_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => uri.push(*b as char),
            b => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri
}

pub fn format_duration(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn timestamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| "/tmp".into())
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
}

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

pub fn cache_dir() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache").join("kadr")
}

pub fn config_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("kadr")
}

/// XDG user directory such as "VIDEOS", from ~/.config/user-dirs.dirs.
pub fn user_dir(key: &str, fallback: &str) -> PathBuf {
    let dirs = std::fs::read_to_string(xdg("XDG_CONFIG_HOME", ".config").join("user-dirs.dirs")).unwrap_or_default();
    let prefix = format!("XDG_{key}_DIR=");
    dirs.lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .map(|v| v.trim_matches('"').replace("$HOME", &home().to_string_lossy()))
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
}

fn hypr_socket() -> Option<PathBuf> {
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    Some(runtime_dir().join("hypr").join(sig).join(".socket.sock"))
}

/// Query Hyprland over its IPC socket (same requests as `hyprctl`).
pub fn hypr(request: &str) -> Option<String> {
    let mut stream = UnixStream::connect(hypr_socket()?).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut out = String::new();
    stream.read_to_string(&mut out).ok()?;
    Some(out)
}

pub fn cursor_pos() -> Option<(f64, f64)> {
    let out = hypr("cursorpos")?;
    let (x, y) = out.trim().split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
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

pub fn monitors() -> Vec<Mon> {
    let Some(json) = hypr("j/monitors").and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) else {
        return Vec::new();
    };
    json.as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let (pw, ph) = (m["width"].as_i64()?, m["height"].as_i64()?);
            let scale = m["scale"].as_f64().unwrap_or(1.0);
            let rotated = m["transform"].as_i64().unwrap_or(0) % 2 == 1;
            let (lw, lh) = if rotated { (ph, pw) } else { (pw, ph) };
            Some(Mon {
                name: m["name"].as_str()?.to_owned(),
                x: m["x"].as_i64()? as i32,
                y: m["y"].as_i64()? as i32,
                w: (lw as f64 / scale) as i32,
                h: (lh as f64 / scale) as i32,
                phys: (pw, ph),
                scale,
                focused: m["focused"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

pub fn focused_monitor() -> Option<Mon> {
    monitors().into_iter().find(|m| m.focused)
}
