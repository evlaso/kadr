//! What keeps an annotated screenshot editable, like CleanShot's projects:
//! the untouched picture, the shapes, the crop and the background, kept next
//! to kadr's other data and keyed by the saved file. `kadr edit` on that file
//! opens the drawing again, as long as the file is still what kadr wrote.
//! The same folder, packed with `bundle`, is a `.kadr` project file.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Doc, Frame, Kind};
#[cfg(test)]
use super::{Shape, TextStyle};

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
    read_dir(&dir, &v)
}

/// A project kept in `dir` (whose `project.json` is `v`).
fn read_dir(dir: &Path, v: &Value) -> Option<Restored> {
    let base = std::fs::read(dir.join("base.png")).ok()?;
    let (doc, frame) = kadr_core::project::from_json(v, &|name| image::open(dir.join(name)).ok().map(|i| i.to_rgba8()))?;
    Some(Restored { base, doc, frame })
}

/// The project in an unpacked `.kadr`.
pub(super) fn load_unpacked(dir: &Path) -> Option<Restored> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("project.json")).ok()?).ok()?;
    read_dir(dir, &v)
}

/// Keep `doc` and `frame` drawn over `base` for `path`, whose contents are
/// now `output`. Nothing drawn and no background: nothing to keep.
pub(super) fn save(path: &Path, output: &[u8], base: &[u8], doc: &Doc, frame: &Frame) -> std::io::Result<()> {
    let dir = dir_for(path);
    let _ = std::fs::remove_dir_all(&dir);
    if doc.shapes.is_empty() && doc.crop.is_none() && doc.view.is_upright() && doc.scale == 1. && !frame.on() {
        return Ok(());
    }
    write_dir(&dir, base, doc, frame, json!({ "output": path.display().to_string(), "output_hash": format!("{:016x}", fnv(output)) })).map(|_| ())
}

/// Write the project into `dir`, with `extra` fields in its JSON. Returns
/// the files written, by name.
pub(super) fn write_dir(dir: &Path, base: &[u8], doc: &Doc, frame: &Frame, extra: Value) -> std::io::Result<Vec<String>> {
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
    let mut v = kadr_core::project::to_json(doc, frame, &name);
    if let (Some(v), Some(extra)) = (v.as_object_mut(), extra.as_object()) {
        v.extend(extra.clone());
    }
    std::fs::write(dir.join("project.json"), serde_json::to_string(&v).unwrap_or_default())?;
    Ok(files)
}

/// Pack the drawing, the picture under it and the result (`output`) into
/// the `.kadr` at `target`.
pub(super) fn save_bundle(target: &Path, output: &[u8], base: &[u8], doc: &Doc, frame: &Frame) -> std::io::Result<()> {
    let tmp = crate::util::runtime_dir().join(format!("kadr-bundle-{}-{:016x}", std::process::id(), fnv(target.as_os_str().as_encoded_bytes())));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_comes_back_only_for_the_file_it_wrote() {
        let tmp = std::env::temp_dir().join(format!("kadr-project-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // SAFETY: tests in this module are the only ones reading XDG_DATA_HOME.
        unsafe { std::env::set_var("XDG_DATA_HOME", &tmp) };
        let file = tmp.join("shot.png");
        std::fs::write(&file, b"annotated").unwrap();
        let red = kadr_core::color::Rgba::new(1., 0., 0., 1.);
        let doc = Doc { shapes: vec![Shape { kind: Kind::Rect, a: (1., 1.), b: (9., 9.), color: red, size: 2., filled: false, text: TextStyle::Plain, bend: None }], crop: Some(((0., 0.), (10., 10.))), ..Doc::default() };
        save(&file, b"annotated", b"original", &doc, &Frame::default()).unwrap();
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
        let red = kadr_core::color::Rgba::new(1., 0., 0., 1.);
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
