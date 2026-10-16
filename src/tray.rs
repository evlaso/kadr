//! `kadr tray`: an icon in the panel's tray (StatusNotifierItem, which the
//! Omarchy shell hosts). kadr itself exits when idle, so the icon lives in
//! this small separate process without GPUI: a click opens the settings, the
//! menu starts captures, each by running `kadr …` like the hotkeys do.

#[cfg(target_os = "linux")]
mod sni {
    use std::ffi::OsString;
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::path::PathBuf;

    use ksni::blocking::TrayMethods;
    use ksni::menu::StandardItem;
    use ksni::{Icon, MenuItem, ToolTip};

    use crate::i18n::t;
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
            ToolTip { title: "kadr".into(), description: t("tray.click_for_settings_right_click").into(), ..Default::default() }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            kadr(&["settings"]);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let item = |label: &str, args: &'static [&'static str]| -> MenuItem<Self> {
                StandardItem { label: label.into(), activate: Box::new(move |_: &mut Self| kadr(args)), ..Default::default() }.into()
            };
            vec![
                item(t("tray.screenshot"), &["shot", "smart"]),
                item(t("common.record_screen"), &["record"]),
                item(t("common.recognize_text"), &["text"]),
                item(t("common.scrolling_screenshot"), &["shot", "scroll"]),
                item(t("common.settings"), &["settings"]),
                MenuItem::Separator,
                StandardItem {
                    label: t("tray.remove_from_tray").into(),
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
            return eprintln!("kadr: can't create {}", lock_path().display());
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
            return eprintln!("kadr: tray icon: {e}");
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
            util::signal(pid, util::Signal::Terminate);
        }
    }

    /// A viewfinder (four corner brackets and a dot) in the theme's text colour,
    /// as ARGB32 for the tray.
    fn icon(size: u32) -> Option<Icon> {
        use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};
        let color = crate::ui::rgb_of(crate::palette::theme_without_gpui().colors.text);
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
}

#[cfg(target_os = "linux")]
pub use sni::{run, start, stop};

/// Not ported to this system yet: there is no tray icon.
#[cfg(not(target_os = "linux"))]
pub fn run() {
    crate::platform::unsupported(t("common.tray_icon"));
}

#[cfg(not(target_os = "linux"))]
pub fn start() {}

#[cfg(not(target_os = "linux"))]
pub fn stop() {}
