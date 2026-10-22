//! A drawing as `project.json` keeps it, in a `.kadr` file or next to an
//! edited screenshot: the shapes, the crop, the turn, the size and the
//! background. Pictures laid on top are files beside it, named in the JSON.
//!
//! What keeps an annotated screenshot editable, like CleanShot's projects:
//! the untouched picture and the drawing, kept in kadr's data folder and
//! keyed by the saved file. `kadr edit` on that file opens the drawing
//! again, as long as the file is still what kadr wrote. The same folder,
//! packed with `bundle`, is a `.kadr` project file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::RgbaImage;
use serde_json::{Value, json};

use crate::color::Rgba;
use crate::document::{Doc, Frame, Hide, Kind, Orientation, P, Shape, TextStyle, cursor};

fn point(v: &Value) -> Option<P> {
    Some((v.get(0)?.as_f64()? as f32, v.get(1)?.as_f64()? as f32))
}

fn points(v: &Value) -> Vec<P> {
    v.as_array().into_iter().flatten().filter_map(point).collect()
}

/// One shape; a picture is saved apart and named by the caller.
pub fn shape_to_json(shape: &Shape) -> Value {
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
        Kind::Cursor(name) => ("cursor", json!(name)),
        Kind::Blur(Hide::Pixelate) => ("pixelate", json!(null)),
        Kind::Blur(Hide::Blur) => ("blur", json!(null)),
        Kind::Spotlight => ("spotlight", json!(null)),
        Kind::Picture(_) => ("picture", json!(null)),
    };
    let c = shape.color;
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

/// One shape back; `picture` loads a picture file by the name in the JSON.
pub fn shape_from_json(v: &Value, picture: &dyn Fn(&str) -> Option<RgbaImage>) -> Option<Shape> {
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
        "cursor" => Kind::Cursor(cursor(data.as_str()?)),
        "pixelate" => Kind::Blur(Hide::Pixelate),
        "blur" => Kind::Blur(Hide::Blur),
        "spotlight" => Kind::Spotlight,
        "picture" => Kind::Picture(Arc::new(picture(v["picture"].as_str()?)?)),
        _ => return None,
    };
    let c = v["color"].as_array()?;
    let f = |i: usize| c.get(i).and_then(Value::as_f64).unwrap_or(1.) as f32;
    Some(Shape {
        kind,
        a: point(&v["a"])?,
        b: point(&v["b"])?,
        color: Rgba::new(f(0), f(1), f(2), f(3)),
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

/// The whole drawing. `picture_name` names the file each laid-on picture is
/// kept in, by the shape's index.
pub fn to_json(doc: &Doc, frame: &Frame, picture_name: &dyn Fn(usize) -> String) -> Value {
    let shapes: Vec<Value> = doc
        .shapes
        .iter()
        .enumerate()
        .map(|(i, shape)| {
            let mut v = shape_to_json(shape);
            if matches!(shape.kind, Kind::Picture(_)) {
                v["picture"] = json!(picture_name(i));
            }
            v
        })
        .collect();
    json!({
        "frame": frame.to_json(),
        "crop": doc.crop.map(|(a, b)| json!([[a.0, a.1], [b.0, b.1]])),
        "view": { "turns": doc.view.turns, "mirrored": doc.view.mirrored },
        "scale": doc.scale,
        "fill": doc.fill,
        "shapes": shapes,
    })
}

/// The drawing back from `project.json`; unknown shapes are left out.
pub fn from_json(v: &Value, picture: &dyn Fn(&str) -> Option<RgbaImage>) -> Option<(Doc, Frame)> {
    let shapes = v["shapes"].as_array()?.iter().filter_map(|s| shape_from_json(s, picture)).collect();
    let crop = v["crop"].as_array().and_then(|c| Some((point(c.first()?)?, point(c.get(1)?)?)));
    let view = Orientation { turns: v["view"]["turns"].as_u64().unwrap_or(0) as u8 % 4, mirrored: v["view"]["mirrored"].as_bool().unwrap_or(false) };
    let scale = v["scale"].as_f64().map_or(1., |s| (s as f32).clamp(0.05, 1.));
    let fill = v["fill"].as_array().filter(|c| c.len() == 4).map(|c| [0, 1, 2, 3].map(|i| c[i].as_u64().unwrap_or(0).min(255) as u8));
    Some((Doc { shapes, crop, view, scale, fill }, Frame::from_json(&v["frame"])))
}

/// One folder per edited file.
pub fn root() -> PathBuf {
    crate::dirs::data().join("edits")
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
pub struct Restored {
    pub base: Vec<u8>,
    pub doc: Doc,
    pub frame: Frame,
}

/// The project for `path`, if the file there is still the one kadr wrote
/// (`bytes` are its contents).
pub fn load(path: &Path, bytes: &[u8]) -> Option<Restored> {
    let dir = dir_for(path);
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("project.json")).ok()?).ok()?;
    if v["output_hash"].as_str() != Some(&format!("{:016x}", fnv(bytes))) {
        return None;
    }
    read_dir(&dir, &v)
}

/// A project kept in `dir` (whose `project.json` is `v`).
fn read_dir(dir: &Path, v: &Value) -> Option<Restored> {
    let base = std::fs::read(dir.join("base.png")).ok()?;
    let (doc, frame) = from_json(v, &|name| image::open(dir.join(name)).ok().map(|i| i.to_rgba8()))?;
    Some(Restored { base, doc, frame })
}

/// The project in an unpacked `.kadr`.
pub fn load_unpacked(dir: &Path) -> Option<Restored> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("project.json")).ok()?).ok()?;
    read_dir(dir, &v)
}

/// Keep `doc` and `frame` drawn over `base` for `path`, whose contents are
/// now `output`. Nothing drawn and no background: nothing to keep.
/// `base` makes the picture's file, only when there's something to keep.
pub fn save(path: &Path, output: &[u8], base: impl FnOnce() -> std::io::Result<Vec<u8>>, doc: &Doc, frame: &Frame) -> std::io::Result<()> {
    let dir = dir_for(path);
    let _ = std::fs::remove_dir_all(&dir);
    if doc.shapes.is_empty() && doc.crop.is_none() && doc.view.is_upright() && doc.scale == 1. && !frame.on() {
        return Ok(());
    }
    write_dir(&dir, &base()?, doc, frame, json!({ "output": path.display().to_string(), "output_hash": format!("{:016x}", fnv(output)) })).map(|_| ())
}

/// Write the project into `dir`, with `extra` fields in its JSON. Returns
/// the files written, by name.
pub fn write_dir(dir: &Path, base: &[u8], doc: &Doc, frame: &Frame, extra: Value) -> std::io::Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("base.png"), base)?;
    let mut files = vec!["project.json".to_owned(), "base.png".to_owned()];
    let name = |i: usize| format!("picture-{i}.png");
    for (i, shape) in doc.shapes.iter().enumerate() {
        if let Kind::Picture(picture) = &shape.kind {
            picture.save(dir.join(name(i))).map_err(std::io::Error::other)?;
            files.push(name(i));
        }
    }
    let mut v = to_json(doc, frame, &name);
    if let (Some(v), Some(extra)) = (v.as_object_mut(), extra.as_object()) {
        v.extend(extra.clone());
    }
    std::fs::write(dir.join("project.json"), serde_json::to_string(&v).unwrap_or_default())?;
    Ok(files)
}

/// Pack the drawing, the picture under it and the result (`output`) into
/// the `.kadr` at `target`.
pub fn save_bundle(target: &Path, output: &[u8], base: &[u8], doc: &Doc, frame: &Frame) -> std::io::Result<()> {
    let tmp = crate::dirs::runtime().join(format!("kadr-bundle-{}-{:016x}", std::process::id(), fnv(target.as_os_str().as_encoded_bytes())));
    let _ = std::fs::remove_dir_all(&tmp);
    let result = (|| {
        let mut names = write_dir(&tmp, base, doc, frame, json!({}))?;
        std::fs::write(tmp.join("preview.png"), output)?;
        names.push("preview.png".into());
        let files: Vec<_> = names.into_iter().map(|n| (n.clone(), tmp.join(n))).collect();
        crate::bundle::pack(target, crate::bundle::Kind::Shot, &files)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Remove projects whose file is gone. Returns the bytes and files freed.
pub fn prune() -> (u64, usize) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_round_trip_through_json() {
        let red = Rgba::hsl(0.01, 0.6, 0.54, 1.);
        let shape = |kind, a, b, size, filled, text, bend| Shape { kind, a, b, color: red, size, filled, text, bend };
        let shapes = [
            shape(Kind::Arrow, (1., 2.), (30., 40.), 4., false, TextStyle::Plain, Some((10., 30.))),
            shape(Kind::Pen(vec![(0., 0.), (5., 6.)]), (0., 0.), (5., 6.), 2., false, TextStyle::Plain, None),
            shape(Kind::Text("Hi\nthere".into()), (3., 4.), (3., 4.), 20., false, TextStyle::Plate, None),
            shape(Kind::Blur(Hide::Blur), (3., 4.), (50., 60.), 2., false, TextStyle::Plain, None),
            shape(Kind::Rect, (3., 4.), (50., 60.), 2., true, TextStyle::Plain, None),
            shape(Kind::Cursor("pointer"), (3., 4.), (3., 4.), 30., false, TextStyle::Plain, None),
        ];
        for shape in &shapes {
            let back = shape_from_json(&shape_to_json(shape), &|_| None).unwrap();
            assert_eq!((back.a, back.b, back.size, back.filled, back.bend, back.color, back.text), (shape.a, shape.b, shape.size, shape.filled, shape.bend, shape.color, shape.text));
            match (&back.kind, &shape.kind) {
                (Kind::Pen(x), Kind::Pen(y)) => assert_eq!(x, y),
                (Kind::Text(x), Kind::Text(y)) => assert_eq!(x, y),
                (Kind::Blur(x), Kind::Blur(y)) => assert_eq!(x, y),
                (Kind::Cursor(x), Kind::Cursor(y)) => assert_eq!(x, y),
                (Kind::Arrow, Kind::Arrow) | (Kind::Rect, Kind::Rect) => {}
                _ => panic!("kind changed"),
            }
        }
    }

    #[test]
    fn a_drawing_round_trips_with_its_pictures() {
        let picture = Arc::new(RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255])));
        let mut doc = Doc { crop: Some(((0., 0.), (10., 10.))), view: Orientation { turns: 3, mirrored: true }, scale: 0.5, fill: Some([1, 2, 3, 4]), ..Doc::default() };
        doc.shapes.push(Shape { kind: Kind::Picture(picture.clone()), a: (1., 1.), b: (4., 3.), color: Rgba::WHITE, size: 1., filled: false, text: TextStyle::Plain, bend: None });
        let v = to_json(&doc, &Frame::default(), &|i| format!("picture-{i}.png"));
        assert_eq!(v["shapes"][0]["picture"], "picture-0.png");
        let (back, _) = from_json(&v, &|name| (name == "picture-0.png").then(|| (*picture).clone())).unwrap();
        assert_eq!((back.crop, back.view, back.scale, back.fill), (doc.crop, doc.view, doc.scale, doc.fill));
        assert!(matches!(&back.shapes[0].kind, Kind::Picture(p) if p.dimensions() == (3, 2)));
    }

    #[test]
    fn a_project_comes_back_only_for_the_file_it_wrote() {
        // The projects go under the test's own temporary data folder.
        let tmp = std::env::temp_dir().join(format!("kadr-project-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("shot.png");
        std::fs::write(&file, b"annotated").unwrap();
        let red = Rgba::new(1., 0., 0., 1.);
        let doc = Doc { shapes: vec![Shape { kind: Kind::Rect, a: (1., 1.), b: (9., 9.), color: red, size: 2., filled: false, text: TextStyle::Plain, bend: None }], crop: Some(((0., 0.), (10., 10.))), ..Doc::default() };
        save(&file, b"annotated", || Ok(b"original".to_vec()), &doc, &Frame::default()).unwrap();
        let back = load(&file, b"annotated").expect("the project is there");
        assert_eq!(back.base, b"original");
        assert_eq!((back.doc.shapes.len(), back.doc.crop), (1, doc.crop));
        assert!(load(&file, b"changed since").is_none(), "a file changed elsewhere opens as it is");
        std::fs::remove_file(&file).unwrap();
        assert_eq!(prune().1, 2, "base and project of a deleted file go");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_drawing_round_trips_through_a_kadr_file() {
        let tmp = std::env::temp_dir().join(format!("kadr-project-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let red = Rgba::new(1., 0., 0., 1.);
        let doc = Doc {
            shapes: vec![
                Shape { kind: Kind::Arrow, a: (1., 1.), b: (9., 9.), color: red, size: 2., filled: false, text: TextStyle::Plain, bend: None },
                Shape { kind: Kind::Text("hi".into()), a: (5., 5.), b: (5., 5.), color: red, size: 20., filled: false, text: TextStyle::Outline, bend: None },
            ],
            crop: Some(((0., 0.), (10., 10.))),
            ..Doc::default()
        };
        let target = tmp.join("Shot.kadr");
        save_bundle(&target, b"result", b"original", &doc, &Frame::default()).unwrap();
        let u = crate::bundle::unpack_into(&target, &tmp.join("cache")).unwrap();
        assert_eq!(u.kind, crate::bundle::Kind::Shot);
        assert_eq!(std::fs::read(u.dir.join("preview.png")).unwrap(), b"result");
        let back = load_unpacked(&u.dir).expect("the drawing is there");
        assert_eq!(back.base, b"original");
        assert_eq!((back.doc.shapes.len(), back.doc.crop), (2, doc.crop));
        assert!(matches!(&back.doc.shapes[1].kind, Kind::Text(t) if t == "hi"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
