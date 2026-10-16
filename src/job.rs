//! Long ffmpeg jobs (studio render, trimming, GIF export) with a Zed-style
//! progress toast at the bottom of the screen that can cancel them.

use std::cell::Cell;
use std::ffi::OsStr;
use std::process::Stdio;
use std::rc::Rc;
use std::time::Duration;

use futures::{AsyncBufReadExt, StreamExt, io::BufReader};
use gpui::{App, AsyncApp, Context, SharedString, Window, WindowHandle, layer_shell::*};
use ui::{IconButtonShape, ProgressBar, Tooltip, prelude::*};

use crate::kit::{self, Hold, InputRegion, LayerSpec};
use crate::util;

/// Shared between the running job and its toast.
#[derive(Default)]
pub struct Job {
    fraction: Cell<f64>,
    cancelled: Cell<bool>,
    pid: Cell<Option<u32>>,
}

impl Job {
    pub fn cancelled(&self) -> bool {
        self.cancelled.get()
    }
}

/// A running job: keeps kadr alive and its toast on screen until `finish`.
pub struct Running {
    pub job: Rc<Job>,
    window: Option<WindowHandle<ProgressView>>,
    _hold: Hold,
}

pub fn start(title: &'static str, icon: IconName, cx: &mut App) -> Running {
    let job = Rc::new(Job::default());
    let spec = LayerSpec {
        namespace: "kadr-progress",
        anchor: Anchor::BOTTOM,
        margin: (0., 0., 24., 0.),
        size: (340. * kit::SCALE, 90. * kit::SCALE),
        keyboard: KeyboardInteractivity::None,
        ignore_zones: false,
        monitor: util::focused_monitor(),
    };
    let state = job.clone();
    let window = kit::open_layer(cx, spec, |window, cx| cx.new(|cx| ProgressView::new(title.into(), icon, state, window, cx)));
    Running { job, window, _hold: kit::hold(cx) }
}

impl Running {
    pub fn finish(self, cx: &mut AsyncApp) {
        if let Some(w) = self.window {
            let _ = w.update(cx, |_, window, _| window.remove_window());
        }
    }
}

/// Run ffmpeg (argv without "-progress", which is added) and report progress
/// against `duration` seconds of output. stderr goes to ~/.cache/kadr/<log>.
pub async fn ffmpeg(job: &Job, argv: &[&OsStr], duration: f64, log: &str) -> Result<(), String> {
    let log = util::cache_dir().join(log);
    let _ = std::fs::create_dir_all(util::cache_dir());
    let stderr = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let (cmd, rest) = argv.split_first().ok_or("пустая команда")?;
    let mut child = async_process::Command::new(cmd)
        .args(["-nostats", "-progress", "pipe:1"])
        .args(rest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .map_err(|e| e.to_string())?;
    job.pid.set(Some(child.id()));

    let stdout = child.stdout.take().ok_or("нет вывода ffmpeg")?;
    let mut lines = BufReader::new(stdout).lines();
    while let Some(Ok(line)) = lines.next().await {
        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
            job.fraction.set(us / 1e6 / duration.max(0.1));
        }
    }
    let status = child.status().await.map_err(|e| e.to_string())?;
    job.pid.set(None);
    if status.success() {
        Ok(())
    } else {
        let tail = std::fs::read_to_string(&log).unwrap_or_default();
        Err(tail.lines().rev().take(3).collect::<Vec<_>>().join("\n"))
    }
}

/// Small notification at the bottom of the screen while ffmpeg works.
pub struct ProgressView {
    title: SharedString,
    icon: IconName,
    state: Rc<Job>,
    region: InputRegion,
}

impl ProgressView {
    fn new(title: SharedString, icon: IconName, state: Rc<Job>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        ProgressView { title, icon, state, region: InputRegion::default() }
    }
}

impl Render for ProgressView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.region.reset();
        let fraction = self.state.fraction.get().clamp(0.0, 0.99) as f32;
        let state = self.state.clone();
        kit::root(cx).flex().items_center().justify_center().child(
            v_flex()
                .relative()
                .w(px(300.))
                .gap_1p5()
                .pl_2p5()
                .pr_1()
                .pt_1()
                .pb_2p5()
                .elevation_3(cx)
                .child(
                    h_flex()
                        .gap_1p5()
                        .child(Icon::new(self.icon).size(IconSize::Small).color(Color::Accent))
                        .child(div().flex_1().child(Label::new(self.title.clone()).size(LabelSize::Small)))
                        .child(Label::new(format!("{}%", (fraction * 100.) as u32)).size(LabelSize::Small).color(Color::Muted))
                        .child(
                            IconButton::new("cancel", IconName::Close)
                                .shape(IconButtonShape::Square)
                                .icon_size(IconSize::XSmall)
                                .tooltip(Tooltip::text("Отменить"))
                                .on_click(move |_, _, _| {
                                    state.cancelled.set(true);
                                    if let Some(pid) = state.pid.get() {
                                        util::signal(pid, libc::SIGTERM);
                                    }
                                }),
                        ),
                )
                .child(div().pr_1p5().child(ProgressBar::new("progress", fraction, 1., cx)))
                .child(self.region.mark()),
        )
    }
}
