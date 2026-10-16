//! Screen recording: a gpu-screen-recorder session with optional webcam
//! overlay and audio, an on-screen control pill, and Omarchy's
//! post-processing before the result lands in a quick-access card.

use std::cell::Cell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnimationExt, AnyWindowHandle, App, AsyncApp, Context, Entity, Global, Window, WindowHandle, div,
    layer_shell::*, prelude::*, px, rgba,
};

use crate::panel::{self, Panel};
use crate::settings::Settings;
use crate::track::Tracker;
use crate::kit::{self, Hold, InputRegion, LayerSpec};
use crate::ui::{Divider, IconButtonShape, Tooltip, prelude::*};
use crate::util::{self, Mon};
use crate::cards;

const PILL_WIDTH: i32 = 240;
const PILL_HEIGHT: i32 = 60;

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

/// Omarchy's webcam resize helper reads the recorded region from here to keep
/// the camera inside it.
fn region_file() -> PathBuf {
    util::runtime_dir().join("omarchy-screenrecord-region")
}

fn log_file() -> PathBuf {
    util::cache_dir().join("record.log")
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Opts {
    pub mic: bool,
    /// PipeWire source name; empty means the system default input.
    pub mic_device: String,
    pub audio: bool,
    pub webcam: bool,
    /// Show shortcuts in the studio render (logs shortcut key codes).
    pub keys: bool,
    /// Studio mode: leave the cursor out of the video (the studio draws its
    /// own from the track) and open the studio editor afterwards.
    pub studio: bool,
}

impl Opts {
    fn path() -> PathBuf {
        util::config_dir().join("record.json")
    }

    /// Last choices from the options panel.
    pub fn load() -> Self {
        let v: serde_json::Value = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let get = |k: &str| v[k].as_bool().unwrap_or(false);
        let mic_device = v["mic_device"].as_str().unwrap_or_default().to_owned();
        Opts { mic: get("mic"), mic_device, audio: get("audio"), webcam: get("webcam"), keys: get("keys"), studio: get("studio") }
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let v = serde_json::json!({ "mic": self.mic, "mic_device": self.mic_device, "audio": self.audio, "webcam": self.webcam, "keys": self.keys, "studio": self.studio });
        let _ = std::fs::write(path, v.to_string());
    }

    fn mic_source(&self) -> &str {
        if self.mic_device.is_empty() { "default_input" } else { &self.mic_device }
    }
}

#[derive(Clone, Debug)]
pub enum Target {
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
    Panel(WindowHandle<Panel>),
    Busy,
    Recording(Session),
}

struct Session {
    gsr: u32,
    file: PathBuf,
    webcam: Option<u32>,
    target: Target,
    clock: Rc<Clock>,
    stopping: bool,
    discard: bool,
    windows: Vec<AnyWindowHandle>,
    tracker: Option<Tracker>,
    studio: bool,
    _hold: Hold,
}

/// Recording time, shared with the control pill so it can redraw without
/// touching the recorder (which may be mid-update when the pill first renders).
struct Clock {
    started: Instant,
    paused_at: Cell<Option<Instant>>,
    paused_total: Cell<Duration>,
}

impl Clock {
    fn new() -> Rc<Self> {
        Rc::new(Clock { started: Instant::now(), paused_at: Cell::new(None), paused_total: Cell::new(Duration::ZERO) })
    }

    fn elapsed(&self) -> Duration {
        let now = self.paused_at.get().unwrap_or_else(Instant::now);
        now.saturating_duration_since(self.started).saturating_sub(self.paused_total.get())
    }

    fn paused(&self) -> bool {
        self.paused_at.get().is_some()
    }

    /// Returns whether the recording is now paused.
    fn toggle(&self) -> bool {
        match self.paused_at.take() {
            Some(at) => {
                self.paused_total.set(self.paused_total.get() + at.elapsed());
                false
            }
            None => {
                self.paused_at.set(Some(Instant::now()));
                true
            }
        }
    }
}

/// What `begin` hands back once capture is running.
struct Started {
    keys: bool,
    studio: bool,
    gsr: async_process::Child,
    file: PathBuf,
    webcam: Option<u32>,
    target: Target,
}

pub struct Recorder {
    state: State,
}

struct RecorderGlobal(Entity<Recorder>);

impl Global for RecorderGlobal {}

pub fn init(cx: &mut App) {
    let recorder = cx.new(|_| Recorder { state: State::Idle });
    cx.set_global(RecorderGlobal(recorder));
}

fn recorder(cx: &App) -> Entity<Recorder> {
    cx.global::<RecorderGlobal>().0.clone()
}

/// `kadr record [mode]`: stops a running recording, otherwise opens the
/// options panel (no mode) or starts right away.
pub fn command(mode: Option<String>, opts: Option<Opts>, cx: &mut App) {
    recorder(cx).update(cx, |rec, cx| rec.command(mode, opts, cx));
}

/// `kadr record area "X,Y WxH"`: the recording bar for a given area.
pub fn command_area(geometry: &str, cx: &mut App) {
    let Some(target) = parse_area(geometry) else {
        return util::notify("kadr", "Область задаётся как \"X,Y WxH\"");
    };
    recorder(cx).update(cx, |rec, cx| {
        if matches!(rec.state, State::Idle)
            && let Some(handle) = panel::open(target, cx)
        {
            rec.state = State::Panel(handle);
        }
    });
}

fn parse_area(out: &str) -> Option<Target> {
    let (pos, size) = out.trim().split_once(' ')?;
    let (x, y) = pos.split_once(',')?;
    let (w, h) = size.split_once('x')?;
    Some(Target::Region { x: x.parse().ok()?, y: y.parse().ok()?, w: w.parse().ok()?, h: h.parse().ok()? })
}

pub fn command_stop(cx: &mut App) {
    recorder(cx).update(cx, |rec, cx| rec.stop(false, cx));
}

/// What the recording bar asks for when it closes.
pub enum PanelChoice {
    Record(Target),
    /// Pick the area again.
    Repick,
    Cancel,
}

/// Where a recording comes from.
enum Area {
    Pick,
    Fullscreen,
    Fixed(Target),
}

/// Called by the recording bar, which has already closed itself.
pub fn panel_done(choice: PanelChoice, opts: Opts, cx: &mut App) {
    recorder(cx).update(cx, |rec, cx| {
        if !matches!(rec.state, State::Panel(_)) {
            return;
        }
        rec.state = State::Idle;
        opts.save();
        match choice {
            // Let the bar's close animation finish so it isn't recorded.
            PanelChoice::Record(target) => rec.start(Area::Fixed(target), opts, Duration::from_millis(250), cx),
            PanelChoice::Repick => rec.command(None, None, cx),
            PanelChoice::Cancel => {}
        }
    });
}

impl Recorder {
    fn command(&mut self, mode: Option<String>, opts: Option<Opts>, cx: &mut Context<Self>) {
        match &self.state {
            State::Recording(_) => self.stop(false, cx),
            State::Busy => {}
            // Pressing the hotkey again dismisses the panel.
            State::Panel(handle) => {
                let handle = *handle;
                self.state = State::Idle;
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            // Like CleanShot: pick the area first, then show the bar by it.
            State::Idle => match mode {
                None => {
                    self.state = State::Busy;
                    let hold = kit::hold(cx);
                    cx.spawn(async move |this, cx| {
                        let _hold = hold;
                        let target = pick_target().await;
                        let _ = this.update(cx, |rec, cx| {
                            rec.state = State::Idle;
                            if let Some(handle) = target.and_then(|t| panel::open(t, cx)) {
                                rec.state = State::Panel(handle);
                            }
                        });
                    })
                    .detach();
                }
                Some(mode) => {
                    let area = if mode == "fullscreen" { Area::Fullscreen } else { Area::Pick };
                    self.start(area, opts.unwrap_or_else(Opts::load), Duration::ZERO, cx)
                }
            },
        }
    }

    fn start(&mut self, area: Area, opts: Opts, delay: Duration, cx: &mut Context<Self>) {
        self.state = State::Busy;
        let hold = kit::hold(cx);
        cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let result = begin(area, &opts, cx).await;
            let _ = this.update(cx, |rec, cx| match result {
                Ok(started) => rec.on_started(started, hold, cx),
                Err(err) => {
                    rec.state = State::Idle;
                    if let Some(err) = err {
                        util::notify("Запись не запустилась", &err);
                    }
                }
            });
        })
        .detach();
    }

    fn on_started(&mut self, started: Started, hold: Hold, cx: &mut Context<Self>) {
        let Started { keys, studio, mut gsr, file, webcam, target } = started;
        let pid = gsr.id();
        cx.spawn(async move |this, cx| {
            let _ = gsr.status().await;
            let _ = this.update(cx, |rec, cx| rec.on_gsr_exit(pid, cx));
        })
        .detach();

        let clock = Clock::new();
        let mut windows = Vec::new();
        // The frame goes first so the controls stack above its dimming.
        if Settings::load().region_frame {
            windows.extend(open_frame(&target, cx));
        }
        windows.extend(self.open_indicator(&target, clock.clone(), cx));
        let tracker = Tracker::start(cx, keys, |rec: &Recorder| rec.clock());
        self.state = State::Recording(Session {
            gsr: pid,
            file,
            webcam,
            target,
            clock,
            stopping: false,
            discard: false,
            windows,
            tracker: Some(tracker),
            studio,
            _hold: hold,
        });
        refresh_omarchy_indicator();
    }

    /// Seconds of recorded video so far, or None while paused or idle.
    fn clock(&self) -> Option<f64> {
        match &self.state {
            State::Recording(s) if !s.clock.paused() => Some(s.clock.elapsed().as_secs_f64()),
            _ => None,
        }
    }

    fn stop(&mut self, discard: bool, cx: &mut Context<Self>) {
        let State::Recording(s) = &mut self.state else { return };
        if std::mem::replace(&mut s.stopping, true) {
            return;
        }
        s.discard = discard;
        // A window that is mid-update (the pill's own button) can't be closed
        // from here; it closed itself, and on_gsr_exit retries the rest.
        for w in &s.windows {
            let _ = w.update(cx, |_, window, _| window.remove_window());
        }
        // SIGINT lets gpu-screen-recorder finalize the file.
        let pid = s.gsr;
        util::signal(pid, util::Signal::Interrupt);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let still = this.update(cx, |rec, _| matches!(&rec.state, State::Recording(s) if s.gsr == pid)).unwrap_or(false);
            if still {
                util::signal(pid, util::Signal::Kill);
            }
        })
        .detach();
    }

    fn toggle_pause(&mut self) -> Option<bool> {
        let State::Recording(s) = &mut self.state else { return None };
        util::signal(s.gsr, util::Signal::Pause);
        Some(s.clock.toggle())
    }

    fn on_gsr_exit(&mut self, pid: u32, cx: &mut Context<Self>) {
        if !matches!(&self.state, State::Recording(s) if s.gsr == pid) {
            return;
        }
        let State::Recording(mut s) = std::mem::replace(&mut self.state, State::Busy) else { unreachable!() };
        for w in s.windows.drain(..) {
            let _ = w.update(cx, |_, window, _| window.remove_window());
        }
        if let Some(cam) = s.webcam {
            util::signal(cam, util::Signal::Terminate);
        }
        let _ = std::fs::remove_file(region_file());
        refresh_omarchy_indicator();

        cx.spawn(async move |this, cx| {
            let file = s.file.clone();
            let has_data = std::fs::metadata(&file).map(|m| m.len() > 0).unwrap_or(false);
            if s.discard || !has_data {
                let _ = std::fs::remove_file(&file);
                if !s.discard {
                    util::notify("Запись не сохранилась", &log_tail());
                }
            } else {
                if let Some(tracker) = s.tracker.take() {
                    tracker.save(s.target.rect(&util::monitors()), s.studio, &file);
                }
                finalize(&file).await;
                // Studio recordings go straight to the editor, like CleanShot.
                let studio = s.studio;
                let _ = cx.update(|cx| if studio { crate::studio_editor::open(file, cx) } else { cards::show(file, cx) });
            }
            let _ = this.update(cx, |rec, _| rec.state = State::Idle);
            drop(s);
        })
        .detach();
    }

    /// Recording controls, kept outside the captured area so they don't end up
    /// in the video. With a whole single monitor recorded there is nowhere to
    /// put them; the Omarchy bar indicator and the hotkey still work.
    fn open_indicator(&self, target: &Target, clock: Rc<Clock>, cx: &mut Context<Self>) -> Option<AnyWindowHandle> {
        let (mon, top, left) = placement(target, &util::monitors())?;
        let spec = LayerSpec {
            namespace: "kadr-recording",
            anchor: Anchor::TOP | Anchor::LEFT,
            margin: (top as f32, 0., 0., left as f32),
            size: (PILL_WIDTH as f32 * kit::SCALE, PILL_HEIGHT as f32 * kit::SCALE),
            keyboard: KeyboardInteractivity::None,
            // Position relative to the monitor itself, not the bar's reserved area.
            ignore_zones: true,
            monitor: Some(mon),
        };
        let recorder = cx.weak_entity();
        kit::open_layer(cx, spec, |window, cx| cx.new(|cx| RecBar::new(recorder, clock, window, cx))).map(Into::into)
    }
}

/// Err(None) means the user cancelled the region picker.
async fn begin(area: Area, opts: &Opts, cx: &mut AsyncApp) -> Result<Started, Option<String>> {
    let mons = util::monitors();
    let target = match area {
        Area::Fullscreen => {
            let focused = mons.iter().find(|m| m.focused).ok_or_else(|| Some("не найден активный монитор".to_string()))?;
            Target::Monitor(focused.name.clone())
        }
        Area::Pick => pick_target().await.ok_or(None)?,
        Area::Fixed(target) => target,
    };

    let webcam = if opts.webcam { start_webcam(&target, cx).await } else { None };

    let settings = Settings::load();
    let dir = settings.video_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("screenrecording-{}.mp4", util::timestamp()));

    let mut args: Vec<OsString> = target.gsr_args(&mons).into_iter().map(OsString::from).collect();
    let fps = settings.fps.to_string();
    let quality = crate::settings::QUALITIES[settings.quality].0;
    if opts.studio {
        args.extend(["-cursor".into(), "no".into()]);
    }
    for a in ["-k", "auto", "-f", &fps, "-q", quality, "-fm", "cfr", "-fallback-cpu-encoding", "yes", "-o"] {
        args.push(a.into());
    }
    args.push(file.clone().into());
    // One merged track: players usually play only the first of several.
    let audio: Vec<&str> = [(opts.audio, "default_output"), (opts.mic, opts.mic_source())]
        .into_iter()
        .filter_map(|(on, dev)| on.then_some(dev))
        .collect();
    if !audio.is_empty() {
        args.extend(["-a".into(), audio.join("|").into(), "-ac".into(), "aac".into()]);
    }

    let _ = std::fs::create_dir_all(util::cache_dir());
    let stop_cam = || {
        if let Some(cam) = webcam {
            util::signal(cam, util::Signal::Terminate);
        }
    };
    let stderr = std::fs::File::create(log_file()).map_err(|e| Some(e.to_string()))?;
    let mut gsr = match async_process::Command::new("gpu-screen-recorder")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            stop_cam();
            return Err(Some(e.to_string()));
        }
    };

    // The output file appears once capture has actually started.
    for _ in 0..100 {
        if file.exists() || !matches!(gsr.try_status(), Ok(None)) {
            break;
        }
        cx.background_executor().timer(Duration::from_millis(100)).await;
    }
    if !file.exists() || !matches!(gsr.try_status(), Ok(None)) {
        let _ = gsr.kill();
        stop_cam();
        let _ = std::fs::remove_file(&file);
        return Err(Some(log_tail()));
    }
    Ok(Started { keys: opts.keys, studio: opts.studio, gsr, file, webcam, target })
}

/// Top-left margins of the control pill, relative to the monitor it goes on.
fn placement(target: &Target, mons: &[Mon]) -> Option<(Mon, i32, i32)> {
    // The pill's window grows with the UI scale.
    let (pill_w, pill_h) = ((PILL_WIDTH as f32 * kit::SCALE) as i32, (PILL_HEIGHT as f32 * kit::SCALE) as i32);
    let elsewhere = |name: &str| {
        let m = mons.iter().find(|m| m.name != name)?;
        Some((m.clone(), 48, (m.w - pill_w) / 2))
    };
    match *target {
        Target::Monitor(ref name) => elsewhere(name),
        Target::Region { x, y, w, h } => {
            let mon = mons.iter().find(|m| m.contains(x + w / 2, y + h / 2))?;
            let left = (x + w / 2 - pill_w / 2 - mon.x).clamp(0, (mon.w - pill_w).max(0));
            let gap = 6;
            if y + h + gap + pill_h <= mon.y + mon.h {
                Some((mon.clone(), y + h + gap - mon.y, left))
            } else if y - gap - pill_h >= mon.y {
                Some((mon.clone(), y - gap - pill_h - mon.y, left))
            } else {
                elsewhere(&mon.name)
            }
        }
    }
}

/// Outline of the recorded region, drawn just outside it (and dimming the
/// rest of the monitor) so it never shows up in the video. Click-through.
fn open_frame(target: &Target, cx: &mut App) -> Option<AnyWindowHandle> {
    let Target::Region { x, y, w, h } = *target else { return None };
    let mon = util::monitors().into_iter().find(|m| m.contains(x + w / 2, y + h / 2))?;
    let rect = ((x - mon.x) as f32, (y - mon.y) as f32, w as f32, h as f32);
    let spec = LayerSpec {
        namespace: "kadr-frame",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mon.w as f32, mon.h as f32),
        keyboard: KeyboardInteractivity::None,
        ignore_zones: true,
        monitor: Some(mon),
    };
    kit::open_layer(cx, spec, |_, cx| cx.new(|_| Frame { rect })).map(Into::into)
}

struct Frame {
    rect: (f32, f32, f32, f32),
}

impl Render for Frame {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Let clicks and scrolling reach the windows underneath.
        window.set_input_region(Some(&[]));
        // A pixel of slack keeps scaled edges out of the capture.
        let (gap, line) = (2., 2.);
        let (x, y, w, h) = self.rect;
        let (x0, y0, x1, y1) = (x - gap, y - gap, x + w + gap, y + h + gap);
        let dim = || div().absolute().bg(rgba(0x00000059));
        div()
            .size_full()
            .relative()
            .child(dim().top_0().left_0().right_0().h(px(y0.max(0.))))
            .child(dim().top(px(y1)).left_0().right_0().bottom_0())
            .child(dim().top(px(y0)).left_0().w(px(x0.max(0.))).h(px(y1 - y0)))
            .child(dim().top(px(y0)).left(px(x1)).right_0().h(px(y1 - y0)))
            .child(
                div()
                    .absolute()
                    .left(px(x0 - line))
                    .top(px(y0 - line))
                    .w(px(x1 - x0 + 2. * line))
                    .h(px(y1 - y0 + 2. * line))
                    .border(px(line))
                    .border_color(rgba(0xe5484de6)),
            )
    }
}

/// The in-recording control pill: timer, pause, stop, discard.
struct RecBar {
    recorder: gpui::WeakEntity<Recorder>,
    clock: Rc<Clock>,
    region: InputRegion,
}

impl RecBar {
    fn new(recorder: gpui::WeakEntity<Recorder>, clock: Rc<Clock>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        RecBar { recorder, clock, region: InputRegion::default() }
    }
}

impl Render for RecBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.region.reset();
        let (elapsed, paused) = (self.clock.elapsed().as_secs_f64(), self.clock.paused());

        let recorder = self.recorder.clone();
        let with_rec = move |close: bool, f: fn(&mut Recorder, &mut Context<Recorder>)| {
            let recorder = recorder.clone();
            move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                if close {
                    window.remove_window();
                }
                let _ = recorder.update(cx, |rec, cx| f(rec, cx));
            }
        };
        let button = |id: &'static str, icon: IconName, tip: &'static str| {
            IconButton::new(id, icon).shape(IconButtonShape::Square).icon_size(IconSize::Small).tooltip(Tooltip::text(tip))
        };

        let colors = cx.theme().status();
        let dot = div().size_2p5().rounded_full().bg(if paused { colors.warning } else { colors.error });
        let dot = if paused {
            dot.into_any_element()
        } else {
            dot.with_animation(
                "pulse",
                gpui::Animation::new(Duration::from_millis(1200)).repeat().with_easing(gpui::ease_in_out),
                |el, d| el.opacity(1. - 0.7 * (1. - (2. * d - 1.).abs())),
            )
            .into_any_element()
        };

        kit::root(cx).flex().items_center().justify_center().child(
            h_flex()
                .relative()
                .gap_1()
                .pl_2p5()
                .pr_1()
                .py_1()
                .elevation_3(cx)
                .child(dot)
                .child(
                    div()
                        .min_w(px(44.))
                        .font_family(kit::MONO_FONT)
                        .text_sm()
                        .child(util::format_duration(elapsed)),
                )
                .child(Divider::vertical().h_4())
                .child(
                    button("pause", if paused { IconName::PlayFilled } else { IconName::DebugPause }, if paused { "Продолжить" } else { "Пауза" })
                        .on_click(with_rec(false, |rec, cx| {
                            rec.toggle_pause();
                            cx.notify();
                        })),
                )
                .child(button("stop", IconName::Stop, "Остановить и сохранить").icon_color(Color::Error).on_click(with_rec(true, |rec, cx| rec.stop(false, cx))))
                .child(button("discard", IconName::Trash, "Остановить и удалить").on_click(with_rec(true, |rec, cx| rec.stop(true, cx))))
                .child(self.region.mark()),
        )
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
    parse_area(out)
}

/// Same mpv overlay as Omarchy's recorder, so its window rules and the
/// webcam resize hotkeys apply.
async fn start_webcam(target: &Target, cx: &mut AsyncApp) -> Option<u32> {
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
        cx.background_executor().timer(Duration::from_millis(50)).await;
    }
    if let Target::Region { x, y, w, h } = target {
        let _ = std::fs::write(region_file(), format!("{w}x{h}+{x}+{y}"));
    }
    util::output(&["omarchy-capture-webcam-resize", "medium"]).await;
    // Let the move settle, or the camera is recorded sliding into its corner.
    cx.background_executor().timer(Duration::from_millis(600)).await;
    Some(webcam)
}

/// Port of Omarchy's finalize step: trim the warmup frame and normalize audio.
async fn finalize(file: &Path) {
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
