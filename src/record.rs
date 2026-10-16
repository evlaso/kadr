//! Screen recording: a gpu-screen-recorder session with optional webcam
//! overlay and audio, an on-screen control pill, and Omarchy's
//! post-processing before the result lands in a quick-access card.

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4 as gtk;
use gtk::{gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::cards::Cards;
use crate::panel;
use crate::track::Tracker;
use crate::util::{self, Mon};

const SIGINT: i32 = 2;

/// Microphones as gpu-screen-recorder names them: (source name, description).
pub fn microphones() -> Vec<(String, String)> {
    let out = std::process::Command::new("gpu-screen-recorder")
        .arg("--list-audio-devices")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    out.lines()
        .filter_map(|l| l.split_once('|'))
        .filter(|(name, _)| !name.starts_with("default_") && !name.ends_with(".monitor"))
        .map(|(n, d)| (n.to_owned(), d.to_owned()))
        .collect()
}
const SIGUSR2: i32 = 12;
const SIGTERM: i32 = 15;

const PILL_WIDTH: i32 = 240;
const PILL_HEIGHT: i32 = 60;

/// Omarchy's webcam resize helper reads the recorded region from here to keep
/// the camera inside it.
fn region_file() -> PathBuf {
    glib::user_runtime_dir().join("omarchy-screenrecord-region")
}

fn log_file() -> PathBuf {
    glib::user_cache_dir().join("kadr").join("record.log")
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Opts {
    pub mic: bool,
    /// PipeWire source name; empty means the system default input.
    pub mic_device: String,
    pub audio: bool,
    pub webcam: bool,
}

impl Opts {
    fn path() -> PathBuf {
        glib::user_config_dir().join("kadr").join("record.json")
    }

    /// Last choices from the options panel.
    pub fn load() -> Self {
        let v: serde_json::Value = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let get = |k: &str| v[k].as_bool().unwrap_or(false);
        let mic_device = v["mic_device"].as_str().unwrap_or_default().to_owned();
        Opts { mic: get("mic"), mic_device, audio: get("audio"), webcam: get("webcam") }
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let v = serde_json::json!({ "mic": self.mic, "mic_device": self.mic_device, "audio": self.audio, "webcam": self.webcam });
        let _ = std::fs::write(path, v.to_string());
    }

    pub fn to_flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        if self.mic {
            flags.push(if self.mic_device.is_empty() { "--mic".into() } else { format!("--mic={}", self.mic_device) });
        }
        if self.audio {
            flags.push("--audio".into());
        }
        if self.webcam {
            flags.push("--webcam".into());
        }
        flags
    }

    fn mic_source(&self) -> &str {
        if self.mic_device.is_empty() { "default_input" } else { &self.mic_device }
    }
}

#[derive(Clone, Debug)]
enum Target {
    Monitor(String),
    Region { x: i32, y: i32, w: i32, h: i32 },
}

impl Target {
    /// Recorded area in logical layout coordinates.
    fn rect(&self, mons: &[Mon]) -> (i32, i32, i32, i32) {
        match *self {
            Target::Region { x, y, w, h } => (x, y, w, h),
            Target::Monitor(ref name) => mons
                .iter()
                .find(|m| &m.name == name)
                .map(|m| (m.x, m.y, m.w, m.h))
                .unwrap_or_default(),
        }
    }

    /// gpu-screen-recorder takes regions in logical coordinates and scales them itself.
    fn gsr_args(&self, mons: &[Mon]) -> Vec<String> {
        match self {
            Target::Monitor(name) => {
                // Cap anything above 4K, like Omarchy does.
                let (pw, ph) = mons.iter().find(|m| &m.name == name).map(|m| m.phys).unwrap_or((0, 0));
                let size = if pw > 3840 || ph > 2160 { "3840x2160" } else { "0x0" };
                vec!["-w".into(), name.clone(), "-s".into(), size.into()]
            }
            Target::Region { x, y, w, h } => vec!["-w".into(), format!("{w}x{h}+{x}+{y}")],
        }
    }
}

enum State {
    Idle,
    Panel(gtk::Window),
    Busy,
    Recording(Rc<Session>),
}

struct Session {
    gsr: gio::Subprocess,
    file: PathBuf,
    webcam: Option<gio::Subprocess>,
    target: Target,
    started: Instant,
    paused_at: Cell<Option<Instant>>,
    paused_total: Cell<Duration>,
    stopping: Cell<bool>,
    discard: Cell<bool>,
    indicator: RefCell<Option<gtk::Window>>,
    frame: RefCell<Option<gtk::Window>>,
    tracker: RefCell<Option<Tracker>>,
    _hold: gio::ApplicationHoldGuard,
}

impl Session {
    fn elapsed(&self) -> Duration {
        let now = self.paused_at.get().unwrap_or_else(Instant::now);
        now.saturating_duration_since(self.started).saturating_sub(self.paused_total.get())
    }
}

pub struct Recorder {
    app: gtk::Application,
    cards: Rc<Cards>,
    state: RefCell<State>,
}

impl Recorder {
    pub fn new(app: &gtk::Application, cards: Rc<Cards>) -> Rc<Self> {
        Rc::new(Recorder { app: app.clone(), cards, state: RefCell::new(State::Idle) })
    }

    /// `kadr record [mode]`: stops a running recording, otherwise opens the
    /// options panel (no mode) or starts right away.
    pub fn command(self: &Rc<Self>, mode: Option<String>, opts: Option<Opts>) {
        let panel = match &*self.state.borrow() {
            State::Recording(_) => None,
            State::Busy => return,
            State::Panel(w) => Some(Some(w.clone())),
            State::Idle => Some(None),
        };
        match panel {
            None => self.stop(false),
            // Pressing the hotkey again dismisses the panel.
            Some(Some(window)) => {
                self.state.replace(State::Idle);
                window.close();
            }
            Some(None) => match mode {
                None => {
                    let window = panel::show(&self.app, self);
                    self.state.replace(State::Panel(window));
                }
                Some(mode) => self.start(mode, opts.unwrap_or_else(Opts::load), Duration::ZERO),
            },
        }
    }

    pub fn command_stop(self: &Rc<Self>) {
        self.stop(false);
    }

    /// Called by the options panel when it closes.
    pub fn panel_done(self: &Rc<Self>, choice: Option<(&str, Opts)>) {
        let window = match &*self.state.borrow() {
            State::Panel(w) => w.clone(),
            _ => return,
        };
        self.state.replace(State::Idle);
        window.close();
        if let Some((mode, opts)) = choice {
            opts.save();
            // Let the panel's close animation finish so it isn't picked up.
            self.start(mode.to_owned(), opts, Duration::from_millis(250));
        }
    }

    fn start(self: &Rc<Self>, mode: String, opts: Opts, delay: Duration) {
        self.state.replace(State::Busy);
        let hold = self.app.hold();
        glib::spawn_future_local(glib::clone!(#[strong(rename_to = rec)] self, async move {
            if !delay.is_zero() {
                glib::timeout_future(delay).await;
            }
            match rec.begin(&mode, opts, hold).await {
                Ok(session) => {
                    // The frame goes first so the controls stack above its dimming.
                    session.frame.replace(rec.build_frame(&session.target));
                    let indicator = rec.build_indicator(&session);
                    session.indicator.replace(indicator);
                    let weak = Rc::downgrade(&session);
                    let tracker = Tracker::start(move || {
                        let s = weak.upgrade()?;
                        s.paused_at.get().is_none().then(|| s.elapsed().as_secs_f64())
                    });
                    session.tracker.replace(Some(tracker));
                    rec.state.replace(State::Recording(session));
                    refresh_omarchy_indicator();
                }
                Err(err) => {
                    rec.state.replace(State::Idle);
                    if let Some(err) = err {
                        util::notify("Запись не запустилась", &err);
                    }
                }
            }
        }));
    }

    /// Err(None) means the user cancelled the region picker.
    async fn begin(self: &Rc<Self>, mode: &str, opts: Opts, hold: gio::ApplicationHoldGuard) -> Result<Rc<Session>, Option<String>> {
        let mons = util::monitors();
        let target = if mode == "fullscreen" {
            let focused = mons.iter().find(|m| m.focused).ok_or_else(|| Some("не найден активный монитор".to_string()))?;
            Target::Monitor(focused.name.clone())
        } else {
            pick_target().await.ok_or(None)?
        };

        let webcam = if opts.webcam { start_webcam(&target).await } else { None };

        let dir = glib::user_special_dir(glib::UserDirectory::Videos)
            .unwrap_or_else(|| glib::home_dir().join("Videos"));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let stamp = glib::DateTime::now_local()
            .and_then(|t| t.format("%Y-%m-%d_%H-%M-%S"))
            .map_err(|e| e.to_string())?;
        let file = dir.join(format!("screenrecording-{stamp}.mp4"));

        let mut argv: Vec<OsString> = vec!["gpu-screen-recorder".into()];
        argv.extend(target.gsr_args(&mons).into_iter().map(OsString::from));
        for a in ["-k", "auto", "-f", "60", "-fm", "cfr", "-fallback-cpu-encoding", "yes", "-o"] {
            argv.push(a.into());
        }
        argv.push(file.clone().into());
        // One merged track: players usually play only the first of several.
        let audio: Vec<&str> = [(opts.audio, "default_output"), (opts.mic, opts.mic_source())]
            .into_iter()
            .filter_map(|(on, dev)| on.then_some(dev))
            .collect();
        if !audio.is_empty() {
            argv.extend(["-a".into(), audio.join("|").into(), "-ac".into(), "aac".into()]);
        }

        let log = log_file();
        if let Some(dir) = log.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_SILENCE);
        launcher.set_stderr_file_path(Some(&log));
        let argv_ref: Vec<&std::ffi::OsStr> = argv.iter().map(OsString::as_os_str).collect();
        let gsr = match launcher.spawn(&argv_ref) {
            Ok(p) => p,
            Err(e) => {
                stop_webcam(webcam.as_ref());
                return Err(Some(e.to_string()));
            }
        };

        let exited = Rc::new(Cell::new(false));
        glib::spawn_future_local(glib::clone!(#[weak(rename_to = rec)] self, #[strong] gsr, #[strong] exited, async move {
            let _ = gsr.wait_future().await;
            exited.set(true);
            rec.on_gsr_exit(&gsr);
        }));

        // The output file appears once capture has actually started.
        for _ in 0..100 {
            if file.exists() || exited.get() {
                break;
            }
            glib::timeout_future(Duration::from_millis(100)).await;
        }
        if exited.get() || !file.exists() {
            gsr.force_exit();
            stop_webcam(webcam.as_ref());
            let _ = std::fs::remove_file(&file);
            return Err(Some(log_tail()));
        }

        Ok(Rc::new(Session {
            gsr,
            file,
            webcam,
            target,
            started: Instant::now(),
            paused_at: Cell::new(None),
            paused_total: Cell::new(Duration::ZERO),
            stopping: Cell::new(false),
            discard: Cell::new(false),
            indicator: RefCell::new(None),
            frame: RefCell::new(None),
            tracker: RefCell::new(None),
            _hold: hold,
        }))
    }

    fn session(&self) -> Option<Rc<Session>> {
        match &*self.state.borrow() {
            State::Recording(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn stop(self: &Rc<Self>, discard: bool) {
        let Some(s) = self.session() else { return };
        if s.stopping.replace(true) {
            return;
        }
        s.discard.set(discard);
        for w in [&s.indicator, &s.frame] {
            if let Some(w) = w.borrow().as_ref() {
                w.set_visible(false);
            }
        }
        // SIGINT lets gpu-screen-recorder finalize the file.
        s.gsr.send_signal(SIGINT);
        let gsr = s.gsr.clone();
        glib::timeout_add_local_once(Duration::from_secs(5), move || gsr.force_exit());
    }

    fn toggle_pause(&self) -> Option<bool> {
        let s = self.session()?;
        s.gsr.send_signal(SIGUSR2);
        match s.paused_at.take() {
            Some(at) => {
                s.paused_total.set(s.paused_total.get() + at.elapsed());
                Some(false)
            }
            None => {
                s.paused_at.set(Some(Instant::now()));
                Some(true)
            }
        }
    }

    fn on_gsr_exit(self: &Rc<Self>, gsr: &gio::Subprocess) {
        let Some(s) = self.session().filter(|s| &s.gsr == gsr) else { return };
        self.state.replace(State::Busy);
        for w in [s.indicator.take(), s.frame.take()].into_iter().flatten() {
            w.close();
        }
        stop_webcam(s.webcam.as_ref());
        let _ = std::fs::remove_file(region_file());
        refresh_omarchy_indicator();

        glib::spawn_future_local(glib::clone!(#[strong(rename_to = rec)] self, async move {
            let file = s.file.clone();
            let has_data = std::fs::metadata(&file).map(|m| m.len() > 0).unwrap_or(false);
            if s.discard.get() || !has_data {
                let _ = std::fs::remove_file(&file);
                if !s.discard.get() {
                    util::notify("Запись не сохранилась", &log_tail());
                }
            } else {
                if let Some(tracker) = s.tracker.take() {
                    tracker.save(s.target.rect(&util::monitors()), &file);
                }
                finalize(&file).await;
                rec.cards.show(&rec.app, file);
            }
            rec.state.replace(State::Idle);
            drop(s);
        }));
    }

    /// Recording controls, kept outside the captured area so they don't end up
    /// in the video. With a whole single monitor recorded there is nowhere to
    /// put them; the Omarchy bar indicator and the hotkey still work.
    fn build_indicator(self: &Rc<Self>, session: &Rc<Session>) -> Option<gtk::Window> {
        let mons = util::monitors();
        let (mon, top, left) = placement(&session.target, &mons)?;

        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("kadr-rec-dot");
        dot.set_valign(gtk::Align::Center);
        let time = gtk::Label::new(Some("0:00"));
        time.add_css_class("kadr-time");
        time.set_xalign(0.0);
        let pause = icon_button("kadr-pause-symbolic", "Пауза");
        let stop = icon_button("kadr-stop-symbolic", "Остановить и сохранить");
        stop.add_css_class("kadr-stop");
        let discard = icon_button("kadr-trash-symbolic", "Остановить и удалить");

        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        bar.add_css_class("kadr-recbar");
        bar.set_halign(gtk::Align::Center);
        bar.set_valign(gtk::Align::Center);
        for w in [dot.upcast_ref::<gtk::Widget>(), time.upcast_ref(), pause.upcast_ref(), stop.upcast_ref(), discard.upcast_ref()] {
            bar.append(w);
        }
        let outer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        outer.set_size_request(PILL_WIDTH, PILL_HEIGHT);
        outer.append(&bar);
        bar.set_hexpand(true);

        let window = gtk::Window::builder()
            .application(&self.app)
            .decorated(false)
            .resizable(false)
            .css_classes(["kadr-overlay"])
            .child(&outer)
            .build();
        window.init_layer_shell();
        window.set_namespace(Some("kadr-recording"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::None);
        // Position relative to the monitor itself, not the bar's reserved area.
        window.set_exclusive_zone(-1);
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_margin(Edge::Top, top);
        window.set_margin(Edge::Left, left);
        window.set_monitor(util::gdk_monitor(&mon.name).as_ref());

        pause.connect_clicked(glib::clone!(#[weak(rename_to = rec)] self, #[weak] dot, move |b| {
            if let Some(paused) = rec.toggle_pause() {
                b.set_icon_name(if paused { "kadr-play-symbolic" } else { "kadr-pause-symbolic" });
                b.set_tooltip_text(Some(if paused { "Продолжить" } else { "Пауза" }));
                if paused { dot.add_css_class("paused") } else { dot.remove_css_class("paused") }
            }
        }));
        stop.connect_clicked(glib::clone!(#[weak(rename_to = rec)] self, move |_| rec.stop(false)));
        discard.connect_clicked(glib::clone!(#[weak(rename_to = rec)] self, move |_| rec.stop(true)));

        let weak = Rc::downgrade(session);
        glib::timeout_add_local(Duration::from_millis(250), glib::clone!(#[weak] time, #[upgrade_or] glib::ControlFlow::Break, move || {
            let Some(s) = weak.upgrade() else { return glib::ControlFlow::Break };
            time.set_text(&util::format_duration(s.elapsed().as_secs_f64()));
            glib::ControlFlow::Continue
        }));

        window.present();
        Some(window)
    }

    /// Outline of the recorded region, drawn just outside it (and dimming the
    /// rest of the monitor) so it never shows up in the video. Click-through.
    fn build_frame(&self, target: &Target) -> Option<gtk::Window> {
        let Target::Region { x, y, w, h } = *target else { return None };
        let mons = util::monitors();
        let mon = mons.iter().find(|m| m.contains(x + w / 2, y + h / 2))?.clone();
        let (rx, ry) = ((x - mon.x) as f64, (y - mon.y) as f64);
        let (rw, rh) = (w as f64, h as f64);

        let area = gtk::DrawingArea::new();
        area.set_draw_func(move |_, cr, aw, ah| {
            // A pixel of slack keeps scaled edges out of the capture.
            let gap = 2.0;
            let line = 2.0;
            cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
            cr.rectangle(0.0, 0.0, aw as f64, ah as f64);
            cr.rectangle(rx - gap, ry - gap, rw + gap * 2.0, rh + gap * 2.0);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
            let _ = cr.fill();
            let o = gap + line / 2.0;
            cr.rectangle(rx - o, ry - o, rw + o * 2.0, rh + o * 2.0);
            cr.set_line_width(line);
            cr.set_source_rgba(0.898, 0.282, 0.302, 0.9);
            let _ = cr.stroke();
        });

        let window = gtk::Window::builder()
            .application(&self.app)
            .decorated(false)
            .css_classes(["kadr-overlay"])
            .child(&area)
            // GTK otherwise keeps its 200x200 fallback while the compositor
            // stretches the surface over the whole monitor.
            .default_width(mon.w)
            .default_height(mon.h)
            .build();
        window.init_layer_shell();
        window.set_namespace(Some("kadr-frame"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(-1);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.set_monitor(util::gdk_monitor(&mon.name).as_ref());
        // Let clicks and scrolling reach the windows underneath.
        let passthrough = |w: &gtk::Window| {
            if let Some(surface) = w.surface() {
                surface.set_input_region(Some(&gtk::cairo::Region::create()));
            }
        };
        window.connect_realize(passthrough);
        window.connect_map(passthrough);
        window.present();
        Some(window)
    }
}

/// Top-left margins of the control pill, relative to the monitor it goes on.
fn placement(target: &Target, mons: &[Mon]) -> Option<(Mon, i32, i32)> {
    let elsewhere = |name: &str| {
        let m = mons.iter().find(|m| m.name != name)?;
        Some((m.clone(), 48, (m.w - PILL_WIDTH) / 2))
    };
    match *target {
        Target::Monitor(ref name) => elsewhere(name),
        Target::Region { x, y, w, h } => {
            let mon = mons.iter().find(|m| m.contains(x + w / 2, y + h / 2))?;
            let left = (x + w / 2 - PILL_WIDTH / 2 - mon.x).clamp(0, (mon.w - PILL_WIDTH).max(0));
            let gap = 6;
            if y + h + gap + PILL_HEIGHT <= mon.y + mon.h {
                Some((mon.clone(), y + h + gap - mon.y, left))
            } else if y - gap - PILL_HEIGHT >= mon.y {
                Some((mon.clone(), y - gap - PILL_HEIGHT - mon.y, left))
            } else {
                elsewhere(&mon.name)
            }
        }
    }
}

/// Omarchy's picker, so selection works like for screenshots.
async fn pick_target() -> Option<Target> {
    let (ok, out) = util::output(&["omarchy-capture-region", "smart", "--match-monitor"]).await?;
    let out = out.trim();
    if !ok || out.is_empty() {
        return None;
    }
    if let Some(name) = out.strip_prefix("monitor:") {
        return Some(Target::Monitor(name.to_owned()));
    }
    // "X,Y WxH"
    let (pos, size) = out.split_once(' ')?;
    let (x, y) = pos.split_once(',')?;
    let (w, h) = size.split_once('x')?;
    Some(Target::Region { x: x.parse().ok()?, y: y.parse().ok()?, w: w.parse().ok()?, h: h.parse().ok()? })
}

/// Same mpv overlay as Omarchy's recorder, so its window rules and the
/// webcam resize hotkeys apply.
async fn start_webcam(target: &Target) -> Option<gio::Subprocess> {
    let (_, list) = util::output(&["omarchy-capture-webcam-list"]).await?;
    let Some(device) = list.split_whitespace().next().map(str::to_owned) else {
        util::notify("Камера не найдена", "Запись пойдёт без камеры");
        return None;
    };
    let (_, formats) = util::output(&["v4l2-ctl", "--list-formats-ext", "-d", &device]).await.unwrap_or_default();
    let mut capture = "framerate=30".to_owned();
    if let Some(res) = ["640x360", "1280x720", "1920x1080"].into_iter().find(|r| formats.contains(r)) {
        capture = format!("video_size={res},{capture}");
    }
    let webcam = util::spawn(&[
        "mpv".to_owned(),
        format!("av://v4l2:{device}"),
        "--profile=low-latency".into(),
        "--untimed".into(),
        "--no-cache".into(),
        format!("--demuxer-lavf-o={capture}"),
        "--vf=lavfi=[crop=ih*8/9:ih]".into(),
        "--title=WebcamOverlay".into(),
        "--wayland-app-id=WebcamOverlay-medium".into(),
        "--no-border".into(),
        "--no-audio".into(),
        "--no-osc".into(),
        "--osd-level=0".into(),
        "--really-quiet".into(),
    ])?;

    for _ in 0..40 {
        let mapped = util::hypr("j/clients")
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.as_array().map(|a| a.iter().any(|c| c["title"] == "WebcamOverlay")))
            .unwrap_or(false);
        if mapped {
            break;
        }
        glib::timeout_future(Duration::from_millis(50)).await;
    }
    if let Target::Region { x, y, w, h } = target {
        let _ = std::fs::write(region_file(), format!("{w}x{h}+{x}+{y}"));
    }
    util::output(&["omarchy-capture-webcam-resize", "medium"]).await;
    // Let the move settle, or the camera is recorded sliding into its corner.
    glib::timeout_future(Duration::from_millis(600)).await;
    Some(webcam)
}

fn stop_webcam(webcam: Option<&gio::Subprocess>) {
    if let Some(p) = webcam {
        p.send_signal(SIGTERM);
    }
}

/// Port of Omarchy's finalize step: trim the warmup frame and normalize audio.
async fn finalize(file: &std::path::Path) {
    let f = file.to_string_lossy().into_owned();
    let warmup = util::output(&["ffprobe", "-v", "error", "-select_streams", "v:0", "-read_intervals", "%+0.2", "-show_entries", "packet=flags", "-of", "csv=p=0", &f])
        .await
        .is_some_and(|(_, out)| out.contains('D'));
    let has_audio = util::output(&["ffprobe", "-v", "error", "-select_streams", "a", "-show_entries", "stream=codec_type", "-of", "csv=p=0", &f])
        .await
        .is_some_and(|(_, out)| out.contains("audio"));

    let processed = format!("{}-processed.mp4", f.trim_end_matches(".mp4"));
    let mut args: Vec<&str> = vec!["ffmpeg", "-y", "-loglevel", "quiet", "-ss", "0.1", "-i", &f];
    if warmup {
        // Stream copy can't drop discardable warmup packets.
        args.extend(["-c:v", "libx264", "-preset", "veryfast", "-crf", "20"]);
    } else {
        args.extend(["-c:v", "copy"]);
    }
    if has_audio {
        // Mute the PipeWire capture-open pop, then normalize to -14 LUFS.
        args.extend(["-af", "volume=enable='lt(t,0.4)':volume=0,afade=t=in:st=0.4:d=0.05,loudnorm=I=-14:TP=-1.5:LRA=11"]);
    }
    args.push(&processed);
    match util::output(&args).await {
        Some((true, _)) => {
            let _ = std::fs::rename(&processed, file);
        }
        _ => {
            let _ = std::fs::remove_file(&processed);
        }
    }
}

fn refresh_omarchy_indicator() {
    util::spawn(&["omarchy-shell", "-q", "omarchy.indicators", "refresh"]);
}

fn log_tail() -> String {
    let log = std::fs::read_to_string(log_file()).unwrap_or_default();
    let lines: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(3)..].join("\n");
    if tail.is_empty() { "gpu-screen-recorder завершился без вывода".into() } else { tail }
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tooltip));
    b
}

