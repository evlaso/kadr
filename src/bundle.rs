//! `.kadr` project files, like CleanShot's `.cleanshot`: one file that keeps
//! an edit whole, to reopen later or on another machine. Inside is a plain
//! tar (any `tar` lists it) with `manifest.json` first, then the pieces:
//! - a screenshot (`shot`): the untouched picture, added pictures, the
//!   drawing (`project.json`, as `editor::project` keeps it) and the result
//!   (`preview.png`);
//! - a recording (`studio`): the video, its click/key/cursor log, the studio
//!   edit, the camera's own recording and the wallpaper behind it.
//!
//! Opening one unpacks it into kadr's cache (once per version of the file)
//! and opens the editor or the studio there; saving from that session writes
//! the `.kadr` again.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use gpui::App;
use serde_json::{Value, json};

use crate::i18n::t;
use crate::util;

pub const EXT: &str = "kadr";
const VERSION: u64 = 1;
const MANIFEST: &str = "manifest.json";
/// Next to an unpacked project: the `.kadr` it came from.
const ORIGIN: &str = ".origin";
/// Next to an unpacked project: which version of the file it is.
const STAMP: &str = ".stamp";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Shot,
    Studio,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Shot => "shot",
            Kind::Studio => "studio",
        }
    }
}

pub fn is_bundle(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXT))
}

/// Open a `.kadr` in the editor or the studio, whichever it was made in.
pub fn open(path: PathBuf, cx: &mut App) {
    match kind(&path) {
        Ok(Kind::Shot) => crate::editor::open(path, cx),
        Ok(Kind::Studio) => crate::studio_editor::open(path, cx),
        Err(e) => util::notify(t("bundle.couldnt_open"), &format!("{}: {e}", path.display())),
    }
}

/// What a `.kadr` holds, from its manifest alone.
pub fn kind(path: &Path) -> std::io::Result<Kind> {
    let mut file = std::fs::File::open(path)?;
    let (name, size) = read_header(&mut file)?.ok_or_else(|| bad("empty"))?;
    if name != MANIFEST || size > 64 * 1024 {
        return Err(bad("no manifest"));
    }
    let mut text = vec![0; size as usize];
    file.read_exact(&mut text)?;
    manifest_kind(&serde_json::from_slice(&text).map_err(|_| bad("manifest"))?)
}

fn manifest_kind(v: &Value) -> std::io::Result<Kind> {
    if v["version"].as_u64().unwrap_or(0) > VERSION {
        return Err(bad("made by a newer kadr"));
    }
    match v["kind"].as_str() {
        Some("shot") => Ok(Kind::Shot),
        Some("studio") => Ok(Kind::Studio),
        _ => Err(bad("unknown kind")),
    }
}

fn bad(what: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, format!("not a kadr project ({what})"))
}

/// Write `files` (name inside, file on disk) as a `.kadr` of `kind` at `out`.
/// It's written beside and moved into place, so a failure leaves the old one.
pub fn pack(out: &Path, kind: Kind, files: &[(String, PathBuf)]) -> std::io::Result<()> {
    let tmp = out.with_extension(format!("{EXT}.part"));
    let result = (|| {
        let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        let manifest = json!({ "kind": kind.word(), "version": VERSION, "app": "kadr", "files": files.iter().map(|(n, _)| n).collect::<Vec<_>>() });
        let manifest = serde_json::to_vec_pretty(&manifest).unwrap_or_default();
        write_entry(&mut w, MANIFEST, manifest.len() as u64, &mut manifest.as_slice())?;
        for (name, path) in files {
            let mut f = std::fs::File::open(path)?;
            let len = f.metadata()?.len();
            write_entry(&mut w, name, len, &mut f)?;
        }
        // The end of a tar: two empty blocks.
        w.write_all(&[0; 1024])?;
        w.into_inner().map_err(|e| e.into_error())?.sync_all()
    })();
    match result {
        Ok(()) => std::fs::rename(&tmp, out),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// A `.kadr` unpacked into kadr's cache.
pub struct Unpacked {
    pub dir: PathBuf,
    pub kind: Kind,
    /// Whether the files were written just now (false: already there from
    /// an earlier opening of the same version).
    pub fresh: bool,
}

/// Unpack `file` (unless this version of it already is) into its own folder
/// under `root`, which remembers where it came from.
pub fn unpack_into(file: &Path, root: &Path) -> std::io::Result<Unpacked> {
    let file = file.canonicalize()?;
    let kind = kind(&file)?;
    let dir = root.join(format!("{:016x}", fnv(file.as_os_str().as_encoded_bytes())));
    let meta = std::fs::metadata(&file)?;
    let mtime = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis());
    let stamp = format!("{}:{mtime}", meta.len());
    if std::fs::read_to_string(dir.join(STAMP)).is_ok_and(|s| s == stamp) {
        return Ok(Unpacked { dir, kind, fresh: false });
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let mut r = std::io::BufReader::new(std::fs::File::open(&file)?);
    while let Some((name, size)) = read_header(&mut r)? {
        let padding = (512 - size % 512) % 512;
        // Only plain names: nothing outside the folder, nothing hidden.
        let safe = !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\']) && name != MANIFEST;
        if safe {
            let mut out = std::fs::File::create(dir.join(&name))?;
            let copied = std::io::copy(&mut (&mut r).take(size), &mut out)?;
            if copied != size {
                return Err(bad("cut short"));
            }
        } else {
            std::io::copy(&mut (&mut r).take(size), &mut std::io::sink())?;
        }
        std::io::copy(&mut (&mut r).take(padding), &mut std::io::sink())?;
    }
    std::fs::write(dir.join(ORIGIN), file.as_os_str().as_encoded_bytes())?;
    std::fs::write(dir.join(STAMP), stamp)?;
    Ok(Unpacked { dir, kind, fresh: true })
}

pub fn unpack(file: &Path) -> std::io::Result<Unpacked> {
    unpack_into(file, &util::cache_dir().join("projects"))
}

/// The `.kadr` a file in an unpacked project came from.
pub fn origin_of(path: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(path.parent()?.join(ORIGIN)).ok()?;
    Some(PathBuf::from(text))
}

/// Where a new `.kadr` for `source` (a screenshot or a recording) goes.
pub fn target_for(source: &Path, dir: &Path) -> PathBuf {
    let stem = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "kadr".into());
    dir.join(format!("{stem}.{EXT}"))
}

/// FNV-1a, stable across builds.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

// A minimal ustar: regular files only, names up to 100 bytes.

fn write_entry(w: &mut impl Write, name: &str, size: u64, data: &mut impl Read) -> std::io::Result<()> {
    if name.len() > 100 {
        return Err(bad("name too long"));
    }
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    let octal = |field: &mut [u8], v: u64| {
        let s = format!("{v:0width$o}", width = field.len() - 1);
        field[..s.len()].copy_from_slice(s.as_bytes());
    };
    octal(&mut h[100..108], 0o644);
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size);
    octal(&mut h[136..148], std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()));
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    // The checksum is counted with its own field as spaces.
    h[148..156].copy_from_slice(b"        ");
    let sum: u64 = h.iter().map(|&b| b as u64).sum();
    let s = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
    w.write_all(&h)?;
    let copied = std::io::copy(&mut data.take(size), w)?;
    if copied != size {
        return Err(bad("file changed while packing"));
    }
    w.write_all(&vec![0; ((512 - size % 512) % 512) as usize])
}

/// The next entry's name and size, or None at the end.
fn read_header(r: &mut impl Read) -> std::io::Result<Option<(String, u64)>> {
    let mut h = [0u8; 512];
    match r.read_exact(&mut h) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    if h.iter().all(|&b| b == 0) {
        return Ok(None);
    }
    let field = |f: &[u8]| String::from_utf8_lossy(&f[..f.iter().position(|&b| b == 0).unwrap_or(f.len())]).trim().to_owned();
    let size = u64::from_str_radix(&field(&h[124..136]), 8).map_err(|_| bad("size"))?;
    Ok(Some((field(&h[..100]), size)))
}

/// Linux desktop integration: `.kadr` files get their own type and icon and
/// open in kadr. Writes under `data` (normally ~/.local/share).
pub fn install_desktop(data: &Path, exe: &Path) -> std::io::Result<()> {
    let write = |rel: &str, text: &str| -> std::io::Result<()> {
        let path = data.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)
    };
    write("mime/packages/kadr.xml", MIME_XML)?;
    write("applications/kadr.desktop", &DESKTOP.replace("{exe}", &exe.display().to_string()))?;
    write("icons/hicolor/scalable/mimetypes/application-x-kadr-project.svg", FILE_ICON)?;
    write("icons/hicolor/scalable/apps/kadr.svg", APP_ICON)?;
    Ok(())
}

pub const MIME_TYPE: &str = "application/x-kadr-project";

const MIME_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="application/x-kadr-project">
    <comment>kadr project</comment>
    <comment xml:lang="ru">Проект kadr</comment>
    <sub-class-of type="application/x-tar"/>
    <icon name="application-x-kadr-project"/>
    <glob pattern="*.kadr"/>
  </mime-type>
</mime-info>
"#;

const DESKTOP: &str = "[Desktop Entry]
Type=Application
Name=kadr
Comment=Screenshots and screen recording
Comment[ru]=Скриншоты и запись экрана
Icon=kadr
Exec={exe} open %f
MimeType=application/x-kadr-project;
Categories=Graphics;Utility;
Terminal=false
StartupNotify=false
";

pub const FILE_ICON: &str = include_str!("../data/icons/kadr-project.svg");
pub const APP_ICON: &str = include_str!("../data/icons/kadr-app.svg");

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kadr-bundle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn round_trip(kind: Kind, files: &[(&str, Vec<u8>)]) {
        let dir = temp(kind.word());
        let src: Vec<(String, PathBuf)> = files
            .iter()
            .map(|(name, bytes)| {
                let p = dir.join(format!("src-{name}"));
                std::fs::write(&p, bytes).unwrap();
                (name.to_string(), p)
            })
            .collect();
        let out = dir.join("Project.kadr");
        pack(&out, kind, &src).unwrap();
        assert_eq!(super::kind(&out).unwrap(), kind);
        let root = dir.join("cache");
        let u = unpack_into(&out, &root).unwrap();
        assert!(u.fresh && u.kind == kind);
        for (name, bytes) in files {
            assert_eq!(&std::fs::read(u.dir.join(name)).unwrap(), bytes, "{name}");
        }
        assert_eq!(origin_of(&u.dir.join(files[0].0)).unwrap(), out.canonicalize().unwrap());
        assert!(!unpack_into(&out, &root).unwrap().fresh, "the same version unpacks once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shot_project_round_trips() {
        round_trip(
            Kind::Shot,
            &[("base.png", vec![0x89, b'P', b'N', b'G', 1, 2, 3]), ("project.json", br#"{"shapes":[]}"#.to_vec()), ("picture-0.png", vec![7; 513]), ("preview.png", Vec::new())],
        );
    }

    #[test]
    fn a_studio_project_round_trips() {
        let video: Vec<u8> = (0..200_000u32).map(|i| (i * 31 % 251) as u8).collect();
        round_trip(Kind::Studio, &[("video.mp4", video), ("track.json", b"{}".to_vec()), ("studio.json", br#"{"pad":0.1}"#.to_vec()), ("camera.mkv", vec![1; 1024])]);
    }

    #[test]
    fn names_that_leave_the_folder_are_skipped() {
        let dir = temp("unsafe");
        let src = dir.join("x");
        std::fs::write(&src, b"x").unwrap();
        let out = dir.join("evil.kadr");
        pack(&out, Kind::Shot, &[("../escape".into(), src.clone()), (".origin".into(), src)]).unwrap();
        let u = unpack_into(&out, &dir.join("cache")).unwrap();
        assert!(!dir.join("cache/escape").exists() && !dir.join("escape").exists());
        assert_eq!(origin_of(&u.dir.join("x")).unwrap(), out.canonicalize().unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn system_tar_reads_it() {
        let dir = temp("tar");
        let src = dir.join("a.txt");
        std::fs::write(&src, b"hello").unwrap();
        let out = dir.join("p.kadr");
        pack(&out, Kind::Shot, &[("a.txt".into(), src)]).unwrap();
        if let Ok(o) = std::process::Command::new("tar").arg("-tf").arg(&out).output() {
            assert_eq!(String::from_utf8_lossy(&o.stdout).lines().collect::<Vec<_>>(), vec![MANIFEST, "a.txt"]);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn desktop_files_land_under_the_given_folder() {
        let dir = temp("desktop");
        install_desktop(&dir, Path::new("/opt/kadr")).unwrap();
        assert!(std::fs::read_to_string(dir.join("applications/kadr.desktop")).unwrap().contains("Exec=/opt/kadr open %f"));
        assert!(dir.join("mime/packages/kadr.xml").exists());
        assert!(dir.join("icons/hicolor/scalable/mimetypes/application-x-kadr-project.svg").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
