//! What the app hands the `kadr-overlay` program (macOS and Windows, where
//! the overlay is a process of its own) and what it hands back: a JSON file
//! naming the monitor and a raw file of its frozen pixels, then one JSON
//! line on the program's output.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::model::{Outcome, Rect};
use crate::{Options, Screen};

/// Writes `screen` and `options` to `dir` (spec.json and frozen.rgba);
/// gives back the spec's path.
pub fn write(dir: &Path, screen: &Screen, options: &Options, lang: &str) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir)?;
    let frozen = dir.join("frozen.rgba");
    let (w, h) = screen.frozen.dimensions();
    let mut f = std::io::BufWriter::new(std::fs::File::create(&frozen)?);
    writeln!(f, "{w} {h}")?;
    f.write_all(screen.frozen.as_raw())?;
    f.flush()?;
    let rect = |r: &Rect| json!([r.0, r.1, r.2, r.3]);
    let a = options.accent;
    let spec = json!({
        "output": screen.output,
        "logical": [screen.logical.0, screen.logical.1],
        "scale": screen.scale,
        "origin": [screen.origin.0, screen.origin.1],
        "frozen": frozen,
        "windows": screen.windows.iter().map(rect).collect::<Vec<_>>(),
        "previous": screen.previous.as_ref().map(rect),
        "snap": options.snap,
        "color": options.color,
        "accent": [a.r, a.g, a.b, a.a],
        "lang": lang,
    });
    let path = dir.join("spec.json");
    std::fs::write(&path, spec.to_string())?;
    Ok(path)
}

/// Reads what `write` wrote: the screen, the options (with kadr-core's
/// words, in the language it names) and that language's code.
pub fn read(path: &Path) -> Option<(Screen, Options, String)> {
    let v: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let f = |v: &Value| v.as_f64().unwrap_or(0.);
    let rect = |r: &Value| (f(&r[0]) as f32, f(&r[1]) as f32, f(&r[2]) as f32, f(&r[3]) as f32);
    let mut file = std::io::BufReader::new(std::fs::File::open(v["frozen"].as_str()?).ok()?);
    let mut header = Vec::new();
    let mut byte = [0u8];
    while file.read_exact(&mut byte).is_ok() && byte[0] != b'\n' {
        header.push(byte[0]);
    }
    let header = String::from_utf8(header).ok()?;
    let mut dims = header.split_whitespace().filter_map(|n| n.parse::<u32>().ok());
    let (w, h) = (dims.next()?, dims.next()?);
    let mut pixels = vec![0; (w * h * 4) as usize];
    file.read_exact(&mut pixels).ok()?;
    let screen = Screen {
        output: v["output"].as_str().unwrap_or_default().to_owned(),
        logical: (f(&v["logical"][0]) as u32, f(&v["logical"][1]) as u32),
        scale: f(&v["scale"]).max(0.1),
        frozen: Arc::new(image::RgbaImage::from_raw(w, h, pixels)?),
        windows: v["windows"].as_array().map(|a| a.iter().map(rect).collect()).unwrap_or_default(),
        previous: v["previous"].as_array().map(|_| rect(&v["previous"])),
        origin: (f(&v["origin"][0]), f(&v["origin"][1])),
    };
    let a = |i: usize| f(&v["accent"][i]) as f32;
    let options = Options {
        snap: v["snap"].as_bool().unwrap_or(false),
        color: v["color"].as_bool().unwrap_or(false),
        accent: kadr_core::color::Rgba { r: a(0), g: a(1), b: a(2), a: a(3) },
        free: kadr_core::i18n::t("picker.free").to_owned(),
        t: |k| kadr_core::i18n::t(k).to_owned(),
        pointer: None,
    };
    Some((screen, options, v["lang"].as_str().unwrap_or("en").to_owned()))
}

/// The outcome as the program's one line of output.
pub fn outcome_line(outcome: Option<Outcome>) -> String {
    match outcome {
        Some(Outcome::Done(Some((r, window)))) => json!({ "done": [r.0, r.1, r.2, r.3], "window": window }),
        Some(Outcome::Color((x, y))) => json!({ "color": [x, y] }),
        _ => json!({ "cancel": true }),
    }
    .to_string()
}

/// The program's line back as an outcome; None when it said nothing
/// readable (it couldn't show itself).
pub fn parse_outcome(line: &str) -> Option<Option<Outcome>> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let f = |v: &Value| v.as_f64().unwrap_or(0.) as f32;
    if let Some(r) = v.get("done") {
        return Some(Some(Outcome::Done(Some(((f(&r[0]), f(&r[1]), f(&r[2]), f(&r[3])), v["window"].as_bool().unwrap_or(false))))));
    }
    if let Some(c) = v.get("color") {
        return Some(Some(Outcome::Color((f(&c[0]), f(&c[1])))));
    }
    v.get("cancel").map(|_| None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_and_its_outcome_go_there_and_back() {
        let dir = std::env::temp_dir().join(format!("kadr-overlay-spec-{}", std::process::id()));
        let frozen = Arc::new(image::RgbaImage::from_pixel(4, 2, image::Rgba([1, 2, 3, 255])));
        let screen = Screen { output: "DP-1".into(), logical: (2, 1), scale: 2., frozen, windows: vec![(0., 0., 1., 1.)], previous: None, origin: (10., 20.) };
        let options = Options { snap: true, color: false, accent: kadr_core::color::Rgba { r: 0.1, g: 0.2, b: 0.3, a: 1. }, free: "Free".into(), t: |k| k.to_owned(), pointer: None };
        let path = write(&dir, &screen, &options, "ru").unwrap();
        let (s, o, lang) = read(&path).unwrap();
        assert_eq!((s.output.as_str(), s.logical, s.origin, s.windows.len(), lang.as_str()), ("DP-1", (2, 1), (10., 20.), 1, "ru"));
        assert_eq!(s.frozen.get_pixel(3, 1).0, [1, 2, 3, 255]);
        assert!(o.snap && !o.color);
        let _ = std::fs::remove_dir_all(&dir);
        for outcome in [Some(Outcome::Done(Some(((1., 2., 3., 4.), true)))), Some(Outcome::Color((5., 6.))), None] {
            assert_eq!(parse_outcome(&outcome_line(outcome)), Some(outcome));
        }
    }
}
