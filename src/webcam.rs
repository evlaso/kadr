//! The camera bubble, like CleanShot's: a live camera picture over the area
//! being recorded. It's set up before recording: dragged, it snaps to a
//! corner or the middle of an edge at a fixed margin; on hover it grows,
//! shrinks and changes its rounding. The look is remembered and the studio
//! starts from it.

use std::sync::Arc;

use gpui::{App, Context, CursorStyle, Global, MouseButton, MouseDownEvent, RenderImage, Window, WindowHandle, canvas, div, px};

use crate::kit::{self, Anchor, KeyboardInteractivity, LayerSpec};
use crate::ui::{Tooltip, prelude::*};
use crate::util::{self, Mon};
use crate::i18n::t;

/// Where the camera can sit, as fractions across the free room: corners and
/// the middles of the edges, clockwise from the top left.
pub const ANCHORS: [(f64, f64); 8] = [(0., 0.), (0.5, 0.), (1., 0.), (1., 0.5), (1., 1.), (0.5, 1.), (0., 1.), (0., 0.5)];
/// Sizes, as a share of the area's short side.
pub const SIZES: [f64; 6] = [0.16, 0.21, 0.26, 0.32, 0.4, 0.5];
/// Rounding: a circle, rounded corners, square.
pub const ROUNDS: [f64; 3] = [1., 0.4, 0.];

/// The camera's look, kept between recordings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub anchor: usize,
    pub size: f64,
    /// 0 square … 1 circle.
    pub round: f64,
}

impl Default for Look {
    fn default() -> Self {
        Look { anchor: 4, size: 0.26, round: 1. }
    }
}

impl Look {
    fn path() -> std::path::PathBuf {
        util::config_dir().join("camera.json")
    }

    pub fn load() -> Look {
        let v: serde_json::Value = std::fs::read_to_string(Self::path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Look::from_json(&v)
    }

    pub fn from_json(v: &serde_json::Value) -> Look {
        let d = Look::default();
        Look {
            anchor: v["anchor"].as_u64().map_or(d.anchor, |a| (a as usize).min(ANCHORS.len() - 1)),
            size: v["size"].as_f64().unwrap_or(d.size).clamp(0.1, 0.6),
            round: v["round"].as_f64().unwrap_or(d.round).clamp(0., 1.),
        }
    }

    pub fn to_json(self) -> serde_json::Value {
        serde_json::json!({ "anchor": self.anchor, "size": self.size, "round": self.round })
    }

    pub fn save(self) {
        let _ = std::fs::create_dir_all(util::config_dir());
        let _ = std::fs::write(Self::path(), self.to_json().to_string());
    }
}

/// The camera's box inside an area (x, y, w, h): left, top and side.
pub fn place(anchor: usize, size_share: f64, (x, y, w, h): (f64, f64, f64, f64)) -> (f64, f64, f64) {
    let short = w.min(h);
    let side = (size_share * short).max(8.);
    let margin = short * 0.04;
    let (fx, fy) = ANCHORS[anchor.min(ANCHORS.len() - 1)];
    (x + margin + fx * (w - side - 2. * margin).max(0.), y + margin + fy * (h - side - 2. * margin).max(0.), side)
}

/// The first camera Omarchy knows about.
pub fn device() -> Option<String> {
    let out = std::process::Command::new("omarchy-capture-webcam-list").output().ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(str::to_owned).or_else(|| std::path::Path::new("/dev/video0").exists().then(|| "/dev/video0".into()))
}

/// The largest usual capture size the camera offers.
pub fn capture_size(device: &str) -> &'static str {
    let formats = std::process::Command::new("v4l2-ctl").args(["--list-formats-ext", "-d", device]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    ["1280x720", "1920x1080", "640x480", "640x360"].into_iter().find(|r| formats.contains(r)).unwrap_or("640x480")
}

/// Seconds the bubble takes to settle into its spot after a drop.
const GLIDE: f32 = 0.22;

/// Side of the live picture, in pixels.
const LIVE: usize = 360;

/// The bubble's state, shared by whichever window draws it: the recording
/// panel while the camera is being set up (the panel surely gets the
/// pointer), the camera's own layer once recording starts.
pub struct BubbleState {
    frame: Option<Arc<RenderImage>>,
    /// Frames no longer shown, freed from the drawing window's atlas.
    stale: Vec<Arc<RenderImage>>,
    /// The recorded area on its monitor.
    area: (f32, f32, f32, f32),
    look: Look,
    /// While dragged: the pointer's offset inside the bubble, and where the
    /// bubble's top left is now.
    drag: Option<((f32, f32), (f32, f32))>,
    /// While resized by its edge: its centre and the pointer's distance from
    /// it when grabbed, and the size then.
    resizing: Option<((f32, f32), f32, f64)>,
    /// After a drop: where the bubble was let go and when; it glides from
    /// there to its spot.
    glide: Option<((f32, f32), std::time::Instant)>,
    hovered: bool,
    /// The panel draws it (set up); otherwise the camera layer does, as part
    /// of the picture (recording).
    hosted: bool,
}

impl BubbleState {
    fn area(&self) -> (f64, f64, f64, f64) {
        let (x, y, w, h) = self.area;
        (x as f64, y as f64, w as f64, h as f64)
    }

    /// The anchor whose spot is closest to where the bubble was dropped.
    fn snap(&mut self, top_left: (f32, f32)) {
        let best = (0..ANCHORS.len())
            .min_by(|&a, &b| {
                let d = |i: usize| {
                    let (x, y, _) = place(i, self.look.size, self.area());
                    (x - top_left.0 as f64).hypot(y - top_left.1 as f64)
                };
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(self.look.anchor);
        self.look.anchor = best;
        self.look.save();
    }

    fn resize(&mut self, step: i32) {
        // The nearest step: a size dragged by the edge is in between.
        let i = (0..SIZES.len()).min_by(|&a, &b| (SIZES[a] - self.look.size).abs().total_cmp(&(SIZES[b] - self.look.size).abs())).unwrap_or(2) as i32;
        self.look.size = SIZES[(i + step).clamp(0, SIZES.len() as i32 - 1) as usize];
        self.look.save();
    }
}

#[derive(Default)]
struct Bubble(Option<(gpui::Entity<BubbleState>, WindowHandle<CameraView>, u32)>);

impl Global for Bubble {}

/// The bubble's state while the camera is on, for the panel to draw.
pub fn state(cx: &App) -> Option<gpui::Entity<BubbleState>> {
    cx.try_global::<Bubble>().and_then(|b| b.0.as_ref().map(|(s, _, _)| s.clone()))
}

/// Show the camera over `area` (in `mon`'s coordinates), or move it there.
/// The panel draws it while it's open (`hosted`).
pub fn show(mon: &Mon, area: (f32, f32, f32, f32), hosted: bool, cx: &mut App) {
    if let Some(state) = state(cx) {
        state.update(cx, |s, cx| {
            s.area = area;
            s.hosted = hosted;
            cx.notify();
        });
        return;
    }
    // KADR_FAKE_CAMERA=<lavfi source> stands in for a camera when trying
    // the bubble out, e.g. "testsrc2=size=1280x720:rate=30".
    let mut camera = None;
    let input: Vec<String> = match std::env::var("KADR_FAKE_CAMERA") {
        Ok(source) => vec!["-f".into(), "lavfi".into(), "-i".into(), source],
        Err(_) => {
            let Some(device) = device() else {
                return util::notify(t("common.no_camera_found"), t("common.recording_without_camera"));
            };
            let capture = capture_size(&device);
            let args = ["-f", "v4l2", "-framerate", "30", "-video_size", capture, "-i", &device].map(String::from).to_vec();
            camera = Some(device);
            args
        }
    };
    let Ok(mut child) = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error"])
        .args(&input)
        .args(["-vf", &format!("crop='min(iw,ih)':'min(iw,ih)',scale={LIVE}:{LIVE}"), "-f", "rawvideo", "-pix_fmt", "bgra", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return util::notify(t("common.camera_didnt_start"), t("camera.ffmpeg_needed"));
    };
    let pid = child.id();
    let (mut tx, mut rx) = futures::channel::mpsc::channel::<Vec<u8>>(2);
    std::thread::spawn(move || {
        use std::io::Read;
        let Some(mut out) = child.stdout.take() else { return };
        let mut shown = false;
        loop {
            let mut buf = vec![0u8; LIVE * LIVE * 4];
            if out.read_exact(&mut buf).is_err() {
                // ffmpeg gave up before a single frame, most often because
                // another app has the camera (a stop from kadr kills it,
                // which leaves no exit code).
                let failed = child.wait().ok().and_then(|status| status.code()).is_some_and(|code| code != 0);
                if failed && !shown {
                    let holder = camera.as_deref().and_then(crate::platform::device_holder);
                    match holder {
                        Some(app) => util::notify(t("camera.busy"), &crate::tr!("camera.busy_with", app)),
                        None => util::notify(t("common.camera_didnt_start"), t("camera.busy_unknown")),
                    }
                }
                return;
            }
            shown = true;
            if futures::executor::block_on(futures::SinkExt::send(&mut tx, buf)).is_err() {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    });

    let state = cx.new(|_| BubbleState { frame: None, stale: Vec::new(), area, look: Look::load(), drag: None, resizing: None, glide: None, hovered: false, hosted });
    let spec = LayerSpec {
        namespace: "kadr-camera",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mon.w as f32, mon.h as f32),
        keyboard: KeyboardInteractivity::None,
        ignore_zones: true,
        monitor: Some(mon.clone()),
    };
    let hold = kit::hold(cx);
    let view_state = state.clone();
    let Some(handle) = kit::open_layer(cx, spec, move |_, cx| {
        cx.new(|cx| {
            cx.observe(&view_state, |_, _, cx| cx.notify()).detach();
            CameraView { state: view_state, _hold: hold }
        })
    }) else {
        util::signal(pid, util::Signal::Terminate);
        return;
    };
    cx.set_global(Bubble(Some((state.clone(), handle, pid))));
    cx.spawn(async move |cx| {
        while let Some(buf) = futures::StreamExt::next(&mut rx).await {
            let Some(frame) = image::RgbaImage::from_raw(LIVE as u32, LIVE as u32, buf) else { break };
            let frame = Arc::new(RenderImage::new([image::Frame::new(frame)]));
            // Hidden (or replaced by a new bubble): stop.
            if cx.update(|cx| self::state(cx).map(|s| s.entity_id())) != Some(state.entity_id()) {
                break;
            }
            state.update(cx, |s, cx| {
                if let Some(old) = s.frame.replace(frame) {
                    s.stale.push(old);
                }
                cx.notify();
            });
        }
    })
    .detach();
}

/// Recording started: the bubble becomes part of the picture, drawn by its
/// own layer, with no controls.
pub fn lock(cx: &mut App) {
    if let Some(state) = state(cx) {
        state.update(cx, |s, cx| {
            s.hosted = false;
            s.hovered = false;
            s.drag = None;
            s.resizing = None;
            cx.notify();
        });
    }
}

/// Close the bubble and free the camera.
pub fn hide(cx: &mut App) {
    if let Some((_, handle, pid)) = cx.try_global::<Bubble>().and_then(|b| b.0.clone()) {
        util::signal(pid, util::Signal::Terminate);
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        cx.set_global(Bubble(None));
    }
}

/// The bubble is being dragged: the host takes the pointer everywhere
/// meanwhile, so a quick move can't slip off the bubble and the release
/// isn't lost to the window beneath.
pub fn dragging(state: &gpui::Entity<BubbleState>, cx: &App) -> bool {
    let s = state.read(cx);
    s.drag.is_some() || s.resizing.is_some()
}

/// The pointer moved over the host window: drag the bubble along.
pub fn pointer_moved(state: &gpui::Entity<BubbleState>, position: gpui::Point<gpui::Pixels>, pressed: Option<MouseButton>, cx: &mut App) {
    // The button came up somewhere the host didn't hear it: drop it here.
    if pressed != Some(MouseButton::Left) {
        return pointer_released(state, cx);
    }
    let p = (f32::from(position.x), f32::from(position.y));
    state.update(cx, |s, cx| {
        // By the edge: as much bigger as the pointer got farther from the
        // centre, between the smallest and the biggest step.
        if let Some((centre, from, size)) = s.resizing {
            let now = (p.0 - centre.0).hypot(p.1 - centre.1);
            s.look.size = (size * (now / from.max(1.)) as f64).clamp(SIZES[0], SIZES[SIZES.len() - 1]);
            cx.notify();
        }
        if let Some((grab, _)) = s.drag {
            s.drag = Some((grab, (p.0 - grab.0, p.1 - grab.1)));
            cx.notify();
        }
    });
}

/// The button came up over the host window: drop the bubble into its spot.
pub fn pointer_released(state: &gpui::Entity<BubbleState>, cx: &mut App) {
    state.update(cx, |s, cx| {
        if s.resizing.take().is_some() {
            s.look.save();
            cx.notify();
        }
        if let Some((_, at)) = s.drag.take() {
            s.snap(at);
            s.glide = Some((at, std::time::Instant::now()));
            cx.notify();
        }
    });
}

/// The bubble as an element in monitor coordinates, for a full-monitor
/// window. `interactive` adds dragging, the wheel and the hover controls;
/// `mark` (the host's input region) goes inside it so it takes the pointer.
pub fn bubble(state: &gpui::Entity<BubbleState>, interactive: bool, mark: Option<AnyElement>, window: &mut Window, cx: &mut App) -> AnyElement {
    let stale: Vec<_> = state.update(cx, |s, _| s.stale.drain(..).collect());
    for old in stale {
        let _ = window.drop_image(old);
    }
    let s = state.read(cx);
    let (x, y, side) = place(s.look.anchor, s.look.size, s.area());
    let (mut left, mut top) = s.drag.map(|(_, at)| at).unwrap_or((x as f32, y as f32));
    let mut gliding = false;
    if let Some((from, since)) = s.glide {
        let t = (since.elapsed().as_secs_f32() / GLIDE).min(1.);
        if t < 1. {
            // Ease out: quick at first, settling into place.
            let k = 1. - (1. - t).powi(3);
            left = from.0 + (left - from.0) * k;
            top = from.1 + (top - from.1) * k;
            gliding = true;
            window.request_animation_frame();
        }
    }
    let side = side as f32;
    let radius = s.look.round as f32 * side / 2.;
    let frame = s.frame.clone();
    let round = s.look.round;
    let shown = interactive && s.hovered && s.drag.is_none() && s.resizing.is_none();
    let anchor = ANCHORS[s.look.anchor.min(ANCHORS.len() - 1)];
    if !gliding && s.glide.is_some() {
        state.update(cx, |s, _| s.glide = None);
    }
    let picture = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if let Some(frame) = frame.clone() {
                let _ = window.paint_image(bounds, bounds, px(radius).into(), crate::ui::SMOOTHING, frame, 0, false);
            }
        },
    )
    .size_full();

    // Controls inside the bubble, along its bottom, while hovered.
    let controls = shown.then(|| {
        let ink = gpui::white();
        let button = |id: &'static str, icon: IconName, tip: &'static str| {
            div()
                .id(id)
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .hover(|d| d.bg(gpui::hsla(0., 0., 1., 0.2)))
                .child(Icon::new(icon).size(IconSize::Small).color(Color::Custom(ink)))
                .tip(Tooltip::text(t(tip)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let next_round = ROUNDS[(ROUNDS.iter().position(|&r| (r - round).abs() < 0.01).unwrap_or(0) + 1) % ROUNDS.len()];
        let (smaller, bigger, shape) = (state.clone(), state.clone(), state.clone());
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px((side * 0.08).max(6.)))
            .flex()
            .justify_center()
            .child(
                h_flex()
                    .gap_0p5()
                    .p_0p5()
                    .rounded_full()
                    .bg(gpui::hsla(0., 0., 0., 0.55))
                    .child(button("cam-smaller", IconName::Dash, "camera.smaller").on_click(move |_, _, cx| smaller.update(cx, |s, cx| {
                        s.resize(-1);
                        cx.notify();
                    })))
                    .child(button("cam-bigger", IconName::Plus, "camera.bigger").on_click(move |_, _, cx| bigger.update(cx, |s, cx| {
                        s.resize(1);
                        cx.notify();
                    })))
                    .child(
                        div()
                            .id("cam-round")
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .hover(|d| d.bg(gpui::hsla(0., 0., 1., 0.2)))
                            .child(div().size(px(12.)).rounded(px(6. * next_round as f32)).rounded_smoothing(crate::ui::SMOOTHING).border_2().border_color(ink))
                            .tip(Tooltip::text(t("camera.shape")))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, _, cx| shape.update(cx, |s, cx| {
                                s.look.round = next_round;
                                s.look.save();
                                cx.notify();
                            })),
                    ),
            )
    });

    let (hover, wheel, press, grow) = (state.clone(), state.clone(), state.clone(), state.clone());
    let bubble = div()
        .id("cam")
        .absolute()
        .when(!interactive, |d| d.left(px(left)).top(px(top)))
        .when(interactive, |d| d.left(px(EDGE)).top(px(EDGE)))
        .w(px(side))
        .h(px(side))
        // A squircle, not a plain rounded square (a circle stays a circle).
        .rounded(px(radius))
        .rounded_smoothing(crate::ui::SMOOTHING)
        .bg(gpui::black())
        .shadow(kit::shadow())
        .when(interactive, |d| {
            // Presses here don't reach the recording area beneath.
            d.occlude()
                .cursor_grab()
                .on_hover(move |on: &bool, _, cx| hover.update(cx, |s, cx| {
                    s.hovered = *on;
                    cx.notify();
                }))
                // The wheel resizes too.
                .on_scroll_wheel(move |e: &gpui::ScrollWheelEvent, _, cx| {
                    let dy = f32::from(e.delta.pixel_delta(px(20.)).y);
                    if dy.abs() > 0.5 {
                        wheel.update(cx, |s, cx| {
                            s.resize(if dy > 0. { 1 } else { -1 });
                            cx.notify();
                        });
                    }
                })
                .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let p = (f32::from(e.position.x), f32::from(e.position.y));
                    press.update(cx, |s, cx| {
                        s.drag = Some(((p.0 - left, p.1 - top), (left, top)));
                        cx.notify();
                    });
                })
        })
        .child(picture)
        .children(controls);
    if !interactive {
        return bubble.into_any_element();
    }
    // A band round the bubble resizes it by dragging, the way it grows: away
    // from the corner or edge it keeps to.
    let cursor = if (anchor.0 - 0.5) * (anchor.1 - 0.5) < 0. { CursorStyle::ResizeUpRightDownLeft } else { CursorStyle::ResizeUpLeftDownRight };
    let centre = (left + side / 2., top + side / 2.);
    div()
        .id("cam-edge")
        .absolute()
        .left(px(left - EDGE))
        .top(px(top - EDGE))
        .size(px(side + 2. * EDGE))
        .rounded(px(radius + EDGE))
        .rounded_smoothing(crate::ui::SMOOTHING)
        .occlude()
        .cursor(cursor)
        .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, _, cx| {
            let p = (f32::from(e.position.x), f32::from(e.position.y));
            grow.update(cx, |s, cx| {
                s.resizing = Some((centre, (p.0 - centre.0).hypot(p.1 - centre.1), s.look.size));
                cx.notify();
            });
        })
        .child(bubble)
        .children(mark)
        .into_any_element()
}

/// How far round the bubble its edge can be grabbed to resize it.
const EDGE: f32 = 10.;

/// The camera's own layer: empty and click-through while the panel draws
/// the bubble, the bubble itself (as part of the picture) once recording.
struct CameraView {
    state: gpui::Entity<BubbleState>,
    _hold: kit::Hold,
}

impl Render for CameraView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_input_region(Some(&[]));
        let root = kit::root(cx).relative();
        if self.state.read(cx).hosted {
            return root;
        }
        let state = self.state.clone();
        root.child(bubble(&state, false, None, window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_snaps_inside_the_area_at_a_margin() {
        let area = (100., 50., 1000., 500.);
        // Bottom right: the margin is 4% of the short side.
        let (x, y, side) = place(4, 0.2, area);
        assert_eq!(side, 100.);
        assert!((x + side - (1100. - 20.)).abs() < 1e-9 && (y + side - (550. - 20.)).abs() < 1e-9);
        // The top edge's middle.
        let (x, y, _) = place(1, 0.2, area);
        assert!((x + 50. - 600.).abs() < 1e-9 && (y - 70.).abs() < 1e-9);
    }
}
