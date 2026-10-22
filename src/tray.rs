//! `kadr tray`: an icon in the panel's tray. kadr itself exits when idle, so
//! the icon lives in this small separate process without GPUI: a click opens
//! the settings, the menu starts captures, each by running `kadr …` like the
//! hotkeys do. On Linux it's a StatusNotifierItem (the Omarchy shell hosts
//! it); on macOS and Windows it's the menu bar or notification area icon, and
//! this process also holds kadr's global hotkeys, so there it runs even with
//! the icon switched off.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::settings::Settings;
use crate::util;

fn lock_path() -> PathBuf {
    util::runtime_dir().join("kadr-tray.lock")
}

/// Run `kadr <args>` as its own process.
fn kadr<S: AsRef<str>>(args: &[S]) {
    let exe = std::env::current_exe().map(OsString::from).unwrap_or_else(|_| "kadr".into());
    let mut argv = vec![exe];
    argv.extend(args.iter().map(|a| OsString::from(a.as_ref())));
    util::spawn(&argv);
}

/// Held for the life of `kadr tray`: there's only one. The system frees it
/// even after a crash.
struct Instance {
    #[allow(dead_code)]
    #[cfg(unix)]
    lock: std::fs::File,
    #[allow(dead_code)]
    #[cfg(windows)]
    port: std::net::TcpListener,
}

fn claim() -> Option<Instance> {
    #[cfg(unix)]
    let instance = {
        use std::os::fd::AsRawFd;
        let lock = match std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(lock_path()) {
            Ok(lock) => lock,
            Err(e) => {
                eprintln!("kadr: can't create {}: {e}", lock_path().display());
                return None;
            }
        };
        // SAFETY: flock(2) on a file we hold open.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return None;
        }
        Instance { lock }
    };
    // No flock here: a fixed localhost port is taken once.
    #[cfg(windows)]
    let instance = Instance { port: std::net::TcpListener::bind("127.0.0.1:47614").ok()? };
    Some(instance)
}

/// Become the one `kadr tray`, leaving its pid for `stop`.
fn become_tray() -> Option<Instance> {
    use std::io::Write;
    let instance = claim()?;
    let _ = std::fs::File::create(lock_path()).and_then(|mut f| write!(f, "{}", std::process::id()));
    Some(instance)
}

/// Whether a `kadr tray` is running now.
fn running() -> bool {
    claim().is_none()
}

/// Start the tray icon (when it's switched on in the settings).
pub fn start() {
    kadr(&["tray"]);
}

/// Stop a running `kadr tray` (when it's switched off in the settings).
pub fn stop() {
    let pid = std::fs::read_to_string(lock_path()).ok().and_then(|s| s.trim().parse::<u32>().ok());
    if let Some(pid) = pid
        && running()
    {
        util::signal(pid, util::Signal::Terminate);
    }
}

/// Start it over, to pick up the settings and hotkeys. Waits a little for the
/// old one to go, or the new one would find it and quit.
pub fn restart() {
    stop();
    for _ in 0..100 {
        if !running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    start();
}

/// Where `kadr tray` holds the hotkeys, have it running.
pub fn ensure() {
    if !cfg!(target_os = "linux") && !running() {
        start();
    }
}

/// A viewfinder (four corner brackets and a dot) in the theme's text colour.
fn pixmap(size: u32) -> Option<tiny_skia::Pixmap> {
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
    Some(pm)
}

/// The icon's pixels as straight (not premultiplied) RGBA.
fn rgba(size: u32) -> Option<Vec<u8>> {
    let pm = pixmap(size)?;
    Some(
        pm.pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect(),
    )
}

/// The menu: a label and the `kadr` arguments it runs.
fn menu() -> [(&'static str, &'static [&'static str]); 5] {
    use crate::i18n::t;
    [
        (t("tray.screenshot"), &["shot", "smart"]),
        (t("common.record_screen"), &["record"]),
        (t("common.recognize_text"), &["text"]),
        (t("common.scrolling_screenshot"), &["shot", "scroll"]),
        (t("common.settings"), &["settings"]),
    ]
}

/// "Remove from tray": off in the settings.
fn remove() {
    let mut settings = Settings::load();
    settings.tray = false;
    settings.save();
}

#[cfg(target_os = "linux")]
mod sni {
    use ksni::blocking::TrayMethods;
    use ksni::menu::StandardItem;
    use ksni::{Icon, MenuItem, ToolTip};

    use super::{become_tray, kadr, menu, remove, rgba};
    use crate::i18n::t;
    use crate::settings::Settings;

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
            let mut items: Vec<MenuItem<Self>> = menu()
                .into_iter()
                .map(|(label, args)| StandardItem { label: label.into(), activate: Box::new(move |_: &mut Self| kadr(args)), ..Default::default() }.into())
                .collect();
            items.push(MenuItem::Separator);
            items.push(
                StandardItem {
                    label: t("tray.remove_from_tray").into(),
                    activate: Box::new(|_: &mut Self| {
                        remove();
                        std::process::exit(0);
                    }),
                    ..Default::default()
                }
                .into(),
            );
            items
        }
    }

    /// The tray wants ARGB.
    fn icon(size: u32) -> Option<Icon> {
        let data = rgba(size)?.chunks_exact(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
        Some(Icon { width: size as i32, height: size as i32, data })
    }

    /// Keep the tray icon until the session ends. Quits at once when the tray is
    /// turned off in the settings or another `kadr tray` already runs.
    pub fn run() {
        if !Settings::load().tray {
            return;
        }
        let Some(_instance) = become_tray() else { return };
        let tray = Tray { icon: [22, 32, 48, 64].into_iter().filter_map(icon).collect() };
        if let Err(e) = tray.spawn() {
            return eprintln!("kadr: tray icon: {e}");
        }
        loop {
            std::thread::park();
        }
    }
}

#[cfg(target_os = "linux")]
pub use sni::run;

/// The system's tray icon (tray-icon) and kadr's hotkeys (global-hotkey),
/// both served by the system's event loop on the main thread.
#[cfg(not(target_os = "linux"))]
mod desktop {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use global_hotkey::hotkey::HotKey;
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::{become_tray, kadr, remove, rgba};
    use crate::hotkeys;
    use crate::i18n::t;
    use crate::settings::Settings;

    thread_local! {
        static TRAY: RefCell<Option<TrayIcon>> = const { RefCell::new(None) };
    }

    const REMOVE: &str = "remove";

    fn show() {
        let menu = Menu::new();
        for (label, args) in super::menu() {
            let _ = menu.append(&MenuItem::with_id(args.join(" "), label, true, None));
        }
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&MenuItem::with_id(REMOVE, t("tray.remove_from_tray"), true, None));
        MenuEvent::set_event_handler(Some(|e: MenuEvent| {
            if e.id.0 == REMOVE {
                remove();
                TRAY.with_borrow(|tray| tray.as_ref().map(|tray| tray.set_visible(false)));
            } else {
                kadr(&e.id.0.split(' ').collect::<Vec<_>>());
            }
        }));
        TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                kadr(&["settings"]);
            }
        }));
        let size = 32;
        let Some(icon) = rgba(size).and_then(|px| tray_icon::Icon::from_rgba(px, size, size).ok()) else { return };
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            // macOS tints it to match the menu bar.
            .with_icon_as_template(true)
            .with_tooltip(t("tray.click_for_settings_right_click"))
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build();
        match tray {
            Ok(tray) => TRAY.set(Some(tray)),
            Err(e) => eprintln!("kadr: tray icon: {e}"),
        }
    }

    /// Hold the hotkeys and the icon until the session ends.
    pub fn run() {
        let Some(_instance) = become_tray() else { return };
        #[cfg(target_os = "macos")]
        let app = {
            use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
            let Some(mtm) = objc2::MainThreadMarker::new() else { return };
            let app = NSApplication::sharedApplication(mtm);
            // A menu bar icon, no Dock icon.
            app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            app
        };

        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => m,
            Err(e) => return eprintln!("kadr: hotkeys: {e}"),
        };
        let mut commands: HashMap<u32, Vec<String>> = HashMap::new();
        for (action, specs) in hotkeys::ACTIONS.iter().zip(hotkeys::load()) {
            for spec in specs {
                let Ok(key) = hotkeys::global(&spec).parse::<HotKey>() else { continue };
                match manager.register(key) {
                    Ok(()) => {
                        commands.insert(key.id(), action.command.split(' ').skip(1).map(String::from).collect());
                    }
                    // Taken by the system or another app.
                    Err(e) => eprintln!("kadr: {}: {e}", hotkeys::label(&spec)),
                }
            }
        }
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state == HotKeyState::Pressed
                && let Some(args) = commands.get(&e.id)
            {
                kadr(args);
            }
        }));

        if Settings::load().tray {
            // macOS wants the icon made once its event loop runs.
            #[cfg(target_os = "macos")]
            dispatch2::DispatchQueue::main().exec_async(show);
            #[cfg(windows)]
            show();
        }

        #[cfg(target_os = "macos")]
        app.run();
        #[cfg(windows)]
        {
            use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, TranslateMessage};
            let mut msg = MSG::default();
            // SAFETY: the standard message loop on this thread's queue.
            unsafe {
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        drop(manager);
    }
}

#[cfg(not(target_os = "linux"))]
pub use desktop::run;
