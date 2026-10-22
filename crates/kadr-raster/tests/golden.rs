//! Golden tests of the editor's rasteriser: each case in
//! `tests/golden/cases/<name>/` is a project as `.kadr` keeps it
//! (`base.png`, `project.json`) plus `expected.png`, what Save made of it.
//! A case passes when the rendering matches closely (SSIM and a cap on
//! clearly changed pixels); blurring and pixelating must match exactly.
//!
//! A failure leaves `target/golden/<name>/{actual,diff}.png`.
//! `UPDATE_GOLDEN=1 just golden` rewrites the expected pictures (say
//! "golden updated: <cases>" in the commit), and `just golden-cases` writes
//! the cases themselves from `cases()` below.

use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use kadr_core::color::Rgba as Color;
use kadr_core::document::{Backdrop, Doc, Frame, Hide, Kind, Orientation, P, Shape, TextStyle};
use kadr_core::project;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn root() -> PathBuf {
    workspace().join("tests/golden/cases")
}

fn out_dir(name: &str) -> PathBuf {
    workspace().join("target/golden").join(name)
}

/// What Save writes for a case: the picture turned as the drawing says, then
/// drawn on.
fn render(dir: &Path) -> RgbaImage {
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("project.json")).expect("project.json")).expect("valid json");
    let (doc, frame) = project::from_json(&json, &|name| image::open(dir.join(name)).ok().map(|i| i.to_rgba8())).unwrap_or_else(|| panic!("{}: not a project", dir.display()));
    let base = image::open(dir.join("base.png")).expect("base.png").to_rgba8();
    kadr_raster::render(&base, &doc, &frame).expect("render")
}

/// Mean SSIM of the two pictures' luma (alpha-weighted over black), on 8×8
/// windows.
fn ssim(a: &RgbaImage, b: &RgbaImage) -> f64 {
    let luma = |p: &Rgba<u8>| {
        let [r, g, bl, al] = p.0.map(|c| c as f64);
        (0.299 * r + 0.587 * g + 0.114 * bl) * al / 255.
    };
    let (w, h) = a.dimensions();
    let (c1, c2) = ((0.01f64 * 255.).powi(2), (0.03f64 * 255.).powi(2));
    let (mut sum, mut n) = (0., 0);
    for by in (0..h.saturating_sub(7)).step_by(8) {
        for bx in (0..w.saturating_sub(7)).step_by(8) {
            let (mut ma, mut mb, mut va, mut vb, mut cov) = (0., 0., 0., 0., 0.);
            for y in by..by + 8 {
                for x in bx..bx + 8 {
                    let (la, lb) = (luma(a.get_pixel(x, y)), luma(b.get_pixel(x, y)));
                    ma += la;
                    mb += lb;
                    va += la * la;
                    vb += lb * lb;
                    cov += la * lb;
                }
            }
            let k = 64.;
            let (ma, mb) = (ma / k, mb / k);
            let (va, vb, cov) = (va / k - ma * ma, vb / k - mb * mb, cov / k - ma * mb);
            sum += ((2. * ma * mb + c1) * (2. * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            n += 1;
        }
    }
    if n == 0 { 1. } else { sum / n as f64 }
}

/// Pixels where some channel is off by more than 8, and the picture marking
/// them red over a faded copy of the expected one.
fn changed(expected: &RgbaImage, actual: &RgbaImage) -> (usize, RgbaImage) {
    let mut count = 0;
    let diff = RgbaImage::from_fn(expected.width(), expected.height(), |x, y| {
        let (e, a) = (expected.get_pixel(x, y).0, actual.get_pixel(x, y).0);
        if (0..4).any(|i| (e[i] as i32 - a[i] as i32).abs() > 8) {
            count += 1;
            Rgba([255, 0, 0, 255])
        } else {
            let l = ((e[0] as u32 + e[1] as u32 + e[2] as u32) / 3 * e[3] as u32 / 255) as u8;
            Rgba([l / 3 + 170, l / 3 + 170, l / 3 + 170, 255])
        }
    });
    (count, diff)
}

/// Cases where hiding is involved must come out the same to the byte.
fn exact(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("project.json")).is_ok_and(|s| s.contains("\"blur\"") || s.contains("\"pixelate\""))
}

#[test]
fn golden() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root()).expect("tests/golden/cases").filter_map(|e| Some(e.ok()?.path())).filter(|p| p.join("project.json").exists()).collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no golden cases");
    let mut failed = Vec::new();
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let actual = render(dir);
        let expected_path = dir.join("expected.png");
        if update {
            actual.save(&expected_path).unwrap();
            continue;
        }
        let Ok(expected) = image::open(&expected_path).map(|i| i.to_rgba8()) else {
            failed.push(format!("{name}: no expected.png (run with UPDATE_GOLDEN=1)"));
            continue;
        };
        let verdict = if expected.dimensions() != actual.dimensions() {
            Some(format!("size {:?}, expected {:?}", actual.dimensions(), expected.dimensions()))
        } else if exact(dir) {
            (expected.as_raw() != actual.as_raw()).then(|| "differs (blur and pixelate must match exactly)".to_owned())
        } else {
            let (count, _) = changed(&expected, &actual);
            let cap = (expected.width() * expected.height()) as usize / 1000;
            let s = ssim(&expected, &actual);
            (s < 0.995 || count > cap).then(|| format!("ssim {s:.4} (min 0.995), {count} pixels off by more than 8 (max {cap})"))
        };
        if let Some(why) = verdict {
            let out = out_dir(&name);
            let _ = std::fs::create_dir_all(&out);
            let _ = actual.save(out.join("actual.png"));
            if expected.dimensions() == actual.dimensions() {
                let _ = changed(&expected, &actual).1.save(out.join("diff.png"));
            }
            failed.push(format!("{name}: {why} → {}", out.display()));
        }
    }
    assert!(failed.is_empty(), "golden cases failed:\n{}", failed.join("\n"));
}

/// A made-up screenshot: a light window with a title bar, text-like bars, a
/// photo-like patch with fine detail (for blurring), and a dark sidebar.
fn base(w: u32, h: u32) -> RgbaImage {
    let s = w as f32 / 480.;
    RgbaImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / s, y as f32 / s);
        if fy < 28. {
            return Rgba([232, 232, 236, 255]);
        }
        if fx < 110. {
            let line = (fy as u32 / 22) % 2 == 0 && (fx as u32 % 110) > 14 && (fx as u32) < 90 && (fy as u32 % 22) > 8 && (fy as u32 % 22) < 14;
            return if line { Rgba([150, 150, 160, 255]) } else { Rgba([36, 36, 42, 255]) };
        }
        if (260. ..440.).contains(&fx) && (150. ..270.).contains(&fy) {
            let v = (((fx * 0.9).sin() * (fy * 0.7).cos() + 1.) * 60.) as u8;
            return Rgba([40 + v, 90 + v / 2, 160 - v / 2, 255]);
        }
        let text = (fy as u32 % 18) > 6 && (fy as u32 % 18) < 11 && fx > 130. && fx < 130. + 300. - ((fy as u32 / 18) % 4) as f32 * 50.;
        if text && fy < 140. { Rgba([60, 60, 68, 255]) } else { Rgba([250, 250, 252, 255]) }
    })
}

fn color(h: f32, s: f32, l: f32) -> Color {
    Color::hsl(h, s, l, 1.)
}

fn shape(kind: Kind, a: P, b: P, size: f32) -> Shape {
    Shape { kind, a, b, color: color(0.01, 0.6, 0.54), size, filled: false, text: TextStyle::Plain, bend: None }
}

fn wave(x0: f32, y0: f32, n: usize) -> Vec<P> {
    (0..n).map(|i| (x0 + i as f32 * 2.5, y0 + (i as f32 / 9.).sin() * 18.)).collect()
}

/// Every case: a name, the picture's size, the drawing and the background.
fn cases() -> Vec<(&'static str, (u32, u32), Doc, Frame)> {
    let doc = |shapes: Vec<Shape>| Doc { shapes, ..Doc::default() };
    let none = Frame::default();
    let text = |s: &str, at: P, size: f32, style: TextStyle| Shape { text: style, ..shape(Kind::Text(s.into()), at, at, size) };
    let blue = color(0.6, 0.9, 0.55);
    vec![
        ("arrow", (480, 300), doc(vec![shape(Kind::Arrow, (140., 60.), (380., 220.), 6.)]), none.clone()),
        ("arrow_bent", (480, 300), doc(vec![Shape { bend: Some((330., 70.)), ..shape(Kind::Arrow, (140., 220.), (420., 120.), 6.) }]), none.clone()),
        ("line", (480, 300), doc(vec![shape(Kind::Line, (130., 250.), (450., 60.), 4.)]), none.clone()),
        ("rect", (480, 300), doc(vec![shape(Kind::Rect, (125., 40.), (435., 135.), 4.)]), none.clone()),
        ("rect_filled", (480, 300), doc(vec![Shape { filled: true, color: blue, ..shape(Kind::Rect, (140., 160.), (240., 260.), 4.) }]), none.clone()),
        ("ellipse", (480, 300), doc(vec![shape(Kind::Ellipse, (255., 145.), (445., 275.), 5.)]), none.clone()),
        ("pen", (480, 300), doc(vec![shape(Kind::Pen(wave(130., 200., 130)), (0., 0.), (0., 0.), 5.)]), none.clone()),
        ("marker", (480, 300), doc(vec![Shape { color: color(0.15, 1., 0.5), ..shape(Kind::Marker(wave(130., 80., 110)), (0., 0.), (0., 0.), 18.) }]), none.clone()),
        ("text_plain", (480, 300), doc(vec![text("Plain text Ёж", (140., 150.), 28., TextStyle::Plain)]), none.clone()),
        ("text_plate", (480, 300), doc(vec![text("On a plate", (140., 150.), 28., TextStyle::Plate)]), none.clone()),
        ("text_outline", (480, 300), doc(vec![text("Outlined", (270., 190.), 36., TextStyle::Outline)]), none.clone()),
        (
            "counters",
            (480, 300),
            doc([1, 2, 9, 10, 42, 99].iter().enumerate().map(|(i, &n)| shape(Kind::Counter(n), (150. + i as f32 * 52., 230.), (150. + i as f32 * 52., 230.), 36.)).collect()),
            none.clone(),
        ),
        ("blur", (480, 300), doc(vec![shape(Kind::Blur(Hide::Blur), (250., 140.), (450., 280.), 1.)]), none.clone()),
        ("pixelate", (480, 300), doc(vec![shape(Kind::Blur(Hide::Pixelate), (120., 35.), (420., 140.), 1.)]), none.clone()),
        ("spotlight", (480, 300), doc(vec![shape(Kind::Spotlight, (250., 140.), (450., 280.), 1.)]), none.clone()),
        (
            "text_on_blur",
            (480, 300),
            doc(vec![shape(Kind::Blur(Hide::Blur), (250., 140.), (450., 280.), 1.), text("Hidden", (290., 190.), 30., TextStyle::Plate)]),
            none.clone(),
        ),
        (
            "arrow_through_crop",
            (480, 300),
            Doc { shapes: vec![shape(Kind::Arrow, (60., 40.), (330., 200.), 6.)], crop: Some(((120., 30.), (460., 290.))), ..Doc::default() },
            none.clone(),
        ),
        (
            "crop_filled_past_edge",
            (480, 300),
            Doc { crop: Some(((-40., -20.), (480., 300.))), fill: Some([255, 255, 255, 255]), ..Doc::default() },
            none.clone(),
        ),
        (
            "turned_mirrored",
            (480, 300),
            Doc { shapes: vec![shape(Kind::Arrow, (40., 60.), (250., 400.), 6.)], view: Orientation { turns: 1, mirrored: true }, ..Doc::default() },
            none.clone(),
        ),
        ("scaled_down", (480, 300), Doc { shapes: vec![shape(Kind::Rect, (125., 40.), (435., 135.), 4.)], scale: 0.5, ..Doc::default() }, none.clone()),
        (
            "frame_gradient",
            (480, 300),
            doc(vec![shape(Kind::Arrow, (140., 60.), (380., 220.), 6.)]),
            Frame { backdrop: Backdrop::Gradient(0), padding: 0.08, radius: 0.02, shadow: 0.6, aspect: 0 },
        ),
        ("frame_transparent", (480, 300), doc(Vec::new()), Frame { backdrop: Backdrop::Transparent, padding: 0.06, radius: 0.03, shadow: 0.5, aspect: 1 }),
        (
            // A 2× screenshot: everything twice as big.
            "hidpi_2x",
            (960, 600),
            doc(vec![
                shape(Kind::Arrow, (280., 120.), (760., 440.), 12.),
                text("Retina", (300., 300.), 56., TextStyle::Plate),
                shape(Kind::Counter(7), (220., 460.), (220., 460.), 72.),
            ]),
            none,
        ),
    ]
}

#[test]
#[ignore = "writes tests/golden/cases; run on purpose"]
fn make_golden_cases() {
    for (name, (w, h), doc, frame) in cases() {
        let dir = root().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        base(w, h).save(dir.join("base.png")).unwrap();
        let json = project::to_json(&doc, &frame, &|i| format!("picture-{i}.png"));
        std::fs::write(dir.join("project.json"), serde_json::to_string(&json).unwrap()).unwrap();
        render(&dir).save(dir.join("expected.png")).unwrap();
    }
}
