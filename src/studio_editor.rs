//! The studio's video editor, like CleanShot's Studio Mode: a live preview
//! of the framed, zoomed picture that plays in place, a timeline to trim the
//! ends, cut pieces out and place zoom segments (on clicks to start with),
//! and a sidebar for the look (format, backdrop, padding, corners, shadow,
//! overlays) and the export. Edits are saved to the recording's project as
//! they happen.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, BorderStyle, BoxShadow, Bounds, Context, CursorStyle, FocusHandle, Image, ImageFormat, ImageSource,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, PathPromptOptions, RenderImage,
    Window, canvas, div, img, linear_color_stop, linear_gradient, point, px, relative,
    rgba, size,
};
use crate::ui::{ButtonLike, IconButtonShape, Switch, ToggleState, Tooltip, prelude::*};

use crate::kit;
use crate::ui::fill;
use crate::studio::{
    self, ASPECTS, CURSOR_RATE, CURSOR_STYLES, MOTION_BLUR, Camera, FRAME_RATES, Format, KEY_SIZES, MAX_CURSOR, MAX_PADDING, MAX_RADIUS, Probe, Project, QUALITIES,
    RESOLUTIONS, Seg, ZOOMS,
};
use crate::track::Track;
use crate::{palette, trim, util};
use crate::i18n::t;

const FILMSTRIP: f32 = 52.;
const RULER: f32 = 18.;
const LANE: f32 = 40.;
const SIDEBAR: f32 = 320. * kit::SCALE;
/// Width of the labels in front of sidebar controls.
const ROW_LABEL: f32 = 74.;
/// Air around the preview inside the window.
const STAGE_PAD: f32 = 24.;
/// Edge grab width on segments, in pixels.
const EDGE: f32 = 9.;
const MIN_SEG: f64 = 0.3;
const NEW_SEG: f64 = 2.0;
const NEW_CUT: f64 = 1.0;
const PLAY_FPS: u32 = 30;
const WALLPAPERS: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Knob {
    Padding,
    Radius,
    Shadow,
    CursorSize,
    CursorSmooth,
    CamSize,
    CamRound,
}

enum Drag {
    Playhead,
    Move { index: usize, grab: f64 },
    Edge { index: usize, start: bool },
    Trim { start: bool },
    CutMove { index: usize, grab: f64 },
    CutEdge { index: usize, start: bool },
    Slider(Knob),
}

pub struct StudioEditor {
    video: PathBuf,
    probe: Probe,
    track: Track,
    project: Project,
    camera: Camera,
    /// The drawn cursor over time (recordings without the system cursor).
    cursor: Vec<(f64, f64, f64)>,
    thumbs: Vec<Arc<Image>>,
    walls: Vec<(PathBuf, Arc<Image>)>,
    /// Saved looks, by name.
    presets: Vec<(String, serde_json::Value)>,
    frame: Option<ImageSource>,
    /// The camera recorded alongside (studio mode), where its frame 0 sits
    /// on the recording's clock, and its frame at the playhead.
    cam_file: Option<PathBuf>,
    cam_offset: f64,
    cam_frame: Option<ImageSource>,
    /// Playing inside the editor; any seek stops it.
    playing: bool,
    /// Played frames no longer on screen, freed from the atlas on render.
    stale: Vec<Arc<RenderImage>>,
    playhead: f64,
    /// The selected zoom segment, or the selected cut; never both.
    selected: Option<usize>,
    cut: Option<usize>,
    drag: Option<Drag>,
    /// Waiting for a click on the preview to set the selected segment's focus.
    placing: bool,
    /// The pointer over the timeline: what a press there would do.
    timeline_cursor: CursorStyle,
    /// Room for the preview (it fits the output's shape inside).
    stage: (f32, f32),
    /// The preview area measured in the last layout; `stage` follows it.
    stage_cell: Rc<Cell<(f32, f32)>>,
    /// Timeline x origin and width, and the segment lane's top, in window
    /// coordinates.
    timeline: Rc<Cell<(f32, f32, f32)>>,
    /// The video box inside the preview, in window coordinates.
    video_box: Rc<Cell<(f32, f32, f32, f32)>>,
    /// Slider tracks (x, width) in window coordinates.
    sliders: Rc<RefCell<HashMap<Knob, (f32, f32)>>>,
    request: Rc<Cell<u64>>,
    focus: FocusHandle,
}

pub fn open(video: PathBuf, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        let Some(probe) = studio::probe(&video).await else {
            return util::notify(t("common.couldnt_open_video"), &video.display().to_string());
        };
        let Some(mon) = util::focused_monitor() else { return };
        let track = Track::load(&video).unwrap_or_else(|| Track { rect: (0.0, 0.0, probe.w, probe.h), ..Default::default() });
        let project = Project::load(&video, &track, &probe);
        // Fit the whole window (sidebar, title, transport, timeline) on screen.
        let chrome_h = 2. * STAGE_PAD + 260. * kit::SCALE + 80.;
        let stage_w = (mon.w as f32 - 80. - 2. * STAGE_PAD - SIDEBAR).min((mon.h as f32 - chrome_h) * 16. / 9.).min(mon.w as f32 * 0.5);
        let stage = (stage_w, stage_w * 9. / 16.);
        let thumbs = trim::filmstrip(&video, probe.duration).await;
        let frame = trim::frame(&video, project.trim.0, (stage.0 * 2.) as u32, 0).await;
        let walls = wallpaper_thumbs().await;
        let _ = cx.update(|cx| {
            let size = (stage.0 + 2. * STAGE_PAD + SIDEBAR, stage.1 + chrome_h - 80.);
            kit::open_window(cx, "kadr-studio", Some(&mon), size, |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    let camera = Camera::new(&track, &probe, &project.segments);
                    let cursor = if track.cursor_hidden { studio::cursor_path(&track, &probe, project.cursor_smooth) } else { Vec::new() };
                    let playhead = project.trim.0;
                    let video_path = video.clone();
                    let cam_file = Some(studio::camera_file(&video_path)).filter(|f| f.exists());
                    if let Some(file) = cam_file {
                        let at = playhead + studio::camera_offset(&video_path);
                        cx.spawn(async move |this, cx| {
                            let still = trim::frame(&file, at, 480, u64::MAX).await;
                            let _ = this.update(cx, |editor: &mut StudioEditor, cx| {
                                editor.cam_frame = still.map(ImageSource::from);
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                    StudioEditor {
                        video,
                        probe,
                        track,
                        project,
                        camera,
                        cursor,
                        thumbs,
                        walls,
                        presets: studio::presets(),
                        frame: frame.map(ImageSource::from),
                        cam_file: Some(studio::camera_file(&video_path)).filter(|f| f.exists()),
                        cam_offset: studio::camera_offset(&video_path),
                        cam_frame: None,
                        playing: false,
                        stale: Vec::new(),
                        playhead,
                        selected: None,
                        cut: None,
                        drag: None,
                        placing: false,
                        timeline_cursor: CursorStyle::PointingHand,
                        stage,
                        stage_cell: Rc::new(Cell::new(stage)),
                        timeline: Rc::new(Cell::new((0., 1., 0.))),
                        video_box: Rc::new(Cell::new((0., 0., 1., 1.))),
                        sliders: Rc::new(RefCell::new(HashMap::new())),
                        request: Rc::new(Cell::new(0)),
                        focus,
                    }
                })
            });
        });
    })
    .detach();
}

/// Omarchy's wallpapers, current theme first, as small previews.
pub(crate) async fn wallpaper_thumbs() -> Vec<(PathBuf, Arc<Image>)> {
    let (tx, rx) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let mut dirs = vec![home.join(".local/state/omarchy/current/theme/backgrounds")];
        let mut add = |dir: &Path| {
            let mut found: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
            found.sort();
            dirs.extend(found);
        };
        add(&home.join(".config/omarchy/backgrounds"));
        let mut themes: Vec<PathBuf> = std::fs::read_dir("/usr/share/omarchy/themes").into_iter().flatten().flatten().map(|e| e.path().join("backgrounds")).collect();
        themes.sort();
        dirs.extend(themes);
        let mut images = Vec::new();
        for dir in dirs {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
            files.sort();
            images.extend(files.into_iter().filter(|f| {
                let ext = f.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp")
            }));
            if images.len() >= WALLPAPERS * 2 {
                break;
            }
        }
        let cache = util::cache_dir().join("thumbs");
        let _ = std::fs::create_dir_all(&cache);
        // Several themes ship the same plain black picture; show it once.
        let mut seen = std::collections::HashSet::new();
        let thumbs = images
            .into_iter()
            .filter_map(|path| {
                let key = path.display().to_string().bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
                let thumb = cache.join(format!("wall-{key:016x}.png"));
                if !thumb.exists() {
                    let small = image::open(&path).ok()?.resize_to_fill(160, 100, image::imageops::FilterType::Triangle);
                    small.save(&thumb).ok()?;
                }
                let bytes = std::fs::read(&thumb).ok()?;
                // Plain one-colour pictures add nothing next to the gradients.
                let luma = image::load_from_memory(&bytes).ok()?.to_luma8();
                let n = luma.len() as f64;
                let mean = luma.iter().map(|&v| v as f64).sum::<f64>() / n;
                let spread = (luma.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / n).sqrt();
                if spread < 4. {
                    return None;
                }
                seen.insert(bytes.clone()).then(|| (path, Arc::new(Image::from_bytes(ImageFormat::Png, bytes))))
            })
            .take(WALLPAPERS)
            .collect();
        let _ = tx.send(thumbs);
    });
    rx.await.unwrap_or_default()
}

impl StudioEditor {
    /// Apply an edit: rebuild the camera and save the project.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.camera = Camera::new(&self.track, &self.probe, &self.project.segments);
        if self.track.cursor_hidden {
            self.cursor = studio::cursor_path(&self.track, &self.probe, self.project.cursor_smooth);
        }
        self.project.save(&self.video);
        // The next recording starts with this look.
        self.project.remember_look();
        cx.notify();
    }

    fn save_preset(&mut self, cx: &mut Context<Self>) {
        let next = self.presets.iter().filter_map(|(name, _)| name.strip_prefix(t("common.preset_prefix"))?.parse::<usize>().ok()).max().unwrap_or(0) + 1;
        self.presets.push((tr!("common.preset_n", next), self.project.look()));
        studio::save_presets(&self.presets);
        cx.notify();
    }

    fn time_at(&self, x: f32) -> f64 {
        let (ox, w, _) = self.timeline.get();
        (((x - ox) / w.max(1.)) as f64).clamp(0., 1.) * self.probe.duration
    }

    fn x_of(&self, t: f64) -> f32 {
        let (_, w, _) = self.timeline.get();
        w * (t / self.probe.duration.max(0.001)) as f32
    }

    fn seek(&mut self, t: f64, cx: &mut Context<Self>) {
        self.playing = false;
        self.playhead = t.clamp(0., self.probe.duration);
        let id = self.request.get() + 1;
        self.request.set(id);
        let (video, width, request, at) = (self.video.clone(), (self.stage.0 * 2.) as u32, self.request.clone(), self.playhead);
        let (cam_file, cam_offset) = (self.cam_file.clone(), self.cam_offset);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(40)).await;
            if request.get() != id {
                return;
            }
            let frame = trim::frame(&video, at, width, id).await;
            let cam = match &cam_file {
                Some(file) => trim::frame(file, at + cam_offset, 480, id).await,
                None => None,
            };
            if request.get() == id && cam.is_some() {
                let _ = this.update(cx, |editor: &mut StudioEditor, cx| {
                    editor.cam_frame = cam.map(ImageSource::from);
                    cx.notify();
                });
            }
            if request.get() == id && frame.is_some() {
                let _ = this.update(cx, |editor, cx| {
                    if let Some(ImageSource::Render(old)) = std::mem::replace(&mut editor.frame, frame.map(ImageSource::from)) {
                        editor.stale.push(old);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// Play from the playhead inside the editor, zooms and all: ffmpeg decodes
    /// raw frames at preview size, paced to the clock. Cut pieces are jumped
    /// over and it stops at the trimmed end. Space again pauses.
    fn toggle_play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.playing {
            // Pausing fetches a sharp frame at full preview size.
            return self.seek(self.playhead, cx);
        }
        let start = self.project.next_kept(self.playhead).or_else(|| self.project.kept().first().map(|k| k.0)).unwrap_or(0.);
        self.play_from(start, window, cx);
    }

    fn play_from(&mut self, start: f64, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.request.get() + 1;
        self.request.set(id);
        self.playing = true;
        self.playhead = start;

        let even = |v: f64| ((v / 2.).round() * 2.).max(2.) as usize;
        let w = even((self.stage.0 as f64 * 1.5).min(self.probe.w));
        let h = even(w as f64 * self.probe.h / self.probe.w);
        let (mut tx, mut rx) = futures::channel::mpsc::channel::<Vec<u8>>(4);
        let video = self.video.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let Ok(mut child) = std::process::Command::new("ffmpeg")
                .args(["-nostdin", "-loglevel", "error", "-ss", &format!("{start:.3}"), "-i"])
                .arg(&video)
                .args(["-an", "-vf", &format!("fps={PLAY_FPS},scale={w}:{h}"), "-f", "rawvideo", "-pix_fmt", "bgra", "-"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
            else {
                return;
            };
            let mut out = child.stdout.take().expect("piped");
            loop {
                let mut buf = vec![0u8; w * h * 4];
                if out.read_exact(&mut buf).is_err() || futures::executor::block_on(futures::SinkExt::send(&mut tx, buf)).is_err() {
                    break;
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        });

        // The camera plays alongside, square, at the same rate.
        const CAM: usize = 240;
        let mut cam_rx = self.cam_file.clone().filter(|_| self.project.cam).map(|file| {
            let (mut tx, rx) = futures::channel::mpsc::channel::<Vec<u8>>(4);
            let from = start + self.cam_offset;
            std::thread::spawn(move || {
                use std::io::Read;
                let Ok(mut child) = std::process::Command::new("ffmpeg")
                    .args(["-nostdin", "-loglevel", "error", "-ss", &format!("{from:.3}"), "-i"])
                    .arg(&file)
                    .args(["-an", "-vf", &format!("fps={PLAY_FPS},crop='min(iw,ih)':'min(iw,ih)',scale={CAM}:{CAM}"), "-f", "rawvideo", "-pix_fmt", "bgra", "-"])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                else {
                    return;
                };
                let mut out = child.stdout.take().expect("piped");
                loop {
                    let mut buf = vec![0u8; CAM * CAM * 4];
                    if out.read_exact(&mut buf).is_err() || futures::executor::block_on(futures::SinkExt::send(&mut tx, buf)).is_err() {
                        break;
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
            });
            rx
        });

        let request = self.request.clone();
        cx.spawn_in(window, async move |this, cx| {
            let began = std::time::Instant::now();
            let mut n = 0u32;
            while let Some(buf) = futures::StreamExt::next(&mut rx).await {
                let cam = match cam_rx.as_mut() {
                    Some(r) => futures::StreamExt::next(r).await,
                    None => None,
                };
                if cam.is_none() {
                    cam_rx = None;
                }
                let cam = cam.and_then(|b| image::RgbaImage::from_raw(CAM as u32, CAM as u32, b)).map(|f| Arc::new(RenderImage::new([image::Frame::new(f)])));
                if request.get() != id {
                    return;
                }
                let due = began + Duration::from_secs_f64(n as f64 / PLAY_FPS as f64);
                if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
                    cx.background_executor().timer(wait).await;
                }
                let Some(frame) = image::RgbaImage::from_raw(w as u32, h as u32, buf) else { return };
                let shown = Arc::new(RenderImage::new([image::Frame::new(frame)]));
                let t = start + n as f64 / PLAY_FPS as f64;
                let keep_going = this.update_in(cx, |editor, window, cx| {
                    if request.get() != id {
                        return false;
                    }
                    match editor.project.next_kept(t) {
                        // Into a cut: start again after it.
                        Some(next) if next > t + 0.05 => {
                            editor.play_from(next, window, cx);
                            false
                        }
                        Some(_) => {
                            if let Some(ImageSource::Render(old)) = editor.frame.replace(ImageSource::Render(shown)) {
                                editor.stale.push(old);
                            }
                            if let Some(cam) = cam.clone()
                                && let Some(ImageSource::Render(old)) = editor.cam_frame.replace(ImageSource::Render(cam))
                            {
                                editor.stale.push(old);
                            }
                            editor.playhead = t.min(editor.probe.duration);
                            cx.notify();
                            true
                        }
                        // Past the trimmed end.
                        None => {
                            let end = editor.project.trim.1;
                            editor.seek(end, cx);
                            false
                        }
                    }
                });
                if !matches!(keep_going, Ok(true)) {
                    return;
                }
                n += 1;
            }
            // The end of the file: stop on the last frame, sharpened.
            let _ = this.update(cx, |editor, cx| {
                if request.get() == id {
                    let end = editor.project.trim.1;
                    editor.seek(end, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn select_zoom(&mut self, index: Option<usize>) {
        self.selected = index;
        self.cut = None;
        self.placing = false;
    }

    fn select_cut(&mut self, index: Option<usize>) {
        self.cut = index;
        self.selected = None;
        self.placing = false;
    }

    /// Cut a second out at the playhead; its edges then set the length.
    fn add_cut(&mut self, cx: &mut Context<Self>) {
        let (lo, hi) = self.project.trim;
        let start = self.playhead.clamp(lo, (hi - MIN_SEG).max(lo));
        let end = (start + NEW_CUT).min(hi);
        if end - start < 0.05 {
            return;
        }
        self.project.cuts.push((start, end));
        self.select_cut(Some(self.project.cuts.len() - 1));
        self.changed(cx);
    }

    fn timeline_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        let (ox, _, lane_top) = self.timeline.get();
        let strip_top = lane_top - 6. - FILMSTRIP;
        let (x, y) = (f32::from(e.position.x), f32::from(e.position.y));
        let t = self.time_at(x);
        let local = x - ox;
        self.placing = false;

        if y >= strip_top && y < lane_top {
            // The filmstrip: trim handles, then cuts, else the playhead.
            let (a, b) = (self.x_of(self.project.trim.0), self.x_of(self.project.trim.1));
            if (local - a).abs() <= EDGE + 3. {
                self.drag = Some(Drag::Trim { start: true });
                return cx.notify();
            }
            if (local - b).abs() <= EDGE + 3. {
                self.drag = Some(Drag::Trim { start: false });
                return cx.notify();
            }
            let hit = self.project.cuts.iter().enumerate().rev().find_map(|(i, &(s, e))| {
                let (a, b) = (self.x_of(s), self.x_of(e));
                if (local - a).abs() <= EDGE {
                    Some(Drag::CutEdge { index: i, start: true })
                } else if (local - b).abs() <= EDGE {
                    Some(Drag::CutEdge { index: i, start: false })
                } else if local > a && local < b {
                    Some(Drag::CutMove { index: i, grab: t - s })
                } else {
                    None
                }
            });
            if let Some(drag) = hit {
                if let Drag::CutEdge { index, .. } | Drag::CutMove { index, .. } = drag {
                    self.select_cut(Some(index));
                }
                self.drag = Some(drag);
                return cx.notify();
            }
        }
        if y < lane_top {
            self.drag = Some(Drag::Playhead);
            return self.seek(t, cx);
        }
        // The segment lane: edges resize, bodies move, empty space adds.
        let hit = self.project.segments.iter().enumerate().rev().find_map(|(i, s)| {
            let (a, b) = (self.x_of(s.start), self.x_of(s.end));
            if (local - a).abs() <= EDGE {
                Some(Drag::Edge { index: i, start: true })
            } else if (local - b).abs() <= EDGE {
                Some(Drag::Edge { index: i, start: false })
            } else if local > a && local < b {
                Some(Drag::Move { index: i, grab: t - s.start })
            } else {
                None
            }
        });
        match hit {
            Some(drag) => {
                if let Drag::Edge { index, .. } | Drag::Move { index, .. } = drag {
                    self.select_zoom(Some(index));
                }
                self.drag = Some(drag);
            }
            None => {
                let start = t.min(self.probe.duration - MIN_SEG).max(0.);
                let end = (start + NEW_SEG).min(self.probe.duration);
                self.project.segments.push(Seg { start, end, zoom: studio::DEFAULT_ZOOM, focus: None });
                self.select_zoom(Some(self.project.segments.len() - 1));
                self.drag = Some(Drag::Edge { index: self.project.segments.len() - 1, start: false });
                self.seek(start + 0.01, cx);
                self.changed(cx);
            }
        }
        cx.notify();
    }

    /// What the pointer over the timeline would grab: edges resize, bodies
    /// move, the rest seeks or adds a zoom.
    fn timeline_cursor_at(&self, x: f32, y: f32) -> CursorStyle {
        let (ox, w, lane_top) = self.timeline.get();
        let strip_top = lane_top - 6. - FILMSTRIP;
        let local = x - ox;
        if local < -EDGE || local > w + EDGE || y < strip_top - RULER {
            return CursorStyle::Arrow;
        }
        let near = |t: f64| (local - self.x_of(t)).abs() <= EDGE;
        if y >= strip_top && y < lane_top {
            if near(self.project.trim.0) || near(self.project.trim.1) || self.project.cuts.iter().any(|&(a, b)| near(a) || near(b)) {
                return CursorStyle::ResizeLeftRight;
            }
            if self.project.cuts.iter().any(|&(a, b)| local > self.x_of(a) && local < self.x_of(b)) {
                return CursorStyle::OpenHand;
            }
            return CursorStyle::PointingHand;
        }
        if y >= lane_top {
            if self.project.segments.iter().any(|s| near(s.start) || near(s.end)) {
                return CursorStyle::ResizeLeftRight;
            }
            if self.project.segments.iter().any(|s| local > self.x_of(s.start) && local < self.x_of(s.end)) {
                return CursorStyle::OpenHand;
            }
        }
        CursorStyle::PointingHand
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        if e.pressed_button != Some(MouseButton::Left) {
            let cursor = self.timeline_cursor_at(f32::from(e.position.x), f32::from(e.position.y));
            if cursor != self.timeline_cursor {
                self.timeline_cursor = cursor;
                cx.notify();
            }
            return;
        }
        let x = f32::from(e.position.x);
        let t = self.time_at(x);
        let d = self.probe.duration;
        match self.drag {
            Some(Drag::Playhead) => self.seek(t, cx),
            Some(Drag::Move { index, grab }) => {
                let seg = &mut self.project.segments[index];
                let len = seg.end - seg.start;
                seg.start = (t - grab).clamp(0., d - len);
                seg.end = seg.start + len;
                self.changed(cx);
            }
            Some(Drag::Edge { index, start }) => {
                let seg = &mut self.project.segments[index];
                if start {
                    seg.start = t.clamp(0., seg.end - MIN_SEG);
                } else {
                    seg.end = t.clamp(seg.start + MIN_SEG, d);
                }
                let at = if start { seg.start } else { seg.end };
                self.changed(cx);
                self.seek(at, cx);
            }
            Some(Drag::Trim { start }) => {
                let trim = &mut self.project.trim;
                if start {
                    trim.0 = t.clamp(0., trim.1 - MIN_SEG);
                } else {
                    trim.1 = t.clamp(trim.0 + MIN_SEG, d);
                }
                let at = if start { trim.0 } else { trim.1 };
                self.changed(cx);
                self.seek(at, cx);
            }
            Some(Drag::CutMove { index, grab }) => {
                let cut = &mut self.project.cuts[index];
                let len = cut.1 - cut.0;
                cut.0 = (t - grab).clamp(0., d - len);
                cut.1 = cut.0 + len;
                self.changed(cx);
            }
            Some(Drag::CutEdge { index, start }) => {
                let cut = &mut self.project.cuts[index];
                if start {
                    cut.0 = t.clamp(0., cut.1 - 0.05);
                } else {
                    cut.1 = t.clamp(cut.0 + 0.05, d);
                }
                let at = if start { cut.0 } else { cut.1 };
                self.changed(cx);
                self.seek(at, cx);
            }
            Some(Drag::Slider(knob)) => self.slide(knob, x, cx),
            None => {}
        }
    }

    fn slide(&mut self, knob: Knob, x: f32, cx: &mut Context<Self>) {
        let Some(&(x0, w)) = self.sliders.borrow().get(&knob) else { return };
        let v = ((x - x0) / w.max(1.)).clamp(0., 1.) as f64;
        match knob {
            Knob::Padding => self.project.padding = v * MAX_PADDING,
            Knob::Radius => self.project.radius = v * MAX_RADIUS,
            Knob::Shadow => self.project.shadow = v,
            Knob::CursorSize => self.project.cursor_size = 0.5 + v * (MAX_CURSOR - 0.5),
            Knob::CursorSmooth => self.project.cursor_smooth = v,
            Knob::CamSize => self.project.cam_size = studio::MIN_CAM + v * (studio::MAX_CAM - studio::MIN_CAM),
            Knob::CamRound => self.project.cam_round = v,
        }
        self.changed(cx);
    }

    /// Clicking the preview while placing sets the segment's focus point.
    fn preview_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(index) = self.selected.filter(|_| self.placing) else { return };
        let (bx, by, bw, bh) = self.video_box.get();
        let (z, cxv, cyv) = self.camera.at(self.playhead);
        // Undo the preview's zoom to find the video pixel under the pointer.
        let (vw, vh) = (self.probe.w, self.probe.h);
        let (cw, ch) = (vw / z, vh / z);
        let (left, top) = ((cxv - cw / 2.).clamp(0., vw - cw), (cyv - ch / 2.).clamp(0., vh - ch));
        let u = ((f32::from(e.position.x) - bx) / bw).clamp(0., 1.) as f64;
        let v = ((f32::from(e.position.y) - by) / bh).clamp(0., 1.) as f64;
        self.project.segments[index].focus = Some((left + u * cw, top + v * ch));
        self.placing = false;
        self.changed(cx);
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(i) = self.cut.take() {
            self.project.cuts.remove(i);
            self.changed(cx);
        } else if let Some(i) = self.selected.take() {
            self.project.segments.remove(i);
            self.changed(cx);
        }
    }

    fn pick_wallpaper(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(t("common.choose_background_image").into()) });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |editor, cx| {
                editor.project.wallpaper = Some(path);
                editor.changed(cx);
            });
        })
        .detach();
    }

    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        studio::export(self.video.clone(), self.project.clone(), false, cx);
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        match (k.key.as_str(), ctrl) {
            ("escape", _) if self.placing => self.placing = false,
            ("escape", _) if self.selected.is_some() || self.cut.is_some() => self.select_zoom(None),
            ("escape", _) => return window.remove_window(),
            ("delete" | "backspace", false) => self.delete_selected(cx),
            ("space", false) => self.toggle_play(window, cx),
            ("x", false) => self.add_cut(cx),
            ("s", true) => return self.export(window, cx),
            ("left", _) => self.seek(self.playhead - 0.5, cx),
            ("right", _) => self.seek(self.playhead + 0.5, cx),
            ("home", _) => self.seek(self.project.trim.0, cx),
            ("end", _) => self.seek(self.project.trim.1, cx),
            _ => {}
        }
        cx.notify();
    }

    /// The output frame as it will look at the playhead, laid out exactly as
    /// the render does, scaled into the stage.
    fn render_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let l = studio::layout(&self.probe, &self.project, true);
        let (sw, sh) = self.stage;
        let k = (sw / l.ow as f32).min(sh / l.oh as f32);
        let (fw, fh) = (l.ow as f32 * k, l.oh as f32 * k);
        let (bx, by, bw, bh) = (l.x as f32 * k, l.y as f32 * k, l.w as f32 * k, l.h as f32 * k);
        let radius = l.radius as f32 * k;
        let short = fw.min(fh);
        let (vw, vh) = (self.probe.w as f32, self.probe.h as f32);

        let (z, cxv, cyv) = self.camera.at(self.playhead);
        let z = z as f32;
        let (cw, ch) = (vw / z, vh / z);
        let left = (cxv as f32 - cw / 2.).clamp(0., vw - cw);
        let top = (cyv as f32 - ch / 2.).clamp(0., vh - ch);
        let scale = bw / vw * z;

        let video_box = self.video_box.clone();
        let tracker = canvas(
            move |bounds, _, _| {
                video_box.set((f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height)));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .size_full();

        let shadow = self.project.shadow as f32;
        let picture = div()
            .id("video")
            .absolute()
            .left(px(bx))
            .top(px(by))
            .w(px(bw))
            .h(px(bh))
            .rounded(px(radius))
            .overflow_hidden()
            .when(shadow > 0. && (bw < fw || bh < fh), |d| {
                d.shadow(vec![BoxShadow {
                    color: gpui::hsla(0., 0., 0., 0.55 * shadow).into(),
                    offset: point(px(0.), px(0.012 * short)),
                    blur_radius: px(0.035 * short),
                    spread_radius: px(0.),
                    inset: false,
                }])
            })
            .bg(gpui::black())
            .when(self.placing, |d| d.cursor(CursorStyle::Crosshair))
            .children(self.frame.clone().map(|frame| {
                // The zoomed frame is bigger than the box, and GPUI clips
                // children to rectangles; painting it ourselves rounds the
                // visible part to the box's corners.
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let data = match frame.clone() {
                            ImageSource::Render(data) => Some(data),
                            ImageSource::Image(image) => image.use_render_image(window, cx),
                            _ => None,
                        };
                        let Some(data) = data else { return };
                        let shown = Bounds::new(point(bounds.origin.x - px(left * scale), bounds.origin.y - px(top * scale)), size(px(vw * scale), px(vh * scale)));
                        let _ = window.paint_image(bounds, shown, px(radius).into(), 0., data, 0, false);
                    },
                )
                .absolute()
                .inset_0()
                .size_full()
            }))
            .children(self.render_cursor(left, top, scale))
            .child(tracker)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.preview_down(e, cx)));

        let (c0, c1) = self.project.colors();
        let backdrop = match &self.project.wallpaper {
            Some(path) => div().size_full().child(img(path.clone()).size_full().object_fit(ObjectFit::Cover)),
            None => div().size_full().bg(linear_gradient(135., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.))),
        };
        // The camera in its corner, cut to its shape like the export does.
        let cam = self.cam_frame.clone().filter(|_| self.project.cam && self.cam_file.is_some()).map(|frame| {
            let (cx0, cy0, side, r) = studio::camera_box(&l, &self.project);
            let (cx0, cy0, side, r) = (cx0 as f32 * k, cy0 as f32 * k, side as f32 * k, r as f32 * k);
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let data = match frame.clone() {
                        ImageSource::Render(data) => Some(data),
                        ImageSource::Image(image) => image.use_render_image(window, cx),
                        _ => None,
                    };
                    let Some(data) = data else { return };
                    // Cover the square with the frame's middle.
                    let s = data.size(0);
                    let (iw, ih) = (s.width.0 as f32, s.height.0 as f32);
                    let fit = side / iw.min(ih).max(1.);
                    let (w, h) = (iw * fit, ih * fit);
                    let shown = Bounds::new(point(bounds.origin.x - px((w - side) / 2.), bounds.origin.y - px((h - side) / 2.)), size(px(w), px(h)));
                    let _ = window.paint_image(bounds, shown, px(r).into(), 0., data, 0, false);
                },
            )
            .absolute()
            .left(px(cx0))
            .top(px(cy0))
            .w(px(side))
            .h(px(side))
        });
        div()
            .relative()
            .w(px(fw))
            .h(px(fh))
            .rounded_md()
            .overflow_hidden()
            .child(backdrop)
            .child(picture)
            .children(cam)
            .when(self.placing, |d| {
                d.child(
                    div()
                        .absolute()
                        .bottom_2()
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(div().px_2().py_1().rounded_md().bg(rgba(0x000000cc)).text_xs().text_color(gpui::white()).child(t("studio.click_video_where_zoom"))),
                )
            })
    }

    /// The drawn cursor at the playhead, placed through the camera like the
    /// render does it.
    fn render_cursor(&self, left: f32, top: f32, scale: f32) -> Option<impl IntoElement> {
        if !self.track.cursor_hidden || !self.project.cursor || self.cursor.is_empty() {
            return None;
        }
        let i = ((self.playhead * CURSOR_RATE).round() as usize).min(self.cursor.len() - 1);
        let (_, x, y) = self.cursor[i];
        let height = studio::cursor_height(&self.track, &self.probe, self.project.cursor_size);
        let (png, (hx, hy), (w, h)) = studio::cursor_image(self.project.cursor_style, height)?;
        let image = Arc::new(Image::from_bytes(ImageFormat::Png, png));
        Some(
            img(image)
                .absolute()
                .left(px((x as f32 - left - hx as f32) * scale))
                .top(px((y as f32 - top - hy as f32) * scale))
                .w(px(w as f32 * scale))
                .h(px(h as f32 * scale)),
        )
    }

    /// Ruler, filmstrip (trim and cuts) and the zoom lane, under the preview.
    fn render_timeline(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let status = cx.theme().status();
        let (accent, lane_bg, text, tick) = (colors.text_accent, colors.element_background, colors.text, colors.border);
        let line = colors.border_variant;
        let (handle_color, cut_color) = (status.warning, status.error);
        let thumbs = self.thumbs.iter().map(|t| img(t.clone()).flex_1().h_full().object_fit(ObjectFit::Cover));
        let d = self.probe.duration.max(0.001);
        let segments: Vec<(f64, f64, bool)> = self.project.segments.iter().enumerate().map(|(i, s)| (s.start, s.end, self.selected == Some(i))).collect();
        let cuts: Vec<(f64, f64, bool)> = self.project.cuts.iter().enumerate().map(|(i, &(a, b))| (a, b, self.cut == Some(i))).collect();
        let trim = self.project.trim;
        let playhead = self.playhead;
        let timeline = self.timeline.clone();
        // About ten labelled ticks, on round seconds.
        let step = [0.5, 1., 2., 5., 10., 15., 30., 60., 120., 300.].into_iter().find(|s| d / s <= 12.).unwrap_or(600.);
        let marks: Vec<f64> = (0..).map(|i| i as f64 * step).take_while(|&t| t <= d).collect();
        let ticks = marks.clone();
        let top = RULER;
        let lane_y = RULER + FILMSTRIP + 6.;

        // Ticks, trim, cuts, segments and the playhead are painted; text goes on top.
        let painter = canvas(
            move |bounds, _, _| bounds,
            move |_, bounds, window, _| {
                let (x0, w) = (bounds.origin.x, bounds.size.width);
                let at = |t: f64| x0 + w * (t / d) as f32;
                let strip_top = bounds.origin.y + px(top);
                let lane_top = bounds.origin.y + px(lane_y);
                timeline.set((f32::from(x0), f32::from(w), f32::from(lane_top)));
                for &t in &ticks {
                    window.paint_quad(fill(Bounds::new(point(at(t), bounds.origin.y + px(top - 6.)), size(px(1.), px(5.))), tick));
                }
                // Cut pieces: red wash with a frame.
                for &(a, b, selected) in &cuts {
                    let mut wash = cut_color;
                    wash.alpha = if selected { 0.6 } else { 0.42 };
                    let block = Bounds::new(point(at(a), strip_top), size(at(b) - at(a), px(FILMSTRIP)));
                    window.paint_quad(crate::ui::quad(block, px(4.), wash, px(if selected { 2. } else { 1. }), cut_color, BorderStyle::Solid));
                }
                // Trimmed-off ends are dimmed, with handles on the kept part.
                let shade = gpui::hsla(0., 0., 0., 0.62);
                if trim.0 > 0. {
                    window.paint_quad(fill(Bounds::new(point(x0, strip_top), size(at(trim.0) - x0, px(FILMSTRIP))), shade));
                }
                if trim.1 < d {
                    window.paint_quad(fill(Bounds::new(point(at(trim.1), strip_top), size(x0 + w - at(trim.1), px(FILMSTRIP))), shade));
                }
                let kept = Bounds::new(point(at(trim.0), strip_top), size(at(trim.1) - at(trim.0), px(FILMSTRIP)));
                window.paint_quad(crate::ui::quad(kept, px(6.), gpui::transparent_black(), px(2.), handle_color, BorderStyle::Solid));
                for (x, left) in [(at(trim.0), true), (at(trim.1), false)] {
                    let hx = if left { x } else { x - px(8.) };
                    window.paint_quad(crate::ui::quad(Bounds::new(point(hx, strip_top), size(px(8.), px(FILMSTRIP))), px(4.), handle_color, px(0.), handle_color, BorderStyle::Solid));
                    let grip = gpui::hsla(0., 0., 0., 0.5);
                    window.paint_quad(fill(Bounds::new(point(hx + px(3.5), strip_top + px(FILMSTRIP / 2. - 7.)), size(px(1.), px(14.))), grip));
                }
                for &(s, e, selected) in &segments {
                    let block = Bounds::new(point(at(s), lane_top + px(2.)), size(at(e) - at(s), px(LANE - 4.)));
                    let mut fill_color = accent;
                    fill_color.alpha = if selected { 0.55 } else { 0.3 };
                    window.paint_quad(crate::ui::quad(block, px(6.), fill_color, px(if selected { 2. } else { 1. }), accent, BorderStyle::Solid));
                    // Grips at both ends, like Final Cut's: something to catch.
                    let mut grip = text;
                    grip.alpha = if selected { 0.9 } else { 0.55 };
                    for x in [at(s) + px(6.), at(e) - px(9.)] {
                        if at(e) - at(s) > px(24.) {
                            for dx in [0., 3.] {
                                window.paint_quad(crate::ui::quad(Bounds::new(point(x + px(dx), lane_top + px(LANE / 2. - 7.)), size(px(1.5), px(14.))), px(1.), grip, px(0.), grip, BorderStyle::Solid));
                            }
                        }
                    }
                    // Ease-in/out ramps either side, faint.
                    let mut ramp = accent;
                    ramp.alpha = 0.12;
                    for (a, b) in [(s - studio::RAMP, s), (e, e + studio::RAMP)] {
                        let (a, b) = (a.max(0.), b.min(d));
                        if b > a {
                            window.paint_quad(fill(Bounds::new(point(at(a), lane_top + px(8.)), size(at(b) - at(a), px(LANE - 16.))), ramp));
                        }
                    }
                }
                let x = at(playhead);
                let line_top = bounds.origin.y + px(top - 2.);
                window.paint_quad(fill(Bounds::new(point(x - px(1.), line_top), size(px(2.), bounds.size.height - px(top - 2.))), text));
                window.paint_quad(crate::ui::quad(Bounds::new(point(x - px(6.), line_top - px(6.)), size(px(12.), px(10.))), px(3.), text, px(0.), text, BorderStyle::Solid));
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        let labels = self.project.segments.iter().map(|s| {
            let (a, b) = ((s.start / d) as f32, (s.end / d) as f32);
            div()
                .absolute()
                .left(relative(a))
                .top(px(lane_y))
                .w(relative((b - a).max(0.)))
                .h(px(LANE))
                .flex()
                .items_center()
                .justify_center()
                .gap_1()
                .overflow_hidden()
                .child(Label::new(format!("×{:.2}", s.zoom).trim_end_matches('0').trim_end_matches('.').to_string()).size(LabelSize::XSmall))
                .when(s.focus.is_some(), |d| d.child(Icon::new(IconName::Crosshair).size(IconSize::XSmall).color(Color::Muted)))
        });
        let cut_marks = self.project.cuts.iter().map(|&(a, b)| {
            let (a, b) = ((a.min(b) / d) as f32, (a.max(b) / d) as f32);
            div()
                .absolute()
                .left(relative(a))
                .top(px(top))
                .w(relative(b - a))
                .h(px(FILMSTRIP))
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .child(Icon::new(IconName::Scissors).size(IconSize::Small).color(Color::Custom(gpui::white())))
        });
        let ruler = marks.into_iter().map(|t| {
            div().absolute().top_0().left(relative((t / d) as f32)).ml_1().child(Label::new(ruler_label(t, step)).size(LabelSize::XSmall).color(Color::Muted))
        });

        // A hairline sets the timeline apart from the preview above.
        div().w_full().px_4().pt_2().pb_3().border_t_1().border_color(line).child(
            div()
                .id("timeline")
                .relative()
                .w_full()
                .h(px(lane_y + LANE))
                .cursor(match self.drag {
                    Some(Drag::Move { .. } | Drag::CutMove { .. }) => CursorStyle::ClosedHand,
                    Some(Drag::Edge { .. } | Drag::CutEdge { .. } | Drag::Trim { .. }) => CursorStyle::ResizeLeftRight,
                    _ => self.timeline_cursor,
                })
                .children(ruler)
                .child(div().absolute().top(px(top)).left_0().right_0().h(px(FILMSTRIP)).rounded_md().overflow_hidden().child(h_flex().size_full().children(thumbs)))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(lane_y))
                        .h(px(LANE))
                        .rounded_md()
                        .bg(lane_bg)
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(self.project.segments.is_empty(), |lane| {
                            lane.child(Label::new(t("studio.click_track_add_zoom")).size(LabelSize::XSmall).color(Color::Placeholder))
                        }),
                )
                .children(labels)
                .child(painter)
                .children(cut_marks)
                .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.timeline_down(e, cx))),
        )
    }

    fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.video.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let gif = self.project.format == Format::Gif;
        h_flex()
            .w_full()
            .justify_between()
            .gap_2()
            // An even inset keeps corner buttons concentric with the window.
            .p_1()
            .bg(cx.theme().colors().title_bar_background)
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .child(
                h_flex()
                    .min_w_0()
                    .pl_2()
                    .gap_2()
                    .child(div().min_w_0().child(Label::new(name).truncate()))
                    .child(Label::new(clock(self.project.kept_duration())).size(LabelSize::Small).color(Color::Muted)),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(
                        Button::new("preview", t("studio.preview"))
                            .style(ButtonStyle::Filled)
                            .size(ButtonSize::Large)
                            .tooltip(Tooltip::text(t("studio.build_720p_preview_with_all")))
                            .on_click(cx.listener(|this, _, _, cx| studio::export(this.video.clone(), this.project.clone(), true, cx))),
                    )
                    .child(div().px_1().child(Label::new(size_label(&self.project, &self.probe)).size(LabelSize::Small).color(Color::Muted)))
                    .child(
                        Button::new("export", if gif { t("studio.export_gif") } else { t("studio.export") })
                            .style(ButtonStyle::Accent)
                            .size(ButtonSize::Large)
                            .start_icon(Icon::new(IconName::Sparkle).size(IconSize::Small))
                            .key_binding(kit::keys("ctrl-s"))
                            .on_click(cx.listener(|this, _, window, cx| this.export(window, cx))),
                    ),
            )
    }

    /// Time, stepping, cutting and the current zoom, under the preview.
    fn render_transport(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (z, _, _) = self.camera.at(self.playhead);
        let mono = |text: String| div().font_family(kit::MONO_FONT).text_sm().text_color(cx.theme().colors().text_muted).child(text);
        h_flex()
            .w_full()
            .justify_between()
            .px_3()
            .py_1p5()
            .child(div().w(px(180.)).child(mono(format!("{} / {}", clock(self.playhead), clock(self.probe.duration)))))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        IconButton::new("back", IconName::ChevronLeft)
                            .shape(IconButtonShape::Square)
                            .size(ButtonSize::Large)
                            .tooltip(Tooltip::text(t("studio.back_half_second")))
                            .on_click(cx.listener(|this, _, _, cx| this.seek(this.playhead - 0.5, cx))),
                    )
                    .child(
                        IconButton::new("play", if self.playing { IconName::DebugPause } else { IconName::PlayFilled })
                            .shape(IconButtonShape::Square)
                            .size(ButtonSize::Large)
                            .tooltip(|_, cx| cx.new(|_| Tooltip::new(t("studio.play_pause")).key_binding(kit::keys("space"))).into())
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_play(window, cx))),
                    )
                    .child(
                        IconButton::new("forward", IconName::ChevronRight)
                            .shape(IconButtonShape::Square)
                            .size(ButtonSize::Large)
                            .tooltip(Tooltip::text(t("studio.forward_half_second")))
                            .on_click(cx.listener(|this, _, _, cx| this.seek(this.playhead + 0.5, cx))),
                    ),
            )
            .child(
                h_flex()
                    .w(px(180.))
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cut", t("studio.cut"))
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Large)
                            .start_icon(Icon::new(IconName::Scissors).size(IconSize::Small))
                            .tooltip(|_, cx| cx.new(|_| Tooltip::new(t("studio.cut_piece_from_here")).meta(t("studio.then_drag_edges_on_filmstrip")).key_binding(kit::keys("x"))).into())
                            .on_click(cx.listener(|this, _, _, cx| this.add_cut(cx))),
                    )
                    .child(mono(format!("×{z:.2}"))),
            )
    }

    fn slider(&self, knob: Knob, value: f64, label: String, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let (track, filled, knob_color, knob_edge) = (colors.element_active, colors.text_accent, gpui::white(), colors.border);
        let sliders = self.sliders.clone();
        let v = value.clamp(0., 1.) as f32;
        let bar = canvas(
            move |bounds, _, _| {
                sliders.borrow_mut().insert(knob, (f32::from(bounds.origin.x), f32::from(bounds.size.width)));
                bounds
            },
            move |_, bounds, window, _| {
                let mid = bounds.origin.y + bounds.size.height / 2.;
                let (x0, w) = (bounds.origin.x, bounds.size.width);
                window.paint_quad(crate::ui::quad(Bounds::new(point(x0, mid - px(2.)), size(w, px(4.))), px(2.), track, px(0.), track, BorderStyle::Solid));
                window.paint_quad(crate::ui::quad(Bounds::new(point(x0, mid - px(2.)), size(w * v, px(4.))), px(2.), filled, px(0.), filled, BorderStyle::Solid));
                let cx0 = x0 + w * v;
                window.paint_quad(crate::ui::quad(Bounds::new(point(cx0 - px(9.), mid - px(8.)), size(px(18.), px(18.))), px(9.), gpui::hsla(0., 0., 0., 0.18), px(0.), knob_edge, BorderStyle::Solid));
                window.paint_quad(crate::ui::quad(Bounds::new(point(cx0 - px(8.), mid - px(8.)), size(px(16.), px(16.))), px(8.), knob_color, px(0.5), knob_edge, BorderStyle::Solid));
            },
        )
        .size_full();
        h_flex()
            .w_full()
            .gap_3()
            .child(div().id(SharedString::from(format!("slider-{}", knob as u8))).flex_1().h(px(22.)).mx_2().cursor(CursorStyle::PointingHand).child(bar).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    this.drag = Some(Drag::Slider(knob));
                    this.slide(knob, f32::from(e.position.x), cx);
                }),
            ))
            .child(div().w(px(44.)).flex().justify_end().child(Label::new(label).size(LabelSize::Small).color(Color::Muted)))
    }

    /// Settings on the right, like CleanShot's studio: the selected zoom or
    /// cut first, then the look of the whole video and the export.
    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let (card_bg, track_bg, thumb_bg, ring, edge) = (colors.element_background, colors.element_hover, colors.elevated_surface_background, colors.text, colors.border_variant);
        let (sidebar_bg, divider) = (colors.editor_background, colors.border_variant);
        let (accent, line) = (colors.text_accent, colors.border);
        // An inset group, like System Settings: a soft card, rows inside.
        let card = || v_flex().w_full().p_3().gap_3().rounded(px(12.)).rounded_smoothing(crate::ui::SMOOTHING).bg(card_bg);
        let label = |text: &'static str| div().w(px(ROW_LABEL)).flex_none().child(Label::new(text).size(LabelSize::Small).color(Color::Muted));
        let row = |text: &'static str, control: AnyElement| h_flex().w_full().gap_2().child(label(text)).child(div().flex_1().min_w_0().child(control));
        let toggle = |text: &'static str, switch: Switch| h_flex().w_full().justify_between().child(Label::new(text).size(LabelSize::Small)).child(switch);
        // A macOS segmented control: options share a track, the chosen one lifts.
        let segment = |id: SharedString, text: String, on: bool| {
            div()
                .id(id)
                // Wide enough for its text; spare room is shared out.
                .flex_grow(1.)
                .px_2()
                .h(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .when(on, |d| d.bg(thumb_bg).shadow(vec![BoxShadow { color: gpui::hsla(0., 0., 0., 0.18).into(), offset: point(px(0.), px(1.)), blur_radius: px(2.), spread_radius: px(0.), inset: false }]))
                .child(Label::new(text).size(LabelSize::Small).color(if on { Color::Default } else { Color::Muted }).truncate())
        };
        let segmented = |items: Vec<AnyElement>| h_flex().w_full().p(px(2.)).gap(px(2.)).rounded(px(8.)).bg(track_bg).children(items).into_any_element();

        // What's selected on the timeline, or the timeline's own tools.
        let context = if let Some((index, (a, b))) = self.cut.and_then(|i| self.project.cuts.get(i).map(|c| (i, *c))) {
            card()
                .child(Label::new(tr!("studio.removing", seconds((b - a).abs()), clock(a.min(b)), clock(a.max(b)))).size(LabelSize::Small))
                .child(
                    Button::new("delete-cut", t("studio.restore_piece"))
                        .style(ButtonStyle::Filled)
                        .start_icon(Icon::new(IconName::Undo).size(IconSize::Small))
                        .key_binding(kit::keys("delete"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.project.cuts.remove(index);
                            this.cut = None;
                            this.changed(cx);
                        })),
                )
        } else if let Some((index, seg)) = self.selected.and_then(|i| self.project.segments.get(i).map(|s| (i, s.clone()))) {
            let zooms = ZOOMS
                .iter()
                .map(|&z| {
                    segment(SharedString::from(format!("zoom-{z}")), format!("×{z}"), (seg.zoom - z).abs() < 0.01)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.project.segments[index].zoom = z;
                            this.changed(cx);
                        }))
                        .into_any_element()
                })
                .collect();
            let toward = vec![
                segment("focus-cursor".into(), t("studio.follow_cursor").into(), seg.focus.is_none() && !self.placing)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.placing = false;
                        this.project.segments[index].focus = None;
                        this.changed(cx);
                    }))
                    .into_any_element(),
                segment("focus-point".into(), t("studio.point").into(), seg.focus.is_some() || self.placing)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.placing = true;
                        cx.notify();
                    }))
                    .into_any_element(),
            ];
            card()
                .child(
                    h_flex()
                        .justify_between()
                        .child(Label::new(t("studio.zoom")).size(LabelSize::Small))
                        .child(
                            IconButton::new("delete", IconName::Trash)
                                .shape(IconButtonShape::Square)
                                .icon_color(Color::Error)
                                .tooltip(Tooltip::text(t("studio.delete_zoom")))
                                .on_click(cx.listener(|this, _, _, cx| this.delete_selected(cx))),
                        ),
                )
                .child(segmented(zooms))
                .child(segmented(toward))
                .when(self.placing, |s| s.child(Label::new(t("studio.click_video_where_zoom")).size(LabelSize::XSmall).color(Color::Accent)))
        } else {
            let edited = self.project.trim != (0., self.probe.duration) || !self.project.cuts.is_empty();
            card().child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new("auto", t("studio.zooms_on_clicks"))
                            .style(ButtonStyle::Filled)
                            .start_icon(Icon::new(IconName::Sparkle).size(IconSize::Small))
                            .disabled(self.track.clicks.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                let auto = Project::auto(&this.track, &this.probe);
                                this.project.segments = auto.segments;
                                this.select_zoom(None);
                                this.changed(cx);
                            })),
                    )
                    .when(edited, |row| {
                        row.child(Button::new("uncut", t("studio.restore_whole_video")).style(ButtonStyle::Subtle).on_click(cx.listener(|this, _, _, cx| {
                            this.project.trim = (0., this.probe.duration);
                            this.project.cuts.clear();
                            this.select_cut(None);
                            this.changed(cx);
                        })))
                    }),
            )
        };

        // Presets: chips, and one button to keep or drop the current look.
        let current = self.project.look();
        let active = self.presets.iter().position(|(_, look)| *look == current);
        let presets = h_flex()
            .w_full()
            .flex_wrap()
            .gap_1()
            .children(self.presets.iter().enumerate().map(|(i, (name, look))| {
                let look = look.clone();
                ButtonLike::new(SharedString::from(format!("preset-{i}")))
                    .style(if active == Some(i) { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .toggle_state(active == Some(i))
                    .child(div().px_1p5().child(Label::new(name.clone()).size(LabelSize::Small)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project = this.project.with_look(&look, &this.probe);
                        this.changed(cx);
                    }))
            }))
            .child(match active {
                None => Button::new("save-preset", t("common.save_as_preset"))
                    .style(ButtonStyle::Subtle)
                    .color(Color::Muted)
                    .start_icon(Icon::new(IconName::Plus).size(IconSize::Small).color(Color::Muted))
                    .on_click(cx.listener(|this, _, _, cx| this.save_preset(cx)))
                    .into_any_element(),
                Some(i) => IconButton::new("delete-preset", IconName::Trash)
                    .shape(IconButtonShape::Square)
                    .icon_color(Color::Muted)
                    .tooltip(Tooltip::text(t("common.delete_preset")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.presets.remove(i);
                        studio::save_presets(&this.presets);
                        cx.notify();
                    }))
                    .into_any_element(),
            });

        // The backdrop: colours and gradients, then wallpapers.
        let swatch = |id: SharedString, selected: bool| {
            div().id(id).flex().items_center().justify_center().rounded(px(9.)).border_2().border_color(if selected { ring } else { gpui::transparent_black() }).cursor_pointer()
        };
        let tiles = palette::backdrops()
            .into_iter()
            .enumerate()
            .map(|(i, (c0, c1))| {
                swatch(SharedString::from(format!("backdrop-{i}")), self.project.wallpaper.is_none() && i == self.project.backdrop)
                    .size(px(32.))
                    .child(div().size(px(24.)).rounded(px(6.)).border_1().border_color(edge).bg(linear_gradient(135., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project.backdrop = i;
                        this.project.wallpaper = None;
                        this.changed(cx);
                    }))
            })
            .collect::<Vec<_>>();
        let walls = self
            .walls
            .iter()
            .enumerate()
            .map(|(i, (path, thumb))| {
                let path = path.clone();
                swatch(SharedString::from(format!("wall-{i}")), self.project.wallpaper.as_ref() == Some(&path))
                    .w(px(52.))
                    .h(px(36.))
                    .child(img(thumb.clone()).w(px(44.)).h(px(28.)).rounded(px(5.)).object_fit(ObjectFit::Cover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project.wallpaper = Some(path.clone());
                        this.changed(cx);
                    }))
                    .into_any_element()
            })
            .chain(std::iter::once(
                swatch("own-wallpaper".into(), self.project.wallpaper.as_ref().is_some_and(|w| !self.walls.iter().any(|(p, _)| p == w)))
                    .w(px(52.))
                    .h(px(36.))
                    .child(div().w(px(44.)).h(px(28.)).rounded(px(5.)).border_1().border_color(edge).flex().items_center().justify_center().child(Icon::new(IconName::Plus).size(IconSize::Small).color(Color::Muted)))
                    .tooltip(Tooltip::text(t("common.your_own_image")))
                    .on_click(cx.listener(|this, _, _, cx| this.pick_wallpaper(cx)))
                    .into_any_element(),
            ))
            .collect::<Vec<_>>();

        let pick = |items: &[&'static str], selected: usize, id: &'static str, set: fn(&mut Project, usize), cx: &mut Context<Self>| -> AnyElement {
            segmented(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, text)| {
                        segment(SharedString::from(format!("{id}-{i}")), t(text).to_string(), selected == i)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                set(&mut this.project, i);
                                this.changed(cx);
                            }))
                            .into_any_element()
                    })
                    .collect(),
            )
        };
        let aspect_names: Vec<&'static str> = ASPECTS.iter().map(|a| a.0).collect();
        let blur_names: Vec<&'static str> = MOTION_BLUR.iter().map(|a| a.0).collect();
        let look = card()
            .child(pick(&aspect_names, self.project.aspect, "aspect", |p, i| p.aspect = i, cx))
            .child(h_flex().flex_wrap().gap(px(2.)).ml(px(-4.)).children(tiles))
            .child(h_flex().flex_wrap().gap(px(2.)).ml(px(-4.)).children(walls))
            .child(row(t("studio.row_padding"), self.slider(Knob::Padding, self.project.padding / MAX_PADDING, format!("{}%", (self.project.padding * 100.).round()), cx).into_any_element()))
            .child(row(t("studio.row_corners"), self.slider(Knob::Radius, self.project.radius / MAX_RADIUS, format!("{}", self.project.radius.round()), cx).into_any_element()))
            .child(row(t("studio.row_shadow"), self.slider(Knob::Shadow, self.project.shadow, format!("{}%", (self.project.shadow * 100.).round()), cx).into_any_element()))
            .child(row(t("studio.row_blur"), pick(&blur_names, self.project.motion_blur, "blur", |p, i| p.motion_blur = i, cx)));

        // The drawn cursor: only recordings made in studio mode have one to change.
        let cursor = self.track.cursor_hidden.then(|| {
            let styles: Vec<&'static str> = CURSOR_STYLES.to_vec();
            card()
                .child(toggle(
                    t("studio.cursor"),
                    Switch::new("cursor", ToggleState::from(self.project.cursor)).on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                        this.project.cursor = *state == ToggleState::Selected;
                        this.changed(cx);
                    })),
                ))
                .when(self.project.cursor, |s| {
                    s.child(pick(&styles, self.project.cursor_style, "cursor-style", |p, i| p.cursor_style = i, cx))
                        .child(row(
                            t("studio.row_size"),
                            self.slider(Knob::CursorSize, (self.project.cursor_size - 0.5) / (MAX_CURSOR - 0.5), crate::i18n::decimal(format!("×{:.1}", self.project.cursor_size)), cx).into_any_element(),
                        ))
                        .child(row(
                            t("studio.row_smooth"),
                            self.slider(Knob::CursorSmooth, self.project.cursor_smooth, format!("{}%", (self.project.cursor_smooth * 100.).round()), cx).into_any_element(),
                        ))
                })
        });

        let camera = self.cam_file.as_ref().map(|_| {
            // A 3×3 grid without the middle: the spots the camera snaps to.
            let spot = |i: usize, cx: &mut Context<Self>| {
                let on = self.project.cam_anchor == i;
                div()
                    .id(SharedString::from(format!("cam-anchor-{i}")))
                    .size(px(14.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if on { accent } else { line })
                    .when(on, |d| d.bg(accent))
                    .cursor_pointer()
                    .hover(|d| d.border_color(accent))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project.cam_anchor = i;
                        this.changed(cx);
                    }))
                    .into_any_element()
            };
            let mut grid = v_flex().gap(px(3.));
            for cells in [[Some(0), Some(1), Some(2)], [Some(7), None, Some(3)], [Some(6), Some(5), Some(4)]] {
                let cells: Vec<AnyElement> = cells.iter().map(|c| c.map(|i| spot(i, cx)).unwrap_or_else(|| div().size(px(14.)).into_any_element())).collect();
                grid = grid.child(h_flex().gap(px(3.)).children(cells));
            }
            card()
                .child(toggle(
                    t("common.camera"),
                    Switch::new("cam", ToggleState::from(self.project.cam)).on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                        this.project.cam = *state == ToggleState::Selected;
                        this.changed(cx);
                    })),
                ))
                .when(self.project.cam, |s| {
                    s.child(row(
                        t("studio.row_size"),
                        self.slider(Knob::CamSize, (self.project.cam_size - studio::MIN_CAM) / (studio::MAX_CAM - studio::MIN_CAM), format!("{}%", (self.project.cam_size * 100.).round()), cx).into_any_element(),
                    ))
                    .child(row(t("studio.rounding"), self.slider(Knob::CamRound, self.project.cam_round, format!("{}%", (self.project.cam_round * 100.).round()), cx).into_any_element()))
                    .child(row(t("studio.row_position"), grid.into_any_element()))
                })
        });

        // Over the video: click rings and shortcut badges.
        let has_clicks = !self.track.clicks.is_empty();
        let has_keys = !self.track.keystrokes.is_empty();
        let keys_on = self.project.keys && has_keys;
        let key_sides: [&'static str; 2] = ["studio.bottom", "studio.top"];
        let key_sizes: Vec<&'static str> = KEY_SIZES.iter().map(|k| k.0).collect();
        let overlays = card()
            .child(toggle(
                t("studio.clicks"),
                Switch::new("clicks", ToggleState::from(self.project.clicks && has_clicks)).disabled(!has_clicks).on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                    this.project.clicks = *state == ToggleState::Selected;
                    this.changed(cx);
                })),
            ))
            .child(toggle(
                t("common.keyboard_shortcuts"),
                Switch::new("keys", ToggleState::from(keys_on)).disabled(!has_keys).on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                    this.project.keys = *state == ToggleState::Selected;
                    this.changed(cx);
                })),
            ))
            .when(keys_on, |s| {
                s.child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(pick(&key_sides, self.project.keys_top as usize, "keys-side", |p, i| p.keys_top = i == 1, cx)))
                        .child(div().flex_1().child(pick(&key_sizes, self.project.keys_size, "keys-size", |p, i| p.keys_size = i, cx))),
                )
            });

        // Export: file, size, rate, compression, sound; what comes out.
        let gif = self.project.format == Format::Gif;
        let files: [&'static str; 2] = ["MP4", "GIF"];
        let resolutions: Vec<&'static str> = RESOLUTIONS.iter().map(|r| r.0).collect();
        let rates: Vec<&'static str> = FRAME_RATES.iter().map(|r| r.0).collect();
        let qualities: Vec<&'static str> = QUALITIES.iter().map(|q| q.0).collect();
        let out = studio::layout(&self.probe, &self.project, false);
        let export = card()
            .child(pick(&files, gif as usize, "format", |p, i| p.format = if i == 1 { Format::Gif } else { Format::Mp4 }, cx))
            .child(pick(&resolutions, self.project.resolution, "res", |p, i| p.resolution = i, cx))
            .child(row(t("studio.row_fps"), pick(&rates, self.project.frame_rate, "fps", |p, i| p.frame_rate = i, cx)))
            .when(!gif, |s| s.child(row(t("studio.row_quality"), pick(&qualities, self.project.quality, "quality", |p, i| p.quality = i, cx))))
            .when(self.probe.audio && !gif, |s| {
                s.child(toggle(
                    t("studio.audio"),
                    Switch::new("audio", ToggleState::from(self.project.audio)).on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                        this.project.audio = *state == ToggleState::Selected;
                        this.changed(cx);
                    })),
                ))
            })
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new(format!("{}×{} · {}", out.ow, out.oh, clock(self.project.kept_duration()))).size(LabelSize::XSmall).color(Color::Muted))
                    .child(Label::new(size_label(&self.project, &self.probe)).size(LabelSize::Small)),
            );

        v_flex()
            .id("sidebar")
            .flex_none()
            .w(px(SIDEBAR))
            .h_full()
            .bg(sidebar_bg)
            .border_l_1()
            .border_color(divider)
            .overflow_y_scroll()
            .gap_2()
            .p_3()
            .child(context)
            .child(presets)
            .child(look)
            .children(cursor)
            .children(camera)
            .child(overlays)
            .child(export)
    }
}

impl Render for StudioEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for old in self.stale.drain(..) {
            let _ = window.drop_image(old);
        }
        // The preview takes whatever room the window leaves it.
        self.stage = self.stage_cell.get();
        let cell = self.stage_cell.clone();
        let measure = canvas(
            move |bounds, window, _| {
                let room = ((f32::from(bounds.size.width) - 2. * STAGE_PAD).max(160.), (f32::from(bounds.size.height) - 2. * STAGE_PAD).max(90.));
                let old = cell.get();
                if (room.0 - old.0).abs() > 0.5 || (room.1 - old.1).abs() > 0.5 {
                    cell.set(room);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        kit::root(cx)
            .flex()
            .flex_col()
            .bg(cx.theme().colors().panel_background)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.drag = None))
            .child(self.render_titlebar(cx))
            .child(
                h_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .child(
                                div()
                                    .relative()
                                    .w_full()
                                    .flex_1()
                                    .min_h_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(cx.theme().colors().editor_background)
                                    .child(measure)
                                    .child(self.render_preview(cx)),
                            )
                            .child(self.render_transport(cx)),
                    )
                    .child(div().h_full().child(self.render_sidebar(cx))),
            )
            .child(self.render_timeline(cx))
    }
}

/// Ruler text: whole seconds as "0:05", halves as "0:05.5".
fn ruler_label(t: f64, step: f64) -> String {
    if step < 1. {
        clock(t)
    } else {
        let t = t.round() as u64;
        format!("{}:{:02}", t / 60, t % 60)
    }
}

/// "1:05.3"
fn clock(t: f64) -> String {
    let t = t.max(0.);
    format!("{}:{:04.1}", (t / 60.) as u64, t % 60.)
}

/// "до 12 МБ" for MP4 (a ceiling), "≈ 30 МБ" for GIF (a guess).
fn size_label(project: &Project, probe: &Probe) -> String {
    let size = studio::human_size(project.estimate(probe));
    if project.format == Format::Gif { format!("≈ {size}") } else { tr!("studio.size_up_to", size) }
}

/// "1,2 с"
fn seconds(t: f64) -> String {
    tr!("common.seconds", crate::i18n::decimal(format!("{:.1}", t)))
}
