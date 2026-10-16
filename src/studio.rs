//! Studio render of a recording, like Screen Studio: the camera eases in on
//! zoom segments and follows the cursor (or holds a point), the video sits on
//! a gradient backdrop with a soft shadow and rounded corners, clicks get
//! rings and shortcuts a pill. Segments are edited in the studio editor and
//! kept as a per-recording project; the heavy lifting is ffmpeg's libplacebo
//! filter on the GPU, with the camera path handed over as crop expressions.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use gpui::{App, Hsla};
use serde_json::json;
use ui::IconName;

use crate::track::{self, Track};
use crate::job::{self, Job};
use crate::settings::Settings;
use crate::{cards, palette, util};

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
pub const ASPECTS: [(&str, Option<f64>); 5] = [("Как видео", None), ("16:9", Some(16. / 9.)), ("1:1", Some(1.)), ("4:5", Some(0.8)), ("9:16", Some(9. / 16.))];
/// Export size by the output's short side.
pub const RESOLUTIONS: [(&str, u32); 4] = [("720p", 720), ("1080p", 1080), ("1440p", 1440), ("4K", 2160)];
pub const FRAME_RATES: [(&str, Option<u32>); 3] = [("Как в записи", None), ("30", Some(30)), ("60", Some(60))];
/// Shortcut pill text height as a share of the output's short side.
pub const KEY_SIZES: [(&str, f64); 3] = [("S", 0.02), ("M", 0.026), ("L", 0.034)];
/// Slider ranges: padding (share of the short side), corner radius (pixels
/// at 1080p) and shadow strength.
pub const MAX_PADDING: f64 = 0.15;
pub const MAX_RADIUS: f64 = 60.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mp4,
    Gif,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub segments: Vec<Seg>,
    /// Index into `palette::backdrops()`, unless a wallpaper is set.
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
    /// Indexes into `ASPECTS`, `RESOLUTIONS` and `FRAME_RATES`.
    pub aspect: usize,
    pub resolution: usize,
    pub frame_rate: usize,
    pub format: Format,
}

impl Project {
    fn path(video: &Path) -> PathBuf {
        let mut p = track::path_for(video).into_os_string();
        p.push(".studio");
        PathBuf::from(p)
    }

    /// The saved edit, or segments placed on the recording's clicks.
    pub fn load(video: &Path, track: &Track, p: &Probe) -> Project {
        let saved = std::fs::read_to_string(Self::path(video)).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let d = Project::auto(track, p);
        let Some(v) = saved else { return d };
        let segments = v["segments"]
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
            .collect();
        let pair = |x: &serde_json::Value| Some((x.get(0)?.as_f64()?, x.get(1)?.as_f64()?));
        let index = |key: &str, default: usize, len: usize| (v[key].as_u64().map_or(default, |i| i as usize)).min(len - 1);
        // Older projects kept padding as an index into PADDINGS.
        let padding = v["pad"].as_f64().or_else(|| v["padding"].as_u64().map(|i| PADDINGS[(i as usize).min(PADDINGS.len() - 1)])).unwrap_or(d.padding);
        let trim = pair(&v["trim"]).map(|(a, b)| (a.clamp(0., p.duration), b.clamp(0., p.duration))).filter(|(a, b)| b - a > 0.1).unwrap_or(d.trim);
        Project {
            segments,
            backdrop: v["backdrop"].as_u64().unwrap_or(0) as usize,
            wallpaper: v["wallpaper"].as_str().map(PathBuf::from).filter(|w| w.exists()),
            padding: padding.clamp(0., MAX_PADDING),
            radius: v["radius"].as_f64().unwrap_or(d.radius).clamp(0., MAX_RADIUS),
            shadow: v["shadow"].as_f64().unwrap_or(d.shadow).clamp(0., 1.),
            clicks: v["clicks"].as_bool().unwrap_or(true),
            keys: v["keys"].as_bool().unwrap_or(true),
            keys_top: v["keys_top"].as_bool().unwrap_or(false),
            keys_size: index("keys_size", 1, KEY_SIZES.len()),
            trim,
            cuts: v["cuts"].as_array().into_iter().flatten().filter_map(pair).collect(),
            audio: v["audio"].as_bool().unwrap_or(true),
            aspect: index("aspect", 0, ASPECTS.len()),
            resolution: index("resolution", d.resolution, RESOLUTIONS.len()),
            frame_rate: index("frame_rate", 0, FRAME_RATES.len()),
            format: if v["format"].as_str() == Some("gif") { Format::Gif } else { Format::Mp4 },
        }
    }

    pub fn save(&self, video: &Path) {
        let segments: Vec<_> = self
            .segments
            .iter()
            .map(|s| json!({ "start": s.start, "end": s.end, "zoom": s.zoom, "focus": s.focus.map(|(x, y)| [x, y]) }))
            .collect();
        let v = json!({
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
            "aspect": self.aspect,
            "resolution": self.resolution,
            "frame_rate": self.frame_rate,
            "format": if self.format == Format::Gif { "gif" } else { "mp4" },
        });
        let path = Self::path(video);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, v.to_string());
    }

    /// Zoom on clicks: a stretch per burst of clicks, held while typing.
    pub fn auto(track: &Track, p: &Probe) -> Project {
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
        Project {
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
            aspect: 0,
            // 1440p for big recordings, like before.
            resolution: if p.h >= 1400.0 || p.w >= 2500.0 { 2 } else { 1 },
            frame_rate: 0,
            format: Format::Mp4,
        }
    }

    pub fn colors(&self) -> (Hsla, Hsla) {
        let all = palette::backdrops();
        all[self.backdrop.min(all.len() - 1)]
    }

    /// Width over height of the output.
    pub fn aspect_ratio(&self, p: &Probe) -> f64 {
        ASPECTS[self.aspect.min(ASPECTS.len() - 1)].1.unwrap_or(p.w / p.h.max(1.))
    }

    /// What's left after the trim and the cuts, in source time.
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

    pub fn kept_duration(&self) -> f64 {
        self.kept().iter().map(|(a, b)| b - a).sum()
    }

    /// Where playback goes from `t`: itself when kept, the end of the cut it
    /// falls into, or None past the end.
    pub fn next_kept(&self, t: f64) -> Option<f64> {
        self.kept().into_iter().find(|&(_, b)| t < b - 0.02).map(|(a, _)| t.max(a))
    }
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub w: f64,
    pub h: f64,
    pub fps: String,
    pub duration: f64,
    pub audio: bool,
}

pub async fn probe(video: &Path) -> Option<Probe> {
    let (ok, out) = util::output(&[
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

/// Piecewise-linear curve as an ffmpeg expression in `t`: a sum of clipped ramps.
fn expr(curve: &[(f64, f64)], precision: usize) -> String {
    let Some(&(_, v0)) = curve.first() else { return "0".into() };
    let mut out = format!("{v0:.precision$}");
    for w in curve.windows(2) {
        let ((t0, a), (t1, b)) = (w[0], w[1]);
        let delta = b - a;
        if delta.abs() < 10f64.powi(-(precision as i32)) || t1 <= t0 {
            continue;
        }
        out += &format!("+{delta:.precision$}*clip((t-{t0:.3})/{:.3},0,1)", t1 - t0);
    }
    out
}

#[derive(Clone)]
pub(crate) struct Layout {
    pub(crate) ow: u32,
    pub(crate) oh: u32,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) radius: f64,
}

/// Output frame and where the video sits in it. `preview` renders small.
pub(crate) fn layout(p: &Probe, project: &Project, preview: bool) -> Layout {
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
    let rgb = |c: Hsla| {
        let c = c.to_rgb();
        [c.r as f64 * 255., c.g as f64 * 255., c.b as f64 * 255.]
    };
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
    let stem = video.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    video.with_file_name(format!("{stem}-studio.{}", if format == Format::Gif { "gif" } else { "mp4" }))
}

fn preview_path(video: &Path) -> PathBuf {
    util::runtime_dir().join(format!("kadr-preview-{}.mp4", video.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()))
}

/// Export with the saved project (or automatic zooms when there's none).
pub fn render(video: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        let Some(p) = probe(&video).await else {
            return util::notify("Не удалось открыть видео", &video.display().to_string());
        };
        let track = Track::load(&video).unwrap_or_else(|| Track { rect: (0.0, 0.0, p.w, p.h), ..Default::default() });
        let project = Project::load(&video, &track, &p);
        let _ = cx.update(|cx| export(video, project, false, cx));
    })
    .detach();
}

/// Render `project`; the full export lands in a card, the quick preview
/// (720p) opens in the player.
pub fn export(video: PathBuf, project: Project, preview: bool, cx: &mut App) {
    let title = if preview { "Готовлю предпросмотр" } else { "Экспортирую видео" };
    let running = job::start(title, if preview { IconName::PlayFilled } else { IconName::Sparkle }, cx);
    cx.spawn(async move |cx| {
        let out = if preview { preview_path(&video) } else { output_path(&video, project.format) };
        let result = run(&video, &project, &out, preview, &running.job).await;
        running.finish(cx);
        match result {
            Ok(true) if preview => {
                util::spawn(&[OsStr::new("mpv"), "--force-window=yes".as_ref(), "--loop-file=inf".as_ref(), out.as_os_str()]);
            }
            Ok(true) => {
                let _ = cx.update(|cx| cards::show(out, cx));
            }
            Ok(false) => {}
            Err(e) => util::notify("Не удалось собрать видео", &e),
        }
    })
    .detach();
}

/// `select` expression keeping the given stretches.
fn keep_expr(kept: &[(f64, f64)]) -> String {
    kept.iter().map(|(a, b)| format!("between(t,{a:.3},{b:.3})")).collect::<Vec<_>>().join("+")
}

/// Ok(false) when cancelled.
async fn run(video: &Path, project: &Project, out: &Path, preview: bool, progress: &Job) -> Result<bool, String> {
    let p = probe(video).await.ok_or("ffprobe не смог прочитать видео")?;
    let track = Track::load(video).unwrap_or_else(|| Track { rect: (0.0, 0.0, p.w, p.h), ..Default::default() });
    let l = layout(&p, project, preview);
    let camera = Camera::new(&track, &p, &project.segments);
    let (z, x, y) = (simplify(&camera.zoom, 0.004), simplify(&camera.x, 2.0), simplify(&camera.y, 2.0));
    let kept = project.kept();
    if kept.is_empty() {
        return Err("После обрезки ничего не осталось".into());
    }
    let whole = kept.len() == 1 && kept[0].0 <= 0.01 && kept[0].1 >= p.duration - 0.01;
    let length = project.kept_duration();

    let tmp = util::runtime_dir().join(format!("kadr-studio-{}", std::process::id()));
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
    let style = overlays::Style { clicks: project.clicks, keys: project.keys, keys_top: project.keys_top, keys_size: KEY_SIZES[project.keys_size.min(KEY_SIZES.len() - 1)].1 };
    let overlays = overlays::build(&track, &p, &l, &tmp, &style).map_err(|e| e.to_string())?;

    // Trim and cuts drop frames at the end and close the gaps; a fixed
    // export rate goes last.
    let mut tail = String::new();
    if !whole {
        tail += &format!(",select='{}',setpts=N/({})/TB", keep_expr(&kept), p.fps);
    }
    if let Some(fps) = FRAME_RATES[project.frame_rate.min(FRAME_RATES.len() - 1)].1 {
        tail += &format!(",fps={fps}");
    }
    let audio = p.audio && project.audio && project.format == Format::Mp4;
    let audio_graph = if audio && !whole { format!(";[0:a]aselect='{}',asetpts=N/SR/TB[aout]", keep_expr(&kept)) } else { String::new() };

    // libplacebo asserts on some fractional crop sizes, so the crop is kept to
    // whole, even pixels.
    // libplacebo measures corner rounding against a quarter of the shorter side.
    let rounding = (4.0 * l.radius / l.w.min(l.h) as f64).clamp(0.0, 1.0);
    let zexpr = expr(&z, 4);
    let graph = format!(
        "{pre}[{src}]libplacebo=w={ow}:h={oh}:pos_x={px}:pos_y={py}:pos_w={pw}:pos_h={ph}\
         :crop_w='2*floor(iw/({zexpr})/2)':crop_h='2*floor(ih/({zexpr})/2)'\
         :crop_x='floor(clip({xe}-cw/2,0,iw-cw))':crop_y='floor(clip({ye}-ch/2,0,ih-ch))'\
         :corner_rounding={rounding:.4}:fillcolor=black@0:format=rgba[fg];\
         [1:v][fg]overlay=0:0:shortest=1[framed];{post}[{last}]format=yuv420p{tail}[out]{audio_graph}",
        pre = overlays.pre, src = overlays.src, post = overlays.post, last = overlays.last,
        ow = l.ow, oh = l.oh, px = l.x, py = l.y, pw = l.w, ph = l.h,
        xe = expr(&x, 1), ye = expr(&y, 1),
    );
    std::fs::write(&script, graph).map_err(|e| e.to_string())?;

    // A GIF is made from an intermediate MP4.
    let gif = project.format == Format::Gif && !preview;
    let mp4 = if gif { tmp.join("studio.mp4") } else { out.to_path_buf() };
    let mut result = Err(String::new());
    let encoders: [&[&str]; 2] = if preview {
        [&["-c:v", "h264_nvenc", "-preset", "p1", "-cq", "30", "-b:v", "0"], &["-c:v", "libx264", "-preset", "ultrafast", "-crf", "30"]]
    } else {
        [&["-c:v", "h264_nvenc", "-preset", "p5", "-cq", "19", "-b:v", "0"], &["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"]]
    };
    for encoder in encoders {
        let mut argv: Vec<&OsStr> = vec![
            "ffmpeg".as_ref(), "-hide_banner".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(),
            "-i".as_ref(), video.as_os_str(),
            "-loop".as_ref(), "1".as_ref(), "-framerate".as_ref(), p.fps.as_ref(), "-i".as_ref(), bg.as_os_str(),
        ];
        argv.extend(overlays.inputs.iter().map(OsString::as_os_str));
        argv.extend::<[&OsStr; 2]>(["-/filter_complex".as_ref(), script.as_os_str()]);
        argv.extend::<[&OsStr; 2]>(["-map".as_ref(), "[out]".as_ref()]);
        match (audio, whole) {
            (false, _) => argv.push("-an".as_ref()),
            (true, true) => argv.extend(["-map", "0:a?", "-c:a", "copy"].map(OsStr::new)),
            (true, false) => argv.extend(["-map", "[aout]", "-c:a", "aac", "-b:a", "192k"].map(OsStr::new)),
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
        let fps = FRAME_RATES[project.frame_rate].1.unwrap_or(15).min(30);
        let width = l.ow.min(1080);
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
    use crate::editor;
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

    pub struct Style {
        pub clicks: bool,
        pub keys: bool,
        pub keys_top: bool,
        /// Pill text height as a share of the output's short side.
        pub keys_size: f64,
    }

    pub fn build(track: &Track, p: &Probe, l: &Layout, tmp: &Path, style: &Style) -> std::io::Result<Overlays> {
        let (clicks, keys) = (style.clicks, style.keys);
        let mut o = Overlays { inputs: Vec::new(), pre: String::new(), src: "0:v".into(), post: String::new(), last: "framed".into() };
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
        let m = editor::metrics(size);
        let pad = size * 0.75;
        let w = (editor::text_width(label, size) + 2. * pad).ceil();
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
        editor::draw_line(&mut pm, label, (pad, baseline), size, gpui::white());
        pm.save_png(path).map_err(std::io::Error::other)?;
        Ok((w as u32, h as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe() -> Probe {
        Probe { w: 1920., h: 1080., fps: "30".into(), duration: 10., audio: false }
    }

    #[test]
    fn auto_segments_merge_close_clicks() {
        let track = Track { rect: (0., 0., 1920., 1080.), clicks: vec![(1.0, 100., 100.), (2.0, 200., 200.), (8.0, 300., 300.)], ..Default::default() };
        let project = Project::auto(&track, &probe());
        assert_eq!(project.segments.len(), 2);
        assert_eq!((project.segments[0].start, project.segments[0].end), (1.0, 2.0 + HOLD));
        assert!(project.clicks && !project.keys);
    }

    #[test]
    fn trim_and_cuts_leave_the_rest() {
        let mut project = Project::auto(&Track::default(), &probe());
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
