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

pub fn copy_text(text: &str) {
    spawn(&["wl-copy", "--", text]);
}

/// Put a file (not its contents) on the clipboard, for pasting into chats and file managers.
pub fn copy_file(path: &Path) {
    copy_files(&[path]);
}

/// Several files at once, as one list.
pub fn copy_files(paths: &[&Path]) {
    let list: String = paths.iter().map(|p| format!("{}\r\n", file_uri(p))).collect();
    spawn(&["wl-copy", "--type", "text/uri-list", &list]);
}

pub fn notify(title: &str, body: &str) {
    spawn(&["notify-send", "-a", "kadr", title, body]);
}

pub fn open_file(path: &Path) {
    spawn(&[OsStr::new("xdg-open"), path.as_os_str()]);
}

/// The name of a process (other than kadr) that has `device` open, such as
/// the app keeping a camera busy.
pub fn device_holder(device: &str) -> Option<String> {
    let me = std::process::id().to_string();
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|entry| {
        let pid = entry.file_name().into_string().ok()?;
        if !pid.bytes().all(|b| b.is_ascii_digit()) || pid == me {
            return None;
        }
        let holds = std::fs::read_dir(entry.path().join("fd")).ok()?.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|l| l == Path::new(device)));
        holds.then(|| std::fs::read_to_string(entry.path().join("comm")).ok().map(|c| c.trim().to_owned())).flatten()
    })
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
/// Tiling managers (Hyprland, sway, i3, niri, river) want no buttons; other
/// desktops get kadr's own close button, since kadr's windows have no title
/// bar there.
pub fn window_controls() -> super::WindowControls {
    let env = |k: &str| std::env::var_os(k).is_some();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let tiling = env("HYPRLAND_INSTANCE_SIGNATURE") || env("SWAYSOCK") || env("I3SOCK") || env("NIRI_SOCKET")
        || ["hyprland", "sway", "i3", "niri", "river"].iter().any(|t| desktop.contains(t));
    if tiling { super::WindowControls::None } else { super::WindowControls::Own }
}

/// kadr draws its own window chrome.
pub fn titlebar() -> Option<gpui::TitlebarOptions> {
    None
}

pub fn transparent(options: &mut WindowOptions) {
    options.linux_window_background = gpui::LinuxWindowBackground::Transparent;
}

/// Overlays are Wayland layer-shell surfaces above every window.
pub fn overlay_kind(spec: &crate::kit::LayerSpec, _cx: &App) -> (gpui::WindowKind, gpui::Bounds<gpui::Pixels>) {
    use crate::kit::{Anchor as A, KeyboardInteractivity as K};
    use gpui::{layer_shell::*, point, px, size};
    let (t, r, b, l) = spec.margin;
    let mut anchor = Anchor::empty();
    for (ours, theirs) in [(A::TOP, Anchor::TOP), (A::BOTTOM, Anchor::BOTTOM), (A::LEFT, Anchor::LEFT), (A::RIGHT, Anchor::RIGHT)] {
        if spec.anchor.contains(ours) {
            anchor |= theirs;
        }
    }
    let keyboard_interactivity = match spec.keyboard {
        K::None => KeyboardInteractivity::None,
        K::Exclusive => KeyboardInteractivity::Exclusive,
        K::OnDemand => KeyboardInteractivity::OnDemand,
    };
    let kind = gpui::WindowKind::LayerShell(LayerShellOptions {
        namespace: spec.namespace.into(),
        layer: Layer::Overlay,
        anchor,
        margin: Some((px(t), px(r), px(b), px(l))),
        exclusive_zone: spec.ignore_zones.then(|| px(-1.)),
        keyboard_interactivity,
        ..Default::default()
    });
    (kind, gpui::Bounds { origin: point(px(0.), px(0.)), size: size(px(spec.size.0), px(spec.size.1)) })
}


/// Link a file (the OCR models share the system's English data).
pub fn link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// While alive, the compositor draws the cursor on a hardware plane, so screen
/// capture leaves it out (studio recordings draw their own). Hyprland turns
/// hardware cursors off on NVIDIA and then paints the cursor into every frame,
/// where `gpu-screen-recorder -cursor no` can't remove it. The user's setting
/// comes back when this is dropped.
pub struct CursorOffCapture(i64);

impl CursorOffCapture {
    pub fn start() -> Option<Self> {
        let json = hypr("j/getoption cursor:no_hardware_cursors")?;
        let current = serde_json::from_str::<serde_json::Value>(&json).ok()?["int"].as_i64()?;
        if current == 0 {
            return None;
        }
        set_hardware_cursors(0);
        Some(CursorOffCapture(current))
    }
}

impl Drop for CursorOffCapture {
    fn drop(&mut self) {
        set_hardware_cursors(self.0);
    }
}

fn set_hardware_cursors(value: i64) {
    // The Lua config takes `eval`, not `keyword`; wait for it so capture
    // starts with the new setting.
    let script = format!("hl.config({{ cursor = {{ no_hardware_cursors = {value} }} }})");
    let _ = std::process::Command::new("hyprctl").args(["eval", &script]).stdout(std::process::Stdio::null()).status();
}

/// Windows' corner radius in logical pixels (Hyprland's rounding), for
/// window shots.
pub fn window_rounding() -> f32 {
    hypr("j/getoption decoration:rounding")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|v| v["int"].as_f64())
        .unwrap_or(0.) as f32
}

/// The monitor as it is now, in physical pixels (grim, without the cursor).
pub fn grab_monitor(mon: &Mon) -> Option<image::RgbaImage> {
    let out = std::process::Command::new("grim").args(["-o", &mon.name, "-t", "ppm", "-"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    image::load_from_memory_with_format(&out.stdout, image::ImageFormat::Pnm).ok().map(|i| i.to_rgba8())
}

/// The desktop's dark or light preference (the colour scheme the portal and
/// GTK read). GPUI learns it only after its first frames on Wayland.
pub fn prefers_dark() -> Option<bool> {
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "color-scheme"]).output().ok()?;
    let scheme = String::from_utf8_lossy(&out.stdout);
    out.status.success().then(|| scheme.contains("dark"))
}

/// Which shape the pointer has (arrow, pointing hand, I-beam…), watched on
/// a thread while recording. Hyprland doesn't tell, so a small grab around
/// the pointer with the cursor in it (`grim -c`) is matched against the
/// user's XCursor theme.
pub struct CursorShapes {
    current: std::sync::Arc<std::sync::Mutex<Option<(&'static str, std::time::Instant)>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl CursorShapes {
    pub fn start() -> Option<Self> {
        let theme = cursor_theme::Theme::load()?;
        let current = std::sync::Arc::new(std::sync::Mutex::new(None));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (shared, stopped) = (current.clone(), stop.clone());
        std::thread::spawn(move || {
            let mons = monitors();
            let mut templates = std::collections::HashMap::new();
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                let began = std::time::Instant::now();
                if let Some(shape) = cursor_pos().and_then(|(x, y)| cursor_theme::shape_at(&theme, &mons, &mut templates, x, y)) {
                    *shared.lock().unwrap() = Some((shape, began));
                }
                // Two grabs and the match take about 30 ms; ~20 a second keeps
                // up with a hover.
                std::thread::sleep(std::time::Duration::from_millis(50).saturating_sub(began.elapsed()));
            }
        });
        Some(CursorShapes { current, stop })
    }

    /// The last shape seen, as a `cursors` name, and how many seconds ago.
    pub fn current(&self) -> Option<(&'static str, f64)> {
        self.current.lock().unwrap().map(|(shape, at)| (shape, at.elapsed().as_secs_f64()))
    }
}

/// The user's own cursor (XCursor theme) for a shape, so the drawn one
/// looks like the real one, only bigger: premultiplied RGBA with the arrow
/// `arrow_height` pixels tall, and its hotspot.
pub fn theme_cursor(shape: &str, arrow_height: f32) -> Option<(image::RgbaImage, (f64, f64))> {
    static THEME: std::sync::OnceLock<Option<cursor_theme::Theme>> = std::sync::OnceLock::new();
    THEME.get_or_init(cursor_theme::Theme::load).as_ref()?.picture(shape, arrow_height)
}

impl Drop for CursorShapes {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

mod cursor_theme {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{Mon, home};

    /// XCursor names for each of kadr's shapes (`cursors::names`).
    const ALIASES: [(&str, &[&str]); 13] = [
        ("arrow", &["default", "left_ptr", "arrow"]),
        ("pointer", &["pointer", "hand2", "hand1", "pointing_hand"]),
        ("text", &["text", "xterm", "ibeam"]),
        ("grab", &["grab", "openhand", "hand"]),
        ("grabbing", &["grabbing", "closedhand", "dnd-move"]),
        ("crosshair", &["crosshair", "cross", "tcross"]),
        ("move", &["move", "fleur", "all-scroll"]),
        ("not-allowed", &["not-allowed", "forbidden", "crossed_circle", "no-drop"]),
        ("wait", &["wait", "watch", "progress", "left_ptr_watch"]),
        ("ew-resize", &["ew-resize", "col-resize", "sb_h_double_arrow", "h_double_arrow", "e-resize", "w-resize", "right_side", "left_side"]),
        ("ns-resize", &["ns-resize", "row-resize", "sb_v_double_arrow", "v_double_arrow", "n-resize", "s-resize", "top_side", "bottom_side"]),
        ("nwse-resize", &["nwse-resize", "bd_double_arrow", "size_fdiag", "nw-resize", "se-resize", "top_left_corner", "bottom_right_corner"]),
        ("nesw-resize", &["nesw-resize", "fd_double_arrow", "size_bdiag", "ne-resize", "sw-resize", "top_right_corner", "bottom_left_corner"]),
    ];

    /// One picture of a cursor file: nominal size, hotspot, RGBA pixels.
    struct Frame {
        nominal: u32,
        hot: (u32, u32),
        image: image::RgbaImage,
    }

    pub struct Theme {
        size: f64,
        shapes: Vec<(&'static str, Vec<Frame>)>,
    }

    /// A shape's picture at the size it's drawn on one monitor.
    pub struct Template {
        name: &'static str,
        hot: (f64, f64),
        /// Premultiplied, as XCursor stores it.
        image: image::RgbaImage,
        /// Top-left of its solid part.
        solid_origin: (u32, u32),
    }

    impl Theme {
        pub fn load() -> Option<Theme> {
            let name = std::env::var("XCURSOR_THEME").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "default".into());
            let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24.);
            // The theme and the ones it inherits from, in order.
            let mut dirs: Vec<PathBuf> = Vec::new();
            let mut names = vec![name];
            while let Some(name) = names.first().cloned() {
                names.remove(0);
                if let Some(dir) = theme_dir(&name).filter(|d| !dirs.contains(d)) {
                    names.extend(inherits(&dir));
                    dirs.push(dir);
                }
                if dirs.len() > 5 {
                    break;
                }
            }
            let shapes: Vec<_> = ALIASES
                .iter()
                .filter_map(|&(shape, names)| Some((shape, dirs.iter().find_map(|dir| names.iter().find_map(|n| read(&dir.join("cursors").join(n))))?)))
                .collect();
            shapes.iter().any(|s| s.0 == "arrow").then_some(Theme { size, shapes })
        }

        /// The largest picture of `shape`, scaled so the arrow's solid part
        /// is `arrow_height` tall.
        pub fn picture(&self, shape: &str, arrow_height: f32) -> Option<(image::RgbaImage, (f64, f64))> {
            let largest = |name: &str| self.shapes.iter().find(|s| s.0 == name)?.1.iter().max_by_key(|f| f.nominal);
            let arrow = largest("arrow")?;
            let rows: Vec<u32> = arrow.image.enumerate_pixels().filter(|p| p.2[3] > 128).map(|p| p.1).collect();
            let solid = (rows.iter().max()? - rows.iter().min()? + 1) as f32;
            let f = largest(shape)?;
            let k = arrow_height / solid;
            let (w, h) = ((f.image.width() as f32 * k).round().max(1.) as u32, (f.image.height() as f32 * k).round().max(1.) as u32);
            let mut image = image::imageops::resize(&f.image, w, h, image::imageops::FilterType::CatmullRom);
            // Premultiplied colour can't exceed its alpha; resampling may overshoot.
            for p in image.pixels_mut() {
                let a = p[3];
                p.0[..3].iter_mut().for_each(|c| *c = (*c).min(a));
            }
            Some((image, (f.hot.0 as f64 * k as f64, f.hot.1 as f64 * k as f64)))
        }

        /// The pictures at `scale`, sized like the compositor draws them.
        fn templates(&self, scale: f64) -> Vec<Template> {
            let want = self.size * scale;
            self.shapes
                .iter()
                .filter_map(|(name, frames)| {
                    let f = frames.iter().min_by_key(|f| (f.nominal as f64 - want).abs() as u32)?;
                    let k = want / f.nominal as f64;
                    let (w, h) = (((f.image.width() as f64) * k).round().max(1.) as u32, ((f.image.height() as f64) * k).round().max(1.) as u32);
                    let image = image::imageops::resize(&f.image, w, h, image::imageops::FilterType::Triangle);
                    let solid = image.enumerate_pixels().filter(|p| p.2[3] > 40);
                    let solid_origin = solid.fold((u32::MAX, u32::MAX), |o, (x, y, _)| (o.0.min(x), o.1.min(y)));
                    Some(Template { name, hot: (f.hot.0 as f64 * k, f.hot.1 as f64 * k), image, solid_origin })
                })
                .collect()
        }
    }

    /// The shape under the pointer at (x, y) in layout coordinates, if one
    /// of the theme's pictures is there.
    pub fn shape_at(theme: &Theme, mons: &[Mon], cache: &mut HashMap<u64, Vec<Template>>, x: f64, y: f64) -> Option<&'static str> {
        let mon = mons.iter().find(|m| x >= m.x as f64 && y >= m.y as f64 && x < (m.x + m.w) as f64 && y < (m.y + m.h) as f64)?;
        // The compositor draws the cursor somewhere between the theme size
        // and that times the monitor's scale (Hyprland: about 1.2× at 1.25):
        // try a few sizes in that range.
        let templates = cache.entry(mon.scale.to_bits()).or_insert_with(|| {
            let top = mon.scale.max(1.) * 1.05;
            let mut factors = vec![1.];
            while factors.last().is_some_and(|f| f * 1.06 < top) {
                factors.push(factors.last().unwrap() * 1.06);
            }
            factors.push(top);
            factors.into_iter().flat_map(|f| theme.templates(f)).collect()
        });
        // A box around the pointer, inside the monitor, with the cursor in it
        // and without. While recording in studio mode the cursor is a
        // hardware one, so the second grab leaves it out and the difference
        // is the cursor alone.
        let reach = (theme.size * mon.scale.max(1.) * 1.6).ceil();
        let (x0, y0) = ((x - reach).max(mon.x as f64).floor(), (y - reach).max(mon.y as f64).floor());
        let (x1, y1) = ((x + reach).min((mon.x + mon.w) as f64).ceil(), (y + reach).min((mon.y + mon.h) as f64).ceil());
        let region = format!("{},{} {}x{}", x0, y0, x1 - x0, y1 - y0);
        let grab = |cursor: bool| {
            let mut cmd = std::process::Command::new("grim");
            if cursor {
                cmd.arg("-c");
            }
            let out = cmd.args(["-g", &region, "-t", "ppm", "-"]).output().ok()?;
            image::load_from_memory_with_format(&out.stdout, image::ImageFormat::Pnm).ok().map(|i| i.to_rgb8())
        };
        let with = grab(true)?;
        let k = with.width() as f64 / (x1 - x0);
        let at = ((x - x0) * k, (y - y0) * k);
        if let Some(without) = grab(false).filter(|g| g.dimensions() == with.dimensions()) {
            if let Some(found) = silhouette(templates, &with, &without, at, theme.size * mon.scale.max(1.)) {
                return found;
            }
        }
        classify(templates, &with, at)
    }

    /// The shape found from a grab with the cursor and one without:
    /// Some(None) when they differ but no template fits, None when they
    /// don't differ (a software cursor is in both, or none at all).
    fn silhouette(templates: &[Template], with: &image::RgbImage, without: &image::RgbImage, at: (f64, f64), size: f64) -> Option<Option<&'static str>> {
        let (w, h) = with.dimensions();
        // Where the cursor is: the box of the pixels that changed near the
        // pointer (the screen may change elsewhere between the two grabs).
        let near = |x: u32, y: u32| (x as f64 - at.0).abs() < size && (y as f64 - at.1).abs() < size;
        let (mut l, mut t, mut r, mut b, mut n) = (u32::MAX, u32::MAX, 0, 0, 0);
        for (x, y, p) in with.enumerate_pixels().filter(|(x, y, _)| near(*x, *y)) {
            let q = without.get_pixel(x, y);
            if (0..3).map(|c| (p[c] as i32 - q[c] as i32).abs()).sum::<i32>() > 40 {
                (l, t, r, b, n) = (l.min(x), t.min(y), r.max(x), b.max(y), n + 1);
            }
        }
        if n < 10 {
            return None;
        }
        // Each template drawn over the grab without the cursor, at about
        // that box, against the grab with it.
        let error = |tp: &Template| {
            let (bl, bt) = tp.solid_origin;
            let mut best = f64::MAX;
            for dy in -3i64..=3 {
                for dx in -3i64..=3 {
                    let (ox, oy) = (l as i64 - bl as i64 + dx, t as i64 - bt as i64 + dy);
                    let (mut sum, mut count) = (0u64, 0u64);
                    for (tx, ty, p) in tp.image.enumerate_pixels() {
                        if p[3] == 0 {
                            continue;
                        }
                        let (gx, gy) = (ox + tx as i64, oy + ty as i64);
                        if gx < 0 || gy < 0 || gx >= w as i64 || gy >= h as i64 {
                            sum += 255 * 3;
                            count += 1;
                            continue;
                        }
                        let (g, bg) = (with.get_pixel(gx as u32, gy as u32), without.get_pixel(gx as u32, gy as u32));
                        // Template pixels are premultiplied.
                        let a = p[3] as u32;
                        for c in 0..3 {
                            let predicted = p[c] as u32 + bg[c] as u32 * (255 - a) / 255;
                            sum += (predicted as i64 - g[c] as i64).unsigned_abs();
                        }
                        count += 1;
                    }
                    if count > 0 {
                        best = best.min(sum as f64 / (count * 3) as f64);
                    }
                }
            }
            best
        };
        let mut scored: Vec<(f64, &'static str)> = templates.iter().map(|tp| (error(tp), tp.name)).collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        let best = scored.first()?;
        let runner_up = scored.iter().find(|s| s.1 != best.1).map_or(f64::MAX, |s| s.0);
        // Thin outlines resampled never match exactly; the wrong shapes are
        // much further off.
        Some((best.0 < 45. && runner_up > best.0 * 1.3).then_some(best.1))
    }

    const SLACK: i64 = 8;

    /// Which template sits with its hotspot near `at` in the grab.
    fn classify(templates: &[Template], grab: &image::RgbImage, at: (f64, f64)) -> Option<&'static str> {
        let score = |t: &Template| {
            let mut best = f64::MAX;
            // Wide: the pointer may move a little between asking where it is
            // and the grab.
            for dy in -SLACK..=SLACK {
                for dx in -SLACK..=SLACK {
                    let left = (at.0 - t.hot.0).round() as i64 + dx;
                    let top = (at.1 - t.hot.1).round() as i64 + dy;
                    best = best.min(mismatch(grab, &t.image, left, top).unwrap_or(f64::MAX));
                }
            }
            best
        };
        let mut scored: Vec<(f64, &'static str)> = templates.iter().map(|t| (score(t), t.name)).collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Thin outlines make even the right picture differ a fair bit, but
        // the wrong ones differ far more.
        let best = scored.first()?;
        let runner_up = scored.iter().find(|s| s.1 != best.1).map_or(f64::MAX, |s| s.0);
        (best.0 < 60. && runner_up > best.0 * 1.3).then_some(best.1)
    }

    /// Mean colour difference over the template's solid pixels placed at
    /// (left, top); None when too little of it is in the grab.
    fn mismatch(grab: &image::RgbImage, t: &image::RgbaImage, left: i64, top: i64) -> Option<f64> {
        let (mut sum, mut n, mut outside) = (0u64, 0u64, 0u64);
        for (tx, ty, p) in t.enumerate_pixels() {
            if p[3] < 250 {
                continue;
            }
            let (gx, gy) = (left + tx as i64, top + ty as i64);
            if gx < 0 || gy < 0 || gx >= grab.width() as i64 || gy >= grab.height() as i64 {
                outside += 1;
                continue;
            }
            let g = grab.get_pixel(gx as u32, gy as u32);
            sum += (0..3).map(|c| (g[c] as i64 - p[c] as i64).unsigned_abs()).sum::<u64>();
            n += 1;
        }
        (n > 20 && outside * 4 < n).then(|| sum as f64 / (n * 3) as f64)
    }

    fn theme_dir(name: &str) -> Option<PathBuf> {
        let mut dirs = vec![home().join(".local/share/icons"), home().join(".icons")];
        let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        dirs.extend(data.split(':').filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("icons")));
        dirs.push("/usr/share/pixmaps".into());
        dirs.into_iter().map(|d| d.join(name)).find(|d| d.join("cursors").is_dir() || d.join("index.theme").is_file())
    }

    fn inherits(dir: &std::path::Path) -> Vec<String> {
        let text = std::fs::read_to_string(dir.join("index.theme")).unwrap_or_default();
        text.lines()
            .find_map(|l| l.trim().strip_prefix("Inherits")?.trim_start().strip_prefix('='))
            .map(|v| v.split([',', ';']).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default()
    }

    /// The first frame of each size in an XCursor file.
    fn read(path: &std::path::Path) -> Option<Vec<Frame>> {
        let d = std::fs::read(path).ok()?;
        let u = |at: usize| d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        if d.get(0..4)? != b"Xcur" {
            return None;
        }
        let count = u(12)? as usize;
        let mut frames: Vec<Frame> = Vec::new();
        for i in 0..count.min(256) {
            let (kind, nominal, pos) = (u(16 + i * 12)?, u(20 + i * 12)?, u(24 + i * 12)? as usize);
            if kind != 0xfffd_0002 || frames.iter().any(|f| f.nominal == nominal) {
                continue;
            }
            let (w, h, xh, yh) = (u(pos + 16)?, u(pos + 20)?, u(pos + 24)?, u(pos + 28)?);
            let start = pos + 36;
            let bytes = d.get(start..start + (w * h * 4) as usize)?;
            // Little-endian ARGB, premultiplied; only opaque pixels are compared.
            let rgba: Vec<u8> = bytes.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
            frames.push(Frame { nominal, hot: (xh, yh), image: image::RgbaImage::from_raw(w, h, rgba)? });
        }
        (!frames.is_empty()).then_some(frames)
    }

    #[cfg(test)]
    mod tests {
        /// Prints the shape under the pointer now: `cargo test -- --ignored live_shape --nocapture`.
        #[test]
        #[ignore]
        fn live_shape() {
            let theme = super::Theme::load().expect("theme");
            let mons = super::super::monitors();
            let mut cache = Default::default();
            for _ in 0..15 {
                let (x, y) = super::super::cursor_pos().unwrap();
                let began = std::time::Instant::now();
                let shape = super::shape_at(&theme, &mons, &mut cache, x, y);
                println!("SHAPE at {x},{y}: {shape:?} in {:?}", began.elapsed());
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    }
}

/// `.kadr` files as their own type with an icon, opening in kadr: the
/// shared-mime-info package, a desktop entry and icons under
/// ~/.local/share, then the caches rebuilt and kadr made their default.
pub fn install_desktop() -> std::io::Result<()> {
    let data = xdg("XDG_DATA_HOME", ".local/share");
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("kadr"));
    crate::bundle::install_desktop(&data, &exe)?;
    let run = |args: &[&str]| {
        let _ = std::process::Command::new(args[0]).args(&args[1..]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    };
    run(&["update-mime-database", &data.join("mime").to_string_lossy()]);
    run(&["update-desktop-database", &data.join("applications").to_string_lossy()]);
    run(&["gtk-update-icon-cache", "-q", "-t", &data.join("icons/hicolor").to_string_lossy()]);
    run(&["xdg-mime", "default", "kadr.desktop", crate::bundle::MIME_TYPE]);
    Ok(())
}

/// The desktop integration, once: when kadr runs and it isn't there yet.
pub fn ensure_desktop() {
    let data = xdg("XDG_DATA_HOME", ".local/share");
    if !data.join("mime/packages/kadr.xml").exists() {
        std::thread::spawn(|| {
            let _ = install_desktop();
        });
    }
}
