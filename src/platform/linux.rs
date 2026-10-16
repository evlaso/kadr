//! Linux: Wayland with Hyprland and Omarchy's tools.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use gpui::{App, WindowOptions};

use super::{Signal, home};
use crate::util::{Mon, spawn};

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

/// Let the desktop show through the window's unpainted parts.
pub fn transparent(options: &mut WindowOptions) {
    options.linux_window_background = gpui::LinuxWindowBackground::Transparent;
}

/// Overlays are Wayland layer-shell surfaces above every window.
pub fn overlay_kind(spec: &crate::kit::LayerSpec, _cx: &App) -> (gpui::WindowKind, gpui::Bounds<gpui::Pixels>) {
    use gpui::{layer_shell::*, point, px, size};
    let (t, r, b, l) = spec.margin;
    let kind = gpui::WindowKind::LayerShell(LayerShellOptions {
        namespace: spec.namespace.into(),
        layer: Layer::Overlay,
        anchor: spec.anchor,
        margin: Some((px(t), px(r), px(b), px(l))),
        exclusive_zone: spec.ignore_zones.then(|| px(-1.)),
        keyboard_interactivity: spec.keyboard,
        ..Default::default()
    });
    (kind, gpui::Bounds { origin: point(px(0.), px(0.)), size: size(px(spec.size.0), px(spec.size.1)) })
}

/// Take a screenshot with Omarchy's own command, so selection, freezing and
/// saving behave exactly like the stock one. None when cancelled.
pub async fn screenshot(mode: &str, dir: &Path) -> Option<PathBuf> {
    let dir = format!("OMARCHY_SCREENSHOT_DIR={}", dir.display());
    let (_, stdout) = crate::util::output(&["env", &dir, "omarchy-capture-screenshot", mode, "save"]).await?;
    let path = stdout.lines().map(str::trim).rfind(|l| !l.is_empty()).map(PathBuf::from)?;
    path.is_file().then_some(path)
}

/// Link a file (the OCR models share the system's English data).
pub fn link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
