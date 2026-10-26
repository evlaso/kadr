//! kadr's tray: an icon in the panel's tray, and on macOS and Windows the
//! global hotkeys. kadr itself exits when idle, so this lives in a small
//! process of its own with no UI framework (`kadr-tray`, or `kadr tray`): a
//! click opens the settings, the menu and the hotkeys start captures. A
//! running kadr gets the command over its socket; otherwise a new one starts.
//! On Linux the icon is a StatusNotifierItem (the Omarchy shell hosts it) and
//! the hotkeys are the compositor's binds; on macOS and Windows it's the menu
//! bar or notification area icon, and the process runs even with the icon
//! switched off, for the hotkeys.

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use kadr_core::settings::{Settings, ThemeMode};
use kadr_platform as platform;

fn lock_path() -> PathBuf {
    platform::runtime_dir().join("kadr-tray.lock")
}

/// The `kadr` binary: this one, or the one beside `kadr-tray`.
fn kadr_exe() -> OsString {
    let Ok(exe) = std::env::current_exe() else { return "kadr".into() };
    let stem = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if stem != "kadr-tray" {
        return exe.into();
    }
    let sibling = exe.with_file_name(if cfg!(windows) { "kadr.exe" } else { "kadr" });
    if sibling.exists() { sibling.into() } else { "kadr".into() }
}

/// `kadr <args>`: to the running kadr over its socket when there is one
/// (no new process to start), else as a new one.
fn kadr<S: AsRef<str>>(args: &[S]) {
    let args: Vec<String> = args.iter().map(|a| a.as_ref().to_owned()).collect();
    if let Some(mut stream) = platform::ipc::connect()
        && writeln!(stream, "{}", serde_json::to_string(&args).unwrap_or_default()).is_ok()
    {
        return;
    }
    let mut argv = vec![kadr_exe()];
    argv.extend(args.into_iter().map(OsString::from));
    platform::spawn(&argv);
}

/// A monotonic clock in milliseconds, the same across processes, for the
/// hotkey → overlay timing.
fn monotonic_ms() -> u64 {
    #[cfg(unix)]
    {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: clock_gettime fills the timespec we pass.
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
    }
    #[cfg(windows)]
    {
        // SAFETY: plain GetTickCount64.
        unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
    }
}

/// Note a hotkey in the tray's log (`hotkey_at=<ms>`), for the bench.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn mark_hotkey(command: &str) {
    let log = kadr_core::dirs::cache().join("kadr-tray.log");
    let _ = std::fs::create_dir_all(kadr_core::dirs::cache());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
        let _ = writeln!(f, "hotkey_at={} command={command}", monotonic_ms());
    }
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

/// Start the tray (when it's switched on in the settings): `kadr-tray` where
/// it's installed beside kadr, `kadr tray` otherwise.
pub fn start() {
    let exe = std::env::current_exe().ok();
    let tray = exe.as_ref().map(|e| e.with_file_name(if cfg!(windows) { "kadr-tray.exe" } else { "kadr-tray" })).filter(|t| t.exists());
    match tray {
        Some(tray) => platform::spawn(&[tray.into_os_string()]),
        None => platform::spawn(&[kadr_exe(), "tray".into()]),
    };
}

/// Stop a running `kadr tray` (when it's switched off in the settings).
pub fn stop() {
    let pid = std::fs::read_to_string(lock_path()).ok().and_then(|s| s.trim().parse::<u32>().ok());
    if let Some(pid) = pid
        && running()
    {
        platform::signal(pid, platform::Signal::Terminate);
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
    let dark = match Settings::load().theme {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => platform::prefers_dark().unwrap_or(true),
    };
    // The palette's text colour: Sumi Dark's, or the light theme's.
    let [r, g, b] = if dark { [0xCE, 0xCD, 0xC3] } else { [0x1C, 0x1B, 0x1A] };
    let mut paint = Paint::default();
    paint.set_color_rgba8(r, g, b, 255);
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
    use kadr_core::i18n::t;
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
    use kadr_core::i18n::t;
    use kadr_core::settings::Settings;

    struct Tray {
        icon: Vec<Icon>,
        /// Where the named icon is, if it could be written.
        theme: Option<String>,
    }

    /// The icon's name: "-symbolic" asks the panel to paint it in its own
    /// text colour, so it shows on any bar, light or dark.
    const NAME: &str = "kadr-tray-symbolic";

    /// The viewfinder as a symbolic SVG, the drawing `pixmap` makes, in
    /// kadr's data folder: loose and in a theme's layout, for hosts that
    /// look either way. Its folder, for the icon theme path.
    fn write_svg() -> Option<String> {
        let (m, arm, s) = (24. * 0.14, 24. * 0.24, 24.);
        let corners: String = [(m, m, 1., 1.), (s - m, m, -1., 1.), (s - m, s - m, -1., -1.), (m, s - m, 1., -1.)]
            .iter()
            .map(|&(x, y, dx, dy): &(f32, f32, f32, f32)| format!("M{x:.2} {:.2}L{x:.2} {y:.2}L{:.2} {y:.2}", y + dy * arm, x + dx * arm))
            .collect();
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\"><path d=\"{corners}\" fill=\"none\" stroke=\"#bebebe\" stroke-width=\"2.16\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/><circle cx=\"12\" cy=\"12\" r=\"2.64\" fill=\"#bebebe\"/></svg>\n"
        );
        let root = kadr_core::dirs::data().join("icons");
        let themed = root.join("hicolor/scalable/apps");
        std::fs::create_dir_all(&themed).ok()?;
        for dir in [&root, &themed] {
            let file = dir.join(format!("{NAME}.svg"));
            if std::fs::read_to_string(&file).ok().as_deref() != Some(&svg) {
                std::fs::write(&file, &svg).ok()?;
            }
        }
        Some(root.to_string_lossy().into_owned())
    }

    impl ksni::Tray for Tray {
        fn id(&self) -> String {
            "kadr".into()
        }

        fn title(&self) -> String {
            "kadr".into()
        }

        fn icon_name(&self) -> String {
            if self.theme.is_some() { NAME.into() } else { String::new() }
        }

        fn icon_theme_path(&self) -> String {
            self.theme.clone().unwrap_or_default()
        }

        /// For hosts that don't look icons up by name.
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
        super::watch_for_updates();
        let tray = Tray { icon: [22, 32, 48, 64].into_iter().filter_map(icon).collect(), theme: write_svg() };
        // Started before the panel (autostart, or a session without one),
        // the icon waits for it instead of giving up.
        if let Err(e) = tray.assume_sni_available(true).spawn() {
            return eprintln!("kadr: tray icon: {e}");
        }
        loop {
            std::thread::park();
        }
    }
}

#[cfg(target_os = "linux")]
pub use sni::run;

/// Looks for a newer release while the tray runs (SPEC 12.3): at start
/// when the last look is over an hour old, then once a day; a newer one
/// gets kadr's card, once. Off in the settings (`check_updates`).
fn watch_for_updates() {
    use kadr_core::update::{CURRENT, LATEST_URL, Latest, State};
    std::thread::spawn(|| {
        let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut at_start = true;
        loop {
            let mut state = State::load();
            if kadr_core::settings::Settings::load().check_updates && state.due(now(), at_start) {
                if let Some(latest) = platform::fetch(LATEST_URL).and_then(|b| Latest::parse(&b)) {
                    if state.found(&latest, now(), CURRENT) {
                        kadr(&["update-available", &latest.version, &latest.notes_url]);
                    }
                    state.save();
                }
            }
            at_start = false;
            std::thread::sleep(std::time::Duration::from_secs(kadr_core::update::AT_START));
        }
    });
}

/// Where kadr finds the warm overlay (`kadr-overlay --serve`).
pub fn overlay_socket() -> std::path::PathBuf {
    kadr_core::dirs::runtime().join("kadr-overlay.sock")
}

/// The area overlay kept running beside the tray, warm, so a hotkey's pick
/// shows at once (`keep_warm`, SPEC 3.6); started again if it ends.
#[cfg(target_os = "macos")]
fn keep_overlay_warm() {
    let Ok(exe) = std::env::current_exe() else { return };
    let overlay = exe.with_file_name("kadr-overlay");
    if !overlay.exists() {
        return;
    }
    std::thread::spawn(move || {
        loop {
            let started = std::time::Instant::now();
            let ran = std::process::Command::new(&overlay).arg("--serve").arg(overlay_socket()).status();
            // Not one that keeps failing at once: give up then.
            if ran.is_err() || started.elapsed() < std::time::Duration::from_secs(2) {
                std::thread::sleep(std::time::Duration::from_secs(5));
                if started.elapsed() < std::time::Duration::from_secs(8) && ran.is_err() {
                    return;
                }
            }
        }
    });
}

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
    use kadr_core::hotkeys;
    use kadr_core::i18n::t;
    use kadr_core::settings::Settings;

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
        super::watch_for_updates();
        #[cfg(target_os = "macos")]
        // KADR_KEEP_WARM=0 turns it off for this run (desktop-e2e's cold step).
        if kadr_core::settings::Settings::load().keep_warm && std::env::var_os("KADR_KEEP_WARM").is_none_or(|v| v != "0") {
            super::keep_overlay_warm();
        }
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
                super::mark_hotkey(&args.join(" "));
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
