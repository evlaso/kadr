//! Studio render of a recording, like Screen Studio: the camera eases in on
//! zoom segments and follows the cursor (or holds a point), the video sits on
//! a gradient backdrop with a soft shadow and rounded corners, clicks get
//! rings and shortcuts a pill. Segments are edited in the studio editor and
//! kept as a per-recording project; the heavy lifting is ffmpeg's libplacebo
//! filter on the GPU, with the camera path handed over as crop expressions.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use kadr_core::color::Rgba;
use kadr_core::dirs;
use serde_json::json;

use crate::track::{self, Track};
use crate::job::{self, Job};
use kadr_core::settings::Settings;
use kadr_core::i18n::t;

/// Seconds to ease the zoom in or out.
pub const RAMP: f64 = 0.8;
/// How long an automatic zoom lingers after the last click or key press.
const HOLD: f64 = 1.6;
/// Clicks closer than this stay in one zoomed-in stretch.
const GAP: f64 = 3.5;
/// Camera path sampling rate.
pub const RATE: f64 = 30.0;
pub const DEFAULT_ZOOM: f64 = 1.8;
pub const ZOOMS: [f64; 5] = [1.25, 1.5, 1.8, 2.2, 3.0];
/// Margin around the video, as a fraction of the output height.
pub const PADDINGS: [f64; 4] = [0.0, 0.04, 0.07, 0.11];

/// A stretch where the camera is zoomed in: fully between `start` and `end`,
/// easing in before and out after.
#[derive(Clone, Debug, PartialEq)]
pub struct Seg {
    pub start: f64,
    pub end: f64,
    pub zoom: f64,
    /// Fixed point to zoom on, in video pixels; None follows the cursor.
    pub focus: Option<(f64, f64)>,
}

/// Output shapes: the recording's own, then landscape, square and vertical.
/// Saved projects keep the index, so new shapes go at the end;
/// `ASPECT_ORDER` is how the menu lists them.
pub const ASPECTS: [(&str, Option<f64>); 9] = [
    ("studio.like_video", None),
    ("16:9", Some(16. / 9.)),
    ("1:1", Some(1.)),
    ("4:5", Some(0.8)),
    ("9:16", Some(9. / 16.)),
    ("5:4", Some(1.25)),
    ("4:3", Some(4. / 3.)),
    ("3:2", Some(1.5)),
    ("3:4", Some(0.75)),
];
/// The aspect menu's order: wide shapes, then tall ones.
pub const ASPECT_ORDER: [&[usize]; 2] = [&[0, 2, 5, 6, 7, 1], &[3, 8, 4]];
/// Export size by the output's short side.
pub const RESOLUTIONS: [(&str, u32); 4] = [("720p", 720), ("1080p", 1080), ("1440p", 1440), ("4K", 2160)];
pub const FRAME_RATES: [(&str, Option<u32>); 4] = [("studio.as_recorded", None), ("30", Some(30)), ("60", Some(60)), ("120", Some(120))];
/// Shortcut pill text height as a share of the output's short side.
/// The loudest the audio goes, as a multiple of the recording's.
pub const MAX_VOLUME: f64 = 2.0;
pub const KEY_SIZES: [(&str, f64); 3] = [("S", 0.015), ("M", 0.02), ("L", 0.026)];
/// Slider ranges: padding (share of the short side), corner radius (pixels
/// at 1080p) and shadow strength.
pub const MAX_PADDING: f64 = 0.15;
pub const MAX_RADIUS: f64 = 60.;
/// Export compression as a bitrate cap in bits per pixel per frame, so the
/// file size has a known ceiling before rendering: bpp × width × height × fps
/// × seconds. Screen recordings often come out well under it.
pub const QUALITIES: [(&str, f64); 4] = [("studio.maximum", 0.12), ("common.high", 0.06), ("common.medium", 0.03), ("studio.small", 0.012)];
pub const AUDIO_BITRATE: f64 = 160_000.;
/// The drawn cursor: the pointer's own shapes, a dot, or the shapes in
/// inverted colours (white with a dark rim). Indexes are saved.
pub const CURSOR_STYLES: [&str; 3] = ["common.arrow", "studio.dot", "studio.light_arrow"];
/// Motion blur, like a camera shutter open this many quarters of the
/// recording's frame time (0 is off): the picture is rendered at up to
/// `SUBFRAMES` times the frame rate, and while the camera or the cursor moves
/// fast the sub-frames that fall in the shutter are averaged.
pub const MOTION_BLUR: [(&str, usize); 3] = [("studio.off", 0), ("studio.light", 2), ("studio.strong", 4)];
const SUBFRAMES: usize = 4;
/// Motion blur renders at most this many frames a second.
const MAX_SUBFRAME_RATE: f64 = 240.;
/// Exports go no higher: "as recorded" for a 240 Hz recording means 120.
pub const MAX_EXPORT_FPS: f64 = 120.;
pub const MAX_CURSOR: f64 = 3.;
/// Drawn cursor height at size 1, in logical screen pixels.
const CURSOR_BASE: f64 = 22.;
/// Drawn cursor path sampling rate.
pub const CURSOR_RATE: f64 = 60.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mp4,
    Gif,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub segments: Vec<Seg>,
    /// Index into `kadr_raster::backdrops()`, unless a wallpaper is set.
    pub backdrop: usize,
    pub wallpaper: Option<PathBuf>,
    /// Share of the output's short side.
    pub padding: f64,
    /// Corner radius in pixels at 1080p.
    pub radius: f64,
    /// 0..1.
    pub shadow: f64,
    pub clicks: bool,
    pub keys: bool,
    pub keys_top: bool,
    /// Index into `KEY_SIZES`.
    pub keys_size: usize,
    /// Kept part of the recording, then pieces cut out of it.
    pub trim: (f64, f64),
    pub cuts: Vec<(f64, f64)>,
    pub audio: bool,
    /// Loudness, 1 = as recorded.
    pub volume: f64,
    /// Both channels mixed into one, heard the same on either side.
    pub mono: bool,
    /// Indexes into `ASPECTS`, `RESOLUTIONS` and `FRAME_RATES`.
    pub aspect: usize,
    pub resolution: usize,
    pub frame_rate: usize,
    pub format: Format,
    /// Index into `QUALITIES`.
    pub quality: usize,
    /// The drawn cursor, for recordings made without the system one.
    pub cursor: bool,
    pub cursor_style: usize,
    /// 0.5..MAX_CURSOR times the usual size.
    pub cursor_size: f64,
    /// 0 follows the pointer exactly, 1 glides.
    pub cursor_smooth: f64,
    /// The cursor grows and tilts while it moves.
    pub cursor_motion: bool,
    /// Index into `MOTION_BLUR`.
    pub motion_blur: usize,
    /// The camera recorded alongside (studio mode), laid over a corner.
    pub cam: bool,
    /// 0 square … 1 circle.
    pub cam_round: f64,
    /// Side as a share of the output's short side.
    pub cam_size: f64,
    /// Index into `webcam::ANCHORS`: a corner or an edge's middle.
    pub cam_anchor: usize,
}

pub const MIN_CAM: f64 = 0.12;
pub const MAX_CAM: f64 = 0.5;

/// The camera's own recording, made next to the screen in studio mode.
pub fn camera_file(video: &Path) -> PathBuf {
    let mut p = track::path_for(video).into_os_string();
    p.push(".camera.mkv");
    PathBuf::from(p)
}

/// Seconds into the camera file where the screen recording begins.
pub fn camera_offset(video: &Path) -> f64 {
    let mut p = track::path_for(video).into_os_string();
    p.push(".camera.json");
    std::fs::read_to_string(PathBuf::from(p))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["offset"].as_f64())
        .unwrap_or(0.)
        .max(0.)
}

/// Whether the current look is a saved preset: every setting the preset
/// keeps is the same now (settings added since don't count against it).
pub fn look_matches(preset: &serde_json::Value, current: &serde_json::Value) -> bool {
    let Some(saved) = preset.as_object() else { return false };
    !saved.is_empty()
        && saved.iter().all(|(key, value)| match (value.as_f64(), current[key].as_f64()) {
            (Some(a), Some(b)) => (a - b).abs() < 1e-6,
            _ => current.get(key) == Some(value),
        })
}

/// Where the camera file starts against the screen, and the bubble's look
/// at the time, which the studio starts from.
/// The camera's alpha as an ffmpeg `geq` expression for a square of half
/// side `half` and corner radius `r`: the squircle GPUI draws the preview
/// with (`ui::SMOOTHING`, its "normalized superellipse": corners reaching
/// `r * (1 + s)` with power `-ln 2 / ln(1 - 0.2929 / (1 + s))`), or a plain
/// rounded square where those corners wouldn't fit, as GPUI falls back too
/// (a circle stays a circle).
fn camera_alpha(half: f64, r: f64) -> String {
    let s = kadr_core::color::SMOOTHING as f64;
    let reach = r * (1. + s);
    if r > 0. && 2. * reach <= 2. * half {
        let power = -(2f64.ln()) / (1. - 0.292_893_218_813_452_4 / (1. + s)).ln();
        let inner = half - reach;
        let axis = |v: &str| format!("pow(max(abs({v}+0.5-{half})-{inner},0)/{reach},{power})");
        format!("255*clip(0.5-{reach}*(pow({}+{},{})-1),0,1)", axis("X"), axis("Y"), 1. / power)
    } else {
        format!("255*clip(0.5-(hypot(max(abs(X+0.5-{half})-{inner},0),max(abs(Y+0.5-{half})-{inner},0))-{r}),0,1)", inner = half - r)
    }
}

pub fn save_camera(video: &Path, offset: f64, look: crate::camera::Look) {
    let mut p = track::path_for(video).into_os_string();
    p.push(".camera.json");
    let path = PathBuf::from(p);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, json!({ "offset": offset, "look": look.to_json() }).to_string());
}

/// The bubble's look when the recording was made.
fn camera_look(video: &Path) -> Option<crate::camera::Look> {
    let mut p = track::path_for(video).into_os_string();
    p.push(".camera.json");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(PathBuf::from(p)).ok()?).ok()?;
    v.get("look").map(crate::camera::Look::from_json)
}

/// Where the camera sits on the output frame: left, top, side and corner
/// radius, in output pixels; placed like the bubble was on screen.
pub fn camera_box(l: &Layout, project: &Project) -> (f64, f64, f64, f64) {
    let (x, y, side) = crate::camera::place(project.cam_anchor, project.cam_size, (0., 0., l.ow as f64, l.oh as f64));
    let side = ((side / 2.).round() * 2.).max(8.);
    (x.round(), y.round(), side, project.cam_round * side / 2.)
}

impl Project {
    fn path(video: &Path) -> PathBuf {
        let mut p = track::path_for(video).into_os_string();
        p.push(".studio");
        PathBuf::from(p)
    }

    /// The saved edit, or segments placed on the clicks of a studio recording.
    pub fn load(video: &Path, track: &Track, p: &Probe) -> Project {
        let saved = std::fs::read_to_string(Self::path(video)).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let mut d = Project::auto(track, p);
        // Recorded with studio mode off: start untouched, zooms only on request.
        if !track.cursor_hidden {
            d.segments.clear();
            d.clicks = false;
        }
        if let Some(look) = camera_look(video) {
            (d.cam_anchor, d.cam_size, d.cam_round) = (look.anchor, look.size.clamp(MIN_CAM, MAX_CAM), look.round);
        }
        match saved {
            Some(v) => Project::from_json(&v, d, p),
            None => d,
        }
    }

    /// A project from its JSON; anything missing keeps `d`'s value.
    fn from_json(v: &serde_json::Value, d: Project, p: &Probe) -> Project {
        let segments = if v["segments"].is_array() {
            v["segments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                Some(Seg {
                    start: s["start"].as_f64()?,
                    end: s["end"].as_f64()?,
                    zoom: s["zoom"].as_f64().unwrap_or(DEFAULT_ZOOM),
                    focus: s["focus"].as_array().and_then(|f| Some((f.first()?.as_f64()?, f.get(1)?.as_f64()?))),
                })
            })
            .collect()
        } else {
            d.segments.clone()
        };
        let pair = |x: &serde_json::Value| Some((x.get(0)?.as_f64()?, x.get(1)?.as_f64()?));
        let index = |key: &str, default: usize, len: usize| (v[key].as_u64().map_or(default, |i| i as usize)).min(len - 1);
        // Older projects kept padding as an index into PADDINGS.
        let padding = v["pad"].as_f64().or_else(|| v["padding"].as_u64().map(|i| PADDINGS[(i as usize).min(PADDINGS.len() - 1)])).unwrap_or(d.padding);
        let trim = pair(&v["trim"]).map(|(a, b)| (a.clamp(0., p.duration), b.clamp(0., p.duration))).filter(|(a, b)| b - a > 0.1).unwrap_or(d.trim);
        Project {
            segments,
            backdrop: v["backdrop"].as_u64().map_or(d.backdrop, |b| b as usize),
            wallpaper: if v.get("wallpaper").is_some() { v["wallpaper"].as_str().map(PathBuf::from).filter(|w| w.exists()) } else { d.wallpaper.clone() },
            padding: padding.clamp(0., MAX_PADDING),
            radius: v["radius"].as_f64().unwrap_or(d.radius).clamp(0., MAX_RADIUS),
            shadow: v["shadow"].as_f64().unwrap_or(d.shadow).clamp(0., 1.),
            clicks: v["clicks"].as_bool().unwrap_or(d.clicks),
            keys: v["keys"].as_bool().unwrap_or(d.keys),
            keys_top: v["keys_top"].as_bool().unwrap_or(d.keys_top),
            keys_size: index("keys_size", d.keys_size, KEY_SIZES.len()),
            trim,
            cuts: if v["cuts"].is_array() { v["cuts"].as_array().into_iter().flatten().filter_map(pair).collect() } else { d.cuts.clone() },
            audio: v["audio"].as_bool().unwrap_or(d.audio),
            volume: v["volume"].as_f64().unwrap_or(d.volume).clamp(0.0, MAX_VOLUME),
            mono: v["mono"].as_bool().unwrap_or(d.mono),
            aspect: index("aspect", d.aspect, ASPECTS.len()),
            resolution: index("resolution", d.resolution, RESOLUTIONS.len()),
            frame_rate: index("frame_rate", d.frame_rate, FRAME_RATES.len()),
            format: match v["format"].as_str() {
                Some("gif") => Format::Gif,
                Some(_) => Format::Mp4,
                None => d.format,
            },
            quality: index("quality", d.quality, QUALITIES.len()),
            cursor: v["cursor"].as_bool().unwrap_or(d.cursor),
            cursor_style: index("cursor_style", d.cursor_style, CURSOR_STYLES.len()),
            cursor_size: v["cursor_size"].as_f64().unwrap_or(d.cursor_size).clamp(0.5, MAX_CURSOR),
            cursor_smooth: v["cursor_smooth"].as_f64().unwrap_or(d.cursor_smooth).clamp(0., 1.),
            cursor_motion: v["cursor_motion"].as_bool().unwrap_or(d.cursor_motion),
            motion_blur: index("motion_blur", d.motion_blur, MOTION_BLUR.len()),
            cam: v["cam"].as_bool().unwrap_or(d.cam),
            cam_round: v["cam_round"].as_f64().unwrap_or(d.cam_round).clamp(0., 1.),
            cam_size: v["cam_size"].as_f64().unwrap_or(d.cam_size).clamp(MIN_CAM, MAX_CAM),
            cam_anchor: index("cam_anchor", d.cam_anchor, crate::camera::ANCHORS.len()),
        }
    }

    pub fn save(&self, video: &Path) {
        let path = Self::path(video);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, self.to_json().to_string());
    }

    fn to_json(&self) -> serde_json::Value {
        let segments: Vec<_> = self
            .segments
            .iter()
            .map(|s| json!({ "start": s.start, "end": s.end, "zoom": s.zoom, "focus": s.focus.map(|(x, y)| [x, y]) }))
            .collect();
        json!({
            "segments": segments,
            "backdrop": self.backdrop,
            "wallpaper": self.wallpaper.as_ref().map(|w| w.display().to_string()),
            "pad": self.padding,
            "radius": self.radius,
            "shadow": self.shadow,
            "clicks": self.clicks,
            "keys": self.keys,
            "keys_top": self.keys_top,
            "keys_size": self.keys_size,
            "trim": [self.trim.0, self.trim.1],
            "cuts": self.cuts.iter().map(|c| [c.0, c.1]).collect::<Vec<_>>(),
            "audio": self.audio,
            "volume": self.volume,
            "mono": self.mono,
            "aspect": self.aspect,
            "resolution": self.resolution,
            "frame_rate": self.frame_rate,
            "format": if self.format == Format::Gif { "gif" } else { "mp4" },
            "quality": self.quality,
            "cursor": self.cursor,
            "cursor_style": self.cursor_style,
            "cursor_size": self.cursor_size,
            "cursor_smooth": self.cursor_smooth,
            "cursor_motion": self.cursor_motion,
            "motion_blur": self.motion_blur,
            "cam": self.cam,
            "cam_round": self.cam_round,
            "cam_size": self.cam_size,
            "cam_anchor": self.cam_anchor,
        })
    }

    /// The look of the video, without its edit: what presets keep and what
    /// carries over to the next recording.
    pub fn look(&self) -> serde_json::Value {
        let all = self.to_json();
        serde_json::Value::Object(LOOK_KEYS.iter().filter_map(|k| Some((k.to_string(), all.get(*k)?.clone()))).collect())
    }

    /// This project with another look; the edit stays.
    pub fn with_look(&self, look: &serde_json::Value, p: &Probe) -> Project {
        let mut merged = self.to_json();
        if let (Some(target), Some(source)) = (merged.as_object_mut(), look.as_object()) {
            for (k, v) in source.iter().filter(|(k, _)| LOOK_KEYS.contains(&k.as_str())) {
                target.insert(k.clone(), v.clone());
            }
        }
        Project::from_json(&merged, self.clone(), p)
    }

    /// Keep this look for the next recording.
    /// ffmpeg audio filters for the volume and channels; empty as recorded.
    pub fn audio_filters(&self) -> Vec<String> {
        let mut f = Vec::new();
        if self.mono {
            // Stereo first, so a mono recording has both channels to mix.
            f.push("aformat=channel_layouts=stereo,pan=mono|c0=0.5*c0+0.5*c1".to_owned());
        }
        if (self.volume - 1.0).abs() > 0.001 {
            f.push(format!("volume={:.2}", self.volume));
        }
        f
    }

    pub fn remember_look(&self) {
        let _ = std::fs::create_dir_all(dirs::config());
        let _ = std::fs::write(last_look_path(), self.look().to_string());
    }

    /// Zoom on clicks: a stretch per burst of clicks, held while typing.
    pub fn auto(track: &Track, p: &Probe) -> Project {
        // The look last used in the studio carries over.
        let look = std::fs::read_to_string(last_look_path()).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        Self::auto_with(track, p, look)
    }

    fn auto_with(track: &Track, p: &Probe, look: Option<serde_json::Value>) -> Project {
        let settings = Settings::load();
        let (rx, ry, rw, rh) = track.rect;
        let (sx, sy) = (p.w / rw.max(1.0), p.h / rh.max(1.0));
        let in_frame = |x: f64, y: f64| {
            let (px, py) = ((x - rx) * sx, (y - ry) * sy);
            px >= 0.0 && py >= 0.0 && px < p.w && py < p.h
        };
        let mut clicks: Vec<f64> = track.clicks.iter().filter(|c| in_frame(c.1, c.2)).map(|c| c.0).collect();
        clicks.sort_by(f64::total_cmp);
        let mut segs: Vec<(f64, f64)> = Vec::new();
        for t in clicks {
            match segs.last_mut() {
                Some(last) if t - last.1 < GAP => last.1 = t,
                _ => segs.push((t, t)),
            }
        }
        // Typing keeps an existing zoom alive but doesn't start one: we don't
        // know where the caret is.
        for &k in &track.keys {
            for seg in segs.iter_mut() {
                if k >= seg.0 && k <= seg.1 + HOLD {
                    seg.1 = seg.1.max(k);
                }
            }
        }
        let project = Project {
            segments: segs
                .into_iter()
                .map(|(s, e)| Seg { start: s.max(0.), end: (e + HOLD).min(p.duration), zoom: settings.studio_zoom, focus: None })
                .collect(),
            backdrop: settings.studio_backdrop,
            wallpaper: None,
            padding: PADDINGS[settings.studio_padding.min(PADDINGS.len() - 1)],
            radius: 22.,
            shadow: 0.5,
            clicks: !track.clicks.is_empty(),
            keys: !track.keystrokes.is_empty(),
            keys_top: false,
            keys_size: 1,
            trim: (0., p.duration),
            cuts: Vec::new(),
            audio: true,
            volume: 1.0,
            mono: false,
            aspect: 0,
            // 1440p for big recordings, like before.
            resolution: if p.h >= 1400.0 || p.w >= 2500.0 { 2 } else { 1 },
            frame_rate: 0,
            format: Format::Mp4,
            quality: 1,
            cursor: true,
            cursor_style: 0,
            cursor_size: 1.5,
            cursor_smooth: 0.5,
            cursor_motion: true,
            motion_blur: 1,
            cam: true,
            cam_round: 1.,
            cam_size: 0.26,
            cam_anchor: 4,
        };
        let mut project = match look {
            Some(look) => project.with_look(&look, p),
            None => project,
        };
        // A look can't switch on what the recording doesn't have.
        project.clicks &= !track.clicks.is_empty();
        project.keys &= !track.keystrokes.is_empty();
        project
    }

    pub fn colors(&self) -> (Rgba, Rgba) {
        let all = kadr_raster::backdrops();
        all[self.backdrop.min(all.len() - 1)]
    }

    /// Width over height of the output.
    pub fn aspect_ratio(&self, p: &Probe) -> f64 {
        ASPECTS[self.aspect.min(ASPECTS.len() - 1)].1.unwrap_or(p.w / p.h.max(1.))
    }

    /// What's left after the trim and the cuts, in source time.
    /// Cuts in order, with overlapping or touching ones joined into one.
    pub fn merge_cuts(&mut self) {
        let mut cuts: Vec<(f64, f64)> = self.cuts.iter().map(|&(a, b)| (a.min(b), a.max(b))).filter(|(a, b)| b - a > 0.01).collect();
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (a, b) in cuts {
            match merged.last_mut() {
                Some(last) if a <= last.1 + 0.01 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        self.cuts = merged;
    }

    pub fn kept(&self) -> Vec<(f64, f64)> {
        let mut cuts: Vec<(f64, f64)> = self.cuts.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect();
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out = Vec::new();
        let mut from = self.trim.0;
        for (a, b) in cuts {
            if b <= from || a >= self.trim.1 {
                continue;
            }
            if a > from {
                out.push((from, a));
            }
            from = from.max(b);
        }
        if self.trim.1 > from {
            out.push((from, self.trim.1));
        }
        out
    }

    /// Output frame rate.
    pub fn fps(&self, p: &Probe) -> f64 {
        FRAME_RATES[self.frame_rate.min(FRAME_RATES.len() - 1)].1.map_or_else(|| p.fps_value().min(MAX_EXPORT_FPS), f64::from)
    }

    /// Target video bitrate of the export, bits per second.
    pub fn bitrate(&self, p: &Probe) -> f64 {
        let l = layout(p, self, false);
        QUALITIES[self.quality.min(QUALITIES.len() - 1)].1 * l.ow as f64 * l.oh as f64 * self.fps(p)
    }

    /// File size in bytes: for MP4 the most it can be (the bitrate is capped),
    /// for GIF a rough guess, since it depends on the picture.
    pub fn estimate(&self, p: &Probe) -> f64 {
        let secs = self.kept_duration();
        match self.format {
            Format::Mp4 => {
                let audio = if p.audio && self.audio { AUDIO_BITRATE } else { 0. };
                (self.bitrate(p) + audio) * secs / 8. * 1.01
            }
            Format::Gif => {
                let (w, h) = gif_size(&layout(p, self, false));
                w as f64 * h as f64 * gif_fps(self) as f64 * secs * 0.045
            }
        }
    }

    pub fn kept_duration(&self) -> f64 {
        self.kept().iter().map(|(a, b)| b - a).sum()
    }

    /// Where playback goes from `t`: itself when kept, the end of the cut it
    /// falls into, or None past the end.
    pub fn next_kept(&self, t: f64) -> Option<f64> {
        self.kept().into_iter().find(|&(_, b)| t < b - 0.02).map(|(a, _)| t.max(a))
    }
}

/// Where `played` seconds of playback land in the video, playing only the
/// `kept` stretches in order; None past the last.
pub fn played_to_source(kept: &[(f64, f64)], played: f64) -> Option<f64> {
    let mut left = played;
    for &(a, b) in kept {
        if left < b - a {
            return Some(a + left);
        }
        left -= b - a;
    }
    None
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub w: f64,
    pub h: f64,
    pub fps: String,
    pub duration: f64,
    pub audio: bool,
}

/// Run a command to completion; returns (success, stdout).
async fn output(argv: &[&OsStr]) -> Option<(bool, String)> {
    let (cmd, args) = argv.split_first()?;
    let out = async_process::Command::new(cmd).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().await.ok()?;
    Some((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned()))
}

pub async fn probe(video: &Path) -> Option<Probe> {
    let (ok, out) = output(&[
        OsStr::new("ffprobe"), "-v".as_ref(), "error".as_ref(), "-show_entries".as_ref(), "stream=codec_type,width,height,r_frame_rate:format=duration".as_ref(),
        "-of".as_ref(), "json".as_ref(), video.as_os_str(),
    ])
    .await?;
    let v: serde_json::Value = serde_json::from_str(&out).ok().filter(|_| ok)?;
    let streams = v["streams"].as_array()?;
    let s = streams.iter().find(|s| s["codec_type"] == "video")?;
    Some(Probe {
        w: s["width"].as_f64()?,
        h: s["height"].as_f64()?,
        fps: s["r_frame_rate"].as_str().unwrap_or("60").to_owned(),
        duration: v["format"]["duration"].as_str()?.parse().ok()?,
        audio: streams.iter().any(|s| s["codec_type"] == "audio"),
    })
}

impl Probe {
    /// "60/1" as 60.
    pub fn fps_value(&self) -> f64 {
        let mut parts = self.fps.split('/').map(|v| v.parse::<f64>().unwrap_or(0.));
        let (n, d) = (parts.next().unwrap_or(60.), parts.next().unwrap_or(1.));
        if n > 0. && d > 0. { n / d } else { 60. }
    }
}

fn gif_fps(project: &Project) -> u32 {
    FRAME_RATES[project.frame_rate.min(FRAME_RATES.len() - 1)].1.unwrap_or(15).min(30)
}

fn gif_size(l: &Layout) -> (u32, u32) {
    let w = l.ow.min(1080);
    (w, (l.oh as f64 * w as f64 / l.ow as f64).round() as u32)
}

/// "12,4 МБ"
pub fn human_size(bytes: f64) -> String {
    let mb = bytes / 1_048_576.;
    if mb < 1. {
        tr!("studio.kb", (bytes / 1024.).round().max(1.))
    } else if mb < 10. {
        tr!("studio.mb", kadr_core::i18n::decimal(format!("{:.1}", mb)))
    } else {
        tr!("studio.mb", mb.round())
    }
}

/// Where the drawn cursor is over time, in video pixels, sampled at
/// `CURSOR_RATE`: the pointer's track through a critically damped spring,
/// stiffer when `smooth` is low.
pub fn cursor_path(track: &Track, p: &Probe, smooth: f64) -> Vec<(f64, f64, f64)> {
    let (rx, ry, rw, rh) = track.rect;
    let (sx, sy) = (p.w / rw.max(1.0), p.h / rh.max(1.0));
    let Some(first) = track.cursor_at(0.) else { return Vec::new() };
    let to_px = |(x, y): (f64, f64)| ((x - rx) * sx, (y - ry) * sy);
    let mut pos = to_px(first);
    let mut vel = (0., 0.);
    let omega = 34. - 28. * smooth.clamp(0., 1.);
    let dt = 1. / CURSOR_RATE;
    let steps = (p.duration * CURSOR_RATE).ceil() as usize;
    let mut out = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f64 * dt;
        let target = track.cursor_at(t).map(to_px).unwrap_or(pos);
        if smooth < 0.02 {
            pos = target;
        } else {
            let h = dt / 4.;
            for _ in 0..4 {
                let ax = omega * omega * (target.0 - pos.0) - 2. * omega * vel.0;
                let ay = omega * omega * (target.1 - pos.1) - 2. * omega * vel.1;
                vel = (vel.0 + ax * h, vel.1 + ay * h);
                pos = (pos.0 + vel.0 * h, pos.1 + vel.1 * h);
            }
        }
        out.push((t, pos.0, pos.1));
    }
    out
}

/// How the drawn cursor tilts and grows along its path, as (t, angle in
/// radians, scale): it leans toward where it's going and swells with speed,
/// both eased so they settle rather than snap, and it dips and springs back
/// on every click.
pub fn cursor_motion(path: &[(f64, f64, f64)], track: &Track, p: &Probe) -> Vec<(f64, f64, f64)> {
    let (mut angle, mut scale) = (0., 1.);
    // About 90 ms to follow a change.
    let follow = 1. - (-1. / (CURSOR_RATE * 0.09)).exp();
    let mut clicks: Vec<f64> = track.clicks.iter().map(|c| c.0).collect();
    clicks.sort_by(f64::total_cmp);
    let mut out = Vec::with_capacity(path.len());
    for (i, &(t, x, y)) in path.iter().enumerate() {
        let (px0, py0) = if i > 0 { (path[i - 1].1, path[i - 1].2) } else { (x, y) };
        // Speed in screen heights per second, so it means the same at any size.
        let (vx, vy) = ((x - px0) * CURSOR_RATE / p.h.max(1.), (y - py0) * CURSOR_RATE / p.h.max(1.));
        let speed = vx.hypot(vy);
        let target_angle = (vx * 0.5).clamp(-0.38, 0.38);
        let target_scale = 1. + 0.4 * (speed / 0.8).min(1.);
        angle += (target_angle - angle) * follow;
        scale += (target_scale - scale) * follow;
        let last_click = clicks.partition_point(|&c| c <= t).checked_sub(1).map(|j| clicks[j]);
        let press = last_click.map_or(1., |c| click_bounce(t - c));
        out.push((t, angle, scale * press));
    }
    out
}

/// The cursor's size on a click `dt` seconds ago: a quick press down to 3/4,
/// then a springy return with a slight overshoot.
fn click_bounce(dt: f64) -> f64 {
    const DOWN: f64 = 0.07;
    match dt {
        dt if !(0. ..0.6).contains(&dt) => 1.,
        dt if dt < DOWN => 1. - 0.25 * dt / DOWN,
        dt => 1. - 0.25 * (-(dt - DOWN) * 9.).exp() * ((dt - DOWN) * 14.).cos(),
    }
}

/// When the drawn cursor takes each shape. The pointer changed shape at a
/// spot; the smoothed cursor gets there a little later, so the change waits
/// for it (the hand shows up on the button, not before it).
pub fn cursor_shapes(track: &Track, path: &[(f64, f64, f64)], p: &Probe) -> Vec<(f64, String)> {
    let (rx, ry, rw, rh) = track.rect;
    let (sx, sy) = (p.w / rw.max(1.0), p.h / rh.max(1.0));
    let mut out: Vec<(f64, String)> = Vec::new();
    for (t, shape) in &track.shapes {
        let mut at = *t;
        if let Some((x, y)) = track.cursor_at(*t).map(|(x, y)| ((x - rx) * sx, (y - ry) * sy)) {
            let from = ((t * CURSOR_RATE).round() as usize).min(path.len());
            let to = ((t + 0.8) * CURSOR_RATE).round() as usize;
            let near = path[from..to.min(path.len())].iter().min_by(|a, b| (a.1 - x).hypot(a.2 - y).total_cmp(&(b.1 - x).hypot(b.2 - y)));
            at = near.map_or(at, |n| n.0);
        }
        // Keep the order; a change overtaken by the next one is dropped.
        while out.last().is_some_and(|l| l.0 >= at) {
            out.pop();
        }
        if out.last().is_none_or(|l| l.1 != *shape) {
            out.push((at, shape.clone()));
        }
    }
    out
}

/// The shape at time t in a `cursor_shapes` list (the arrow before any).
pub fn shape_at(shapes: &[(f64, String)], t: f64) -> &str {
    let i = shapes.partition_point(|s| s.0 <= t);
    i.checked_sub(1).map_or("arrow", |j| shapes[j].1.as_str())
}

/// The drawn cursor's shapes over time for ffmpeg's concat demuxer: one
/// picture per shape, all on one square with the hotspot in the middle, so
/// the shape can change without the cursor jumping. Returns the list file
/// and the square's side.
fn cursor_frames(track: &Track, path: &[(f64, f64, f64)], p: &Probe, project: &Project, tmp: &Path) -> Option<(PathBuf, u32)> {
    use std::fmt::Write as _;
    let height = cursor_height(track, p, project.cursor_size);
    let shapes = cursor_shapes(track, path, p);
    // Changes in order, merged, starting at 0.
    let mut changes: Vec<(f64, &str)> = vec![(0., shape_at(&shapes, 0.))];
    for (t, shape) in &shapes {
        if *t > 0. && *t < p.duration && changes.last().is_some_and(|l| l.1 != shape) {
            changes.push((*t, shape));
        }
    }
    let mut used: Vec<&str> = changes.iter().map(|c| c.1).collect();
    used.sort();
    used.dedup();
    let pictures: Vec<(&str, image::RgbaImage, (f64, f64))> = used
        .iter()
        .filter_map(|&shape| {
            let (png, hot, _) = cursor_image(project.cursor_style, shape, height, 0.)?;
            Some((shape, image::load_from_memory(&png).ok()?.to_rgba8(), hot))
        })
        .collect();
    let reach = pictures.iter().map(|(_, img, (hx, hy))| hx.max(img.width() as f64 - hx).max(*hy).max(img.height() as f64 - hy)).fold(1., f64::max);
    let side = (2. * reach).ceil() as u32 / 2 * 2 + 2;
    let mut files = std::collections::HashMap::new();
    for (i, (shape, img, (hx, hy))) in pictures.iter().enumerate() {
        let mut square = image::RgbaImage::new(side, side);
        image::imageops::overlay(&mut square, img, (side as f64 / 2. - hx).round() as i64, (side as f64 / 2. - hy).round() as i64);
        let file = tmp.join(format!("cursor-{i}.png"));
        square.save(&file).ok()?;
        files.insert(*shape, file);
    }
    let mut list = String::from("ffconcat version 1.0\n");
    for (i, (t, shape)) in changes.iter().enumerate() {
        let end = changes.get(i + 1).map_or(p.duration + 1., |n| n.0);
        let _ = writeln!(list, "file '{}'\nduration {:.3}", files.get(shape)?.display(), end - t);
    }
    // The demuxer needs the last picture again to honour its duration.
    let _ = writeln!(list, "file '{}'", files.get(changes.last()?.1)?.display());
    let file = tmp.join("cursor.ffconcat");
    std::fs::write(&file, list).ok()?;
    Some((file, side))
}

/// The drawn cursor's height in video pixels.
pub fn cursor_height(track: &Track, p: &Probe, size: f64) -> f64 {
    CURSOR_BASE * size * p.h / track.rect.3.max(1.)
}

/// The cursor picture as PNG, with its hotspot and size in pixels, turned
/// clockwise by `angle` radians around the hotspot. `shape` is a `cursors`
/// name; the dot style looks the same for all.
pub fn cursor_image(style: usize, shape: &str, height: f64, angle: f64) -> Option<(Vec<u8>, (f64, f64), (u32, u32))> {
    let (pm, hot) = cursor_pixmap(style, shape, height)?;
    if angle.abs() < 1e-3 {
        return Some((pm.encode_png().ok()?, hot, (pm.width(), pm.height())));
    }
    // A square around the hotspot, big enough for the picture at any angle.
    let (w, h) = (pm.width() as f64, pm.height() as f64);
    let reach = hot.0.max(w - hot.0).hypot(hot.1.max(h - hot.1));
    let side = (2. * reach).ceil() as u32 + 2;
    let half = side as f32 / 2.;
    let mut turned = tiny_skia::Pixmap::new(side, side)?;
    let transform = tiny_skia::Transform::from_rotate_at(angle.to_degrees() as f32, half, half).pre_translate(half - hot.0 as f32, half - hot.1 as f32);
    turned.draw_pixmap(0, 0, pm.as_ref(), &tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() }, transform, None);
    Some((turned.encode_png().ok()?, (half as f64, half as f64), (side, side)))
}

fn cursor_pixmap(style: usize, shape: &str, height: f64) -> Option<(tiny_skia::Pixmap, (f64, f64))> {
    use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};
    let h = height.max(8.) as f32;
    let pad = (h * 0.2).ceil();
    let paint = |r: u8, g: u8, b: u8, a: u8| {
        let mut p = Paint::default();
        p.set_color_rgba8(r, g, b, a);
        p.anti_alias = true;
        p
    };
    if style == 1 {
        // A soft dark dot with a white rim.
        let r = h * 0.32;
        let size = (2. * (r + pad)) as u32;
        let mut pm = Pixmap::new(size, size)?;
        let c = size as f32 / 2.;
        let circle = |r: f32| PathBuilder::from_circle(c, c, r);
        pm.fill_path(&circle(r * 1.35)?, &paint(0, 0, 0, 40), FillRule::Winding, Transform::identity(), None);
        pm.fill_path(&circle(r)?, &paint(24, 24, 28, 190), FillRule::Winding, Transform::identity(), None);
        let rim = Stroke { width: (h * 0.06).max(1.5), ..Stroke::default() };
        pm.stroke_path(&circle(r)?, &paint(255, 255, 255, 235), &rim, Transform::identity(), None);
        return Some((pm, (c as f64, c as f64)));
    }
    // A macOS-like picture of the pointer's shape, the arrow `h` tall.
    let (mut pm, hot) = kadr_raster::cursors::render(shape, h)?;
    if style == 2 {
        // Inverted: premultiplied, so each channel turns into alpha less it.
        for px in pm.data_mut().chunks_exact_mut(4) {
            let a = px[3];
            for c in &mut px[..3] {
                *c = a.saturating_sub(*c);
            }
        }
    }
    Some((pm, hot))
}

fn ease(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    if x < 0.5 { 4.0 * x * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 }
}

/// How far into a segment the camera is at `t`: 0 out, 1 fully zoomed.
fn weight(seg: &Seg, t: f64) -> f64 {
    let rise = ease((t - (seg.start - RAMP)) / RAMP);
    let fall = 1.0 - ease((t - seg.end) / RAMP);
    rise.min(fall)
}

pub type Curve = Vec<(f64, f64)>;

/// The camera over time, sampled at `RATE`: zoom factor and center (video
/// pixels). Shared by the editor preview and the render.
pub struct Camera {
    pub zoom: Curve,
    pub x: Curve,
    pub y: Curve,
}

impl Camera {
    pub fn new(track: &Track, p: &Probe, segs: &[Seg]) -> Camera {
        let (rx, ry, rw, rh) = track.rect;
        let (sx, sy) = (p.w / rw.max(1.0), p.h / rh.max(1.0));
        let to_px = |x: f64, y: f64| ((x - rx) * sx, (y - ry) * sy);

        let (mut z, mut cx, mut cy) = (Vec::new(), Vec::new(), Vec::new());
        let mut pos = (p.w / 2.0, p.h / 2.0);
        let mut vel = (0.0, 0.0);
        let dt = 1.0 / RATE;
        // Critically damped spring: smooth, no overshoot, settles in ~0.6 s.
        let omega = 7.0;
        let steps = (p.duration * RATE).ceil() as usize;
        for i in 0..=steps {
            let t = i as f64 * dt;
            // The strongest segment at `t` decides zoom and focus.
            let (seg, w) = segs
                .iter()
                .map(|s| (Some(s), weight(s, t)))
                .fold((None, 0.0), |best, cur| if cur.1 > best.1 { cur } else { best });
            let cursor = track.cursor_at(t).map(|(x, y)| to_px(x, y)).unwrap_or(pos);
            let target = seg.and_then(|s| s.focus).unwrap_or(cursor);
            let target = (target.0.clamp(0.0, p.w), target.1.clamp(0.0, p.h));
            if w < 0.01 {
                pos = target;
                vel = (0.0, 0.0);
            } else {
                let h = dt / 4.0;
                for _ in 0..4 {
                    let ax = omega * omega * (target.0 - pos.0) - 2.0 * omega * vel.0;
                    let ay = omega * omega * (target.1 - pos.1) - 2.0 * omega * vel.1;
                    vel = (vel.0 + ax * h, vel.1 + ay * h);
                    pos = (pos.0 + vel.0 * h, pos.1 + vel.1 * h);
                }
            }
            let zoom = seg.map_or(1.0, |s| s.zoom);
            z.push((t, 1.0 + (zoom - 1.0) * w));
            cx.push((t, pos.0));
            cy.push((t, pos.1));
        }
        Camera { zoom: z, x: cx, y: cy }
    }

    /// Zoom and center at `t`, for the preview.
    pub fn at(&self, t: f64) -> (f64, f64, f64) {
        let sample = |c: &Curve| {
            let i = ((t * RATE).floor() as usize).min(c.len().saturating_sub(1));
            let (a, b) = (c[i], c[(i + 1).min(c.len() - 1)]);
            let k = if b.0 > a.0 { ((t - a.0) / (b.0 - a.0)).clamp(0., 1.) } else { 0. };
            a.1 + (b.1 - a.1) * k
        };
        if self.zoom.is_empty() {
            return (1., 0., 0.);
        }
        (sample(&self.zoom), sample(&self.x), sample(&self.y))
    }
}

/// Ramer–Douglas–Peucker on a time series.
fn simplify(points: &[(f64, f64)], eps: f64) -> Curve {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (points[a], points[b]);
        let (mut worst, mut idx) = (0.0, 0);
        for (i, p) in points.iter().enumerate().take(b).skip(a + 1) {
            let k = (p.0 - pa.0) / (pb.0 - pa.0);
            let d = (p.1 - (pa.1 + (pb.1 - pa.1) * k)).abs();
            if d > worst {
                worst = d;
                idx = i;
            }
        }
        if worst > eps {
            keep[idx] = true;
            stack.push((a, idx));
            stack.push((idx, b));
        }
    }
    points.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Piecewise-linear curve as an ffmpeg expression in `t`: a sum of clipped
/// ramps. The sum is nested as a balanced tree: ffmpeg 9 refuses expressions
/// nested deeper than about 60 levels, and a flat chain of terms nests one
/// level per term.
fn expr(curve: &[(f64, f64)], precision: usize) -> String {
    let Some(&(_, v0)) = curve.first() else { return "0".into() };
    let mut terms = vec![format!("{v0:.precision$}")];
    for w in curve.windows(2) {
        let ((t0, a), (t1, b)) = (w[0], w[1]);
        let delta = b - a;
        if delta.abs() < 10f64.powi(-(precision as i32)) || t1 <= t0 {
            continue;
        }
        terms.push(format!("{delta:.precision$}*clip((t-{t0:.3})/{:.3},0,1)", t1 - t0));
    }
    balanced_sum(&terms)
}

fn balanced_sum(terms: &[String]) -> String {
    if terms.len() <= 8 {
        return terms.join("+");
    }
    let (left, right) = terms.split_at(terms.len() / 2);
    format!("({})+({})", balanced_sum(left), balanced_sum(right))
}

#[derive(Clone)]
pub struct Layout {
    pub ow: u32,
    pub oh: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub radius: f64,
}

/// Output frame and where the video sits in it. `preview` renders small.
pub fn layout(p: &Probe, project: &Project, preview: bool) -> Layout {
    let even = |v: f64| (((v / 2.0).round() * 2.0) as u32).max(2);
    let short = if preview { 720. } else { RESOLUTIONS[project.resolution.min(RESOLUTIONS.len() - 1)].1 as f64 };
    let ar = project.aspect_ratio(p);
    let (ow, oh) = if ar >= 1. { (even(short * ar), even(short)) } else { (even(short), even(short / ar)) };
    let pad = project.padding * ow.min(oh) as f64;
    let k = ((ow as f64 - 2.0 * pad) / p.w).min((oh as f64 - 2.0 * pad) / p.h);
    let (w, h) = (even(p.w * k).min(ow), even(p.h * k).min(oh));
    // No rounding needed when the video fills the frame.
    let fills = w == ow && h == oh;
    let radius = if fills { 0. } else { project.radius * ow.min(oh) as f64 / 1080.0 };
    Layout { ow, oh, x: even((ow - w) as f64 / 2.0).min(ow - w), y: even((oh - h) as f64 / 2.0).min(oh - h), w, h, radius }
}

/// The backdrop picture: the wallpaper cropped to fill, or the gradient.
fn backdrop_pixels(l: &Layout, project: &Project) -> image::RgbImage {
    if let Some(img) = project.wallpaper.as_ref().and_then(|w| image::open(w).ok()) {
        return img.resize_to_fill(l.ow, l.oh, image::imageops::FilterType::Triangle).to_rgb8();
    }
    let rgb = |c: Rgba| [c.r as f64 * 255., c.g as f64 * 255., c.b as f64 * 255.];
    let (c0, c1) = project.colors();
    let (c0, c1) = (rgb(c0), rgb(c1));
    image::RgbImage::from_fn(l.ow, l.oh, |x, y| {
        let g = ((x as f64 / l.ow as f64) * 0.6 + (y as f64 / l.oh as f64) * 0.4).clamp(0.0, 1.0);
        image::Rgb(std::array::from_fn(|ch| (c0[ch] + (c1[ch] - c0[ch]) * g).round().clamp(0., 255.) as u8))
    })
}

/// Backdrop with the video's drop shadow baked in, as a binary PPM.
fn write_background(path: &Path, l: &Layout, project: &Project) -> std::io::Result<()> {
    let mut img = backdrop_pixels(l, project);
    let blur = 0.035 * l.ow.min(l.oh) as f64;
    let (cx, cy) = (l.x as f64 + l.w as f64 / 2.0, l.y as f64 + l.h as f64 / 2.0 + 0.012 * l.ow.min(l.oh) as f64);
    let (hw, hh) = (l.w as f64 / 2.0, l.h as f64 / 2.0);
    let r = l.radius;
    let strength = project.shadow.clamp(0., 1.);
    if strength > 0. {
        for (x, y, px) in img.enumerate_pixels_mut() {
            // Signed distance to the shadow's rounded rectangle.
            let qx = (x as f64 - cx).abs() - (hw - r);
            let qy = (y as f64 - cy).abs() - (hh - r);
            let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
            let shade = 1.0 - 0.7 * strength * (1.0 - smoothstep(-blur * 0.4, blur, d));
            for v in px.0.iter_mut() {
                *v = (*v as f64 * shade).round() as u8;
            }
        }
    }
    let mut buf = Vec::with_capacity((l.ow * l.oh * 3) as usize + 32);
    write!(buf, "P6\n{} {}\n255\n", l.ow, l.oh)?;
    buf.extend_from_slice(img.as_raw());
    std::fs::write(path, buf)
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let k = ((x - a) / (b - a)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

pub fn output_path(video: &Path, format: Format) -> PathBuf {
    // From a `.kadr`: next to the project file, not in kadr's cache.
    let named = kadr_core::bundle::origin_of(video).unwrap_or_else(|| video.to_path_buf());
    let stem = named.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    named.with_file_name(format!("{stem}-studio.{}", if format == Format::Gif { "gif" } else { "mp4" }))
}

/// The pieces of a recording's edit kept beside kadr's cache, by the name
/// they have inside a `.kadr`.
fn sidecars(video: &Path) -> [(&'static str, PathBuf); 4] {
    let with = |suffix: &str| {
        let mut p = track::path_for(video).into_os_string();
        p.push(suffix);
        PathBuf::from(p)
    };
    [("track.json", track::path_for(video)), ("studio.json", with(".studio")), ("camera.mkv", with(".camera.mkv")), ("camera.json", with(".camera.json"))]
}

/// Pack a recording with its whole edit (`project`, saved first) into the
/// `.kadr` at `target`. Slow for long videos: run it off the UI thread.
pub fn save_bundle(video: &Path, project: &Project, target: &Path) -> std::io::Result<()> {
    project.save(video);
    let ext = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "bin".into());
    let mut files = vec![(format!("video.{}", ext(video)), video.to_path_buf())];
    files.extend(sidecars(video).into_iter().filter(|(_, p)| p.exists()).map(|(n, p)| (n.to_owned(), p)));
    if let Some(wall) = project.wallpaper.as_ref().filter(|w| w.exists()) {
        files.push((format!("wallpaper.{}", ext(wall)), wall.clone()));
    }
    kadr_core::bundle::pack(target, kadr_core::bundle::Kind::Studio, &files)
}

/// The recording in an unpacked `.kadr` (from `kadr_file`), its edit put
/// where the studio looks for it.
pub fn from_bundle(kadr_file: &Path, unpacked: &kadr_core::bundle::Unpacked) -> std::io::Result<PathBuf> {
    let dir = &unpacked.dir;
    let files: Vec<PathBuf> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).collect();
    let named = |prefix: &str| files.iter().find(|p| p.file_stem().is_some_and(|s| s == prefix)).cloned();
    // A name of its own (the folder is per project file), since the studio
    // keys its files by the video's name.
    let stem = kadr_file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let tag = dir.file_name().map(|n| n.to_string_lossy().chars().take(8).collect::<String>()).unwrap_or_default();
    let ext_of = |p: &Path| p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
    if !unpacked.fresh
        && let Some(video) = files.iter().find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy() == format!("{stem}-{tag}")))
    {
        return Ok(video.clone());
    }
    let source = named("video").ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no video inside"))?;
    let video = dir.join(format!("{stem}-{tag}.{}", ext_of(&source)));
    std::fs::rename(&source, &video)?;
    let wallpaper = named("wallpaper");
    for (name, to) in sidecars(&video) {
        let from = dir.join(name);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !from.exists() {
            let _ = std::fs::remove_file(&to);
            continue;
        }
        if name == "studio.json" {
            // The wallpaper travels inside; point the edit at that copy.
            let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&from)?).unwrap_or_default();
            if let (Some(wall), Some(o)) = (&wallpaper, v.as_object_mut()) {
                o.insert("wallpaper".into(), json!(wall.display().to_string()));
            }
            std::fs::write(&to, v.to_string())?;
        } else if std::fs::rename(&from, &to).is_err() {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(video)
}

pub fn preview_path(video: &Path) -> PathBuf {
    dirs::runtime().join(format!("kadr-preview-{}.mp4", video.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()))
}

/// When the picture moves fast enough to blur: the camera zooming or panning,
/// or the drawn cursor flying across. Source-time ranges, slightly padded.
fn motion_ranges(camera: &Camera, cursor: Option<&[(f64, f64, f64)]>, p: &Probe) -> Vec<(f64, f64)> {
    let mut moving: Vec<f64> = Vec::new();
    for i in 1..camera.zoom.len().min(camera.x.len()).min(camera.y.len()) {
        let dt = (camera.zoom[i].0 - camera.zoom[i - 1].0).max(1e-6);
        let zoom = camera.zoom[i].1;
        let zoom_speed = (camera.zoom[i].1 - camera.zoom[i - 1].1).abs() / dt;
        // Panning as seen on screen: in frame widths per second.
        let pan = (camera.x[i].1 - camera.x[i - 1].1).hypot(camera.y[i].1 - camera.y[i - 1].1) / dt / p.w * zoom;
        if zoom > 1.01 && (zoom_speed > 0.25 || pan > 0.25) || zoom_speed > 0.25 {
            moving.push(camera.zoom[i].0);
        }
    }
    for w in cursor.unwrap_or(&[]).windows(2) {
        let dt = (w[1].0 - w[0].0).max(1e-6);
        if (w[1].1 - w[0].1).hypot(w[1].2 - w[0].2) / dt / p.w > 1.2 {
            moving.push(w[1].0);
        }
    }
    moving.sort_by(f64::total_cmp);
    let mut ranges: Vec<(f64, f64)> = Vec::new();
    for t in moving {
        match ranges.last_mut() {
            Some(last) if t - last.1 < 0.12 => last.1 = t,
            _ => ranges.push((t, t)),
        }
    }
    ranges.into_iter().map(|(a, b)| ((a - 0.05).max(0.), b + 0.05)).collect()
}

/// What a preset or the remembered look carries: everything but the edit
/// (zooms, trim, cuts).
const LOOK_KEYS: [&str; 27] = [
    "backdrop", "wallpaper", "pad", "radius", "shadow", "clicks", "keys", "keys_top", "keys_size", "audio", "volume", "mono", "aspect", "resolution",
    "frame_rate", "format", "quality", "cursor", "cursor_style", "cursor_size", "cursor_smooth", "cursor_motion", "motion_blur", "cam", "cam_round",
    "cam_size", "cam_anchor",
];

fn last_look_path() -> PathBuf {
    dirs::config().join("studio-look.json")
}

fn presets_path() -> PathBuf {
    dirs::config().join("studio-presets.json")
}

/// Saved looks, by name.
pub fn presets() -> Vec<(String, serde_json::Value)> {
    let saved = std::fs::read_to_string(presets_path()).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    saved
        .as_ref()
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["name"].as_str()?.to_owned(), p["look"].clone())))
        .collect()
}

pub fn save_presets(presets: &[(String, serde_json::Value)]) {
    let v: Vec<_> = presets.iter().map(|(name, look)| json!({ "name": name, "look": look })).collect();
    let _ = std::fs::create_dir_all(dirs::config());
    let _ = std::fs::write(presets_path(), serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// `select` expression keeping the given stretches.
pub fn keep_expr(kept: &[(f64, f64)]) -> String {
    kept.iter().map(|(a, b)| format!("between(t,{a:.3},{b:.3})")).collect::<Vec<_>>().join("+")
}

/// Render `project` of `video` to `out`, ffmpeg's encoder on an NVIDIA card
/// when `nvidia` says there is one. Ok(false) when cancelled.
pub async fn run(video: &Path, project: &Project, out: &Path, preview: bool, progress: &Job, nvidia: bool) -> Result<bool, String> {
    let p = probe(video).await.ok_or(t("studio.ffprobe_couldnt_read_video"))?;
    let track = Track::load(video).unwrap_or_else(|| Track { rect: (0.0, 0.0, p.w, p.h), ..Default::default() });
    let l = layout(&p, project, preview);
    let camera = Camera::new(&track, &p, &project.segments);
    let (z, x, y) = (simplify(&camera.zoom, 0.004), simplify(&camera.x, 2.0), simplify(&camera.y, 2.0));
    let kept = project.kept();
    if kept.is_empty() {
        return Err(t("studio.nothing_left_after_trimming").into());
    }
    let whole = kept.len() == 1 && kept[0].0 <= 0.01 && kept[0].1 >= p.duration - 0.01;
    let length = project.kept_duration();
    // Everything runs at the export's rate from the start (a 240 fps
    // recording exported at 60 used to go through every step at 240), and
    // cut pieces are dropped before the heavy steps, not after.
    let rate = project.fps(&p);

    let tmp = dirs::runtime().join(format!("kadr-studio-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let bg = tmp.join("bg.ppm");
    let script = tmp.join("filter.txt");
    {
        let (bg, l, project) = (bg.clone(), l.clone(), project.clone());
        // Big wallpapers take a moment to scale.
        smol_unblock(move || write_background(&bg, &l, &project)).await.map_err(|e| e.to_string())?;
    }

    // Click rings go on the recording itself (so they zoom with the camera),
    // shortcut pills on the finished frame. Inputs 0 and 1 are the video and
    // the backdrop; the overlays follow.
    let cursor = if track.cursor_hidden && project.cursor {
        let path = cursor_path(&track, &p, project.cursor_smooth);
        let (list, side) = cursor_frames(&track, &path, &p, project, &tmp).ok_or(t("studio.couldnt_draw_cursor"))?;
        let motion = project.cursor_motion.then(|| cursor_motion(&path, &track, &p));
        let half = side as f64 / 2.;
        Some(overlays::Cursor { list, hot: (half, half), size: (side as f64, side as f64), path, motion, trail: project.motion_blur > 0 })
    } else {
        None
    };
    // Motion blur: the picture is rendered at several times the frame rate,
    // so the camera and the cursor take in-between positions; while they
    // move fast those sub-frames are averaged, then the rate comes back down.
    let shutter = MOTION_BLUR[project.motion_blur.min(MOTION_BLUR.len() - 1)].1;
    let blur_rate = (rate * SUBFRAMES as f64).min(MAX_SUBFRAME_RATE).max(rate);
    // Sub-frames in the shutter; under two (a 240 fps recording's shutter is
    // a 960th of a second) there's nothing to blur.
    let frames = (shutter as f64 * blur_rate / (SUBFRAMES as f64 * p.fps_value().max(1.))).round() as usize;
    let ranges = if frames >= 2 { motion_ranges(&camera, cursor.as_ref().map(|c| c.path.as_slice()), &p) } else { Vec::new() };
    // The rate everything before the blur runs at.
    let work = if ranges.is_empty() { rate } else { blur_rate };
    let wp = Probe { fps: format!("{work}"), ..p.clone() };
    let out_fps = format!("{rate}");
    let ranges_in: Vec<(f64, f64)> = ranges.iter().map(|&(a, b)| ((a - frames as f64 / work).max(0.), b)).collect();
    let blur = (!ranges.is_empty()).then(|| {
        let weights = vec!["1"; frames].join(" ");
        // In YUV (a third of the work of RGBA), before the backdrop goes
        // under at the export's rate. tmix can't be switched on and off in
        // time (it would mix in frames from before the pause), so it always
        // runs and its result is laid over the sharp picture only while
        // things move.
        format!(
            "[fg0]split[sub][mix];[sub]fps={out_fps}[sharp];[mix]select='{}',tmix=frames={frames}:weights='{weights}',fps={out_fps}[blurred];[sharp][blurred]overlay=0:0:enable='{}'[fg];",
            keep_expr(&ranges_in),
            keep_expr(&ranges),
        )
    });
    // Sub-frames only where they're blurred (and just before, for the
    // first average): elsewhere one in `ratio` goes on.
    let ratio = work / rate;
    let thin = (!ranges.is_empty() && (ratio - ratio.round()).abs() < 1e-6).then(|| format!("(not(mod(n,{}))+{})", ratio.round(), keep_expr(&ranges_in)));
    let first = match (thin, whole) {
        (None, true) => format!("fps={}", wp.fps),
        (None, false) => format!("fps={},select='{}'", wp.fps, keep_expr(&kept)),
        (Some(thin), true) => format!("fps={},select='{thin}'", wp.fps),
        (Some(thin), false) => format!("fps={},select='{thin}*({})'", wp.fps, keep_expr(&kept)),
    };
    let style = overlays::Style {
        clicks: project.clicks,
        keys: project.keys,
        keys_top: project.keys_top,
        keys_size: KEY_SIZES[project.keys_size.min(KEY_SIZES.len() - 1)].1,
        cursor,
        first,
    };
    let overlays = overlays::build(&track, &wp, &l, &tmp, &style).map_err(|e| e.to_string())?;

    // Trim and cuts drop frames at the end and close the gaps; a fixed
    // export rate goes last.
    // The backdrop runs on through the gaps (showing the last picture), so
    // those frames go here and the rest close up.
    let mut tail = String::new();
    if !whole {
        tail += &format!(",select='{}',setpts=N/({})/TB", keep_expr(&kept), out_fps);
    }
    let audio = p.audio && project.audio && project.format == Format::Mp4;
    let mut audio_filters = project.audio_filters();
    if !whole {
        audio_filters.insert(0, format!("aselect='{}',asetpts=N/SR/TB", keep_expr(&kept)));
    }
    let audio_graph = if audio && !audio_filters.is_empty() { format!(";[0:a]{}[aout]", audio_filters.join(",")) } else { String::new() };

    // libplacebo asserts on some fractional crop sizes, so the crop is kept to
    // whole, even pixels.
    // libplacebo measures corner rounding against a quarter of the shorter side.
    let rounding = (4.0 * l.radius / l.w.min(l.h) as f64).clamp(0.0, 1.0);
    let zexpr = expr(&z, 4);
    // The camera goes over the finished frame, in its corner, cut to its
    // shape with an antialiased alpha mask.
    let cam_file = camera_file(video);
    let cam = (project.cam && cam_file.exists()).then(|| {
        let (cx, cy, side, r) = camera_box(&l, project);
        let index = 2 + overlays.inputs.iter().filter(|a| a.as_os_str() == "-i").count();
        let half = side / 2.;
        let alpha = camera_alpha(half, r);
        format!(
            "[{index}:v]fps={fps},crop='min(iw,ih)':'min(iw,ih)',scale={side}:{side},format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='{alpha}'[cam];[{last}][cam]overlay={cx}:{cy}:eof_action=pass[withcam];",
            fps = out_fps,
            last = overlays.last,
        )
    });
    let last = if cam.is_some() { "withcam".to_owned() } else { overlays.last.clone() };
    // A picture that fills the frame (no padding, square corners) hides the
    // backdrop: laying it over that took a third of the export. Otherwise
    // it goes on in YUV, not RGBA.
    let covers = l.x == 0 && l.y == 0 && l.w == l.ow && l.h == l.oh && rounding <= 0.;
    let compose = if covers { "[fg]null[framed];" } else { "[1:v]format=yuv420p[bgy];[bgy][fg]overlay=0:0:shortest=1[framed];" };
    let graph = format!(
        "{pre}[{src}]libplacebo=w={ow}:h={oh}:pos_x={px}:pos_y={py}:pos_w={pw}:pos_h={ph}\
         :crop_w='2*floor(iw/({zexpr})/2)':crop_h='2*floor(ih/({zexpr})/2)'\
         :crop_x='floor(clip({xe}-cw/2,0,iw-cw))':crop_y='floor(clip({ye}-ch/2,0,ih-ch))'\
         :corner_rounding={rounding:.4}:fillcolor=black@0:format={format}[fg0];{blur}{compose}\
         {post}{cam}[{last}]format=yuv420p{tail}[out]{audio_graph}",
        pre = overlays.pre, src = overlays.src, post = overlays.post, cam = cam.clone().unwrap_or_default(),
        format = if covers { "yuv420p" } else { "yuva420p" },
        blur = blur.clone().unwrap_or_else(|| "[fg0]null[fg];".into()),
        ow = l.ow, oh = l.oh, px = l.x, py = l.y, pw = l.w, ph = l.h,
        xe = expr(&x, 1), ye = expr(&y, 1),
    );
    // A copy next to the log, to see what was rendered when something's off.
    let _ = std::fs::write(dirs::cache().join("studio-filter.txt"), &graph);
    std::fs::write(&script, graph).map_err(|e| e.to_string())?;

    // A GIF is made from an intermediate MP4.
    let gif = project.format == Format::Gif && !preview;
    let mp4 = if gif { tmp.join("studio.mp4") } else { out.to_path_buf() };
    let cam_start = format!("{:.3}", camera_offset(video));
    let mut result = Err(String::new());
    // The export is encoded to a bitrate so its size matches the estimate; a
    // GIF's intermediate MP4 just needs to be clean.
    let rate = if gif { Project { quality: 0, ..project.clone() }.bitrate(&p) } else { project.bitrate(&p) };
    let (b, max, buf) = (format!("{}", rate.round()), format!("{}", rate.round()), format!("{}", (rate * 0.5).round()));
    let encoders: [Vec<&str>; 2] = if preview {
        [vec!["-c:v", "h264_nvenc", "-preset", "p1", "-cq", "30", "-b:v", "0"], vec!["-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"]]
    } else {
        [
            // p4: the same quality as p5 here (SSIM 0.9779 against 0.9780)
            // at the same size, in two thirds of the time.
            vec!["-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr", "-b:v", &b, "-maxrate", &max, "-bufsize", &buf],
            vec!["-c:v", "libx264", "-preset", "veryfast", "-b:v", &b, "-maxrate", &max, "-bufsize", &buf],
        ]
    };
    // On an NVIDIA card: decoded there first, then as before if that fails.
    let attempts = encoders.iter().enumerate().flat_map(|(i, e)| {
        let gpu = i == 0 && nvidia;
        [(true, e), (false, e)].into_iter().filter(move |(g, _)| !g || gpu)
    });
    for (gpu_decode, encoder) in attempts {
        let mut argv: Vec<&OsStr> = vec![
            "ffmpeg".as_ref(), "-hide_banner".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(),
            "-i".as_ref(), video.as_os_str(),
            "-loop".as_ref(), "1".as_ref(), "-framerate".as_ref(), out_fps.as_ref(), "-i".as_ref(), bg.as_os_str(),
        ];
        if gpu_decode {
            // The recording decoded on the graphics card: a fraction of the CPU.
            argv.splice(5..5, ["-hwaccel".as_ref(), "cuda".as_ref()]);
        }
        argv.extend(overlays.inputs.iter().map(OsString::as_os_str));
        if cam.is_some() {
            argv.extend::<[&OsStr; 4]>(["-ss".as_ref(), cam_start.as_ref(), "-i".as_ref(), cam_file.as_os_str()]);
        }
        argv.extend::<[&OsStr; 2]>(["-/filter_complex".as_ref(), script.as_os_str()]);
        argv.extend::<[&OsStr; 2]>(["-map".as_ref(), "[out]".as_ref()]);
        match (audio, audio_graph.is_empty()) {
            (false, _) => argv.push("-an".as_ref()),
            (true, true) => argv.extend(["-map", "0:a?", "-c:a", "aac", "-b:a", "160k"].map(OsStr::new)),
            (true, false) => argv.extend(["-map", "[aout]", "-c:a", "aac", "-b:a", "160k"].map(OsStr::new)),
        }
        argv.extend(encoder.iter().map(OsStr::new));
        argv.extend(["-movflags", "+faststart"].map(OsStr::new));
        argv.push(mp4.as_os_str());
        result = job::ffmpeg(progress, &argv, length, "studio.log").await;
        if progress.cancelled() || result.is_ok() {
            break;
        }
    }
    if gif && result.is_ok() && !progress.cancelled() {
        let fps = gif_fps(project);
        let width = gif_size(&l).0;
        let filter = format!(
            "fps={fps},scale={width}:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle"
        );
        let mut argv: Vec<&OsStr> = ["ffmpeg", "-hide_banner", "-y", "-loglevel", "error", "-i"].map(OsStr::new).to_vec();
        argv.push(mp4.as_os_str());
        argv.extend(["-vf", &filter, "-loop", "0"].map(OsStr::new));
        argv.push(out.as_os_str());
        result = job::ffmpeg(progress, &argv, length, "studio-gif.log").await;
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if progress.cancelled() {
        let _ = std::fs::remove_file(out);
        return Ok(false);
    }
    result.map(|()| true)
}

/// Run blocking work off the UI thread.
async fn smol_unblock<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.await.expect("worker thread")
}

/// Click rings and shortcut pills, as extra ffmpeg inputs and filter chains.
mod overlays {
    use std::ffi::OsString;
    use std::fmt::Write as _;
    use std::path::Path;

    use super::{Layout, Probe};
    use crate::track::Track;

    /// More than this many of either would mostly slow the render down.
    const MAX: usize = 120;
    const RING_FRAMES: usize = 16;
    const RING_FPS: f64 = 30.;

    pub struct Overlays {
        pub inputs: Vec<OsString>,
        /// Chains over the recording, ending in label `src`.
        pub pre: String,
        pub src: String,
        /// Chains over the framed picture (label "framed"), ending in `last`.
        pub post: String,
        pub last: String,
    }

    /// The drawn cursor: its picture, hotspot and size, its path in video
    /// pixels, how it tilts and grows, and whether it leaves a blur trail.
    pub struct Cursor {
        /// Its pictures over time, as a concat list (see `cursor_frames`).
        pub list: std::path::PathBuf,
        pub hot: (f64, f64),
        pub size: (f64, f64),
        pub path: Vec<(f64, f64, f64)>,
        pub motion: Option<Vec<(f64, f64, f64)>>,
        pub trail: bool,
    }

    pub struct Style {
        pub clicks: bool,
        pub keys: bool,
        pub keys_top: bool,
        /// Pill text height as a share of the output's short side.
        pub keys_size: f64,
        pub cursor: Option<Cursor>,
        /// Filters first on the recording: the working rate, the cuts.
        pub first: String,
    }

    pub fn build(track: &Track, p: &Probe, l: &Layout, tmp: &Path, style: &Style) -> std::io::Result<Overlays> {
        let (clicks, keys) = (style.clicks, style.keys);
        let mut o = Overlays { inputs: Vec::new(), pre: String::new(), src: "0:v".into(), post: String::new(), last: "framed".into() };
        if !style.first.is_empty() {
            let _ = write!(o.pre, "[0:v]{}[ss];", style.first);
            o.src = "ss".into();
        }
        let mut next_input = 2;

        let (rx, ry, rw, rh) = track.rect;
        let (sx, sy) = (p.w / rw.max(1.), p.h / rh.max(1.));
        let clicks: Vec<(f64, f64, f64)> = track
            .clicks
            .iter()
            .map(|&(t, x, y)| (t, (x - rx) * sx, (y - ry) * sy))
            .filter(|&(t, x, y)| clicks && t >= 0. && t < p.duration && x >= 0. && y >= 0. && x < p.w && y < p.h)
            .take(MAX)
            .collect();
        if !clicks.is_empty() {
            let radius = (p.h * 0.03).clamp(16., 64.) as u32;
            write_ring(tmp, radius)?;
            let pattern = tmp.join("ring-%02d.png");
            for (i, &(t, x, y)) in clicks.iter().enumerate() {
                o.inputs.extend(["-framerate".into(), format!("{RING_FPS}").into(), "-itsoffset".into(), format!("{t:.3}").into(), "-i".into(), pattern.clone().into()]);
                let (from, to) = (o.src.clone(), format!("c{i}"));
                let half = radius as f64 + 4.;
                let _ = write!(o.pre, "[{from}][{next_input}:v]overlay=x={:.1}:y={:.1}:eof_action=pass[{to}];", x - half, y - half);
                o.src = to;
                next_input += 1;
            }
        }

        // The cursor goes over the click rings and moves with the camera.
        if let Some(c) = &style.cursor {
            let (hx, hy) = c.hot;
            let (w0, h0) = c.size;
            // A square box around the hotspot, big enough for the cursor at
            // its largest and turned: it grows and tilts around its tip.
            let reach = (hx.max(w0 - hx)).hypot(hy.max(h0 - hy)) * 1.35;
            let boxed = (2. * reach).ceil() as u32 / 2 * 2 + 2;
            let half = boxed as f64 / 2.;
            let position = |lag: f64| {
                let xs: Vec<(f64, f64)> = c.path.iter().map(|&(t, x, _)| (t + lag, x)).collect();
                let ys: Vec<(f64, f64)> = c.path.iter().map(|&(t, _, y)| (t + lag, y)).collect();
                (super::expr(&super::simplify(&xs, 0.6), 1), super::expr(&super::simplify(&ys, 0.6), 1))
            };
            let fps = &p.fps;
            o.inputs.extend(["-f".into(), "concat".into(), "-safe".into(), "0".into(), "-i".into(), c.list.clone().into()]);
            let shape = match &c.motion {
                Some(motion) => {
                    let angles: Vec<(f64, f64)> = motion.iter().map(|&(t, a, _)| (t, a)).collect();
                    let scales: Vec<(f64, f64)> = motion.iter().map(|&(t, _, s)| (t, s)).collect();
                    let (ae, se) = (super::expr(&super::simplify(&angles, 0.004), 4), super::expr(&super::simplify(&scales, 0.004), 4));
                    format!(
                        "fps={fps},format=rgba,scale=w='trunc({w0}*({se})/2)*2':h='trunc({h0}*({se})/2)*2':eval=frame,\
                         pad=w={boxed}:h={boxed}:x='{half}-{hx}*iw/{w0}':y='{half}-{hy}*ih/{h0}':color=black@0:eval=frame,\
                         rotate=a='{ae}':ow={boxed}:oh={boxed}:c=none"
                    )
                }
                None => format!("fps={fps},format=rgba,pad=w={boxed}:h={boxed}:x={:.1}:y={:.1}:color=black@0", half - hx, half - hy),
            };
            let from = o.src.clone();
            if c.trail {
                // A cursor-only motion blur: two faint copies a moment behind.
                // Within one frame, so the copies blur together instead of
                // reading as a second cursor.
                let lag = 1. / p.fps_value().max(1.) / 4.;
                let _ = write!(o.pre, "[{next_input}:v]{shape},split=3[cur0][cur1][cur2];[cur1]colorchannelmixer=aa=0.3[trail1];[cur2]colorchannelmixer=aa=0.15[trail2];");
                let mut base = from;
                for (i, (label, lag)) in [("trail2", 2. * lag), ("trail1", lag), ("cur0", 0.)].into_iter().enumerate() {
                    let (xe, ye) = position(lag);
                    let out = format!("curl{i}");
                    let _ = write!(o.pre, "[{base}][{label}]overlay=x='{xe}-{half}':y='{ye}-{half}':eval=frame:shortest=1[{out}];");
                    base = out;
                }
                o.src = base;
            } else {
                let (xe, ye) = position(0.);
                let _ = write!(o.pre, "[{next_input}:v]{shape}[curs];[{from}][curs]overlay=x='{xe}-{half}':y='{ye}-{half}':eval=frame:shortest=1[cur];");
                o.src = "cur".into();
            }
            next_input += 1;
        }
        if o.src != "0:v" && o.src != "ss" {
            // Overlay can hand on frames with alpha; libplacebo's crop path
            // asserts on those, so go back to plain YUV first.
            let _ = write!(o.pre, "[{}]format=yuv420p[clicked];", o.src);
            o.src = "clicked".into();
        }

        let strokes: Vec<&(f64, String)> = track.keystrokes.iter().filter(|(t, _)| keys && *t >= 0. && *t < p.duration).take(MAX).collect();
        for (i, (t, label)) in strokes.iter().enumerate() {
            // Shown until the next shortcut, at most 1.4 s.
            let next = strokes.get(i + 1).map_or(p.duration, |n| n.0);
            let show = (next - t).clamp(0.35, 1.4);
            let png = tmp.join(format!("key-{i}.png"));
            let (w, h) = write_pill(&png, label, l.ow.min(l.oh) as f32 * style.keys_size as f32)?;
            o.inputs.extend(["-loop".into(), "1".into(), "-framerate".into(), "30".into(), "-t".into(), format!("{show:.3}").into(), "-itsoffset".into(), format!("{t:.3}").into(), "-i".into(), png.into()]);
            let x = (l.ow as i64 - w as i64) / 2;
            let margin = (l.h as f64 * 0.05) as i64;
            let y = if style.keys_top { l.y as i64 + margin } else { (l.y + l.h) as i64 - h as i64 - margin };
            let (from, faded, to) = (o.last.clone(), format!("kf{i}"), format!("k{i}"));
            let _ = write!(
                o.post,
                "[{next_input}:v]format=rgba,fade=t=in:st={t:.3}:d=0.12:alpha=1,fade=t=out:st={:.3}:d=0.2:alpha=1[{faded}];[{from}][{faded}]overlay=x={x}:y={y}:eof_action=pass[{to}];",
                t + show - 0.2
            );
            o.last = to;
            next_input += 1;
        }
        Ok(o)
    }

    fn ease_out(u: f32) -> f32 {
        1. - (1. - u).powi(3)
    }

    /// A ring that expands and fades, as ring-00.png … ring-15.png.
    fn write_ring(dir: &Path, radius: u32) -> std::io::Result<()> {
        let r = radius as f32;
        let size = radius * 2 + 8;
        let c = size as f32 / 2.;
        for k in 0..RING_FRAMES {
            let u = k as f32 / (RING_FRAMES - 1) as f32;
            let ring = r * (0.3 + 0.7 * ease_out(u));
            let width = (r * 0.16 * (1. - 0.5 * u)).max(2.);
            let alpha = 0.9 * (1. - u).powf(1.3);
            let img = image::RgbaImage::from_fn(size, size, |x, y| {
                let d = ((x as f32 + 0.5 - c).hypot(y as f32 + 0.5 - c) - ring).abs();
                let core = (width / 2. - d + 0.5).clamp(0., 1.);
                // A soft dark halo keeps the ring visible on white pages.
                let halo = ((width / 2. + 2.5 - d) / 2.5).clamp(0., 1.) * 0.35;
                let a = (core + halo * (1. - core)) * alpha;
                let v = (255. * core / (core + halo * (1. - core)).max(1e-3)) as u8;
                image::Rgba([v, v, v, (a * 255.) as u8])
            });
            img.save(dir.join(format!("ring-{k:02}.png"))).map_err(std::io::Error::other)?;
        }
        Ok(())
    }

    /// "Ctrl + C" on a dark rounded pill; returns its size.
    fn write_pill(path: &Path, label: &str, size: f32) -> std::io::Result<(u32, u32)> {
        use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
        let m = kadr_raster::text::metrics(size);
        let pad = size * 0.75;
        let w = (kadr_raster::text::text_width(label, size) + 2. * pad).ceil();
        let h = (size * 1.9).ceil();
        let mut pm = Pixmap::new(w as u32, h as u32).ok_or_else(|| std::io::Error::other("pill size"))?;
        let r = h / 2.;
        let mut b = PathBuilder::new();
        b.move_to(r, 0.);
        b.line_to(w - r, 0.);
        b.quad_to(w, 0., w, r);
        b.quad_to(w, h, w - r, h);
        b.line_to(r, h);
        b.quad_to(0., h, 0., r);
        b.quad_to(0., 0., r, 0.);
        b.close();
        let mut paint = Paint::default();
        paint.set_color_rgba8(22, 22, 24, 220);
        paint.anti_alias = true;
        if let Some(path) = b.finish() {
            pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
        let baseline = (h - (m.ascent - m.descent)) / 2. + m.ascent;
        kadr_raster::text::draw_line(&mut pm, label, (pad, baseline), size, kadr_core::color::Rgba::WHITE);
        pm.save_png(path).map_err(std::io::Error::other)?;
        Ok((w as u32, h as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recording_and_its_edit_round_trip_through_a_kadr_file() {
        let tmp = std::env::temp_dir().join(format!("kadr-studio-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // SAFETY: only this test reads XDG_CACHE_HOME in this module's tests.
        unsafe { std::env::set_var("XDG_CACHE_HOME", tmp.join("cache")) };
        let video = tmp.join(format!("rec-{}.mp4", std::process::id()));
        std::fs::write(&video, vec![5u8; 3000]).unwrap();
        let wall = tmp.join("wall.png");
        std::fs::write(&wall, b"wall").unwrap();
        let track = track::path_for(&video);
        std::fs::create_dir_all(track.parent().unwrap()).unwrap();
        std::fs::write(&track, br#"{"clicks":[]}"#).unwrap();
        std::fs::write(camera_file(&video), b"cam").unwrap();
        let mut project = Project::load(&video, &Track::default(), &Probe { w: 100., h: 100., fps: "60/1".into(), duration: 10., audio: false });
        project.wallpaper = Some(wall.clone());
        project.radius = 0.25;
        let target = tmp.join("Demo.kadr");
        save_bundle(&video, &project, &target).unwrap();

        let u = kadr_core::bundle::unpack_into(&target, &tmp.join("opened")).unwrap();
        assert_eq!(u.kind, kadr_core::bundle::Kind::Studio);
        let back = from_bundle(&target, &u).unwrap();
        assert_ne!(back, video);
        assert_eq!(std::fs::read(&back).unwrap(), vec![5u8; 3000]);
        assert_eq!(std::fs::read(track::path_for(&back)).unwrap(), br#"{"clicks":[]}"#);
        assert_eq!(std::fs::read(camera_file(&back)).unwrap(), b"cam");
        let restored = Project::load(&back, &Track::default(), &Probe { w: 100., h: 100., fps: "60/1".into(), duration: 10., audio: false });
        assert_eq!(restored.radius, 0.25);
        let wall_back = restored.wallpaper.expect("the wallpaper came along");
        assert!(wall_back.starts_with(&u.dir) && std::fs::read(wall_back).unwrap() == b"wall");
        assert_eq!(output_path(&back, Format::Mp4), target.canonicalize().unwrap().with_file_name("Demo-studio.mp4"));
        // Opened again unchanged: the same video, nothing unpacked twice.
        let again = kadr_core::bundle::unpack_into(&target, &tmp.join("opened")).unwrap();
        assert_eq!(from_bundle(&target, &again).unwrap(), back);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn probe() -> Probe {
        Probe { w: 1920., h: 1080., fps: "30".into(), duration: 10., audio: false }
    }

    #[test]
    fn auto_segments_merge_close_clicks() {
        let track = Track { rect: (0., 0., 1920., 1080.), clicks: vec![(1.0, 100., 100.), (2.0, 200., 200.), (8.0, 300., 300.)], ..Default::default() };
        let project = Project::auto_with(&track, &probe(), None);
        assert_eq!(project.segments.len(), 2);
        assert_eq!((project.segments[0].start, project.segments[0].end), (1.0, 2.0 + HOLD));
        assert!(project.clicks && !project.keys);
    }

    #[test]
    fn playback_runs_through_cuts() {
        // 1.5 s in is half a second after the cut.
        assert_eq!(super::played_to_source(&[(0., 1.), (2., 3.)], 1.5), Some(2.5));
        assert_eq!(super::played_to_source(&[(0., 1.), (2., 3.)], 2.5), None);
    }

    #[test]
    fn trim_and_cuts_leave_the_rest() {
        let mut project = Project::auto_with(&Track::default(), &probe(), None);
        project.trim = (1., 9.);
        project.cuts = vec![(3., 4.), (0., 1.5), (8.5, 12.)];
        assert_eq!(project.kept(), vec![(1.5, 3.), (4., 8.5)]);
        assert!((project.kept_duration() - 6.).abs() < 1e-9);
        assert_eq!(project.next_kept(3.5), Some(4.));
        assert_eq!(project.next_kept(9.), None);
        let l = layout(&probe(), &Project { aspect: 4, padding: 0., ..project }, false);
        assert_eq!((l.ow, l.oh, l.w), (1080, 1920, 1080));
    }

    #[test]
    fn a_look_moves_between_projects_without_the_edit() {
        let mut source = Project::auto_with(&Track::default(), &probe(), None);
        source.padding = 0.02;
        source.aspect = 4;
        source.cursor_size = 2.5;
        source.cuts = vec![(1., 2.)];
        let mut target = Project::auto_with(&Track::default(), &probe(), None);
        target.trim = (1., 9.);
        let moved = target.with_look(&source.look(), &probe());
        assert_eq!((moved.padding, moved.aspect, moved.cursor_size), (0.02, 4, 2.5));
        assert_eq!(moved.trim, (1., 9.), "the edit stays");
        assert!(moved.cuts.is_empty(), "cuts aren't part of a look");
        assert_eq!(moved.look(), source.look());
    }

    #[test]
    fn long_curves_stay_shallow() {
        let curve: Vec<(f64, f64)> = (0..500).map(|i| (i as f64 * 0.1, (i % 7) as f64)).collect();
        let e = expr(&curve, 1);
        // Never more than a handful of nested parentheses, however long.
        let (mut depth, mut deepest) = (0i32, 0i32);
        for c in e.chars() {
            depth += match c { '(' => 1, ')' => -1, _ => 0 };
            deepest = deepest.max(depth);
        }
        assert!(deepest < 20, "nested {deepest} deep");
    }

    #[test]
    fn overlapping_cuts_join() {
        let mut project = Project::auto_with(&Track::default(), &probe(), None);
        project.cuts = vec![(5., 6.), (1., 2.), (1.5, 3.), (3., 4.), (8., 7.5)];
        project.merge_cuts();
        assert_eq!(project.cuts, vec![(1., 4.), (5., 6.), (7.5, 8.)]);
    }

    #[test]
    fn camera_zooms_and_holds_a_fixed_point() {
        let track = Track { rect: (0., 0., 1920., 1080.), cursor: vec![(0., 100., 100.)], ..Default::default() };
        let segs = vec![Seg { start: 3., end: 6., zoom: 2.2, focus: Some((1500., 800.)) }];
        let cam = Camera::new(&track, &probe(), &segs);
        let (z0, ..) = cam.at(0.5);
        assert!((z0 - 1.).abs() < 1e-6, "no zoom before the segment");
        let (z, x, y) = cam.at(5.5);
        assert!((z - 2.2).abs() < 1e-3, "full zoom inside: {z}");
        assert!((x - 1500.).abs() < 5. && (y - 800.).abs() < 5., "settled on the focus: {x},{y}");
        let (z9, ..) = cam.at(9.5);
        assert!((z9 - 1.).abs() < 1e-6, "eased back out after");
    }
}
