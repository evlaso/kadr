//! "Studio" render of a recording: the camera eases in towards clicks and
//! follows the cursor, the video sits on a gradient backdrop with a soft
//! shadow and rounded corners. The heavy lifting is ffmpeg's libplacebo
//! filter on the GPU; kadr computes the camera path and hands it over as
//! time-based crop expressions.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use gpui::App;
use ui::IconName;

use crate::track::Track;
use crate::job::{self, Job};
use crate::{cards, util};

/// Seconds to ease the zoom in or out.
const RAMP: f64 = 0.8;
/// How long the zoom lingers after the last click or key press.
const HOLD: f64 = 1.6;
/// Clicks closer than this stay in one zoomed-in stretch.
const GAP: f64 = 3.5;
/// Camera path sampling rate before simplification.
const RATE: f64 = 30.0;

struct Settings {
    zoom: f64,
    /// Margin around the video, as a fraction of the output height.
    padding: f64,
    /// Corner radius in pixels at 1440p.
    radius: f64,
    colors: [(u8, u8, u8); 2],
}

impl Settings {
    /// ~/.config/kadr/studio.json, e.g. {"zoom": 2.0, "colors": ["#1e3c72", "#2a5298"]}
    fn load() -> Self {
        let mut s = Settings { zoom: 1.8, padding: 0.07, radius: 22.0, colors: [(0x5b, 0x4b, 0xff), (0xe2, 0x6f, 0xd0)] };
        let path = util::config_dir().join("studio.json");
        let Some(v) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else {
            return s;
        };
        if let Some(z) = v["zoom"].as_f64() {
            s.zoom = z.clamp(1.0, 4.0);
        }
        if let Some(p) = v["padding"].as_f64() {
            s.padding = p.clamp(0.0, 0.3);
        }
        if let Some(r) = v["radius"].as_f64() {
            s.radius = r.max(0.0);
        }
        if let Some(c) = v["colors"].as_array() {
            for (slot, hex) in s.colors.iter_mut().zip(c.iter().filter_map(|h| h.as_str())) {
                if let Some(rgb) = parse_hex(hex) {
                    *slot = rgb;
                }
            }
        }
        s
    }
}

fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim_start_matches('#');
    let n = u32::from_str_radix(h, 16).ok().filter(|_| h.len() == 6)?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

pub(crate) struct Probe {
    w: f64,
    h: f64,
    fps: String,
    duration: f64,
}

async fn probe(video: &Path) -> Option<Probe> {
    let (ok, out) = util::output(&[
        OsStr::new("ffprobe"), "-v".as_ref(), "error".as_ref(), "-select_streams".as_ref(), "v:0".as_ref(),
        "-show_entries".as_ref(), "stream=width,height,r_frame_rate:format=duration".as_ref(),
        "-of".as_ref(), "json".as_ref(), video.as_os_str(),
    ])
    .await?;
    let v: serde_json::Value = serde_json::from_str(&out).ok().filter(|_| ok)?;
    let s = &v["streams"][0];
    Some(Probe {
        w: s["width"].as_f64()?,
        h: s["height"].as_f64()?,
        fps: s["r_frame_rate"].as_str().unwrap_or("60").to_owned(),
        duration: v["format"]["duration"].as_str()?.parse().ok()?,
    })
}

/// Zoomed-in stretches as (first event, last event) times.
fn segments(track: &Track, in_frame: impl Fn(f64, f64) -> bool) -> Vec<(f64, f64)> {
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
    segs
}

fn ease(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    if x < 0.5 { 4.0 * x * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 }
}

/// 0 = full view, 1 = fully zoomed in.
fn envelope(segs: &[(f64, f64)], t: f64) -> f64 {
    segs.iter()
        .map(|&(s, e)| {
            let rise = ease((t - (s - RAMP)) / RAMP);
            let fall = 1.0 - ease((t - (e + HOLD)) / RAMP);
            rise.min(fall)
        })
        .fold(0.0, f64::max)
}

type Curve = Vec<(f64, f64)>;

/// Zoom factor and camera center (video pixels) over time.
fn camera(track: &Track, p: &Probe, zoom: f64) -> (Curve, Curve, Curve) {
    let (rx, ry, rw, rh) = track.rect;
    let (sx, sy) = (p.w / rw.max(1.0), p.h / rh.max(1.0));
    let to_px = |x: f64, y: f64| ((x - rx) * sx, (y - ry) * sy);
    let segs = segments(track, |x, y| {
        let (px, py) = to_px(x, y);
        px >= 0.0 && py >= 0.0 && px < p.w && py < p.h
    });

    let (mut z, mut cx, mut cy) = (Vec::new(), Vec::new(), Vec::new());
    let mut pos = (p.w / 2.0, p.h / 2.0);
    let mut vel = (0.0, 0.0);
    let dt = 1.0 / RATE;
    // Critically damped spring: smooth, no overshoot, settles in ~0.6 s.
    let omega = 7.0;
    let steps = (p.duration * RATE).ceil() as usize;
    for i in 0..=steps {
        let t = i as f64 * dt;
        let env = envelope(&segs, t);
        let target = track.cursor_at(t).map(|(x, y)| to_px(x, y)).unwrap_or(pos);
        let target = (target.0.clamp(0.0, p.w), target.1.clamp(0.0, p.h));
        if env < 0.01 {
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
        z.push((t, 1.0 + (zoom - 1.0) * env));
        cx.push((t, pos.0));
        cy.push((t, pos.1));
    }
    (simplify(&z, 0.004), simplify(&cx, 2.0), simplify(&cy, 2.0))
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

pub(crate) struct Layout {
    ow: u32,
    oh: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    radius: f64,
}

fn layout(p: &Probe, s: &Settings) -> Layout {
    let (ow, oh) = if p.h >= 1400.0 || p.w >= 2500.0 { (2560u32, 1440u32) } else { (1920, 1080) };
    let pad = s.padding * oh as f64;
    let k = ((ow as f64 - 2.0 * pad) / p.w).min((oh as f64 - 2.0 * pad) / p.h);
    let even = |v: f64| ((v / 2.0).round() * 2.0) as u32;
    let (w, h) = (even(p.w * k), even(p.h * k));
    Layout { ow, oh, x: even((ow - w) as f64 / 2.0), y: even((oh - h) as f64 / 2.0), w, h, radius: s.radius * oh as f64 / 1440.0 }
}

/// Gradient backdrop with the video's drop shadow baked in, as a binary PPM.
fn write_background(path: &Path, l: &Layout, s: &Settings) -> std::io::Result<()> {
    let [c0, c1] = s.colors.map(|(r, g, b)| [r as f64, g as f64, b as f64]);
    let blur = 0.035 * l.oh as f64;
    let (cx, cy) = (l.x as f64 + l.w as f64 / 2.0, l.y as f64 + l.h as f64 / 2.0 + 0.012 * l.oh as f64);
    let (hw, hh) = (l.w as f64 / 2.0, l.h as f64 / 2.0);
    let r = l.radius;
    let mut buf = Vec::with_capacity((l.ow * l.oh * 3) as usize + 32);
    write!(buf, "P6\n{} {}\n255\n", l.ow, l.oh)?;
    for y in 0..l.oh {
        for x in 0..l.ow {
            let (fx, fy) = (x as f64, y as f64);
            let g = ((fx / l.ow as f64) * 0.6 + (fy / l.oh as f64) * 0.4).clamp(0.0, 1.0);
            // Signed distance to the shadow's rounded rectangle.
            let qx = (fx - cx).abs() - (hw - r);
            let qy = (fy - cy).abs() - (hh - r);
            let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
            let shade = 1.0 - 0.5 * (1.0 - smoothstep(-blur * 0.4, blur, d));
            for ch in 0..3 {
                let v = (c0[ch] + (c1[ch] - c0[ch]) * g) * shade;
                buf.push(v.round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    std::fs::write(path, buf)
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let k = ((x - a) / (b - a)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

pub fn output_path(video: &Path) -> PathBuf {
    let stem = video.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    video.with_file_name(format!("{stem}-studio.mp4"))
}

pub fn render(video: PathBuf, cx: &mut App) {
    let running = job::start("Улучшаю видео", IconName::Sparkle, cx);
    cx.spawn(async move |cx| {
        let result = run(&video, &running.job).await;
        running.finish(cx);
        let _ = cx.update(|cx| match result {
            Ok(Some(out)) => cards::show(out, cx),
            Ok(None) => {}
            Err(e) => util::notify("Не удалось улучшить видео", &e),
        });
    })
    .detach();
}

/// Ok(None) when cancelled.
async fn run(video: &Path, progress: &Job) -> Result<Option<PathBuf>, String> {
    let settings = Settings::load();
    let p = probe(video).await.ok_or("ffprobe не смог прочитать видео")?;
    let track = Track::load(video).unwrap_or_else(|| Track { rect: (0.0, 0.0, p.w, p.h), ..Default::default() });
    let l = layout(&p, &settings);
    let (z, x, y) = camera(&track, &p, settings.zoom);

    let tmp = util::runtime_dir().join(format!("kadr-studio-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let bg = tmp.join("bg.ppm");
    let script = tmp.join("filter.txt");
    write_background(&bg, &l, &settings).map_err(|e| e.to_string())?;

    // Click rings go on the recording itself (so they zoom with the camera),
    // shortcut pills on the finished frame. Inputs 0 and 1 are the video and
    // the backdrop; the overlays follow.
    let overlays = overlays::build(&track, &p, &l, &tmp).map_err(|e| e.to_string())?;

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
         [1:v][fg]overlay=0:0:shortest=1[framed];{post}[{last}]format=yuv420p[out]",
        pre = overlays.pre, src = overlays.src, post = overlays.post, last = overlays.last,
        ow = l.ow, oh = l.oh, px = l.x, py = l.y, pw = l.w, ph = l.h,
        xe = expr(&x, 1), ye = expr(&y, 1),
    );
    std::fs::write(&script, graph).map_err(|e| e.to_string())?;

    let out = output_path(video);
    let mut result = Err(String::new());
    for encoder in [&["-c:v", "h264_nvenc", "-preset", "p5", "-cq", "19", "-b:v", "0"][..], &["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"][..]] {
        let mut argv: Vec<&OsStr> = vec![
            "ffmpeg".as_ref(), "-hide_banner".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(),
            "-i".as_ref(), video.as_os_str(),
            "-loop".as_ref(), "1".as_ref(), "-framerate".as_ref(), p.fps.as_ref(), "-i".as_ref(), bg.as_os_str(),
        ];
        argv.extend(overlays.inputs.iter().map(OsString::as_os_str));
        argv.extend::<[&OsStr; 2]>([
            "-/filter_complex".as_ref(), script.as_os_str(),
        ]);
        argv.extend::<[&OsStr; 4]>(["-map".as_ref(), "[out]".as_ref(), "-map".as_ref(), "0:a?".as_ref()]);
        argv.extend(encoder.iter().map(OsStr::new));
        argv.extend(["-c:a", "copy", "-movflags", "+faststart"].map(OsStr::new));
        argv.push(out.as_os_str());
        result = job::ffmpeg(progress, &argv, p.duration, "studio.log").await;
        if progress.cancelled() || result.is_ok() {
            break;
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if progress.cancelled() {
        let _ = std::fs::remove_file(&out);
        return Ok(None);
    }
    result.map(|()| Some(out))
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

    pub fn build(track: &Track, p: &Probe, l: &Layout, tmp: &Path) -> std::io::Result<Overlays> {
        let mut o = Overlays { inputs: Vec::new(), pre: String::new(), src: "0:v".into(), post: String::new(), last: "framed".into() };
        let mut next_input = 2;

        let (rx, ry, rw, rh) = track.rect;
        let (sx, sy) = (p.w / rw.max(1.), p.h / rh.max(1.));
        let clicks: Vec<(f64, f64, f64)> = track
            .clicks
            .iter()
            .map(|&(t, x, y)| (t, (x - rx) * sx, (y - ry) * sy))
            .filter(|&(t, x, y)| t >= 0. && t < p.duration && x >= 0. && y >= 0. && x < p.w && y < p.h)
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

        let strokes: Vec<&(f64, String)> = track.keystrokes.iter().filter(|(t, _)| *t >= 0. && *t < p.duration).take(MAX).collect();
        for (i, (t, label)) in strokes.iter().enumerate() {
            // Shown until the next shortcut, at most 1.4 s.
            let next = strokes.get(i + 1).map_or(p.duration, |n| n.0);
            let show = (next - t).clamp(0.35, 1.4);
            let png = tmp.join(format!("key-{i}.png"));
            let (w, h) = write_pill(&png, label, l.oh as f32 * 0.026)?;
            o.inputs.extend(["-loop".into(), "1".into(), "-framerate".into(), "30".into(), "-t".into(), format!("{show:.3}").into(), "-itsoffset".into(), format!("{t:.3}").into(), "-i".into(), png.into()]);
            let x = (l.ow as i64 - w as i64) / 2;
            let y = (l.y + l.h) as i64 - h as i64 - (l.h as f64 * 0.05) as i64;
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
