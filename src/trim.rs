//! Trim a recording and export GIFs, CleanShot-style: a modal with a large
//! frame preview, a filmstrip timeline with start/end handles, and Save
//! (a new "-trim" file) or GIF. Quick GIF export from a card skips the modal.

use std::cell::Cell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::kit::{Anchor, KeyboardInteractivity};
use gpui::{
    App, BorderStyle, Bounds, Context, CursorStyle, FocusHandle, Image, ImageFormat, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, Window, canvas, div, img,
    point, px, rgba, size,
};
use crate::ui::{ButtonLike, Divider, Tooltip, prelude::*};

use crate::ui::{fill, outline};
use crate::kit::{self, LayerSpec};
use crate::{job, track, util};
use crate::i18n::t;

const THUMBS: usize = 14;
const STRIP_HEIGHT: f32 = 56.;
const GIF_WIDTHS: [u32; 3] = [480, 720, 960];
const GIF_FPS: [u32; 3] = [10, 15, 24];
/// Shortest selection, in seconds.
const MIN_LENGTH: f64 = 0.2;

#[derive(Clone, Copy, PartialEq)]
enum End {
    Start,
    End,
}

pub struct Trimmer {
    path: PathBuf,
    duration: f64,
    video: (u32, u32),
    disp: (f32, f32),
    thumbs: Vec<Arc<Image>>,
    preview: Option<Arc<Image>>,
    preview_at: f64,
    start: f64,
    end: f64,
    drag: Option<End>,
    /// Filmstrip x origin and width in window coordinates.
    strip: Rc<Cell<(f32, f32)>>,
    /// Latest preview request; older frames that arrive late are dropped.
    request: Rc<Cell<u64>>,
    gif_width: usize,
    gif_fps: usize,
    focus: FocusHandle,
}

struct Probe {
    w: u32,
    h: u32,
    duration: f64,
}

async fn probe(path: &Path) -> Option<Probe> {
    let (ok, out) = util::output::<&OsStr>(&[
        "ffprobe".as_ref(), "-v".as_ref(), "error".as_ref(), "-select_streams".as_ref(), "v:0".as_ref(),
        "-show_entries".as_ref(), "stream=width,height:format=duration".as_ref(), "-of".as_ref(), "json".as_ref(), path.as_os_str(),
    ])
    .await?;
    let v: serde_json::Value = serde_json::from_str(&out).ok().filter(|_| ok)?;
    Some(Probe {
        w: v["streams"][0]["width"].as_u64()? as u32,
        h: v["streams"][0]["height"].as_u64()? as u32,
        duration: v["format"]["duration"].as_str()?.parse().ok()?,
    })
}

fn temp_dir() -> PathBuf {
    let dir = util::runtime_dir().join(format!("kadr-trim-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn load_png(path: &Path) -> Option<Arc<Image>> {
    let bytes = std::fs::read(path).ok()?;
    let _ = std::fs::remove_file(path);
    Some(Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
}

/// One frame at `t` seconds, `width` pixels wide.
pub(crate) async fn frame(path: &Path, t: f64, width: u32, tag: u64) -> Option<Arc<Image>> {
    let out = temp_dir().join(format!("frame-{tag}.png"));
    let (ss, scale) = (format!("{t:.3}"), format!("scale={width}:-2"));
    let (ok, _) = util::output::<&OsStr>(&[
        "ffmpeg".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(), "-ss".as_ref(), ss.as_ref(),
        "-i".as_ref(), path.as_os_str(), "-frames:v".as_ref(), "1".as_ref(), "-vf".as_ref(), scale.as_ref(), out.as_os_str(),
    ])
    .await?;
    ok.then(|| load_png(&out)).flatten()
}

/// Evenly spaced thumbnails for the filmstrip.
pub(crate) async fn filmstrip(path: &Path, duration: f64) -> Vec<Arc<Image>> {
    let dir = temp_dir();
    let pattern = dir.join("thumb-%02d.png");
    let filter = format!("fps={}/{:.3},scale=-2:{}", THUMBS, duration.max(0.1), (STRIP_HEIGHT * 2.) as u32);
    let _ = util::output::<&OsStr>(&[
        "ffmpeg".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(), "-i".as_ref(), path.as_os_str(),
        "-vf".as_ref(), filter.as_ref(), "-frames:v".as_ref(), "14".as_ref(), pattern.as_os_str(),
    ])
    .await;
    (1..=THUMBS).filter_map(|i| load_png(&dir.join(format!("thumb-{i:02}.png")))).collect()
}

/// Open the trimming modal for a recording.
pub fn open(path: PathBuf, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        let Some(p) = probe(&path).await else {
            return util::notify(t("common.couldnt_open_video"), &path.display().to_string());
        };
        let Some(mon) = util::focused_monitor() else { return };
        let fit = (mon.w as f32 * 0.7 / p.w as f32).min(mon.h as f32 * 0.5 / p.h as f32);
        let disp = ((p.w as f32 * fit).max(560.), p.h as f32 * fit);
        let thumbs = filmstrip(&path, p.duration).await;
        let preview = frame(&path, 0., (disp.0 * 2.) as u32, 0).await;
        let _ = cx.update(|cx| {
            let spec = LayerSpec {
                namespace: "kadr-trim",
                anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                margin: (0., 0., 0., 0.),
                size: (mon.w as f32, mon.h as f32),
                keyboard: KeyboardInteractivity::Exclusive,
                ignore_zones: true,
                monitor: Some(mon),
            };
            kit::open_layer(cx, spec, |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    Trimmer {
                        path,
                        duration: p.duration,
                        video: (p.w, p.h),
                        disp,
                        thumbs,
                        preview,
                        preview_at: 0.,
                        start: 0.,
                        end: p.duration,
                        drag: None,
                        strip: Rc::new(Cell::new((0., 1.))),
                        request: Rc::new(Cell::new(0)),
                        gif_width: 1,
                        gif_fps: 1,
                        focus,
                    }
                })
            });
        });
    })
    .detach();
}

impl Trimmer {
    fn time_at(&self, x: f32) -> f64 {
        let (ox, w) = self.strip.get();
        (((x - ox) / w.max(1.)) as f64).clamp(0., 1.) * self.duration
    }

    fn set(&mut self, end: End, t: f64) {
        match end {
            End::Start => self.start = t.min(self.end - MIN_LENGTH).max(0.),
            End::End => self.end = t.max(self.start + MIN_LENGTH).min(self.duration),
        }
    }

    /// Show the frame at `t`, keeping only the newest request.
    fn show_frame(&mut self, t: f64, cx: &mut Context<Self>) {
        self.preview_at = t;
        let id = self.request.get() + 1;
        self.request.set(id);
        let (path, width, request) = (self.path.clone(), (self.disp.0 * 2.) as u32, self.request.clone());
        cx.spawn(async move |this, cx| {
            // Let a drag settle a little before decoding.
            cx.background_executor().timer(Duration::from_millis(40)).await;
            if request.get() != id {
                return;
            }
            let image = frame(&path, t, width, id).await;
            if request.get() == id {
                let _ = this.update(cx, |trim, cx| {
                    if image.is_some() {
                        trim.preview = image;
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        let t = self.time_at(f32::from(e.position.x));
        let end = if (t - self.start).abs() <= (t - self.end).abs() { End::Start } else { End::End };
        self.drag = Some(end);
        self.set(end, t);
        let at = if end == End::Start { self.start } else { self.end };
        self.show_frame(at, cx);
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(end) = self.drag.filter(|_| e.pressed_button == Some(MouseButton::Left)) else { return };
        self.set(end, self.time_at(f32::from(e.position.x)));
        let at = if end == End::Start { self.start } else { self.end };
        self.show_frame(at, cx);
        cx.notify();
    }

    fn trimmed(&self) -> bool {
        self.start > 0.01 || self.end < self.duration - 0.01
    }

    fn play(&self) {
        util::spawn(&[
            OsStr::new("mpv"),
            format!("--start={:.3}", self.start).as_ref(),
            format!("--end={:.3}", self.end).as_ref(),
            "--force-window=yes".as_ref(),
            self.path.as_os_str(),
        ]);
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (path, start, end) = (self.path.clone(), self.start, self.end);
        window.remove_window();
        if !self.trimmed() {
            return;
        }
        save_trim(path, start, end, cx);
    }

    fn gif(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (path, start, end) = (self.path.clone(), self.start, self.end);
        let width = GIF_WIDTHS[self.gif_width].min(self.video.0);
        let fps = GIF_FPS[self.gif_fps];
        window.remove_window();
        export_gif(path, start, end, width, fps, cx);
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        match (k.key.as_str(), ctrl) {
            ("escape", _) => window.remove_window(),
            ("space", false) => self.play(),
            ("s", true) => self.save(window, cx),
            ("g", false) => self.gif(window, cx),
            _ => {}
        }
    }

    fn render_timeline(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let thumbs = self.thumbs.iter().map(|t| img(t.clone()).flex_1().h_full().object_fit(ObjectFit::Cover));
        let (d, start, end) = (self.duration.max(0.001), self.start, self.end);
        let accent = cx.theme().colors().text_accent;
        let strip_cell = self.strip.clone();
        let overlay = canvas(
            move |bounds, _, _| {
                strip_cell.set((f32::from(bounds.origin.x), f32::from(bounds.size.width)));
            },
            move |bounds, _, window, _| {
                let (x0, w) = (bounds.origin.x, bounds.size.width);
                let h = bounds.size.height;
                let at = |t: f64| x0 + w * (t / d) as f32;
                let shade = rgba(0x000000b3);
                window.paint_quad(fill(Bounds::new(bounds.origin, size(at(start) - x0, h)), shade));
                window.paint_quad(fill(Bounds::new(point(at(end), bounds.origin.y), size(x0 + w - at(end), h)), shade));
                let selected = Bounds::new(point(at(start), bounds.origin.y), size(at(end) - at(start), h));
                window.paint_quad(outline(selected, accent, BorderStyle::Solid));
                for x in [at(start), at(end)] {
                    let grip = Bounds::new(point(x - px(4.), bounds.origin.y), size(px(8.), h));
                    window.paint_quad(crate::ui::quad(grip, px(3.), accent, px(0.), accent, BorderStyle::Solid));
                    let notch = Bounds::new(point(x - px(1.), bounds.origin.y + h * 0.35), size(px(2.), h * 0.3));
                    window.paint_quad(fill(notch, gpui::white()));
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        v_flex()
            .w(px(self.disp.0))
            .gap_1()
            .p_1p5()
            .elevation_3(cx)
            .child(
                div()
                    .id("strip")
                    .relative()
                    .h(px(STRIP_HEIGHT))
                    .w_full()
                    .rounded_md()
                    .overflow_hidden()
                    .cursor(CursorStyle::ResizeLeftRight)
                    .child(h_flex().size_full().children(thumbs))
                    .child(overlay)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.mouse_down(e, cx))),
            )
            .child(
                h_flex()
                    .justify_between()
                    .px_0p5()
                    .child(Label::new(clock(self.start)).size(LabelSize::XSmall).color(Color::Muted))
                    .child(Label::new(tr!("trim.selected", clock(self.end - self.start))).size(LabelSize::XSmall))
                    .child(Label::new(clock(self.end)).size(LabelSize::XSmall).color(Color::Muted)),
            )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let chips = |prefix: &'static str, labels: Vec<String>, selected: usize, pick: fn(&mut Trimmer, usize), cx: &mut Context<Self>| {
            h_flex().gap_0p5().children(labels.into_iter().enumerate().map(move |(i, label)| {
                ButtonLike::new(SharedString::from(format!("{prefix}-{i}")))
                    .style(if i == selected { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .toggle_state(i == selected)
                    .child(div().px_1p5().child(Label::new(label).size(LabelSize::Small)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        pick(this, i);
                        cx.notify();
                    }))
            }))
        };
        h_flex()
            .w(px(self.disp.0))
            .justify_between()
            .gap_2()
            .pl_2p5()
            .pr_1()
            .py_1()
            .elevation_3(cx)
            .child(
                h_flex()
                    .min_w_0()
                    .gap_2()
                    .child(Icon::new(IconName::PlayFilled).size(IconSize::Small).color(Color::Muted))
                    .child(div().min_w_0().child(Label::new(name).size(LabelSize::Small).truncate())),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(Label::new("GIF").size(LabelSize::Small).color(Color::Muted))
                    .child(chips("w", GIF_WIDTHS.iter().map(|w| format!("{w}px")).collect(), self.gif_width, |t, i| t.gif_width = i, cx))
                    .child(chips("fps", GIF_FPS.iter().map(|f| tr!("common.fps", f)).collect(), self.gif_fps, |t, i| t.gif_fps = i, cx))
                    .child(Divider::vertical().h_5().mx_1())
                    .child(
                        Button::new("play", t("trim.play"))
                            .style(ButtonStyle::Subtle)
                            .key_binding(kit::keys("space"))
                            .on_click(cx.listener(|this, _, _, _| this.play())),
                    )
                    .child(
                        Button::new("cancel", t("common.cancel"))
                            .style(ButtonStyle::Subtle)
                            .color(Color::Muted)
                            .key_binding(kit::keys("escape"))
                            .on_click(|_, window, _| window.remove_window()),
                    )
                    .child(
                        Button::new("gif", t("trim.save_gif"))
                            .style(ButtonStyle::Subtle)
                            .key_binding(kit::keys("g"))
                            .on_click(cx.listener(|this, _, window, cx| this.gif(window, cx))),
                    )
                    .child(
                        Button::new("save", t("common.save"))
                            .style(ButtonStyle::Accent)
                            .disabled(!self.trimmed())
                            .tooltip(Tooltip::text(t("trim.as_new_file_original_stays")))
                            .key_binding(kit::keys("ctrl-s"))
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}

impl Render for Trimmer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let preview = div()
            .relative()
            .w(px(self.disp.0))
            .h(px(self.disp.1))
            .rounded_md()
            .overflow_hidden()
            .bg(gpui::black())
            .shadow(kit::shadow())
            .children(self.preview.clone().map(|p| img(p).size_full().object_fit(ObjectFit::Contain)))
            .child(
                div()
                    .absolute()
                    .top_2()
                    .left_2()
                    .px_1p5()
                    .py_0p5()
                    .rounded_md()
                    .bg(rgba(0x000000b3))
                    .font_family(kit::MONO_FONT)
                    .text_xs()
                    .text_color(gpui::white())
                    .child(clock(self.preview_at)),
            );
        kit::root(cx)
            .bg(rgba(0x00000099))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.drag = None))
            .child(preview)
            .child(self.render_timeline(cx))
            .child(self.render_footer(cx))
    }
}

/// "1:05.3"
fn clock(t: f64) -> String {
    let t = t.max(0.);
    format!("{}:{:04.1}", (t / 60.) as u64, t % 60.)
}

fn with_suffix(path: &Path, suffix: &str, ext: &str) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{stem}{suffix}.{ext}"))
}

/// Cut `start..end` into a new "-trim" file, re-encoded so the cut is frame
/// exact (stream copy can only cut at keyframes).
pub fn save_trim(path: PathBuf, start: f64, end: f64, cx: &mut App) {
    let running = job::start(t("trim.trimming_video"), IconName::Scissors, Some(path.as_path()), cx);
    cx.spawn(async move |cx| {
        let out = with_suffix(&path, "-trim", "mp4");
        let (ss, t) = (format!("{start:.3}"), format!("{:.3}", end - start));
        let mut result = Err(String::new());
        for encoder in [&["-c:v", "h264_nvenc", "-preset", "p5", "-cq", "19", "-b:v", "0"][..], &["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"][..]] {
            let mut argv: Vec<&OsStr> = ["ffmpeg", "-hide_banner", "-y", "-loglevel", "error", "-ss", &ss, "-i"].map(OsStr::new).to_vec();
            argv.push(path.as_os_str());
            argv.extend(["-t", &t, "-map", "0:v", "-map", "0:a?"].map(OsStr::new));
            argv.extend(encoder.iter().map(OsStr::new));
            argv.extend(["-c:a", "aac", "-movflags", "+faststart"].map(OsStr::new));
            argv.push(out.as_os_str());
            result = job::ffmpeg(&running.job, &argv, end - start, "trim.log").await;
            if running.job.cancelled() || result.is_ok() {
                break;
            }
        }
        let cancelled = running.job.cancelled();
        if result.is_ok() && !cancelled {
            track::trim(&path, &out, start, end);
        }
        running.finish((result.is_ok() && !cancelled).then(|| out.clone()), cx);
        match result {
            _ if cancelled => {
                let _ = std::fs::remove_file(&out);
            }
            Ok(()) => {}
            Err(e) => util::notify(crate::i18n::t("trim.couldnt_trim_video"), &e),
        }
    })
    .detach();
}

/// GIF of `start..end` with a palette built for the clip, so it isn't muddy.
pub fn export_gif(path: PathBuf, start: f64, end: f64, width: u32, fps: u32, cx: &mut App) {
    let running = job::start(t("trim.making_gif"), IconName::Image, Some(path.as_path()), cx);
    cx.spawn(async move |cx| {
        let whole = start <= 0.01 && probe(&path).await.is_some_and(|p| end >= p.duration - 0.01);
        let out = with_suffix(&path, if whole { "" } else { "-trim" }, "gif");
        let (ss, t) = (format!("{start:.3}"), format!("{:.3}", end - start));
        let filter = format!(
            "fps={fps},scale={width}:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle"
        );
        let mut argv: Vec<&OsStr> = ["ffmpeg", "-hide_banner", "-y", "-loglevel", "error", "-ss", &ss, "-t", &t, "-i"].map(OsStr::new).to_vec();
        argv.push(path.as_os_str());
        argv.extend(["-vf", &filter, "-loop", "0"].map(OsStr::new));
        argv.push(out.as_os_str());
        let result = job::ffmpeg(&running.job, &argv, end - start, "gif.log").await;
        let cancelled = running.job.cancelled();
        running.finish((result.is_ok() && !cancelled).then(|| out.clone()), cx);
        match result {
            _ if cancelled => {
                let _ = std::fs::remove_file(&out);
            }
            Ok(()) => {}
            Err(e) => util::notify(crate::i18n::t("trim.couldnt_make_gif"), &e),
        }
    })
    .detach();
}

/// Card shortcut: whole clip as a 720px, 15 fps GIF.
pub fn quick_gif(path: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        let Some(p) = probe(&path).await else { return };
        let width = GIF_WIDTHS[1].min(p.w);
        let _ = cx.update(|cx| export_gif(path, 0., p.duration, width, GIF_FPS[1], cx));
    })
    .detach();
}
