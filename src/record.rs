//! Screen recording: a gpu-screen-recorder session with optional webcam
//! overlay and audio, an on-screen control pill, and Omarchy's
//! post-processing before the result lands in a quick-access card.

use std::cell::Cell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::kit::{Anchor, KeyboardInteractivity};
use gpui::{
    AnimationExt, AnyWindowHandle, App, AsyncApp, Context, Entity, Global, Window, WindowHandle, div,
    prelude::*, px, rgba,
};

use crate::panel::{self, Panel};
use crate::settings::Settings;
use crate::track::Tracker;
use crate::kit::{self, Hold, InputRegion, LayerSpec};
use crate::ui::{Divider, IconButtonShape, Tooltip, prelude::*};
use crate::util::{self, Mon};
use crate::cards;
use crate::webcam;
use crate::i18n::t;

/// The recording bar sits where the panel's bar was: this far under the
/// area, assuming this height (the panel's).
const BAR_GAP: f32 = 14.;
const BAR_HEIGHT: f32 = 56. * kit::SCALE;

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
    /// Seconds to count down before recording; 0 starts at once.
    pub countdown: u32,
}

/// Countdown choices, cycled by the panel's timer button.
pub const COUNTDOWNS: [u32; 3] = [0, 3, 5];

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
        let get = |k: &str| v[k].as_bool().unwrap_or(k == "studio");
        let mic_device = v["mic_device"].as_str().unwrap_or_default().to_owned();
        let countdown = v["countdown"].as_u64().map(|n| n as u32).filter(|n| COUNTDOWNS.contains(n)).unwrap_or(3);
        Opts { mic: get("mic"), mic_device, audio: get("audio"), webcam: get("webcam"), keys: get("keys"), studio: get("studio"), countdown }
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let v = serde_json::json!({ "mic": self.mic, "mic_device": self.mic_device, "audio": self.audio, "webcam": self.webcam, "keys": self.keys, "studio": self.studio, "countdown": self.countdown });
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
    /// Discarded to record the same area again with the same options.
    restart: bool,
    opts: Opts,
    windows: Vec<AnyWindowHandle>,
    tracker: Option<Tracker>,
    studio: bool,
    /// Restores the user's cursor setting when the session ends.
    _cursor_off: Option<crate::platform::CursorOffCapture>,
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
    cursor_off: Option<crate::platform::CursorOffCapture>,
    gsr: async_process::Child,
    file: PathBuf,
    webcam: Option<u32>,
    target: Target,
    opts: Opts,
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
        return util::notify("kadr", t("record.area_given_as_x_y"));
    };
    open_panel(target, cx);
}

/// `kadr record previous`: the recording bar for the last area picked.
pub fn command_previous(cx: &mut App) {
    let Some((_, (x, y, w, h))) = crate::picker::previous_area() else {
        return util::notify("kadr", t("picker.no_previous_area"));
    };
    open_panel(Target::Region { x, y, w, h }, cx);
}

fn open_panel(target: Target, cx: &mut App) {
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
            PanelChoice::Repick => {
                webcam::hide(cx);
                rec.command(None, None, cx)
            }
            PanelChoice::Cancel => webcam::hide(cx),
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
                webcam::hide(cx);
            }
            // Like CleanShot: pick the area first, then show the bar by it.
            State::Idle => match mode {
                None => {
                    self.state = State::Busy;
                    let hold = kit::hold(cx);
                    cx.spawn(async move |this, cx| {
                        let _hold = hold;
                        let target = pick_target(cx).await;
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
                    webcam::hide(cx);
                    if let Some(err) = err {
                        util::notify(t("record.recording_didnt_start"), &err);
                    }
                }
            });
        })
        .detach();
    }

    fn on_started(&mut self, started: Started, hold: Hold, cx: &mut Context<Self>) {
        let Started { keys, studio, cursor_off, mut gsr, file, webcam, target, opts } = started;
        let pid = gsr.id();
        cx.spawn(async move |this, cx| {
            let _ = gsr.status().await;
            let _ = this.update(cx, |rec, cx| rec.on_gsr_exit(pid, cx));
        })
        .detach();

        webcam::lock(cx);
        let clock = Clock::new();
        let mut windows = Vec::new();
        // The frame goes first so the controls stack above its dimming.
        if Settings::load().region_frame {
            windows.extend(open_frame(&target, cx));
        }
        windows.extend(self.open_indicator(&target, clock.clone(), cx));
        let tracker = Tracker::start(cx, keys, studio, |rec: &Recorder| rec.clock());
        self.state = State::Recording(Session {
            gsr: pid,
            file,
            webcam,
            target,
            clock,
            stopping: false,
            discard: false,
            restart: false,
            opts,
            windows,
            tracker: Some(tracker),
            studio,
            _cursor_off: cursor_off,
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
        webcam::hide(cx);
        refresh_omarchy_indicator();

        cx.spawn(async move |this, cx| {
            let file = s.file.clone();
            let has_data = std::fs::metadata(&file).map(|m| m.len() > 0).unwrap_or(false);
            if s.discard || !has_data {
                let _ = std::fs::remove_file(&file);
                if !s.discard {
                    util::notify(t("record.recording_wasnt_saved"), &log_tail());
                }
            } else {
                if let Some(tracker) = s.tracker.take() {
                    tracker.save(s.target.rect(&util::monitors()), s.studio, &file);
                }
                finalize(&file, s.opts.audio, s.opts.mic).await;
                // Studio recordings go straight to the editor, like CleanShot.
                let studio = s.studio;
                let _ = cx.update(|cx| if studio { crate::studio_editor::open(file, cx) } else { cards::after_capture(file, crate::storage::Capture::Recording, cx) });
            }
            // Let the session go (the cursor setting comes back) before
            // starting over in the same area.
            let again = s.restart.then(|| (s.target.clone(), s.opts.clone()));
            drop(s);
            let _ = this.update(cx, |rec, cx| {
                rec.state = State::Idle;
                if let Some((target, opts)) = again {
                    rec.start(Area::Fixed(target), opts, Duration::from_millis(250), cx);
                }
            });
        })
        .detach();
    }

    /// Throw the take away and record the same area again.
    fn restart(&mut self, cx: &mut Context<Self>) {
        let State::Recording(s) = &mut self.state else { return };
        if s.stopping {
            return;
        }
        s.restart = true;
        self.stop(true, cx);
    }

    /// Recording controls, kept outside the captured area so they don't end up
    /// in the video. With a whole single monitor recorded there is nowhere to
    /// put them; the Omarchy bar indicator and the hotkey still work.
    fn open_indicator(&self, target: &Target, clock: Rc<Clock>, cx: &mut Context<Self>) -> Option<AnyWindowHandle> {
        let (mon, top, center) = placement(target, &util::monitors())?;
        // A whole-monitor layer, clickable only on the bar, so the bar can sit
        // exactly where the panel's was and its shadow isn't clipped.
        let spec = LayerSpec {
            namespace: "kadr-recording",
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            margin: (0., 0., 0., 0.),
            size: (mon.w as f32, mon.h as f32),
            keyboard: KeyboardInteractivity::None,
            // Position relative to the monitor itself, not the bar's reserved area.
            ignore_zones: true,
            monitor: Some(mon),
        };
        let recorder = cx.weak_entity();
        kit::open_layer(cx, spec, move |window, cx| cx.new(|cx| RecBar::new(recorder, clock, (top, center), window, cx))).map(Into::into)
    }
}

/// Err(None) means the user cancelled the region picker.
async fn begin(area: Area, opts: &Opts, cx: &mut AsyncApp) -> Result<Started, Option<String>> {
    let mons = util::monitors();
    let target = match area {
        Area::Fullscreen => {
            let focused = mons.iter().find(|m| m.focused).ok_or_else(|| Some(t("record.no_active_monitor_found").to_string()))?;
            Target::Monitor(focused.name.clone())
        }
        Area::Pick => pick_target(cx).await.ok_or(None)?,
        Area::Fixed(target) => target,
    };

    // 3, 2, 1 over the area; Esc there cancels.
    if !countdown(&target, opts.countdown, &mons, cx).await {
        return Err(None);
    }

    let settings = Settings::load();
    let dir = settings.video_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = crate::storage::capture_path(&dir, crate::storage::Capture::Recording, "mp4");

    // In studio mode the camera is recorded on its own, so the studio can
    // shape, size and place it; otherwise it's a window on screen.
    let (webcam, camera_began) = match (opts.webcam, opts.studio) {
        (false, _) => {
            cx.update(webcam::hide);
            (None, None)
        }
        // The bubble is recorded where it was put.
        (true, false) => {
            cx.update(|cx| show_bubble(&target, &mons, cx));
            (None, None)
        }
        // The camera goes to its own file; the bubble frees it first.
        (true, true) => {
            cx.update(webcam::hide);
            cx.background_executor().timer(Duration::from_millis(300)).await;
            match record_camera(&file, cx).await {
                Some((pid, began)) => (Some(pid), Some(began)),
                None => (None, None),
            }
        }
    };

    let mut args: Vec<OsString> = target.gsr_args(&mons).into_iter().map(OsString::from).collect();
    let fps = settings.fps.to_string();
    let quality = crate::settings::QUALITIES[settings.quality].0;
    // Studio recordings leave the cursor out; the studio draws its own.
    let cursor_off = opts.studio.then(crate::platform::CursorOffCapture::start).flatten();
    if opts.studio {
        args.extend(["-cursor".into(), "no".into()]);
    }
    for a in ["-k", "auto", "-f", &fps, "-q", quality, "-fm", "cfr", "-fallback-cpu-encoding", "yes", "-o"] {
        args.push(a.into());
    }
    args.push(file.clone().into());
    // Separate tracks (system first) so finalize() can center the mic
    // before mixing them into one: players usually play only the first.
    let audio: Vec<&str> = [(opts.audio, "default_output"), (opts.mic, opts.mic_source())]
        .into_iter()
        .filter_map(|(on, dev)| on.then_some(dev))
        .collect();
    for dev in &audio {
        args.extend(["-a".into(), (*dev).into()]);
    }
    if !audio.is_empty() {
        args.extend(["-ac".into(), "aac".into()]);
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
    if let Some(began) = camera_began {
        crate::studio::save_camera(&file, began.elapsed().as_secs_f64(), webcam::Look::load());
    }
    Ok(Started { keys: opts.keys, studio: opts.studio, cursor_off, gsr, file, webcam, target, opts: opts.clone() })
}

/// The camera bubble over the recorded area.
pub fn show_bubble(target: &Target, mons: &[Mon], cx: &mut App) {
    let found = match target {
        Target::Monitor(name) => mons.iter().find(|m| &m.name == name).map(|m| (m.clone(), (0., 0., m.w as f32, m.h as f32))),
        Target::Region { x, y, w, h } => mons
            .iter()
            .find(|m| m.contains(x + w / 2, y + h / 2))
            .map(|m| (m.clone(), ((x - m.x) as f32, (y - m.y) as f32, *w as f32, *h as f32))),
    };
    if let Some((mon, area)) = found {
        webcam::show(&mon, area, false, cx);
    }
}

/// Record the camera to the studio's camera file. Returns ffmpeg's pid and
/// when its first frame was written, to line it up with the screen.
async fn record_camera(video: &std::path::Path, cx: &mut AsyncApp) -> Option<(u32, std::time::Instant)> {
    let Some(device) = webcam::device() else {
        util::notify(t("common.no_camera_found"), t("common.recording_without_camera"));
        return None;
    };
    let size = webcam::capture_size(&device);
    let out = crate::studio::camera_file(video);
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(&out);
    let out_arg = out.display().to_string();
    let pid = util::spawn(&[
        "ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error", "-f", "v4l2", "-framerate", "30", "-video_size", size, "-i", &device,
        "-c:v", "libx264", "-preset", "ultrafast", "-crf", "24", "-pix_fmt", "yuv420p", "-y", &out_arg,
    ])?;
    // The file grows once frames come in.
    for _ in 0..60 {
        if std::fs::metadata(&out).is_ok_and(|m| m.len() > 0) {
            return Some((pid, std::time::Instant::now()));
        }
        cx.background_executor().timer(Duration::from_millis(50)).await;
    }
    util::signal(pid, util::Signal::Terminate);
    util::notify(t("common.camera_didnt_start"), t("common.recording_without_camera"));
    None
}

/// Top-left margins of the control pill, relative to the monitor it goes on.
/// Where the recording bar goes, in the monitor's coordinates: its top and
/// horizontal centre. Under the area like the panel's bar, or above it; when
/// the whole monitor is recorded (or there's no room), on another monitor.
fn placement(target: &Target, mons: &[Mon]) -> Option<(Mon, f32, f32)> {
    let elsewhere = |name: &str| {
        let m = mons.iter().find(|m| m.name != name)?;
        Some((m.clone(), 48., m.w as f32 / 2.))
    };
    match *target {
        Target::Monitor(ref name) => elsewhere(name),
        Target::Region { x, y, w, h } => {
            let mon = mons.iter().find(|m| m.contains(x + w / 2, y + h / 2))?;
            let (x, y, w, h) = ((x - mon.x) as f32, (y - mon.y) as f32, w as f32, h as f32);
            let center = x + w / 2.;
            if y + h + BAR_GAP + BAR_HEIGHT <= mon.h as f32 - 8. {
                Some((mon.clone(), y + h + BAR_GAP, center))
            } else if y - BAR_GAP - BAR_HEIGHT >= 8. {
                Some((mon.clone(), y - BAR_GAP - BAR_HEIGHT, center))
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
    /// The bar's top and horizontal centre on the monitor.
    at: (f32, f32),
    region: InputRegion,
}

impl RecBar {
    fn new(recorder: gpui::WeakEntity<Recorder>, clock: Rc<Clock>, at: (f32, f32), window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        RecBar { recorder, clock, at, region: InputRegion::default() }
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
        // The same buttons as the recording panel's bar.
        let button = |id: &'static str, icon: IconName, tip: &'static str| {
            IconButton::new(id, icon).shape(IconButtonShape::Square).size(ButtonSize::Large).icon_size(IconSize::Small).tip(Tooltip::text(tip))
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

        let (top, center) = self.at;
        let room = 600. * kit::SCALE;
        let bar = h_flex()
                .gap_0p5()
                .p_1()
                .elevation_3(cx)
                // Compact, so it stays out of the video right above it.
                .shadow(vec![
                    gpui::BoxShadow { color: gpui::hsla(0., 0., 0., 0.35).into(), offset: gpui::point(px(0.), px(4.)), blur_radius: px(12.), spread_radius: px(0.), inset: false },
                    gpui::BoxShadow { color: gpui::hsla(0., 0., 0., 0.2).into(), offset: gpui::point(px(0.), px(1.)), blur_radius: px(3.), spread_radius: px(0.), inset: false },
                ])
                .child(
                    h_flex()
                        .gap_2()
                        .px_2()
                        .child(dot)
                        .child(div().min_w(px(44.)).font_family(kit::MONO_FONT).text_sm().child(util::format_duration(elapsed))),
                )
                .child(Divider::vertical().h_5().mx_1())
                .child(
                    button("pause", if paused { IconName::PlayFilled } else { IconName::DebugPause }, if paused { t("record.resume") } else { t("record.pause") })
                        .on_click(with_rec(false, |rec, cx| {
                            rec.toggle_pause();
                            cx.notify();
                        })),
                )
                .child(button("stop", IconName::Stop, t("record.stop_save")).icon_color(Color::Error).on_click(with_rec(true, |rec, cx| rec.stop(false, cx))))
                .child(
                    IconButton::new("restart", IconName::Restart)
                        .shape(IconButtonShape::Square)
                        .size(ButtonSize::Large)
                        .icon_size(IconSize::Small)
                        .tip(|_: &mut Window, cx: &mut App| cx.new(|_| Tooltip::new(t("record.restart"))).into())
                        .on_click(with_rec(true, |rec, cx| rec.restart(cx))),
                )
                .child(button("discard", IconName::Trash, t("record.stop_delete")).on_click(with_rec(true, |rec, cx| rec.stop(true, cx))))
                .child(self.region.mark());
        kit::root(cx)
            .relative()
            .child(div().absolute().left(px(center - room / 2.)).w(px(room)).top(px(top)).flex().justify_center().child(bar))
    }
}

/// kadr's picker, like for screenshots: it closes on release, and the
/// recording bar then lets the area be adjusted, like CleanShot's.
async fn pick_target(cx: &mut AsyncApp) -> Option<Target> {
    let picked = crate::picker::pick(true, false, cx).await?;
    if picked.whole {
        return Some(Target::Monitor(picked.mon.name));
    }
    let (x, y, w, h) = picked.area;
    Some(Target::Region { x, y, w, h })
}

/// Port of Omarchy's finalize step: trim the warmup frame and normalize audio.
/// Also mixes the system and mic tracks record() writes into one, with the
/// mic downmixed to the center: interfaces like the Audient iD4 expose a
/// stereo source with the mic on input 1 only, which plays in the left ear.
async fn finalize(file: &Path, system: bool, mic: bool) {
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
    let graph;
    if has_audio {
        let center = "aformat=channel_layouts=mono,aformat=channel_layouts=stereo";
        let mix = match (system, mic) {
            (true, true) => format!("[0:a:0]aformat=channel_layouts=stereo[sys];[0:a:1]{center}[mic];[sys][mic]amix=inputs=2:normalize=0,"),
            (false, true) => format!("[0:a:0]{center},"),
            _ => "[0:a:0]".into(),
        };
        // Mute the PipeWire capture-open pop, then normalize to -14 LUFS.
        graph = format!("{mix}volume=enable='lt(t,0.4)':volume=0,afade=t=in:st=0.4:d=0.05,loudnorm=I=-14:TP=-1.5:LRA=11[a]");
        args.extend(["-filter_complex", &graph, "-map", "0:v:0", "-map", "[a]"]);
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
    if tail.is_empty() { t("record.gpu_screen_recorder_exited_without").into() } else { tail }
}

/// A countdown over the area before recording starts, like CleanShot's.
/// False when cancelled with Esc.
pub(crate) async fn countdown(target: &Target, seconds: u32, mons: &[Mon], cx: &mut AsyncApp) -> bool {
    if seconds == 0 {
        return true;
    }
    let (mon, area) = match *target {
        Target::Region { x, y, w, h } => {
            let Some(mon) = mons.iter().find(|m| m.contains(x + w / 2, y + h / 2)) else { return true };
            (mon.clone(), Some(((x - mon.x) as f32, (y - mon.y) as f32, w as f32, h as f32)))
        }
        Target::Monitor(ref name) => {
            let Some(mon) = mons.iter().find(|m| &m.name == name) else { return true };
            (mon.clone(), None)
        }
    };
    let center = area.map(|(x, y, w, h)| (x + w / 2., y + h / 2.)).unwrap_or((mon.w as f32 / 2., mon.h as f32 / 2.));
    let left = Rc::new(Cell::new(seconds));
    let cancelled = Rc::new(Cell::new(false));
    let spec = LayerSpec {
        namespace: "kadr-countdown",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mon.w as f32, mon.h as f32),
        keyboard: KeyboardInteractivity::Exclusive,
        ignore_zones: true,
        monitor: Some(mon),
    };
    let (shown_left, shown_cancel) = (left.clone(), cancelled.clone());
    let window = cx.update(|cx| {
        kit::open_layer(cx, spec, move |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                Countdown { left: shown_left, cancelled: shown_cancel, center, area, focus }
            })
        })
    });
    let Some(window) = window else { return true };
    for n in (1..=seconds).rev() {
        left.set(n);
        let _ = window.update(cx, |_, _, cx| cx.notify());
        // Tick in small steps so Esc answers at once.
        for _ in 0..10 {
            cx.background_executor().timer(Duration::from_millis(100)).await;
            if cancelled.get() {
                break;
            }
        }
        if cancelled.get() {
            break;
        }
    }
    let _ = window.update(cx, |_, window, _| window.remove_window());
    // Let it leave the screen before the recorder looks at it.
    cx.background_executor().timer(Duration::from_millis(150)).await;
    !cancelled.get()
}

struct Countdown {
    left: Rc<Cell<u32>>,
    cancelled: Rc<Cell<bool>>,
    /// Middle of the area, on the monitor.
    center: (f32, f32),
    /// The area on the monitor, framed like in the panel; None for a whole
    /// monitor.
    area: Option<(f32, f32, f32, f32)>,
    focus: gpui::FocusHandle,
}

impl Render for Countdown {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.left.get();
        let side = 150. * kit::SCALE;
        let cancelled = self.cancelled.clone();
        let dim = || div().absolute().bg(gpui::rgba(0x00000073));
        kit::root(cx)
            .relative()
            .track_focus(&self.focus)
            .on_key_down(move |e: &gpui::KeyDownEvent, _, _| {
                if e.keystroke.key == "escape" {
                    cancelled.set(true);
                }
            })
            // The same frame as in the panel; it turns red once recording starts.
            .when_some(self.area, |root, (x, y, w, h)| {
                root.child(dim().top_0().left_0().right_0().h(px(y)))
                    .child(dim().top(px(y + h)).left_0().right_0().bottom_0())
                    .child(dim().top(px(y)).left_0().w(px(x)).h(px(h)))
                    .child(dim().top(px(y)).left(px(x + w)).right_0().h(px(h)))
                    .child(div().absolute().left(px(x - 2.)).top(px(y - 2.)).w(px(w + 4.)).h(px(h + 4.)).border_2().border_color(gpui::white()))
            })
            .child(
                // Only the digit changes from second to second, like CleanShot.
                div()
                    .absolute()
                    .left(px(self.center.0 - side / 2.))
                    .top(px(self.center.1 - side / 2.))
                    .size(px(side))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .elevation_3(cx)
                    .rounded(px(side * 0.28))
                    .child(div().text_size(px(side * 0.46)).line_height(px(side * 0.56)).font_weight(gpui::FontWeight::SEMIBOLD).child(n.to_string()))
                    .child(Label::new(t("record.esc_cancel")).size(LabelSize::XSmall).color(Color::Muted)),
            )
    }
}
