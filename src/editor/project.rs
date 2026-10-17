//! What keeps an annotated screenshot editable, like CleanShot's projects:
//! the untouched picture, the shapes, the crop and the background, kept next
//! to kadr's other data and keyed by the saved file. `kadr edit` on that file
//! opens the drawing again, as long as the file is still what kadr wrote.

use std::path::{Path, PathBuf};

use palette::IntoColor;
use serde_json::{Value, json};

use super::{Doc, Frame, Hide, Kind, P, Picture, Shape, TextStyle};

/// One folder per edited file.
pub(crate) fn root() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("kadr/edits")
}

/// FNV-1a: stable across builds, unlike std's hasher, and quick enough for a
/// screenshot's bytes.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn dir_for(path: &Path) -> PathBuf {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    root().join(format!("{:016x}", fnv(path.as_os_str().as_encoded_bytes())))
}

/// A project found for a file: the picture to draw on and what was drawn.
pub(super) struct Restored {
    pub base: Vec<u8>,
    pub doc: Doc,
    pub frame: Frame,
}

/// The project for `path`, if the file there is still the one kadr wrote
/// (`bytes` are its contents).
pub(super) fn load(path: &Path, bytes: &[u8]) -> Option<Restored> {
    let dir = dir_for(path);
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("project.json")).ok()?).ok()?;
    if v["output_hash"].as_str() != Some(&format!("{:016x}", fnv(bytes))) {
        return None;
    }
    let base = std::fs::read(dir.join("base.png")).ok()?;
    let shapes = v["shapes"].as_array()?.iter().filter_map(|s| shape_from_json(s, &dir)).collect();
    let crop = v["crop"].as_array().and_then(|c| Some((point(c.first()?)?, point(c.get(1)?)?)));
    Some(Restored { base, doc: Doc { shapes, crop }, frame: Frame::from_json(&v["frame"]) })
}

/// Keep `doc` and `frame` drawn over `base` for `path`, whose contents are
/// now `output`. Nothing drawn and no background: nothing to keep.
pub(super) fn save(path: &Path, output: &[u8], base: &[u8], doc: &Doc, frame: &Frame) -> std::io::Result<()> {
    let dir = dir_for(path);
    let _ = std::fs::remove_dir_all(&dir);
    if doc.shapes.is_empty() && doc.crop.is_none() && !frame.on() {
        return Ok(());
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("base.png"), base)?;
    let mut shapes = Vec::new();
    for (i, shape) in doc.shapes.iter().enumerate() {
        let mut v = shape_to_json(shape);
        if let Kind::Picture(picture) = &shape.kind {
            let name = format!("picture-{i}.png");
            picture.pixels.save(dir.join(&name)).map_err(std::io::Error::other)?;
            v["picture"] = json!(name);
        }
        shapes.push(v);
    }
    let v = json!({
        "output": path.display().to_string(),
        "output_hash": format!("{:016x}", fnv(output)),
        "frame": frame.to_json(),
        "crop": doc.crop.map(|(a, b)| json!([[a.0, a.1], [b.0, b.1]])),
        "shapes": shapes,
    });
    std::fs::write(dir.join("project.json"), serde_json::to_string(&v).unwrap_or_default())
}

/// Remove projects whose file is gone. Returns the bytes and files freed.
pub(crate) fn prune() -> (u64, usize) {
    let mut freed = (0, 0);
    let Ok(entries) = std::fs::read_dir(root()) else { return freed };
    for entry in entries.flatten() {
        let dir = entry.path();
        let output = std::fs::read_to_string(dir.join("project.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["output"].as_str().map(PathBuf::from));
        if output.is_some_and(|p| p.exists()) {
            continue;
        }
        for file in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if let Ok(meta) = file.metadata() {
                freed.0 += meta.len();
                freed.1 += 1;
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    freed
}

fn point(v: &Value) -> Option<P> {
    Some((v.get(0)?.as_f64()? as f32, v.get(1)?.as_f64()? as f32))
}

fn points(v: &Value) -> Vec<P> {
    v.as_array().into_iter().flatten().filter_map(point).collect()
}

fn shape_to_json(shape: &Shape) -> Value {
    let pts = |ps: &[P]| ps.iter().map(|p| json!([p.0, p.1])).collect::<Vec<_>>();
    let (kind, extra) = match &shape.kind {
        Kind::Arrow => ("arrow", json!(null)),
        Kind::Rect => ("rect", json!(null)),
        Kind::Ellipse => ("ellipse", json!(null)),
        Kind::Line => ("line", json!(null)),
        Kind::Pen(ps) => ("pen", json!(pts(ps))),
        Kind::Marker(ps) => ("marker", json!(pts(ps))),
        Kind::Text(text) => ("text", json!(text)),
        Kind::Counter(n) => ("counter", json!(n)),
        Kind::Blur(Hide::Pixelate, _) => ("pixelate", json!(null)),
        Kind::Blur(Hide::Blur, _) => ("blur", json!(null)),
        Kind::Spotlight => ("spotlight", json!(null)),
        Kind::Picture(_) => ("picture", json!(null)),
    };
    let c = crate::ui::rgb_of(shape.color);
    json!({
        "kind": kind,
        "data": extra,
        "a": [shape.a.0, shape.a.1],
        "b": [shape.b.0, shape.b.1],
        "color": [c.r, c.g, c.b, c.a],
        "size": shape.size,
        "filled": shape.filled,
        "text": match shape.text {
            TextStyle::Plain => "plain",
            TextStyle::Plate => "plate",
            TextStyle::Outline => "outline",
        },
        "bend": shape.bend.map(|p| json!([p.0, p.1])),
    })
}

fn shape_from_json(v: &Value, dir: &Path) -> Option<Shape> {
    let data = &v["data"];
    let kind = match v["kind"].as_str()? {
        "arrow" => Kind::Arrow,
        "rect" => Kind::Rect,
        "ellipse" => Kind::Ellipse,
        "line" => Kind::Line,
        "pen" => Kind::Pen(points(data)),
        "marker" => Kind::Marker(points(data)),
        "text" => Kind::Text(data.as_str()?.to_owned()),
        "counter" => Kind::Counter(data.as_u64()? as u32),
        "pixelate" => Kind::Blur(Hide::Pixelate, None),
        "blur" => Kind::Blur(Hide::Blur, None),
        "spotlight" => Kind::Spotlight,
        "picture" => Kind::Picture(Picture::decode(&std::fs::read(dir.join(v["picture"].as_str()?)).ok()?)?),
        _ => return None,
    };
    let c = v["color"].as_array()?;
    let f = |i: usize| c.get(i).and_then(Value::as_f64).unwrap_or(1.) as f32;
    Some(Shape {
        kind,
        a: point(&v["a"])?,
        b: point(&v["b"])?,
        color: gpui::Rgba::new(f(0), f(1), f(2), f(3)).into_color(),
        size: v["size"].as_f64()? as f32,
        filled: v["filled"].as_bool().unwrap_or(false),
        text: match v["text"].as_str() {
            Some("plate") => TextStyle::Plate,
            Some("outline") => TextStyle::Outline,
            _ => TextStyle::Plain,
        },
        bend: point(&v["bend"]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_round_trip_through_json() {
        let dir = std::env::temp_dir();
        let red = gpui::hsla(0.01, 0.6, 0.54, 1.);
        let shapes = [
            Shape { kind: Kind::Arrow, a: (1., 2.), b: (30., 40.), color: red, size: 4., filled: false, text: TextStyle::Plain, bend: Some((10., 30.)) },
            Shape { kind: Kind::Pen(vec![(0., 0.), (5., 6.)]), a: (0., 0.), b: (5., 6.), color: red, size: 2., filled: false, text: TextStyle::Plain, bend: None },
            Shape { kind: Kind::Text("Hi\nthere".into()), a: (3., 4.), b: (3., 4.), color: red, size: 20., filled: false, text: TextStyle::Plate, bend: None },
            Shape { kind: Kind::Blur(Hide::Blur, None), a: (3., 4.), b: (50., 60.), color: red, size: 2., filled: false, text: TextStyle::Plain, bend: None },
            Shape { kind: Kind::Rect, a: (3., 4.), b: (50., 60.), color: red, size: 2., filled: true, text: TextStyle::Plain, bend: None },
        ];
        for shape in &shapes {
            let back = shape_from_json(&shape_to_json(shape), &dir).unwrap();
            assert_eq!((back.a, back.b, back.size, back.filled, back.bend), (shape.a, shape.b, shape.size, shape.filled, shape.bend));
            let (x, y) = (crate::ui::rgb_of(back.color), crate::ui::rgb_of(shape.color));
            assert!(back.text == shape.text && (x.r - y.r).abs() + (x.g - y.g).abs() + (x.b - y.b).abs() + (x.a - y.a).abs() < 1e-4);
            match (&back.kind, &shape.kind) {
                (Kind::Pen(x), Kind::Pen(y)) => assert_eq!(x, y),
                (Kind::Text(x), Kind::Text(y)) => assert_eq!(x, y),
                (Kind::Blur(x, _), Kind::Blur(y, _)) => assert!(x == y),
                (Kind::Arrow, Kind::Arrow) | (Kind::Rect, Kind::Rect) => {}
                _ => panic!("kind changed"),
            }
        }
    }

    #[test]
    fn a_project_comes_back_only_for_the_file_it_wrote() {
        let tmp = std::env::temp_dir().join(format!("kadr-project-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // SAFETY: tests in this module are the only ones reading XDG_DATA_HOME.
        unsafe { std::env::set_var("XDG_DATA_HOME", &tmp) };
        let file = tmp.join("shot.png");
        std::fs::write(&file, b"annotated").unwrap();
        let red = gpui::hsla(0., 1., 0.5, 1.);
        let doc = Doc { shapes: vec![Shape { kind: Kind::Rect, a: (1., 1.), b: (9., 9.), color: red, size: 2., filled: false, text: TextStyle::Plain, bend: None }], crop: Some(((0., 0.), (10., 10.))) };
        save(&file, b"annotated", b"original", &doc, &Frame::default()).unwrap();
        let back = load(&file, b"annotated").expect("the project is there");
        assert_eq!(back.base, b"original");
        assert_eq!((back.doc.shapes.len(), back.doc.crop), (1, doc.crop));
        assert!(load(&file, b"changed since").is_none(), "a file changed elsewhere opens as it is");
        std::fs::remove_file(&file).unwrap();
        assert_eq!(prune().1, 2, "base and project of a deleted file go");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
