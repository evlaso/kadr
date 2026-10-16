use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use gtk4 as gtk;
use gtk::{gdk, gio, graphene, gsk, prelude::*};

pub fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    if let Some(display) = gdk::Display::default() {
        // Icons ship inside the binary so they don't depend on the icon theme.
        gio::resources_register_include!("kadr.gresource").expect("bundled resources");
        gtk::IconTheme::for_display(&display).add_resource_path("/dev/foxqa/Kadr/icons");
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Fire-and-forget process; GSubprocess reaps it for us.
pub fn spawn<S: AsRef<OsStr>>(argv: &[S]) -> Option<gio::Subprocess> {
    let argv: Vec<&OsStr> = argv.iter().map(AsRef::as_ref).collect();
    match gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("kadr: {:?}: {e}", argv.first());
            None
        }
    }
}

pub fn copy_image(path: &Path) {
    // wl-copy keeps serving the clipboard after we exit; GTK's clipboard would not.
    let script = r#"exec wl-copy --type image/png < "$1""#;
    spawn(&[OsStr::new("sh"), "-c".as_ref(), script.as_ref(), "sh".as_ref(), path.as_os_str()]);
}

pub fn notify(title: &str, body: &str) {
    spawn(&["notify-send", "-a", "kadr", title, body]);
}

pub fn open_folder(path: &Path) {
    if let Some(dir) = path.parent() {
        let uri = gio::File::for_path(dir).uri();
        if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
            notify("Не удалось открыть папку", &e.to_string());
        }
    }
}

pub fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn hypr_socket() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    Some(PathBuf::from(runtime).join("hypr").join(sig).join(".socket.sock"))
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

pub fn focused_monitor() -> Option<gdk::Monitor> {
    gdk_monitor(&monitors().into_iter().find(|m| m.focused)?.name)
}

/// A paintable that draws `texture` into a `w`×`h` box (cropping to fill it)
/// and reports that box as its intrinsic size, so widgets don't grow to the
/// full screenshot resolution.
pub fn scaled_paintable(texture: &gdk::Texture, w: f64, h: f64) -> gdk::Paintable {
    let (tw, th) = (texture.width() as f64, texture.height() as f64);
    let k = (w / tw).max(h / th);
    let (sw, sh) = (tw * k, th * k);
    let snapshot = gtk::Snapshot::new();
    let bounds = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
    snapshot.push_clip(&bounds);
    snapshot.append_scaled_texture(
        texture,
        gsk::ScalingFilter::Trilinear,
        &graphene::Rect::new(((w - sw) / 2.0) as f32, ((h - sh) / 2.0) as f32, sw as f32, sh as f32),
    );
    snapshot.pop();
    snapshot
        .to_paintable(Some(&graphene::Size::new(w as f32, h as f32)))
        .expect("snapshot has content")
}

/// Run a command to completion; returns (success, stdout).
pub async fn output<S: AsRef<OsStr>>(argv: &[S]) -> Option<(bool, String)> {
    let argv: Vec<&OsStr> = argv.iter().map(AsRef::as_ref).collect();
    let flags = gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE;
    let proc = gio::Subprocess::newv(&argv, flags).ok()?;
    let (stdout, _) = proc.communicate_utf8_future(None).await.ok()?;
    Some((proc.is_successful(), stdout.map(|s| s.to_string()).unwrap_or_default()))
}

/// Put a file (not its contents) on the clipboard, for pasting into chats and file managers.
pub fn copy_file(path: &Path) {
    let uri = gio::File::for_path(path).uri();
    spawn(&["wl-copy", "--type", "text/uri-list", &format!("{uri}\r\n")]);
}

pub fn open_file(path: &Path) {
    let uri = gio::File::for_path(path).uri();
    if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
        notify("Не удалось открыть файл", &e.to_string());
    }
}

pub fn format_duration(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
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
    pub focused: bool,
}

impl Mon {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
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
                focused: m["focused"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

pub fn gdk_monitor(name: &str) -> Option<gdk::Monitor> {
    let monitors = gdk::Display::default()?.monitors();
    (0..monitors.n_items())
        .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
        .find(|m| m.connector().as_deref() == Some(name))
}
