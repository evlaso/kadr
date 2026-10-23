//! The studio's video editor: a live preview
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
    Animation, AnimationExt, App, BorderStyle, BoxShadow, Bounds, Context, CursorStyle, FocusHandle, Image, ImageFormat, ImageSource,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, PathPromptOptions, Pixels, RenderImage,
    Window, canvas, div, img, linear_color_stop, linear_gradient, point, px, relative,
    rgba, size,
};
use crate::ui::{ButtonLike, IconButtonShape, Switch, ToggleState, Tooltip, prelude::*};

use crate::kit;
use crate::ui::fill;
use crate::studio::{
    self, ASPECT_ORDER, ASPECTS, CURSOR_RATE, CURSOR_STYLES, MOTION_BLUR, Camera, FRAME_RATES, Format, KEY_SIZES, MAX_CURSOR, MAX_PADDING, MAX_RADIUS, Probe, Project, QUALITIES,
    RESOLUTIONS, Seg,
};
use crate::track::Track;
use crate::{palette, trim, util};
use crate::i18n::t;

const FILMSTRIP: f32 = 52.;
const RULER: f32 = 22.;
/// The zoom lane, under the filmstrip.
const LANE: f32 = 34.;
/// The bottom row, that cuts are drawn on, and its gap above.
const CUT_LANE: f32 = 26.;
const CUT_GAP: f32 = 6.;
/// The section rail at the left, and the panel beside it.
const RAIL: f32 = 52.;
const PANEL: f32 = 264.;
const SIDEBAR: f32 = RAIL + PANEL;
/// Width of the labels in front of the export settings.
const ROW_LABEL: f32 = 64.;
/// Background tiles, like the screenshot editor's.
const TILE: f32 = 36.;
const TILE_RADIUS: f32 = 10.;
const TILE_GAP: f32 = 4.;
const DOT: f32 = 22.;
/// Gradients shown before "Show more": two rows.
const GRADIENT_ROWS: usize = 10;
/// The zoom level slider's range.
const ZOOM_RANGE: (f64, f64) = (1., 4.);
/// Air around the preview inside the window.
const STAGE_PAD: f32 = 24.;
/// Edge grab width on segments, in pixels.
const EDGE: f32 = 9.;
/// The timeline stretches up to this many times the window's width.
const MAX_TIMELINE_ZOOM: f32 = 8.;
const MIN_SEG: f64 = 0.3;
const NEW_SEG: f64 = 2.0;
const NEW_CUT: f64 = 1.0;
const PLAY_FPS: u32 = 60;
const WALLPAPERS: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Knob {
    Padding,
    Radius,
    Shadow,
    CursorSize,
    CamSize,
    CamRound,
    Volume,
    TimelineZoom,
    Zoom,
}

/// The rail's sections: one shown at a time.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Zoom,
    Cursor,
    Keys,
    Audio,
    Camera,
    Background,
    Cut,
}

impl Section {
    /// Top to bottom; Cut sits apart at the bottom.
    const RAIL: [Section; 6] = [Section::Zoom, Section::Cursor, Section::Keys, Section::Audio, Section::Camera, Section::Background];

    fn icon(self) -> &'static str {
        match self {
            Section::Zoom => "icons/zoom_in.svg",
            Section::Cursor => "icons/pointer.svg",
            Section::Keys => "icons/keyboard.svg",
            Section::Audio => "icons/audio_on.svg",
            Section::Camera => "icons/video.svg",
            Section::Background => "icons/image.svg",
            Section::Cut => "icons/scissors.svg",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Section::Zoom => t("studio.zoom"),
            Section::Cursor => t("studio.cursor"),
            Section::Keys => t("common.keyboard_shortcuts"),
            Section::Audio => t("studio.audio"),
            Section::Camera => t("common.camera"),
            Section::Background => t("studio.background"),
            Section::Cut => t("studio.cut_section"),
        }
    }
}

/// A choice's effect, for the shared control helpers.
type Set<T> = Rc<dyn Fn(&mut StudioEditor, T, &mut Context<StudioEditor>)>;

enum Drag {
    Playhead,
    Move { index: usize, grab: f64 },
    Edge { index: usize, start: bool },
    Trim { start: bool },
    CutMove { index: usize, grab: f64 },
    CutEdge { index: usize, start: bool },
    /// A cut being drawn across the cut row from where the press was.
    CutNew { index: usize, anchor: f64 },
    Slider(Knob),
}

pub struct StudioEditor {
    /// Show the recording's card again on closing, unless an export
    /// showed one.
    card_on_close: bool,
    video: PathBuf,
    probe: Probe,
    track: Track,
    project: Project,
    camera: Camera,
    /// The drawn cursor over time (recordings without the system cursor).
    cursor: Vec<(f64, f64, f64)>,
    /// How it tilts and grows along that path: (t, angle, scale).
    cursor_motion: Vec<(f64, f64, f64)>,
    /// Its shape changes, timed to that path.
    cursor_shapes: Vec<(f64, String)>,
    thumbs: Vec<Arc<Image>>,
    walls: Vec<(PathBuf, Arc<Image>)>,
    /// Saved looks, by name.
    presets: crate::presets::Presets,
    /// The side panel's scroll, and its scrollbar's grab.
    side_scroll: gpui::ScrollHandle,
    side_grab: std::rc::Rc<std::cell::Cell<Option<f32>>>,
    /// The rail's chosen section, shown in the panel beside it.
    section: Section,
    /// Something changed since the window opened.
    edited: bool,
    /// The export settings under the Export button, and the aspect menu;
    /// an open one's bounds, so clicks inside don't close it.
    export_open: bool,
    aspect_open: bool,
    menu_bounds: crate::ui::MenuBounds,
    more_gradients: bool,
    /// The zoom level new zooms get.
    new_zoom: f64,
    /// The preview alone, without the rail and its panel.
    expanded: bool,
    /// The cursor styles as small pictures, for their tiles.
    style_thumbs: Vec<Option<Arc<Image>>>,
    /// The timeline's sideways scroll when it's zoomed, and its bar's grab.
    timeline_scroll: gpui::ScrollHandle,
    timeline_grab: Rc<Cell<Option<f32>>>,
    frame: Option<ImageSource>,
    /// The quick frame a sharp one fades in over, and that fade's number.
    under: Option<ImageSource>,
    fade: usize,
    /// The camera recorded alongside (studio mode), where its frame 0 sits
    /// on the recording's clock, and its frame at the playhead.
    cam_file: Option<PathBuf>,
    cam_offset: f64,
    cam_frame: Option<ImageSource>,
    /// Playing inside the editor; any seek stops it.
    playing: bool,
    /// The recording's sound while playing.
    sound: Option<Sound>,
    /// Played frames no longer on screen, freed from the atlas on render.
    stale: Vec<Arc<RenderImage>>,
    /// Small frames shown at once while dragging over the timeline.
    scrub: Option<Arc<crate::scrub::Scrub>>,
    /// The seek whose frame (quick or exact) is on screen.
    shown: u64,
    playhead: f64,
    /// The selected zoom segment, or the selected cut; never both.
    selected: Option<usize>,
    cut: Option<usize>,
    drag: Option<Drag>,
    /// Waiting for a click on the preview to set the selected segment's focus.
    placing: bool,
    /// The pointer over the timeline: what a press there would do.
    timeline_cursor: CursorStyle,
    /// How many times wider than the window the timeline is drawn.
    timeline_zoom: f32,
    /// Room for the preview (it fits the output's shape inside).
    stage: (f32, f32),
    /// The preview area measured in the last layout; `stage` follows it.
    stage_cell: Rc<Cell<(f32, f32)>>,
    /// Timeline x origin and width, and the zoom lane's and the filmstrip's
    /// tops, in window coordinates.
    timeline: Rc<Cell<(f32, f32, f32, f32)>>,
    /// The video box inside the preview, in window coordinates.
    video_box: Rc<Cell<(f32, f32, f32, f32)>>,
    /// Slider tracks (x, width) in window coordinates.
    sliders: Rc<RefCell<HashMap<Knob, (f32, f32)>>>,
    request: Rc<Cell<u64>>,
    focus: FocusHandle,
}

pub fn open(video: PathBuf, cx: &mut App) {
    open_as(video, false, cx);
}

/// Opened from a recording's card: closing without exporting brings the
/// card back (an export shows its own).
pub fn open_from_card(video: PathBuf, cx: &mut App) {
    open_as(video, true, cx);
}

fn open_as(video: PathBuf, card: bool, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        // A `.kadr`: the recording and its edit, unpacked into the cache.
        let video = if crate::bundle::is_bundle(&video) {
            let file = video.clone();
            let unpacked = cx
                .background_executor()
                .spawn(async move { crate::bundle::unpack(&file).and_then(|u| studio::from_bundle(&file, &u)) })
                .await;
            match unpacked {
                Ok(v) => v,
                Err(e) => return util::notify(t("bundle.couldnt_open"), &format!("{}: {e}", video.display())),
            }
        } else {
            video
        };
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
        // All at once: the window waits for the slowest, not their sum.
        // The first frame comes while the window is made (setting up the
        // graphics takes as long); the filmstrip follows.
        let frame = cx.background_executor().spawn({
            let (video, at, width, executor) = (video.clone(), project.trim.0, (stage.0 * 2.) as u32, cx.background_executor().clone());
            async move { trim::sharp_frame(&video, at, width, 0, executor).await }
        });
        let walls = wallpaper_thumbs().await;
        let _ = cx.update(|cx| {
            let size = (stage.0 + 2. * STAGE_PAD + SIDEBAR, stage.1 + chrome_h - 80.);
            kit::open_window(cx, "kadr-studio", Some(&mon), size, (960., 620.), |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    let camera = Camera::new(&track, &probe, &project.segments);
                    let cursor = if track.cursor_hidden { studio::cursor_path(&track, &probe, project.cursor_smooth) } else { Vec::new() };
                    let cursor_motion = studio::cursor_motion(&cursor, &track, &probe);
                    let cursor_shapes = studio::cursor_shapes(&track, &cursor, &probe);
                    let playhead = project.trim.0;
                    let video_path = video.clone();
                    let scrub = crate::scrub::Scrub::start(&video_path, probe.fps_value());
                    cx.spawn(async move |this, cx| {
                        let frame = frame.await;
                        let _ = this.update(cx, |editor: &mut StudioEditor, cx| {
                            // Unless a seek got there first.
                            if editor.frame.is_none() {
                                editor.frame = frame.map(ImageSource::Render);
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                    let duration = probe.duration;
                    let strip_of = video_path.clone();
                    cx.spawn(async move |this, cx| {
                        let thumbs = trim::filmstrip(&strip_of, duration).await;
                        let _ = this.update(cx, |editor: &mut StudioEditor, cx| {
                            // Unless the exact frames came first.
                            if editor.thumbs.is_empty() {
                                editor.thumbs = thumbs;
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                    trim::exact_strip(scrub.clone(), trim::THUMBS, |editor: &mut StudioEditor| &mut editor.thumbs, cx);
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
                    if card {
                        cx.on_release(|editor: &mut StudioEditor, cx: &mut App| {
                            if editor.card_on_close && editor.video.exists() {
                                crate::cards::show(editor.video.clone(), cx);
                            }
                        })
                        .detach();
                    }
                    StudioEditor {
                        card_on_close: card,
                        video,
                        probe,
                        track,
                        project,
                        camera,
                        cursor,
                        cursor_motion,
                        cursor_shapes,
                        thumbs: Vec::new(),
                        walls,
                        presets: crate::presets::Presets::new(studio::presets(), studio::save_presets, cx),
                        side_scroll: gpui::ScrollHandle::new(),
                        side_grab: Default::default(),
                        section: Section::Zoom,
                        edited: false,
                        export_open: false,
                        aspect_open: false,
                        menu_bounds: Default::default(),
                        more_gradients: false,
                        new_zoom: crate::settings::Settings::load().studio_zoom,
                        expanded: false,
                        style_thumbs: [0, 2, 1]
                            .into_iter()
                            .map(|style| studio::cursor_image(style, "default", 26., 0.).map(|(png, _, _)| Arc::new(Image::from_bytes(ImageFormat::Png, png))))
                            .collect(),
                        timeline_scroll: gpui::ScrollHandle::new(),
                        timeline_grab: Default::default(),
                        frame: None,
                        under: None,
                        fade: 0,
                        cam_file: Some(studio::camera_file(&video_path)).filter(|f| f.exists()),
                        cam_offset: studio::camera_offset(&video_path),
                        cam_frame: None,
                        playing: false,
                        sound: None,
                        stale: Vec::new(),
                        scrub,
                        shown: 0,
                        playhead,
                        selected: None,
                        cut: None,
                        drag: None,
                        placing: false,
                        timeline_cursor: CursorStyle::PointingHand,
                        timeline_zoom: 1.,
                        stage,
                        stage_cell: Rc::new(Cell::new(stage)),
                        timeline: Rc::new(Cell::new((0., 1., 0., 0.))),
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
            self.cursor_motion = studio::cursor_motion(&self.cursor, &self.track, &self.probe);
            self.cursor_shapes = studio::cursor_shapes(&self.track, &self.cursor, &self.probe);
        }
        self.project.save(&self.video);
        self.edited = true;
        // The next recording starts with this look.
        self.project.remember_look();
        cx.notify();
    }

    fn time_at(&self, x: f32) -> f64 {
        let (ox, w, _, _) = self.timeline.get();
        (((x - ox) / w.max(1.)) as f64).clamp(0., 1.) * self.probe.duration
    }

    fn x_of(&self, t: f64) -> f32 {
        let (_, w, _, _) = self.timeline.get();
        w * (t / self.probe.duration.max(0.001)) as f32
    }

    /// Put a frame on the stage. `fade`: a sharp frame fading in over the
    /// quick one, rather than snapping.
    fn show(&mut self, frame: ImageSource, fade: bool) {
        if let Some(ImageSource::Render(old)) = self.under.take() {
            self.stale.push(old);
        }
        let old = self.frame.replace(frame);
        if fade && old.is_some() {
            self.under = old;
            self.fade += 1;
        } else if let Some(ImageSource::Render(old)) = old {
            self.stale.push(old);
        }
    }

    fn seek(&mut self, t: f64, cx: &mut Context<Self>) {
        self.playing = false;
        self.sound = None;
        self.playhead = t.clamp(0., self.probe.duration);
        let id = self.request.get() + 1;
        self.request.set(id);
        let (video, width, request, at) = (self.video.clone(), (self.stage.0 * 2.) as u32, self.request.clone(), self.playhead);
        let (cam_file, cam_offset) = (self.cam_file.clone(), self.cam_offset);
        // The quick frame right away, unless a later one got there first.
        if let Some(scrub) = self.scrub.clone() {
            scrub.want(id);
            cx.spawn(async move |this, cx| {
                let Some(quick) = cx.background_executor().spawn(async move { scrub.frame(at, id) }).await else { return };
                let _ = this.update(cx, |editor: &mut StudioEditor, cx| {
                    if id > editor.shown && !editor.playing {
                        editor.shown = id;
                        editor.show(ImageSource::Render(quick), false);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(40)).await;
            if request.get() != id {
                return;
            }
            let frame = trim::sharp_frame(&video, at, width, id, cx.background_executor().clone()).await;
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
                    editor.shown = id;
                    if let Some(frame) = frame {
                        editor.show(ImageSource::Render(frame), true);
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

    /// Sound from `start`, keeping what `select` keeps (see `play_from`).
    fn play_sound(&mut self, start: f64, select: &str) {
        let mut filters = vec![format!("aselect='{select}',asetpts=N/SR/TB")];
        filters.extend(self.project.audio_filters());
        self.sound = (self.probe.audio && self.project.audio).then(|| Sound::play(&self.video, start, &filters.join(","))).flatten();
    }

    /// While playing, the sound starts over from the playhead with the new
    /// volume and channels.
    fn sound_changed(&mut self) {
        if !self.playing {
            return;
        }
        let start = self.playhead;
        let kept: Vec<(f64, f64)> = self.project.kept().into_iter().filter(|&(_, b)| b > start + 0.01).map(|(a, b)| (a.max(start) - start, b - start)).collect();
        self.sound = None;
        self.play_sound(start, &studio::keep_expr(&kept));
    }

    fn play_from(&mut self, start: f64, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.request.get() + 1;
        self.request.set(id);
        self.playing = true;
        self.playhead = start;
        // What's kept from here on, on the clock of the decoders below (they
        // start at 0 at `start`): one stream each, cut pieces dropped by
        // ffmpeg, so playback runs through a cut without stopping.
        let kept: Vec<(f64, f64)> = self.project.kept().into_iter().filter(|&(_, b)| b > start + 0.01).map(|(a, b)| (a.max(start) - start, b - start)).collect();
        let select = studio::keep_expr(&kept);
        // The previous one stops as this one starts.
        self.sound = None;
        self.play_sound(start, &select);

        let even = |v: f64| ((v / 2.).round() * 2.).max(2.) as usize;
        let w = even((self.stage.0 as f64 * 1.5).min(self.probe.w));
        let h = even(w as f64 * self.probe.h / self.probe.w);
        let (mut tx, mut rx) = futures::channel::mpsc::channel::<Vec<u8>>(4);
        let video = self.video.clone();
        let cam_select = select.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let Ok(mut child) = std::process::Command::new("ffmpeg")
                .args(["-nostdin", "-loglevel", "error", "-ss", &format!("{start:.3}"), "-i"])
                .arg(&video)
                .args(["-an", "-vf", &format!("fps={PLAY_FPS},select='{select}',setpts=N/{PLAY_FPS}/TB,scale={w}:{h}"), "-f", "rawvideo", "-pix_fmt", "bgra", "-"])
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
                    .args(["-an", "-vf", &format!("fps={PLAY_FPS},select='{cam_select}',setpts=N/{PLAY_FPS}/TB,crop='min(iw,ih)':'min(iw,ih)',scale={CAM}:{CAM}"), "-f", "rawvideo", "-pix_fmt", "bgra", "-"])
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
                let t = studio::played_to_source(&kept, n as f64 / PLAY_FPS as f64).map(|t| start + t);
                let keep_going = this.update_in(cx, |editor, _, cx| {
                    if request.get() != id {
                        return false;
                    }
                    match t {
                        Some(t) => {
                            editor.show(ImageSource::Render(shown), false);
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

    /// Picking a zoom or a cut on the timeline shows its section.
    fn select_zoom(&mut self, index: Option<usize>) {
        self.selected = index;
        self.cut = None;
        self.placing = false;
        if index.is_some() {
            self.show_section(Section::Zoom);
        }
    }

    fn select_cut(&mut self, index: Option<usize>) {
        self.cut = index;
        self.selected = None;
        self.placing = false;
        if index.is_some() {
            self.show_section(Section::Cut);
        }
    }

    fn show_section(&mut self, section: Section) {
        if self.section != section {
            self.section = section;
            self.side_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.expanded = false;
    }

    /// The zoom level the slider shows: the selected zoom's, or the one new
    /// zooms get.
    fn zoom_level(&self) -> f64 {
        self.selected.and_then(|i| self.project.segments.get(i)).map_or(self.new_zoom, |s| s.zoom)
    }

    /// Cut a second out at the playhead; its edges then set the length.
    /// Cut a piece from the playhead. Inside a cut already, that one is
    /// selected instead; a new cut stops where the next one begins.
    fn add_cut(&mut self, cx: &mut Context<Self>) {
        let (lo, hi) = self.project.trim;
        let start = self.playhead.clamp(lo, (hi - MIN_SEG).max(lo));
        if let Some(i) = self.project.cuts.iter().position(|&(a, b)| start >= a.min(b) - 0.01 && start <= a.max(b) + 0.01) {
            self.select_cut(Some(i));
            return cx.notify();
        }
        let next = self.project.cuts.iter().map(|&(a, b)| a.min(b)).filter(|&a| a > start).fold(hi, f64::min);
        let end = (start + NEW_CUT).min(next);
        if end - start < 0.05 {
            return;
        }
        self.project.cuts.push((start, end));
        self.project.merge_cuts();
        let index = self.project.cuts.iter().position(|&(a, _)| (a - start).abs() < 0.001);
        self.select_cut(index);
        self.changed(cx);
    }

    /// A drag on the timeline ended: cuts dragged onto each other join.
    fn drag_done(&mut self, cx: &mut Context<Self>) {
        let drag = self.drag.take();
        if let Some(Drag::Slider(Knob::Volume)) = drag {
            self.sound_changed();
        }
        // A click on the cut row, not a drag: no cut, the playhead goes there.
        if let Some(Drag::CutNew { index, anchor }) = drag
            && self.project.cuts.get(index).is_some_and(|&(a, b)| b - a < 0.05)
        {
            self.project.cuts.remove(index);
            self.cut = None;
            self.seek(anchor, cx);
            return self.changed(cx);
        }
        if matches!(drag, Some(Drag::CutMove { .. } | Drag::CutEdge { .. } | Drag::CutNew { .. })) {
            let selected = self.cut.and_then(|i| self.project.cuts.get(i).copied());
            self.project.merge_cuts();
            self.cut = selected.and_then(|(a, b)| {
                let mid = (a + b) / 2.;
                self.project.cuts.iter().position(|&(x, y)| mid >= x && mid <= y)
            });
            self.changed(cx);
        }
    }

    fn timeline_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        let (ox, _, lane_top, strip_top) = self.timeline.get();
        let (x, y) = (f32::from(e.position.x), f32::from(e.position.y));
        let t = self.time_at(x);
        let local = x - ox;
        self.placing = false;
        self.export_open = false;
        self.aspect_open = false;

        if y >= strip_top && y < strip_top + FILMSTRIP {
            // The filmstrip: its trim handles, else the playhead.
            let (a, b) = (self.x_of(self.project.trim.0), self.x_of(self.project.trim.1));
            if (local - a).abs() <= EDGE + 3. {
                self.drag = Some(Drag::Trim { start: true });
                return cx.notify();
            }
            if (local - b).abs() <= EDGE + 3. {
                self.drag = Some(Drag::Trim { start: false });
                return cx.notify();
            }
        }
        let cut_top = lane_top + LANE + CUT_GAP;
        if (cut_top..cut_top + CUT_LANE).contains(&y) {
            // The cut row: edges resize, bodies move, a drag on empty space
            // cuts that stretch out.
            let hit = self.project.cuts.iter().enumerate().rev().find_map(|(i, &(s, e))| block_hit(local, self.x_of(s), self.x_of(e)).map(|part| match part {
                Part::Start => Drag::CutEdge { index: i, start: true },
                Part::End => Drag::CutEdge { index: i, start: false },
                Part::Body => Drag::CutMove { index: i, grab: t - s },
            }));
            let drag = hit.unwrap_or_else(|| {
                self.project.cuts.push((t, t));
                Drag::CutNew { index: self.project.cuts.len() - 1, anchor: t }
            });
            if let Drag::CutEdge { index, .. } | Drag::CutMove { index, .. } | Drag::CutNew { index, .. } = drag {
                self.select_cut(Some(index));
            }
            self.drag = Some(drag);
            return cx.notify();
        }
        if !(lane_top..lane_top + LANE).contains(&y) {
            // The ruler, the filmstrip's free parts and the gaps: the playhead.
            self.drag = Some(Drag::Playhead);
            return self.seek(t, cx);
        }
        // The zoom lane: edges resize, bodies move, empty space adds.
        let hit = self.project.segments.iter().enumerate().rev().find_map(|(i, s)| block_hit(local, self.x_of(s.start), self.x_of(s.end)).map(|part| match part {
            Part::Start => Drag::Edge { index: i, start: true },
            Part::End => Drag::Edge { index: i, start: false },
            Part::Body => Drag::Move { index: i, grab: t - s.start },
        }));
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
                self.project.segments.push(Seg { start, end, zoom: self.new_zoom, focus: None });
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
        let (ox, w, lane_top, strip_top) = self.timeline.get();
        let local = x - ox;
        let cut_top = lane_top + LANE + CUT_GAP;
        if local < -EDGE || local > w + EDGE || y < strip_top - RULER - 6. || y > cut_top + CUT_LANE {
            return CursorStyle::Arrow;
        }
        let near = |t: f64| (local - self.x_of(t)).abs() <= EDGE + 3.;
        if (strip_top..strip_top + FILMSTRIP).contains(&y) && (near(self.project.trim.0) || near(self.project.trim.1)) {
            return CursorStyle::ResizeLeftRight;
        }
        let over = |blocks: Vec<(f64, f64)>, empty: CursorStyle| {
            match blocks.into_iter().find_map(|(a, b)| block_hit(local, self.x_of(a), self.x_of(b))) {
                Some(Part::Body) => CursorStyle::OpenHand,
                Some(_) => CursorStyle::ResizeLeftRight,
                None => empty,
            }
        };
        if (cut_top..cut_top + CUT_LANE).contains(&y) {
            return over(self.project.cuts.clone(), CursorStyle::Crosshair);
        }
        if (lane_top..lane_top + LANE).contains(&y) {
            return over(self.project.segments.iter().map(|s| (s.start, s.end)).collect(), CursorStyle::PointingHand);
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
            Some(Drag::CutNew { index, anchor }) => {
                let t = t.clamp(0., d);
                self.project.cuts[index] = (anchor.min(t), anchor.max(t));
                self.changed(cx);
                self.seek(t, cx);
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
            Knob::CamSize => self.project.cam_size = studio::MIN_CAM + v * (studio::MAX_CAM - studio::MIN_CAM),
            Knob::CamRound => self.project.cam_round = v,
            // In steps of 5%, like the label shows it.
            Knob::Volume => self.project.volume = (v * studio::MAX_VOLUME * 20.).round() / 20.,
            Knob::TimelineZoom => {
                self.timeline_zoom = 1. + v as f32 * (MAX_TIMELINE_ZOOM - 1.);
                return cx.notify();
            }
            Knob::Zoom => {
                // In steps of 5%, like the label shows it.
                let level = ((ZOOM_RANGE.0 + v * (ZOOM_RANGE.1 - ZOOM_RANGE.0)) * 20.).round() / 20.;
                match self.selected.and_then(|i| self.project.segments.get_mut(i)) {
                    Some(seg) => seg.zoom = level,
                    None => {
                        self.new_zoom = level;
                        return cx.notify();
                    }
                }
            }
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
        // The export's own card follows.
        self.card_on_close = false;
        window.remove_window();
        // Opened from a `.kadr`: the project file keeps up with the edit.
        if let Some(target) = crate::bundle::origin_of(&self.video) {
            self.write_bundle(target, false, cx);
        }
        studio::export(self.video.clone(), self.project.clone(), false, cx);
    }

    /// Keep the recording and its whole edit as a `.kadr`, beside the
    /// recordings (or where this session's came from).
    fn save_project(&mut self, cx: &mut Context<Self>) {
        let target = crate::bundle::origin_of(&self.video).unwrap_or_else(|| crate::bundle::target_for(&self.video, &crate::settings::Settings::load().video_dir()));
        self.write_bundle(target, true, cx);
    }

    fn write_bundle(&self, target: PathBuf, tell: bool, cx: &mut Context<Self>) {
        let (video, project) = (self.video.clone(), self.project.clone());
        let hold = kit::hold(cx);
        cx.spawn(async move |_, cx| {
            let _hold = hold;
            let result = cx.background_executor().spawn(async move { studio::save_bundle(&video, &project, &target).map(|_| target) }).await;
            match result {
                Ok(target) if tell => util::notify(t("bundle.project_saved"), &target.display().to_string()),
                Ok(_) => {}
                Err(e) => util::notify(t("bundle.couldnt_save"), &e.to_string()),
            }
        })
        .detach();
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        match (k.key.as_str(), ctrl) {
            ("escape", _) if self.export_open || self.aspect_open => {
                self.export_open = false;
                self.aspect_open = false;
            }
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
            .children(self.under.clone().into_iter().chain(self.frame.clone()).enumerate().map(|(i, frame)| {
                // Over the quick frame, the sharp one fades in.
                let fading = i == 1;
                // The zoomed frame is bigger than the box, and GPUI clips
                // children to rectangles; painting it ourselves rounds the
                // visible part to the box's corners.
                let painted = canvas(
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
                .size_full();
                let layer = div().absolute().inset_0().child(painted);
                if fading {
                    layer.with_animation(("fade", self.fade), Animation::new(trim::FADE), |el, d| el.opacity(d)).into_any_element()
                } else {
                    layer.into_any_element()
                }
            }))
            .children(self.render_cursor(left, top, scale))
            .child(tracker)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.preview_down(e, cx)));

        let (c0, c1) = self.project.colors();
        let (c0, c1) = (crate::ui::from_rgba(c0), crate::ui::from_rgba(c1));
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
                    let _ = window.paint_image(bounds, shown, px(r).into(), crate::ui::SMOOTHING, data, 0, false);
                },
            )
            .absolute()
            .left(px(cx0))
            .top(px(cy0))
            .w(px(side))
            .h(px(side))
        });
        // The shortcut being shown at the playhead, as the export draws it.
        let strokes = &self.track.keystrokes;
        let key_pill = (self.project.keys).then(|| {
            let i = strokes.iter().rposition(|(t, _)| *t <= self.playhead)?;
            let (t, label) = &strokes[i];
            let next = strokes.get(i + 1).map_or(self.probe.duration, |n| n.0);
            let show = (next - t).clamp(0.35, 1.4);
            (self.playhead < t + show).then(|| label.clone())
        })
        .flatten()
        .map(|label| {
            let text = (l.ow.min(l.oh) as f64 * studio::KEY_SIZES[self.project.keys_size.min(studio::KEY_SIZES.len() - 1)].1) as f32 * k;
            let margin = l.h as f32 * 0.05 * k;
            let pill = div()
                .px(px(text * 0.75))
                .h(px(text * 1.9))
                .flex()
                .items_center()
                .rounded_full()
                .bg(gpui::rgba(0x161618dc))
                .text_color(gpui::white())
                .text_size(px(text))
                .font_family(kit::ANNOTATION_FONT)
                .child(label);
            let row = div().absolute().left_0().right_0().flex().justify_center().child(pill);
            if self.project.keys_top { row.top(px(by + margin)) } else { row.top(px(by + bh - margin - text * 1.9)) }
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
            .children(key_pill)
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
        // It tilts, grows and bounces on clicks here as in the export.
        let (angle, grow) = self.cursor_motion.get(i).filter(|_| self.project.cursor_motion).map_or((0., 1.), |m| (m.1, m.2));
        let height = studio::cursor_height(&self.track, &self.probe, self.project.cursor_size);
        let (png, (hx, hy), (w, h)) = studio::cursor_image(self.project.cursor_style, studio::shape_at(&self.cursor_shapes, self.playhead), height, angle)?;
        let (hx, hy, w, h) = (hx * grow, hy * grow, w as f64 * grow, h as f64 * grow);
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

    /// The timeline: a ruler, the filmstrip (with the trim handles, cut
    /// pieces dimmed), the zoom lane, and the row cuts are drawn on.
    fn render_timeline(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let status = cx.theme().status();
        let accent = colors.text_accent;
        let faint = |alpha: f32| {
            let mut c = colors.text;
            c.alpha = alpha;
            c
        };
        let (grid, tick, block) = (faint(0.05), faint(0.22), faint(0.2));
        let lane_bg = colors.element_background;
        let (handle_color, red) = (status.warning, status.error);
        let thumbs = self.thumbs.iter().map(|t| img(t.clone()).flex_1().h_full().object_fit(ObjectFit::Cover));
        let d = self.probe.duration.max(0.001);
        let segments: Vec<(f64, f64, bool)> = self.project.segments.iter().enumerate().map(|(i, s)| (s.start, s.end, self.selected == Some(i))).collect();
        let cuts: Vec<(f64, f64, bool)> = self.project.cuts.iter().enumerate().map(|(i, &(a, b))| (a.min(b), a.max(b), self.cut == Some(i))).collect();
        let trim = self.project.trim;
        let playhead = self.playhead;
        let timeline = self.timeline.clone();
        // About ten labelled ticks, on round seconds, more as it's zoomed.
        let shown = d / self.timeline_zoom as f64;
        let step = [0.5, 1., 2., 5., 10., 15., 30., 60., 120., 300.].into_iter().find(|s| shown / s <= 12.).unwrap_or(600.);
        let marks: Vec<f64> = (0..).map(|i| i as f64 * step).take_while(|&t| t <= d).collect();
        let ticks = marks.clone();
        // The frames first, then the zooms, then the cuts.
        let strip_y = RULER + 6.;
        let lane_y = strip_y + FILMSTRIP + 8.;
        let cut_y = lane_y + LANE + CUT_GAP;
        let height = cut_y + CUT_LANE + 4.;
        // The cut row stands out while its section is open.
        let cut_lane_bg = if self.section == Section::Cut { colors.element_selected } else { lane_bg };
        let cut_color = status.error;

        // Grid, lane, blocks, trim, cuts and the playhead are painted; text
        // goes on top.
        let painter = canvas(
            move |bounds, _, _| bounds,
            move |_, bounds, window, _| {
                let (x0, w, y0) = (bounds.origin.x, bounds.size.width, bounds.origin.y);
                let at = |t: f64| x0 + w * (t / d) as f32;
                let (lane_top, strip_top) = (y0 + px(lane_y), y0 + px(strip_y));
                timeline.set((f32::from(x0), f32::from(w), f32::from(lane_top), f32::from(strip_top)));
                for &t in &ticks {
                    window.paint_quad(fill(Bounds::new(point(at(t), y0 + px(RULER - 5.)), size(px(1.), px(5.))), tick));
                    window.paint_quad(fill(Bounds::new(point(at(t), y0 + px(RULER)), size(px(1.), px(height - RULER))), grid));
                }
                // The zoom lane: a soft track, the zooms on it.
                window.paint_quad(crate::ui::quad(Bounds::new(point(x0, lane_top), size(w, px(LANE))), px(9.), lane_bg, px(0.), lane_bg, BorderStyle::Solid));
                for &(s, e, selected) in &segments {
                    let rect = Bounds::new(point(at(s), lane_top + px(3.)), size((at(e) - at(s)).max(px(4.)), px(LANE - 6.)));
                    let color = if selected { accent } else { block };
                    window.paint_quad(crate::ui::quad(rect, px(6.), color, px(0.), color, BorderStyle::Solid));
                }
                // Trimmed-off ends are dimmed, with handles on the kept part.
                let shade = gpui::hsla(0., 0., 0., 0.62);
                if trim.0 > 0. {
                    window.paint_quad(crate::ui::quad(Bounds::new(point(x0, strip_top), size(at(trim.0) - x0, px(FILMSTRIP))), px(8.), shade, px(0.), shade, BorderStyle::Solid));
                }
                if trim.1 < d {
                    window.paint_quad(crate::ui::quad(Bounds::new(point(at(trim.1), strip_top), size(x0 + w - at(trim.1), px(FILMSTRIP))), px(8.), shade, px(0.), shade, BorderStyle::Solid));
                }
                if trim.0 > 0. || trim.1 < d {
                    let kept = Bounds::new(point(at(trim.0), strip_top), size(at(trim.1) - at(trim.0), px(FILMSTRIP)));
                    window.paint_quad(crate::ui::quad(kept, px(8.), gpui::transparent_black(), px(2.), handle_color, BorderStyle::Solid));
                }
                for (x, left) in [(at(trim.0), true), (at(trim.1), false)] {
                    let hx = if left { x } else { x - px(8.) };
                    window.paint_quad(crate::ui::quad(Bounds::new(point(hx, strip_top), size(px(8.), px(FILMSTRIP))), px(4.), handle_color, px(0.), handle_color, BorderStyle::Solid));
                    let grip = gpui::hsla(0., 0., 0., 0.5);
                    window.paint_quad(fill(Bounds::new(point(hx + px(3.5), strip_top + px(FILMSTRIP / 2. - 7.)), size(px(1.), px(14.))), grip));
                }
                // Cut pieces: dimmed in the strip, so what goes stays
                // visible; blocks on the cut row under it.
                let cut_top = lane_top + px(LANE + CUT_GAP);
                window.paint_quad(crate::ui::quad(Bounds::new(point(x0, cut_top), size(w, px(CUT_LANE))), px(7.), cut_lane_bg, px(0.), cut_lane_bg, BorderStyle::Solid));
                for &(a, b, selected) in &cuts {
                    let width = (at(b) - at(a)).max(px(2.));
                    window.paint_quad(fill(Bounds::new(point(at(a), strip_top), size(width, px(FILMSTRIP))), shade));
                    let mut soft = cut_color;
                    soft.alpha = 0.28;
                    let block = Bounds::new(point(at(a), cut_top + px(2.)), size(width.max(px(4.)), px(CUT_LANE - 4.)));
                    let body = if selected { cut_color } else { soft };
                    window.paint_quad(crate::ui::quad(block, px(5.), body, px(1.), cut_color, BorderStyle::Solid));
                }
                // The playhead: a red line with a rounded head on the ruler.
                let x = at(playhead);
                window.paint_quad(fill(Bounds::new(point(x - px(1.), y0 + px(6.)), size(px(2.), px(height - 6.))), red));
                window.paint_quad(crate::ui::quad(Bounds::new(point(x - px(6.), y0), size(px(12.), px(14.))), px(6.), red, px(0.), red, BorderStyle::Solid));
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        let labels = self.project.segments.iter().enumerate().map(|(i, s)| {
            let (a, b) = ((s.start / d) as f32, (s.end / d) as f32);
            let on = self.selected == Some(i);
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
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(if on { gpui::white() } else { colors.text })
                        .child(format!("{}%", (s.zoom * 100.).round())),
                )
                .when(s.focus.is_some(), |d| d.child(Icon::new(IconName::Crosshair).size(IconSize::XSmall).color(if on { Color::Custom(gpui::white()) } else { Color::Muted })))
        });
        // On each cut block, the scissors and how long it is, where it fits.
        let badges = self.project.cuts.iter().enumerate().map(|(i, &(a, b))| {
            let (l, r) = ((a.min(b) / d) as f32, (a.max(b) / d) as f32);
            let on = self.cut == Some(i);
            let ink = if on { gpui::white() } else { colors.text };
            div()
                .absolute()
                .left(relative(l))
                .w(relative((r - l).max(0.)))
                .top(px(cut_y))
                .h(px(CUT_LANE))
                .flex()
                .items_center()
                .justify_center()
                .gap_1()
                .overflow_hidden()
                .child(Icon::new(IconName::Scissors).size(IconSize::XSmall).color(Color::Custom(ink)))
                .child(div().text_xs().whitespace_nowrap().text_color(ink).child(seconds((b - a).abs())))
        });
        let cut_hint = self.project.cuts.is_empty().then(|| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(cut_y))
                .h(px(CUT_LANE))
                .flex()
                .items_center()
                .justify_center()
                .gap_1()
                .child(Icon::new(IconName::Scissors).size(IconSize::XSmall).color(Color::Placeholder))
                .child(Label::new(t("studio.drag_to_cut")).size(LabelSize::XSmall).color(Color::Placeholder))
        });
        let ruler = marks.into_iter().map(|t| div().absolute().top(px(2.)).left(relative((t / d) as f32)).ml_1().child(Label::new(ruler_label(t, step)).size(LabelSize::XSmall).color(Color::Muted)));
        let hint = self.project.segments.is_empty().then(|| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(lane_y))
                .h(px(LANE))
                .flex()
                .items_center()
                .justify_center()
                .child(Label::new(t("studio.click_track_add_zoom")).size(LabelSize::XSmall).color(Color::Placeholder))
        });

        let lane = div()
            .id("timeline")
            .relative()
            .w(relative(self.timeline_zoom))
            .h(px(height))
            .cursor(match self.drag {
                Some(Drag::Move { .. } | Drag::CutMove { .. }) => CursorStyle::ClosedHand,
                Some(Drag::Edge { .. } | Drag::CutEdge { .. } | Drag::CutNew { .. } | Drag::Trim { .. }) => CursorStyle::ResizeLeftRight,
                _ => self.timeline_cursor,
            })
            .children(ruler)
            .child(div().absolute().top(px(strip_y)).left_0().right_0().h(px(FILMSTRIP)).rounded(px(8.)).overflow_hidden().child(h_flex().size_full().children(thumbs)))
            .children(hint)
            .child(painter)
            .children(labels)
            .children(cut_hint)
            .children(badges)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.timeline_down(e, cx)));
        div()
            .relative()
            .w_full()
            .px_4()
            .pt_2()
            .pb_3()
            .child(div().id("timeline-scroll").w_full().overflow_x_scroll().track_scroll(&self.timeline_scroll).child(lane))
            .when(self.timeline_zoom > 1.01, |d| d.child(hscrollbar(&self.timeline_scroll, &self.timeline_grab, cx)))
    }

    /// The title centred with "Edited" once changed; the project and the
    /// export at the right, with the window's own controls where the
    /// system wants them.
    fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.video.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let colors = cx.theme().colors().clone();
        let left = h_flex().flex_1().min_w_0().h_full().children(kit::window_controls(false, cx)).child(kit::drags_window(div().flex_1().h_full()));
        let title = h_flex()
            .flex_none()
            .max_w(px(560.))
            .min_w_0()
            .gap_2()
            .child(div().min_w_0().child(Label::new(name).truncate()))
            .when(self.edited, |d| d.child(Label::new(t("studio.edited")).size(LabelSize::Small).color(Color::Muted)));
        let right = h_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .justify_end()
            .gap_1()
            .child(kit::drags_window(div().flex_1().h_full()))
            .child(
                Button::new("save-project", t("bundle.project"))
                    .style(ButtonStyle::Subtle)
                    .start_icon(Icon::from_path("icons/kadr/project.svg").size(IconSize::Small))
                    .tip(Tooltip::text(t("bundle.save_project_tip")))
                    .on_click(cx.listener(|this, _, _, cx| this.save_project(cx))),
            )
            .child(self.render_export(cx))
            .children(kit::window_controls(true, cx));
        h_flex()
            .w_full()
            .flex_none()
            .h(px(42.))
            .gap_2()
            // An even inset keeps corner buttons concentric with the window.
            .px_1()
            .bg(colors.title_bar_background)
            .border_b_1()
            .border_color(colors.border_variant)
            .child(left)
            .child(title)
            .child(right)
    }

    /// Export: the button opens the output settings, and
    /// the export starts from there (or with Ctrl+S right away).
    fn render_export(&self, cx: &mut Context<Self>) -> AnyElement {
        let gif = self.project.format == Format::Gif;
        let button = Button::new("export", if gif { t("studio.export_gif") } else { t("studio.export") })
            .style(ButtonStyle::Accent)
            .size(ButtonSize::Large)
            .on_click(cx.listener(|this, _, _, cx| {
                this.export_open = !this.export_open;
                this.aspect_open = false;
                cx.notify();
            }));
        let popover = self.export_open.then(|| {
            let files = vec!["MP4".to_string(), "GIF".to_string()];
            let resolutions = RESOLUTIONS.iter().map(|r| r.0.to_string()).collect();
            let rates = FRAME_RATES.iter().map(|r| t(r.0).to_string()).collect();
            let qualities = QUALITIES.iter().map(|q| t(q.0).to_string()).collect();
            let row = |text: &'static str, control: AnyElement| {
                h_flex().w_full().gap_2().child(div().w(px(ROW_LABEL)).flex_none().child(Label::new(text).size(LabelSize::Small).color(Color::Muted))).child(div().flex_1().min_w_0().child(control))
            };
            let out = studio::layout(&self.probe, &self.project, false);
            let format = self.choices("format", files, gif as usize, Rc::new(|this, i, cx| {
                this.project.format = if i == 1 { Format::Gif } else { Format::Mp4 };
                this.changed(cx);
            }), cx);
            let size = self.choices("res", resolutions, self.project.resolution, Rc::new(|this, i, cx| {
                this.project.resolution = i;
                this.changed(cx);
            }), cx);
            let fps = self.choices("fps", rates, self.project.frame_rate, Rc::new(|this, i, cx| {
                this.project.frame_rate = i;
                this.changed(cx);
            }), cx);
            let quality = (!gif).then(|| {
                self.choices("quality", qualities, self.project.quality, Rc::new(|this, i, cx| {
                    this.project.quality = i;
                    this.changed(cx);
                }), cx)
            });
            gpui::deferred(
                v_flex()
                    .id("export-popover")
                    .occlude()
                    .absolute()
                    .top(px(40.))
                    .right_0()
                    .w(px(420.))
                    .p_3()
                    .gap_3()
                    .elevation_3(cx)
                    .child(row(t("studio.row_format"), format))
                    .child(row(t("studio.row_size"), size))
                    .child(row(t("studio.row_fps"), fps))
                    .children(quality.map(|q| row(t("studio.row_quality"), q)))
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(Label::new(format!("{}×{} · {}", out.ow, out.oh, clock(self.project.kept_duration()))).size(LabelSize::XSmall).color(Color::Muted))
                            .child(Label::new(size_label(&self.project, &self.probe)).size(LabelSize::Small)),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                Button::new("preview", t("studio.preview"))
                                    .style(ButtonStyle::Filled)
                                    .size(ButtonSize::Large)
                                    .tip(Tooltip::text(t("studio.build_720p_preview_with_all")))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.export_open = false;
                                        studio::export(this.video.clone(), this.project.clone(), true, cx);
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("export-now", if gif { t("studio.export_gif") } else { t("studio.export") })
                                    .style(ButtonStyle::Accent)
                                    .size(ButtonSize::Large)
                                    .key_binding(kit::keys("secondary-s"))
                                    .on_click(cx.listener(|this, _, window, cx| this.export(window, cx))),
                            ),
                    )
                    .child(crate::ui::track_bounds(&self.menu_bounds)),
            )
        });
        div()
            .relative()
            .on_mouse_down_out(cx.listener(|this, e: &MouseDownEvent, _, cx| {
                if this.export_open && !this.menu_bounds.get().contains(&e.position) {
                    this.export_open = false;
                    cx.notify();
                }
            }))
            .child(button)
            .children(popover)
            .into_any_element()
    }

    /// Time on the left, play controls in the middle, the timeline's zoom on
    /// the right.
    fn render_transport(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let time = h_flex()
            .gap_1()
            .font_family(kit::MONO_FONT)
            .text_sm()
            .child(div().text_color(colors.text).child(timecode(self.playhead)))
            .child(div().text_color(colors.text_muted).child(format!("/ {}", timecode(self.probe.duration))));
        let (start, end) = self.project.trim;
        h_flex()
            .w_full()
            .flex_none()
            .px_4()
            .py_1p5()
            .child(div().flex_1().child(time))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        ButtonLike::new("to-start")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Large)
                            .child(div().px_1().child(Icon::from_path("icons/kadr/skip_back.svg").size(IconSize::Medium)))
                            .tip(Tooltip::text(t("studio.to_start")))
                            .on_click(cx.listener(move |this, _, _, cx| this.seek(start, cx))),
                    )
                    .child(
                        IconButton::new("play", if self.playing { IconName::DebugPause } else { IconName::PlayFilled })
                            .shape(IconButtonShape::Square)
                            .size(ButtonSize::Large)
                            .icon_size(IconSize::Large)
                            .tip(|_, cx| cx.new(|_| Tooltip::new(t("studio.play_pause")).key_binding(kit::keys("space"))).into())
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_play(window, cx))),
                    )
                    .child(
                        ButtonLike::new("to-end")
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Large)
                            .child(div().px_1().child(Icon::from_path("icons/kadr/skip_forward.svg").size(IconSize::Medium)))
                            .tip(Tooltip::text(t("studio.to_end")))
                            .on_click(cx.listener(move |this, _, _, cx| this.seek(end, cx))),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .justify_end()
                    .gap_1()
                    .child(
                        IconButton::new("timeline-out", IconName::Dash)
                            .shape(IconButtonShape::Square)
                            .icon_size(IconSize::XSmall)
                            .icon_color(Color::Muted)
                            .tip(Tooltip::text(t("studio.timeline_zoom")))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.timeline_zoom = (this.timeline_zoom / 1.5).max(1.);
                                cx.notify();
                            })),
                    )
                    .child(div().flex_none().w(px(120.)).child(self.slider(Knob::TimelineZoom, ((self.timeline_zoom - 1.) / (MAX_TIMELINE_ZOOM - 1.)) as f64, 0, cx)))
                    .child(
                        IconButton::new("timeline-in", IconName::Plus)
                            .shape(IconButtonShape::Square)
                            .icon_size(IconSize::XSmall)
                            .icon_color(Color::Muted)
                            .tip(Tooltip::text(t("studio.timeline_zoom")))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.timeline_zoom = (this.timeline_zoom * 1.5).min(MAX_TIMELINE_ZOOM);
                                cx.notify();
                            })),
                    ),
            )
    }

    /// A slider's track and knob; `ticks` marks under it.
    fn slider(&self, knob: Knob, value: f64, ticks: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let (track, filled, knob_color, knob_edge) = (colors.element_active, colors.text_accent, gpui::white(), colors.border);
        let mut tick = colors.text;
        tick.alpha = 0.3;
        let sliders = self.sliders.clone();
        let v = value.clamp(0., 1.) as f32;
        let bar = canvas(
            move |bounds, _, _| {
                sliders.borrow_mut().insert(knob, (f32::from(bounds.origin.x), f32::from(bounds.size.width)));
                bounds
            },
            move |_, bounds, window, _| {
                let mid = bounds.origin.y + bounds.size.height / 2. - px(if ticks > 0 { 3. } else { 0. });
                let (x0, w) = (bounds.origin.x, bounds.size.width);
                for i in 0..=ticks.max(1) {
                    if ticks == 0 {
                        break;
                    }
                    let x = x0 + w * (i as f32 / ticks as f32);
                    window.paint_quad(crate::ui::quad(Bounds::new(point(x - px(1.), mid + px(9.)), size(px(2.), px(2.))), px(1.), tick, px(0.), tick, BorderStyle::Solid));
                }
                window.paint_quad(crate::ui::quad(Bounds::new(point(x0, mid - px(2.)), size(w, px(4.))), px(2.), track, px(0.), track, BorderStyle::Solid));
                window.paint_quad(crate::ui::quad(Bounds::new(point(x0, mid - px(2.)), size(w * v, px(4.))), px(2.), filled, px(0.), filled, BorderStyle::Solid));
                let cx0 = x0 + w * v;
                window.paint_quad(crate::ui::quad(Bounds::new(point(cx0 - px(9.), mid - px(8.)), size(px(18.), px(18.))), px(9.), gpui::hsla(0., 0., 0., 0.18), px(0.), knob_edge, BorderStyle::Solid));
                window.paint_quad(crate::ui::quad(Bounds::new(point(cx0 - px(8.), mid - px(8.)), size(px(16.), px(16.))), px(8.), knob_color, px(0.5), knob_edge, BorderStyle::Solid));
            },
        )
        .size_full();
        div().id(SharedString::from(format!("slider-{}", knob as u8))).w_full().h(px(if ticks > 0 { 28. } else { 22. })).px_2().cursor(CursorStyle::PointingHand).child(bar).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                this.drag = Some(Drag::Slider(knob));
                this.slide(knob, f32::from(e.position.x), cx);
            }),
        )
    }

    /// A slider under its name, the value at the right.
    fn labeled_slider(&self, name: &'static str, knob: Knob, value: f64, shown: String, ticks: usize, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .gap_1()
            .child(h_flex().w_full().justify_between().child(Label::new(name).size(LabelSize::Small)).child(Label::new(shown).size(LabelSize::Small).color(Color::Muted)))
            .child(div().mx(px(-8.)).child(self.slider(knob, value, ticks, cx)))
            .into_any_element()
    }

    /// Options side by side, the chosen one filled with the accent.
    fn choices(&self, id: &'static str, labels: Vec<String>, selected: usize, set: Set<usize>, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let (accent, idle, hover) = (colors.text_accent, colors.element_background, colors.element_hover);
        h_flex()
            .w_full()
            .gap(px(4.))
            .children(labels.into_iter().enumerate().map(|(i, text)| {
                let on = i == selected;
                let set = set.clone();
                div()
                    .id(SharedString::from(format!("{id}-{i}")))
                    .flex_grow(1.)
                    .flex_basis(px(0.))
                    .min_w_0()
                    .px_1()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .cursor_pointer()
                    .bg(if on { accent } else { idle })
                    .when(!on, |d| d.hover(move |d| d.bg(hover)))
                    .child(div().text_size(px(13.)).truncate().text_color(if on { gpui::white() } else { colors.text }).child(text))
                    .on_click(cx.listener(move |this, _, _, cx| set(this, i, cx)))
            }))
            .into_any_element()
    }

    /// A checkbox and its label, like the crop mode's "Snap to edges".
    fn check(&self, id: &'static str, text: &'static str, on: bool, enabled: bool, set: Set<bool>, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let (accent, edge) = (colors.text_accent, colors.border);
        let boxed = div()
            .size(px(16.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(if on { accent } else { edge })
            .when(on, |d| d.bg(accent).child(gpui::svg().path("icons/check.svg").size(px(12.)).text_color(gpui::white())));
        h_flex()
            .id(id)
            .gap_2()
            .h(px(26.))
            .when(enabled, |d| d.cursor_pointer())
            .when(!enabled, |d| d.opacity(0.45))
            .child(boxed)
            .child(Label::new(text).size(LabelSize::Small))
            .on_click(cx.listener(move |this, _, _, cx| {
                if enabled {
                    set(this, !on, cx);
                }
            }))
            .into_any_element()
    }

    /// A switch with its name, and a muted line under it.
    fn switch_row(&self, id: &'static str, text: &'static str, note: Option<&'static str>, on: bool, enabled: bool, set: Set<bool>, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(Label::new(text).size(LabelSize::Small))
                    .children(note.map(|n| Label::new(n).size(LabelSize::XSmall).color(Color::Muted))),
            )
            .child(Switch::new(id, ToggleState::from(on)).disabled(!enabled).on_click(cx.listener(move |this, state: &ToggleState, _, cx| set(this, *state == ToggleState::Selected, cx))))
            .into_any_element()
    }

    /// The icons down the left: one section shown at a time.
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        let item = |section: Section, cx: &mut Context<Self>| {
            let on = self.section == section;
            div()
                .id(SharedString::from(format!("rail-{}", section as u8)))
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(9.))
                .cursor_pointer()
                .when(on, |d| d.bg(colors.text_accent))
                .when(!on, |d| d.hover(|d| d.bg(colors.ghost_element_hover)))
                .child(Icon::from_path(section.icon()).size(IconSize::Medium).color(if on { Color::Custom(gpui::white()) } else { Color::Muted }))
                .tip(Tooltip::text(section.label()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.show_section(section);
                    cx.notify();
                }))
        };
        let top: Vec<_> = Section::RAIL.into_iter().map(|s| item(s, cx)).collect();
        v_flex()
            .flex_none()
            .w(px(RAIL))
            .h_full()
            .py_2()
            .items_center()
            .gap_1()
            .bg(colors.title_bar_background)
            .children(top)
            .child(div().flex_1())
            .child(item(Section::Cut, cx))
    }

    /// The chosen section's settings, scrolling.
    fn render_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.section {
            Section::Zoom => self.zoom_section(cx),
            Section::Cursor => self.cursor_section(cx),
            Section::Keys => self.keys_section(cx),
            Section::Audio => self.audio_section(cx),
            Section::Camera => self.camera_section(cx),
            Section::Background => self.background_section(cx),
            Section::Cut => self.cut_section(cx),
        };
        div()
            .relative()
            .flex_none()
            .w(px(PANEL))
            .h_full()
            .bg(cx.theme().colors().panel_background)
            .child(v_flex().id("section").size_full().overflow_y_scroll().track_scroll(&self.side_scroll).p_3().gap_5().children(content))
            .child(crate::ui::scrollbar(&self.side_scroll, &self.side_grab, cx))
    }

    /// A group in a section: its heading, then its controls.
    fn group(heading: &'static str, items: Vec<AnyElement>) -> AnyElement {
        v_flex().w_full().gap_2().child(Label::new(heading).size(LabelSize::Small).color(Color::Muted)).children(items).into_any_element()
    }

    fn note(text: impl Into<SharedString>) -> AnyElement {
        Label::new(text).size(LabelSize::XSmall).color(Color::Muted).into_any_element()
    }

    fn zoom_section(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let level = self.zoom_level();
        let selected = self.selected.and_then(|i| self.project.segments.get(i).map(|s| (i, s.clone())));
        let span = ZOOM_RANGE.1 - ZOOM_RANGE.0;
        let mut items = vec![
            self.labeled_slider(t("studio.zoom_level"), Knob::Zoom, (level - ZOOM_RANGE.0) / span, format!("{}%", (level * 100.).round()), 6, cx),
            wide(Button::new("zoom-all", t("studio.apply_zoom_to_all"))
                .style(ButtonStyle::Filled)
                .disabled(self.project.segments.is_empty())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let level = this.zoom_level();
                    for seg in &mut this.project.segments {
                        seg.zoom = level;
                    }
                    this.new_zoom = level;
                    this.changed(cx);
                }))
            ),
        ];
        match &selected {
            Some((index, seg)) => {
                let index = *index;
                let manual = seg.focus.is_some() || self.placing;
                let mode = self.choices(
                    "zoom-mode",
                    vec![t("studio.follow_cursor_short").into(), t("studio.manual").into()],
                    manual as usize,
                    Rc::new(move |this, i, cx| {
                        if i == 0 {
                            this.placing = false;
                            this.project.segments[index].focus = None;
                            this.changed(cx);
                        } else {
                            this.placing = true;
                            cx.notify();
                        }
                    }),
                    cx,
                );
                let mut mode_items = vec![mode];
                if self.placing {
                    mode_items.push(Label::new(t("studio.click_video_where_zoom")).size(LabelSize::XSmall).color(Color::Accent).into_any_element());
                } else if manual {
                    mode_items.push(Self::note(t("studio.manual_note")));
                }
                items.push(Self::group(t("studio.zoom_mode"), mode_items));
                items.push(
                    Button::new("delete-zoom", t("studio.delete_zoom"))
                        .style(ButtonStyle::Subtle)
                        .start_icon(Icon::new(IconName::Trash).size(IconSize::Small).color(Color::Error))
                        .key_binding(kit::keys("delete"))
                        .on_click(cx.listener(|this, _, _, cx| this.delete_selected(cx)))
                        .into_any_element(),
                );
            }
            None => items.push(Self::note(if self.project.segments.is_empty() {
                t("studio.no_zooms_click_lower_track").into()
            } else {
                SharedString::from(tr!("studio.zooms_pick_one_on_timeline", self.project.segments.len()))
            })),
        }
        let has_clicks = !self.track.clicks.is_empty();
        items.push(Self::group(
            t("studio.automatic"),
            vec![
                wide(Button::new("auto", t("studio.zooms_on_clicks"))
                    .style(ButtonStyle::Filled)
                    .start_icon(Icon::new(IconName::Sparkle).size(IconSize::Small))
                    .disabled(!has_clicks)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let auto = Project::auto(&this.track, &this.probe);
                        this.project.segments = auto.segments;
                        this.select_zoom(None);
                        this.changed(cx);
                    }))
                ),
            ]
            .into_iter()
            .chain((!has_clicks).then(|| Self::note(t("studio.no_clicks_in_recording"))))
            .collect(),
        ));
        let blur = MOTION_BLUR.iter().map(|b| t(b.0).to_string()).collect();
        items.push(Self::group(
            t("studio.motion_blur"),
            vec![
                self.choices("blur", blur, self.project.motion_blur, Rc::new(|this, i, cx| {
                    this.project.motion_blur = i;
                    this.changed(cx);
                }), cx),
                Self::note(t("studio.on_zooms_camera_moves_fast")),
            ],
        ));
        items
    }

    fn cursor_section(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let mut items = Vec::new();
        if self.track.cursor_hidden {
            items.push(self.switch_row("cursor", t("studio.show_cursor"), None, self.project.cursor, true, Rc::new(|this, on, cx| {
                this.project.cursor = on;
                this.changed(cx);
            }), cx));
            if self.project.cursor {
                let size = self.project.cursor_size;
                items.push(self.labeled_slider(t("studio.row_size"), Knob::CursorSize, (size - 0.5) / (MAX_CURSOR - 0.5), crate::i18n::decimal(format!("{size:.1}×")), 10, cx));
                // The styles as tiles, each showing itself.
                let tiles = [0usize, 2, 1].into_iter().zip(self.style_thumbs.iter()).map(|(style, thumb)| {
                    let on = self.project.cursor_style == style;
                    div()
                        .id(SharedString::from(format!("cursor-style-{style}")))
                        .size(px(52.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(12.))
                        .bg(colors.element_background)
                        .border_2()
                        .border_color(if on { colors.text_accent } else { gpui::transparent_black() })
                        .cursor_pointer()
                        .hover(|d| d.bg(colors.element_hover))
                        .children(thumb.clone().map(|i| img(i).max_w(px(30.)).max_h(px(30.)).object_fit(ObjectFit::Contain)))
                        .tip(Tooltip::text(t(CURSOR_STYLES[style])))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.project.cursor_style = style;
                            this.changed(cx);
                        }))
                        .into_any_element()
                });
                items.push(Self::group(t("studio.style"), vec![h_flex().gap_2().children(tiles).into_any_element()]));
                let smooth = self.project.cursor_smooth >= 0.5;
                items.push(Self::group(
                    t("studio.motion"),
                    vec![self.choices("cursor-motion-kind", vec![t("studio.natural").into(), t("studio.smooth").into()], smooth as usize, Rc::new(|this, i, cx| {
                        this.project.cursor_smooth = if i == 1 { 0.7 } else { 0.25 };
                        this.changed(cx);
                    }), cx)],
                ));
            }
        } else {
            items.push(Self::note(t("studio.change_cursors_size_smoothness_record")));
        }
        let has_clicks = !self.track.clicks.is_empty();
        let mut clicks = Vec::new();
        if self.track.cursor_hidden && self.project.cursor {
            clicks.push(self.check("press-effect", t("studio.press_effect"), self.project.cursor_motion, true, Rc::new(|this, on, cx| {
                this.project.cursor_motion = on;
                this.changed(cx);
            }), cx));
        }
        clicks.push(self.check("ripple", t("studio.ripple_animation"), self.project.clicks && has_clicks, has_clicks, Rc::new(|this, on, cx| {
            this.project.clicks = on;
            this.changed(cx);
        }), cx));
        if !has_clicks {
            clicks.push(Self::note(t("studio.no_clicks_in_recording")));
        }
        items.push(Self::group(t("studio.clicks"), clicks));
        items
    }

    fn keys_section(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let has_keys = !self.track.keystrokes.is_empty();
        let on = self.project.keys && has_keys;
        let mut items = vec![self.switch_row("keys", t("studio.show_shortcuts"), Some(t("studio.badge_with_shortcuts_pressed")), on, has_keys, Rc::new(|this, on, cx| {
            this.project.keys = on;
            this.changed(cx);
        }), cx)];
        if !has_keys {
            items.push(Self::note(t("studio.record_with_shortcuts_on")));
        }
        if on {
            let sides = vec![t("studio.bottom").into(), t("studio.top").into()];
            items.push(Self::group(t("studio.row_position"), vec![self.choices("keys-side", sides, self.project.keys_top as usize, Rc::new(|this, i, cx| {
                this.project.keys_top = i == 1;
                this.changed(cx);
            }), cx)]));
            let sizes = KEY_SIZES.iter().map(|k| k.0.to_string()).collect();
            items.push(Self::group(t("studio.row_size"), vec![self.choices("keys-size", sizes, self.project.keys_size, Rc::new(|this, i, cx| {
                this.project.keys_size = i;
                this.changed(cx);
            }), cx)]));
        }
        items
    }

    fn audio_section(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.probe.audio {
            return vec![Self::note(t("studio.no_audio_in_recording"))];
        }
        let gif = self.project.format == Format::Gif;
        let mut items = vec![self.switch_row("audio", t("studio.audio"), Some(t("studio.microphone_system_as_recorded")), self.project.audio && !gif, !gif, Rc::new(|this, on, cx| {
            this.project.audio = on;
            this.sound_changed();
            this.changed(cx);
        }), cx)];
        if gif {
            items.push(Self::note(t("studio.gifs_have_no_audio")));
        } else if self.project.audio {
            let volume = self.project.volume;
            items.push(self.labeled_slider(t("studio.volume"), Knob::Volume, volume / studio::MAX_VOLUME, format!("{}%", (volume * 100.).round()), 4, cx));
            let channels = vec![t("studio.stereo").into(), t("studio.mono").into()];
            items.push(Self::group(t("studio.channels"), vec![self.choices("audio-channels", channels, self.project.mono as usize, Rc::new(|this, i, cx| {
                this.project.mono = i == 1;
                this.sound_changed();
                this.changed(cx);
            }), cx)]));
        }
        items
    }

    fn camera_section(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.cam_file.is_none() {
            return vec![Self::note(t("studio.no_camera_in_recording"))];
        }
        let colors = cx.theme().colors().clone();
        let (accent, line) = (colors.text_accent, colors.border);
        let mut items = vec![self.switch_row("cam", t("studio.show_camera"), Some(t("studio.camera_recorded_separately")), self.project.cam, true, Rc::new(|this, on, cx| {
            this.project.cam = on;
            this.changed(cx);
        }), cx)];
        if self.project.cam {
            let size = (self.project.cam_size - studio::MIN_CAM) / (studio::MAX_CAM - studio::MIN_CAM);
            items.push(self.labeled_slider(t("studio.row_size"), Knob::CamSize, size, format!("{}%", (self.project.cam_size * 100.).round()), 0, cx));
            items.push(self.labeled_slider(t("studio.rounding"), Knob::CamRound, self.project.cam_round, format!("{}%", (self.project.cam_round * 100.).round()), 0, cx));
            // A 3×3 grid without the middle: the spots the camera snaps to.
            let spot = |i: usize, cx: &mut Context<Self>| {
                let on = self.project.cam_anchor == i;
                div()
                    .id(SharedString::from(format!("cam-anchor-{i}")))
                    .size(px(18.))
                    .rounded(px(5.))
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
            let mut grid = v_flex().gap(px(4.));
            for cells in [[Some(0), Some(1), Some(2)], [Some(7), None, Some(3)], [Some(6), Some(5), Some(4)]] {
                let cells: Vec<AnyElement> = cells.iter().map(|c| c.map(|i| spot(i, cx)).unwrap_or_else(|| div().size(px(18.)).into_any_element())).collect();
                grid = grid.child(h_flex().gap(px(4.)).children(cells));
            }
            items.push(Self::group(t("studio.row_position"), vec![grid.into_any_element()]));
        }
        items
    }

    fn background_section(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let (accent, edge) = (colors.text_accent, colors.border_variant);
        let clear = gpui::transparent_black();
        // A square tile with an accent ring when chosen; the ring, a gap and
        // the swatch keep their corners concentric.
        let tile = |id: SharedString, on: bool| div().id(id).flex_none().p(px(2.)).rounded(px(TILE_RADIUS)).border_2().border_color(if on { accent } else { clear }).cursor_pointer();
        let swatch = || div().size(px(TILE)).rounded(px(TILE_RADIUS - 4.)).overflow_hidden().border_1().border_color(edge);
        let grid = || h_flex().w_full().flex_wrap().gap(px(TILE_GAP));

        let all = palette::backdrops();
        let gradients: Vec<usize> = (0..all.len()).filter(|&i| all[i].0 != all[i].1).collect();
        let plains: Vec<usize> = (0..all.len()).filter(|&i| all[i].0 == all[i].1).collect();
        let shown = if self.more_gradients { gradients.len() } else { gradients.len().min(GRADIENT_ROWS) };
        let chosen = |i: usize| self.project.wallpaper.is_none() && self.project.backdrop == i;
        let gradient_tiles: Vec<AnyElement> = gradients[..shown]
            .iter()
            .map(|&i| {
                let (c0, c1) = all[i];
                tile(SharedString::from(format!("gradient-{i}")), chosen(i))
                    .child(swatch().bg(linear_gradient(135., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project.backdrop = i;
                        this.project.wallpaper = None;
                        this.changed(cx);
                    }))
                    .into_any_element()
            })
            .collect();
        let toggle_more = (gradients.len() > GRADIENT_ROWS).then(|| {
            Button::new("more-gradients", t(if self.more_gradients { "editor.show_less" } else { "editor.show_more" }))
                .style(ButtonStyle::Subtle)
                .label_size(LabelSize::Small)
                .color(Color::Muted)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.more_gradients = !this.more_gradients;
                    cx.notify();
                }))
        });

        let mut walls: Vec<AnyElement> = self
            .walls
            .iter()
            .enumerate()
            .map(|(i, (path, thumb))| {
                let path = path.clone();
                tile(SharedString::from(format!("wall-{i}")), self.project.wallpaper.as_ref() == Some(&path))
                    .child(swatch().child(img(thumb.clone()).size_full().object_fit(ObjectFit::Cover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project.wallpaper = Some(path.clone());
                        this.changed(cx);
                    }))
                    .into_any_element()
            })
            .collect();
        let own = self.project.wallpaper.as_ref().is_some_and(|w| !self.walls.iter().any(|(p, _)| p == w));
        walls.push(
            tile("own-wallpaper".into(), own)
                .child(div().size(px(TILE)).rounded(px(TILE_RADIUS - 4.)).border_1().border_dashed().border_color(colors.border).flex().items_center().justify_center().child(Icon::new(IconName::Plus).size(IconSize::Small).color(Color::Muted)))
                .tip(Tooltip::text(t("common.your_own_image")))
                .on_click(cx.listener(|this, _, _, cx| this.pick_wallpaper(cx)))
                .into_any_element(),
        );

        let dots: Vec<AnyElement> = plains
            .iter()
            .map(|&i| {
                div()
                    .id(SharedString::from(format!("plain-{i}")))
                    .flex_none()
                    .p(px(2.))
                    .rounded_full()
                    .border_2()
                    .border_color(if chosen(i) { accent } else { clear })
                    .cursor_pointer()
                    .child(div().size(px(DOT)).rounded_full().border_1().border_color(edge).bg(all[i].0))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project.backdrop = i;
                        this.project.wallpaper = None;
                        this.changed(cx);
                    }))
                    .into_any_element()
            })
            .collect();

        vec![
            crate::presets::render(&self.presets, &self.project.look(), cx).into_any_element(),
            v_flex()
                .w_full()
                .gap_2()
                .child(h_flex().w_full().justify_between().child(Label::new(t("editor.gradients")).size(LabelSize::Small).color(Color::Muted)).children(toggle_more))
                .child(grid().children(gradient_tiles))
                .into_any_element(),
            Self::group(t("editor.wallpapers"), vec![grid().children(walls).into_any_element()]),
            Self::group(t("editor.plain_color"), vec![h_flex().w_full().flex_wrap().gap(px(2.)).children(dots).into_any_element()]),
            Self::group(t("editor.aspect_ratio"), vec![self.aspect_dropdown(cx)]),
            self.labeled_slider(t("studio.row_padding"), Knob::Padding, self.project.padding / MAX_PADDING, format!("{}%", (self.project.padding * 100.).round()), 0, cx),
            self.labeled_slider(t("studio.row_corners"), Knob::Radius, self.project.radius / MAX_RADIUS, format!("{}", self.project.radius.round()), 0, cx),
            self.labeled_slider(t("studio.row_shadow"), Knob::Shadow, self.project.shadow, format!("{}%", (self.project.shadow * 100.).round()), 0, cx),
        ]
    }

    /// The output's shape as a dropdown, wide shapes then tall ones.
    fn aspect_dropdown(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let (field, hover) = (colors.element_background, colors.element_hover);
        let label = |i: usize| t(ASPECTS[i].0);
        let field_el = h_flex()
            .id("aspect")
            .w_full()
            .h(px(30.))
            .px_2p5()
            .justify_between()
            .rounded(px(8.))
            .bg(field)
            .cursor_pointer()
            .hover(move |d| d.bg(hover))
            .child(Label::new(label(self.project.aspect)).size(LabelSize::Small))
            .child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted))
            .on_click(cx.listener(|this, _, _, cx| {
                this.aspect_open = !this.aspect_open;
                this.export_open = false;
                cx.notify();
            }));
        let menu = self.aspect_open.then(|| {
            let mut rows: Vec<AnyElement> = Vec::new();
            for (g, group) in ASPECT_ORDER.iter().enumerate() {
                if g > 0 {
                    rows.push(div().mx_2().my_1().h(px(1.)).bg(colors.border_variant).into_any_element());
                }
                for &i in group.iter() {
                    let on = self.project.aspect == i;
                    rows.push(
                        h_flex()
                            .id(SharedString::from(format!("aspect-{i}")))
                            .w_full()
                            .gap_1p5()
                            .px_2()
                            .h(px(28.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(move |d| d.bg(hover))
                            .child(div().w(px(14.)).children(on.then(|| Icon::new(IconName::Check).size(IconSize::XSmall))))
                            .child(Label::new(label(i)).size(LabelSize::Small))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.aspect_open = false;
                                this.project.aspect = i;
                                this.changed(cx);
                            }))
                            .into_any_element(),
                    );
                }
            }
            gpui::deferred(v_flex().id("aspect-menu").occlude().absolute().top(px(34.)).left_0().right_0().p_1().elevation_3(cx).children(rows).child(crate::ui::track_bounds(&self.menu_bounds)))
        });
        div()
            .relative()
            .w_full()
            .on_mouse_down_out(cx.listener(|this, e: &MouseDownEvent, _, cx| {
                if this.aspect_open && !this.menu_bounds.get().contains(&e.position) {
                    this.aspect_open = false;
                    cx.notify();
                }
            }))
            .child(field_el)
            .children(menu)
            .into_any_element()
    }

    fn cut_section(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut items = vec![Self::note(t("studio.cut_how")), Self::note(t("studio.filmstrips_yellow_edges_trim_start"))];
        if let Some((index, (a, b))) = self.cut.and_then(|i| self.project.cuts.get(i).map(|c| (i, *c))) {
            items.push(Self::group(
                t("studio.selected_cut"),
                vec![
                    Label::new(tr!("studio.removing", seconds((b - a).abs()), clock(a.min(b)), clock(a.max(b)))).size(LabelSize::Small).into_any_element(),
                    wide(Button::new("delete-cut", t("studio.restore_piece"))
                        .style(ButtonStyle::Filled)
                        .start_icon(Icon::new(IconName::Undo).size(IconSize::Small))
                        .key_binding(kit::keys("delete"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.project.cuts.remove(index);
                            this.cut = None;
                            this.changed(cx);
                        }))
                    ),
                ],
            ));
        }
        if self.project.trim != (0., self.probe.duration) || !self.project.cuts.is_empty() {
            items.push(
                wide(Button::new("uncut", t("studio.restore_whole_video"))
                    .style(ButtonStyle::Subtle)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.project.trim = (0., this.probe.duration);
                        this.project.cuts.clear();
                        this.select_cut(None);
                        this.changed(cx);
                    }))
                ),
            );
        }
        items
    }
}

/// Which part of a block on the timeline (from `a` to `b`, in pixels) the
/// pointer at `x` is on.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    Start,
    End,
    Body,
}

fn block_hit(x: f32, a: f32, b: f32) -> Option<Part> {
    // A narrow block keeps a middle to grab: its edges share what's there.
    let edge = EDGE.min(((b - a) / 3.).max(2.));
    if x < a - edge || x > b + edge {
        None
    } else if (x - a).abs() <= edge && (x - a).abs() <= (x - b).abs() {
        Some(Part::Start)
    } else if (x - b).abs() <= edge {
        Some(Part::End)
    } else {
        Some(Part::Body)
    }
}

/// A button across a section's width.
fn wide(button: Button) -> AnyElement {
    h_flex().w_full().child(button.full_width()).into_any_element()
}

/// A horizontal scrollbar under a sideways-scrolling pane, like
/// `ui::scrollbar` turned on its side.
fn hscrollbar(handle: &gpui::ScrollHandle, grab: &Rc<Cell<Option<f32>>>, cx: &App) -> impl IntoElement {
    let (handle, grab) = (handle.clone(), grab.clone());
    let mut color = cx.theme().colors.text;
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let view = handle.bounds();
            let max = f32::from(handle.max_offset().x);
            let w = f32::from(view.size.width);
            if max < 1. || w < 40. {
                return;
            }
            let thumb_w = (w * w / (w + max)).max(32.);
            let travel = w - thumb_w;
            let left = f32::from(view.left()) + travel * (-f32::from(handle.offset().x) / max).clamp(0., 1.);
            let track_y = f32::from(view.bottom()) + 2.;
            color.alpha = if grab.get().is_some() { 0.5 } else { 0.28 };
            let thumb = Bounds::new(point(px(left + 3.), px(track_y + 2.)), size(px(thumb_w - 6.), px(5.)));
            window.paint_quad(fill(thumb, color).corner_radii(px(2.5)));

            let scroll_to = {
                let handle = handle.clone();
                move |thumb_left: f32| {
                    let at = ((thumb_left - f32::from(view.left())) / travel.max(1.)).clamp(0., 1.) * max;
                    handle.set_offset(point(px(-at), handle.offset().y));
                }
            };
            let on_track = move |p: gpui::Point<Pixels>| f32::from(p.y) >= track_y && f32::from(p.y) <= track_y + 10. && p.x >= view.left() && p.x <= view.right();
            let g = grab.clone();
            let jump = scroll_to.clone();
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Capture || e.button != MouseButton::Left || !on_track(e.position) {
                    return;
                }
                let x = f32::from(e.position.x);
                if x < left || x > left + thumb_w {
                    jump(x - thumb_w / 2.);
                    g.set(Some(thumb_w / 2.));
                } else {
                    g.set(Some(x - left));
                }
                cx.stop_propagation();
                window.refresh();
            });
            let g = grab.clone();
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, _| {
                if phase != gpui::DispatchPhase::Capture {
                    return;
                }
                if let Some(at) = g.get() {
                    if !e.dragging() {
                        g.set(None);
                    } else {
                        scroll_to(f32::from(e.position.x) - at);
                    }
                    window.refresh();
                }
            });
            let g = grab.clone();
            window.on_mouse_event(move |_: &gpui::MouseUpEvent, _, window, _| {
                if g.take().is_some() {
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .left_4()
    .right_4()
    .bottom_0()
    .h(px(12.))
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
        let colors = cx.theme().colors().clone();
        let expand = ButtonLike::new("expand")
            .style(ButtonStyle::Subtle)
            .child(div().p_1().child(Icon::from_path("icons/expand.svg").size(IconSize::Small).color(Color::Muted)))
            .tip(Tooltip::text(t(if self.expanded { "studio.show_settings" } else { "studio.preview_only" })))
            .on_click(cx.listener(|this, _, _, cx| {
                this.expanded = !this.expanded;
                cx.notify();
            }));
        let expand = div().absolute().top_2().right_2().child(expand);
        let stage = div()
            .relative()
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .child(measure)
            .child(self.render_preview(cx))
            .child(expand);
        kit::root(cx)
            .flex()
            .flex_col()
            .bg(colors.panel_background)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.drag_done(cx)))
            .child(self.render_titlebar(cx))
            .child(
                h_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .when(!self.expanded, |row| row.child(self.render_rail(cx)).child(self.render_panel(cx)))
                    .child(v_flex().flex_1().min_w_0().h_full().bg(colors.editor_background).child(stage).child(self.render_transport(cx))),
            )
            .child(self.render_timeline(cx))
    }
}

/// Ruler text: "00:05", halves as "00:05.5".
fn ruler_label(t: f64, step: f64) -> String {
    if step < 1. {
        format!("{:02}:{:04.1}", (t / 60.) as u64, t % 60.)
    } else {
        let t = t.round() as u64;
        format!("{:02}:{:02}", t / 60, t % 60)
    }
}

/// The transport's time: "00:20".
fn timecode(t: f64) -> String {
    let t = t.max(0.).floor() as u64;
    format!("{:02}:{:02}", t / 60, t % 60)
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

impl crate::presets::HasPresets for StudioEditor {
    fn presets(&mut self) -> &mut crate::presets::Presets {
        &mut self.presets
    }

    fn current_look(&self) -> serde_json::Value {
        self.project.look()
    }

    fn apply_look(&mut self, look: &serde_json::Value, cx: &mut Context<Self>) {
        self.project = self.project.with_look(look, &self.probe);
        self.changed(cx);
    }
}


/// The recording's sound from a point on, through ffplay; stops when dropped.
struct Sound(std::process::Child);

impl Sound {
    /// From `start` through the audio filters `af` (on a clock starting at 0
    /// there), so it runs through cuts along with the picture. Where ffmpeg
    /// can't play sound itself, ffplay does.
    fn play(video: &std::path::Path, start: f64, af: &str) -> Option<Sound> {
        let ss = format!("{start:.3}");
        let mut cmd = match crate::platform::sound_output() {
            Some(output) => {
                let mut cmd = std::process::Command::new("ffmpeg");
                cmd.args(["-nostdin", "-loglevel", "quiet", "-ss", &ss, "-i"])
                    .arg(video)
                    .args(["-vn", "-af", af])
                    .args(output);
                cmd
            }
            None => {
                let mut cmd = std::process::Command::new("ffplay");
                cmd.args(["-nodisp", "-autoexit", "-vn", "-loglevel", "quiet", "-ss", &ss, "-af", af, "-i"]).arg(video);
                cmd
            }
        };
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().ok().map(Sound)
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::{Part, block_hit};

    #[test]
    fn narrow_blocks_keep_a_middle() {
        // Wide: the edges within reach, the body between.
        assert_eq!(block_hit(100., 100., 300.), Some(Part::Start));
        assert_eq!(block_hit(295., 100., 300.), Some(Part::End));
        assert_eq!(block_hit(200., 100., 300.), Some(Part::Body));
        assert_eq!(block_hit(50., 100., 300.), None);
        // Six pixels wide (a second of a long recording): its middle still
        // moves it instead of grabbing an edge.
        assert_eq!(block_hit(103., 100., 106.), Some(Part::Body));
        assert_eq!(block_hit(100., 100., 106.), Some(Part::Start));
        assert_eq!(block_hit(106., 100., 106.), Some(Part::End));
    }
}
