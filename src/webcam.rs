//! The camera bubble, like CleanShot's: a live camera picture over the area
//! being recorded. It's set up before recording: dragged, it snaps to a
//! corner or the middle of an edge at a fixed margin; on hover it grows,
//! shrinks and changes its rounding. The look is remembered and the studio
//! starts from it.

use std::sync::Arc;

use gpui::{App, Context, Global, MouseButton, MouseDownEvent, MouseMoveEvent, RenderImage, Window, WindowHandle, canvas, div, px};

use crate::kit::{self, Anchor, InputRegion, KeyboardInteractivity, LayerSpec};
use crate::ui::{ButtonLike, Divider, IconButtonShape, Tooltip, prelude::*};
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

#[derive(Default)]
struct Bubble(Option<(WindowHandle<CameraView>, u32)>);

impl Global for Bubble {}

/// Show the camera over `area` (in `mon`'s coordinates), or move it there.
pub fn show(mon: &Mon, area: (f32, f32, f32, f32), cx: &mut App) {
    if let Some((handle, _)) = cx.try_global::<Bubble>().and_then(|b| b.0) {
        let _ = handle.update(cx, |view, _, cx| {
            view.area = area;
            cx.notify();
        });
        return;
    }
    // KADR_FAKE_CAMERA=<lavfi source> stands in for a camera when trying
    // the bubble out, e.g. "testsrc2=size=1280x720:rate=30".
    let input: Vec<String> = match std::env::var("KADR_FAKE_CAMERA") {
        Ok(source) => vec!["-f".into(), "lavfi".into(), "-i".into(), source],
        Err(_) => {
            let Some(device) = device() else {
                return util::notify(t("Камера не найдена"), t("Запись пойдёт без камеры"));
            };
            let capture = capture_size(&device);
            ["-f", "v4l2", "-framerate", "30", "-video_size", capture, "-i", &device].map(String::from).to_vec()
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
        return util::notify(t("Камера не запустилась"), t("Нужен ffmpeg"));
    };
    let pid = child.id();
    let (mut tx, mut rx) = futures::channel::mpsc::channel::<Vec<u8>>(2);
    std::thread::spawn(move || {
        use std::io::Read;
        let Some(mut out) = child.stdout.take() else { return };
        loop {
            let mut buf = vec![0u8; LIVE * LIVE * 4];
            if out.read_exact(&mut buf).is_err() || futures::executor::block_on(futures::SinkExt::send(&mut tx, buf)).is_err() {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    });

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
    let Some(handle) = kit::open_layer(cx, spec, move |_, cx| {
        cx.new(|_| CameraView { frame: None, stale: Vec::new(), area, look: Look::load(), drag: None, glide: None, hovered: false, locked: false, region: InputRegion::default(), _hold: hold })
    }) else {
        util::signal(pid, util::Signal::Terminate);
        return;
    };
    cx.set_global(Bubble(Some((handle, pid))));
    cx.spawn(async move |cx| {
        while let Some(buf) = futures::StreamExt::next(&mut rx).await {
            let Some(frame) = image::RgbaImage::from_raw(LIVE as u32, LIVE as u32, buf) else { break };
            let frame = Arc::new(RenderImage::new([image::Frame::new(frame)]));
            let alive = handle.update(cx, |view, _, cx| {
                if let Some(old) = view.frame.replace(frame) {
                    view.stale.push(old);
                }
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Recording started: set up is over.
pub fn lock(cx: &mut App) {
    if let Some((handle, _)) = cx.try_global::<Bubble>().and_then(|b| b.0) {
        let _ = handle.update(cx, |view, _, cx| {
            view.locked = true;
            view.hovered = false;
            view.drag = None;
            cx.notify();
        });
    }
}

/// Close the bubble and free the camera.
pub fn hide(cx: &mut App) {
    if let Some((handle, pid)) = cx.try_global::<Bubble>().and_then(|b| b.0) {
        util::signal(pid, util::Signal::Terminate);
        let _ = handle.update(cx, |_, window, _| window.remove_window());
        cx.set_global(Bubble(None));
    }
}

struct CameraView {
    frame: Option<Arc<RenderImage>>,
    /// Frames no longer shown, freed from the atlas on render.
    stale: Vec<Arc<RenderImage>>,
    /// The recorded area on this monitor.
    area: (f32, f32, f32, f32),
    look: Look,
    /// While dragged: the pointer's offset inside the bubble, and where the
    /// bubble's top left is now.
    drag: Option<((f32, f32), (f32, f32))>,
    /// After a drop: where the bubble was let go and when; it glides from
    /// there to its spot.
    glide: Option<((f32, f32), std::time::Instant)>,
    hovered: bool,
    /// Recording: the bubble is part of the picture, so it shows no controls
    /// and lets clicks through.
    locked: bool,
    region: InputRegion,
    _hold: kit::Hold,
}

impl CameraView {
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
        let i = SIZES.iter().position(|&s| (s - self.look.size).abs() < 0.001).unwrap_or(2) as i32;
        self.look.size = SIZES[(i + step).clamp(0, SIZES.len() as i32 - 1) as usize];
        self.look.save();
    }
}

impl Render for CameraView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for old in self.stale.drain(..) {
            let _ = window.drop_image(old);
        }
        self.region.reset();
        let (x, y, side) = place(self.look.anchor, self.look.size, self.area());
        let (mut left, mut top) = self.drag.map(|(_, at)| at).unwrap_or((x as f32, y as f32));
        if let Some((from, since)) = self.glide {
            let t = (since.elapsed().as_secs_f32() / GLIDE).min(1.);
            if t < 1. {
                // Ease out: quick at first, settling into place.
                let k = 1. - (1. - t).powi(3);
                left = from.0 + (left - from.0) * k;
                top = from.1 + (top - from.1) * k;
                window.request_animation_frame();
            } else {
                self.glide = None;
            }
        }
        let side = side as f32;
        let radius = self.look.round as f32 * side / 2.;
        let frame = self.frame.clone();
        let picture = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                if let Some(frame) = frame.clone() {
                    let _ = window.paint_image(bounds, bounds, px(radius).into(), 0., frame, 0, false);
                }
            },
        )
        .size_full();
        if self.locked {
            window.set_input_region(Some(&[]));
        }
        let shown = !self.locked && self.hovered && window.is_window_hovered() && self.drag.is_none();
        // Controls under the bubble, or above it at the bottom of the area.
        let below = top + side + 52. < self.area.1 + self.area.3 + 40.;
        let controls = shown.then(|| {
            let round = |i: usize| {
                let r = ROUNDS[i];
                let on = (self.look.round - r).abs() < 0.01;
                ButtonLike::new(("cam-round", i))
                    .style(if on { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(on)
                    .child(div().px_1().child(div().size(px(14.)).rounded(px(7. * r as f32)).border_2().border_color(cx.theme().colors().text)))
                    .tooltip(Tooltip::text([t("Круг"), t("Скруглённая"), t("Квадрат")][i]))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.look.round = ROUNDS[i];
                        this.look.save();
                        cx.notify();
                    }))
            };
            h_flex()
                .id("cam-controls")
                .absolute()
                .left(px(left + side / 2. - 110.))
                .w(px(220.))
                .when(below, |d| d.top(px(top + side + 8.)))
                .when(!below, |d| d.top(px(top - 48.)))
                .justify_center()
                .child(
                    h_flex()
                        .gap_0p5()
                        .p_1()
                        .elevation_3(cx)
                        .child(
                            IconButton::new("cam-smaller", IconName::Dash)
                                .shape(IconButtonShape::Square)
                                .size(ButtonSize::Large)
                                .tooltip(Tooltip::text("Меньше"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.resize(-1);
                                    cx.notify();
                                })),
                        )
                        .child(
                            IconButton::new("cam-bigger", IconName::Plus)
                                .shape(IconButtonShape::Square)
                                .size(ButtonSize::Large)
                                .tooltip(Tooltip::text("Больше"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.resize(1);
                                    cx.notify();
                                })),
                        )
                        .child(Divider::vertical().h_5().mx_1())
                        .child(round(0))
                        .child(round(1))
                        .child(round(2))
                        .child(self.region.mark()),
                )
        });
        kit::root(cx)
            .relative()
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if let Some((grab, _)) = this.drag {
                    let p = (f32::from(e.position.x), f32::from(e.position.y));
                    this.drag = Some((grab, (p.0 - grab.0, p.1 - grab.1)));
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
                if let Some((_, at)) = this.drag.take() {
                    this.snap(at);
                    this.glide = Some((at, std::time::Instant::now()));
                    cx.notify();
                }
            }))
            .child(
                div()
                    .id("cam-hover")
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(side))
                    .h(px(side + if below { 60. } else { 0. }))
                    .when(!below, |d| d.top(px(top - 56.)).h(px(side + 56.)))
                    .on_hover(cx.listener(|this, on: &bool, _, cx| {
                        this.hovered = *on;
                        cx.notify();
                    }))
                    // The bubble and the gap to its controls stay under the
                    // pointer; the rest of the screen lets clicks through.
                    .when(shown, |d| d.child(self.region.mark())),
            )
            .child(
                div()
                    .id("cam")
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(side))
                    .h(px(side))
                    .rounded(px(radius))
                    .bg(gpui::black())
                    .shadow(kit::shadow())
                    .when(!self.locked, |d| d.cursor_grab())
                    .on_hover(cx.listener(|this, on: &bool, _, cx| {
                        if *on {
                            this.hovered = true;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        if this.locked {
                            return;
                        }
                        let p = (f32::from(e.position.x), f32::from(e.position.y));
                        this.drag = Some(((p.0 - left, p.1 - top), (left, top)));
                        cx.notify();
                    }))
                    .child(picture)
                    .when(!self.locked, |d| d.child(self.region.mark())),
            )
            .children(controls)
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
