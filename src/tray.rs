//! `kadr tray`: an icon in the panel's tray (StatusNotifierItem, which the
//! Omarchy shell hosts). kadr itself exits when idle, so the icon lives in
//! this small separate process without GPUI: a click opens the settings, the
//! menu starts captures, each by running `kadr …` like the hotkeys do.

use std::ffi::OsString;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip};

use crate::settings::Settings;
use crate::util;

fn lock_path() -> PathBuf {
    util::runtime_dir().join("kadr-tray.lock")
}

/// Run `kadr <args>` as its own process.
fn kadr(args: &[&str]) {
    let exe = std::env::current_exe().map(OsString::from).unwrap_or_else(|_| "kadr".into());
    let mut argv = vec![exe];
    argv.extend(args.iter().map(OsString::from));
    util::spawn(&argv);
}

struct Tray {
    icon: Vec<Icon>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "kadr".into()
    }

    fn title(&self) -> String {
        "kadr".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: "kadr".into(), description: "Клик — настройки, правый клик — меню".into(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        kadr(&["settings"]);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let item = |label: &str, args: &'static [&'static str]| -> MenuItem<Self> {
            StandardItem { label: label.into(), activate: Box::new(move |_: &mut Self| kadr(args)), ..Default::default() }.into()
        };
        vec![
            item("Снимок области", &["shot", "region"]),
            item("Снимок экрана", &["shot", "fullscreen"]),
            item("Запись экрана", &["record"]),
            item("Распознать текст", &["text"]),
            MenuItem::Separator,
            item("История", &["history"]),
            item("Настройки", &["settings"]),
            MenuItem::Separator,
            StandardItem {
                label: "Убрать из трея".into(),
                activate: Box::new(|_: &mut Self| {
                    let mut settings = Settings::load();
                    settings.tray = false;
                    settings.save();
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Keep the tray icon until the session ends. Quits at once when the tray is
/// turned off in the settings or another `kadr tray` already runs.
pub fn run() {
    if !Settings::load().tray {
        return;
    }
    let Ok(mut lock) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(lock_path()) else {
        return eprintln!("kadr: не удалось создать {}", lock_path().display());
    };
    // The lock is held for the life of the process, so the kernel frees it
    // even after a crash.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return;
    }
    let _ = lock.set_len(0);
    let _ = write!(lock, "{}", std::process::id());

    let tray = Tray { icon: [22, 32, 48, 64].into_iter().filter_map(icon).collect() };
    if let Err(e) = tray.spawn() {
        return eprintln!("kadr: иконка в трее: {e}");
    }
    loop {
        std::thread::park();
    }
}

/// Start the tray icon (when it's switched on in the settings).
pub fn start() {
    kadr(&["tray"]);
}

/// Stop a running `kadr tray` (when it's switched off in the settings).
pub fn stop() {
    let pid = std::fs::read_to_string(lock_path()).ok().and_then(|s| s.trim().parse::<u32>().ok());
    if let Some(pid) = pid {
        util::signal(pid, libc::SIGTERM);
    }
}

/// A viewfinder (four corner brackets and a dot) in the theme's text colour,
/// as ARGB32 for the tray.
fn icon(size: u32) -> Option<Icon> {
    use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};
    let color = crate::ui::rgb_of(crate::palette::theme().colors.text);
    let mut paint = Paint::default();
    paint.set_color_rgba8((color.r * 255.) as u8, (color.g * 255.) as u8, (color.b * 255.) as u8, 255);
    paint.anti_alias = true;

    let s = size as f32;
    let (m, arm) = (s * 0.14, s * 0.24);
    let mut pm = Pixmap::new(size, size)?;
    let mut b = PathBuilder::new();
    for (x, y, dx, dy) in [(m, m, 1., 1.), (s - m, m, -1., 1.), (s - m, s - m, -1., -1.), (m, s - m, 1., -1.)] {
        b.move_to(x, y + dy * arm);
        b.line_to(x, y);
        b.line_to(x + dx * arm, y);
    }
    let stroke = Stroke { width: (s * 0.09).max(1.5), line_cap: LineCap::Round, ..Stroke::default() };
    pm.stroke_path(&b.finish()?, &paint, &stroke, Transform::identity(), None);
    pm.fill_path(&PathBuilder::from_circle(s / 2., s / 2., s * 0.11)?, &paint, FillRule::Winding, Transform::identity(), None);

    // tiny-skia keeps premultiplied RGBA; the tray wants straight ARGB.
    let data = pm
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.alpha(), c.red(), c.green(), c.blue()]
        })
        .collect();
    Some(Icon { width: size as i32, height: size as i32, data })
}
