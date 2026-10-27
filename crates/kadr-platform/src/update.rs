//! Taking a newer release (SPEC 12.3): how kadr was installed, downloading
//! a release's file, putting it in place of the running one, and putting
//! the old one back when the new one won't start. What the release is and
//! whether a file is genuine is `kadr_core::update`'s.
//!
//! - Linux, kadr's own archive (`~/.local/bin` and the like): `kadr` and
//!   `kadr-tray` replaced beside each other, the old ones kept as `*.prev`.
//! - macOS, `Kadr.app`: the new app copied out of the .dmg beside it, then
//!   swapped in by renaming; the old one kept as `.Kadr.app.prev`.
//! - Windows, installed: the installer run quietly once kadr is closed (it
//!   stops kadr and its tray itself and starts the tray again).
//! - Windows, the portable kadr.exe: swapped by renaming (a running exe can
//!   be renamed, not overwritten), the old one kept as `kadr.exe.prev`.
//!
//! A package manager's kadr (pacman, Homebrew) and a build from source
//! aren't touched: the card says where the update comes from.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use kadr_core::update::{Pending, Start};

/// How this kadr was installed, which decides how it's updated.
#[derive(Clone, PartialEq, Debug)]
pub enum Install {
    /// kadr's own files, which it replaces itself.
    Own(Layout),
    /// A package manager's (named), which updates it.
    Managed(&'static str),
    /// A build from source (`target/…`): nothing to replace.
    Built,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Layout {
    /// The folder holding `kadr` and `kadr-tray` (Linux).
    Dir(PathBuf),
    /// `Kadr.app` (macOS).
    App(PathBuf),
    /// The installer's folder (Windows).
    Installed(PathBuf),
    /// The portable kadr.exe (Windows).
    Portable(PathBuf),
}

impl Layout {
    /// Its file in `latest.json`'s `platforms`.
    pub fn asset(&self) -> &'static str {
        match self {
            Layout::Dir(_) => "linux-x86_64",
            Layout::App(_) => "macos-universal",
            Layout::Installed(_) => "windows-x86_64",
            Layout::Portable(_) => "windows-x86_64-portable",
        }
    }
}

/// How the running kadr was installed, from where its program is.
pub fn install() -> Install {
    let Ok(exe) = std::env::current_exe() else { return Install::Built };
    install_at(&exe)
}

fn install_at(exe: &Path) -> Install {
    if exe.components().any(|c| c.as_os_str() == "target") {
        return Install::Built;
    }
    let Some(dir) = exe.parent() else { return Install::Built };
    if cfg!(target_os = "linux") {
        if exe.starts_with("/usr") {
            let manager = ["pacman", "apt", "dnf", "zypper"].into_iter().find(|m| Path::new("/usr/bin").join(m).exists());
            return Install::Managed(manager.unwrap_or("the package manager"));
        }
        Install::Own(Layout::Dir(dir.to_owned()))
    } else if cfg!(target_os = "macos") {
        let s = exe.to_string_lossy();
        if s.contains("/Caskroom/") || s.contains("/Cellar/") || s.starts_with("/opt/homebrew/") {
            return Install::Managed("Homebrew");
        }
        // …/Kadr.app/Contents/MacOS/kadr
        match dir.parent().and_then(Path::parent) {
            Some(app) if app.extension().is_some_and(|e| e == "app") => Install::Own(Layout::App(app.to_owned())),
            _ => Install::Built,
        }
    } else if dir.join("unins000.exe").exists() {
        Install::Own(Layout::Installed(dir.to_owned()))
    } else {
        Install::Own(Layout::Portable(exe.to_owned()))
    }
}

/// Where downloads and unpacked releases go.
fn folder() -> PathBuf {
    kadr_core::dirs::cache().join("updates")
}

/// Downloads `url` to `to` through the system's curl, telling `progress`
/// the bytes so far. HTTPS only (a `file://` one too with `KADR_DEV`, to
/// try a release from disk).
pub fn download(url: &str, to: &Path, progress: &dyn Fn(u64)) -> Result<(), String> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let proto = if std::env::var_os("KADR_DEV").is_some() { "=https,file" } else { "=https" };
    let mut command = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    command.args(["-fsSL", "--retry", "2", "--connect-timeout", "20", "--proto", proto, "-o"]).arg(to).arg(url);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|e| format!("curl: {e}"))?;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if status.success() {
                return Ok(());
            }
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut e, &mut err);
            }
            return Err(err.trim().to_owned());
        }
        progress(std::fs::metadata(to).map_or(0, |m| m.len()));
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// What's left to do once a release's file is in place.
#[derive(Debug)]
pub enum Applied {
    /// The new files are in; the tray starts again with this.
    Done { tray: Vec<OsString> },
    /// The installer takes over: `run_installer` once kadr may close.
    Installer(PathBuf),
}

/// Puts `file` (the release's file for `layout`, checked already) in place
/// of the running kadr, which is `version` after it. Keeps the old files
/// and notes them in `Pending`, for `started` to put back.
pub fn apply(layout: &Layout, file: &Path, version: &str) -> Result<Applied, String> {
    let work = folder().join(format!("kadr-{version}"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    match layout {
        Layout::Dir(dir) => {
            run(Command::new("tar").arg("-xJf").arg(file).arg("-C").arg(&work))?;
            let unpacked = std::fs::read_dir(&work).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.join("kadr").is_file()).ok_or("no kadr in the archive")?;
            answers(&unpacked.join("kadr"), version)?;
            let mut files = vec![];
            for name in ["kadr", "kadr-tray"] {
                let (new, at) = (unpacked.join(name), dir.join(name));
                if new.is_file() {
                    files.push(swap(&new, &at)?);
                }
            }
            note(version, files);
            Ok(Applied::Done { tray: vec![dir.join("kadr-tray").into()] })
        }
        Layout::App(app) => {
            let mount = work.join("mnt");
            run(Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"]).arg(&mount).arg(file))?;
            let name = app.file_name().ok_or("no app name")?;
            let new = app.with_file_name(format!(".{}.new", name.to_string_lossy()));
            let _ = std::fs::remove_dir_all(&new);
            let copied = run(Command::new("ditto").arg(mount.join("Kadr.app")).arg(&new));
            let _ = run(Command::new("hdiutil").args(["detach", "-force"]).arg(&mount));
            copied?;
            answers(&new.join("Contents/MacOS/kadr"), version)?;
            let swapped = swap(&new, app)?;
            note(version, vec![swapped]);
            Ok(Applied::Done { tray: vec![app.join("Contents/MacOS/kadr-tray").into()] })
        }
        Layout::Installed(_) => {
            let setup = work.join("kadr-setup.exe");
            std::fs::rename(file, &setup).or_else(|_| std::fs::copy(file, &setup).map(|_| ())).map_err(|e| e.to_string())?;
            Ok(Applied::Installer(setup))
        }
        Layout::Portable(exe) => {
            let new = work.join("kadr.exe");
            std::fs::copy(file, &new).map_err(|e| e.to_string())?;
            answers(&new, version)?;
            let swapped = swap(&new, exe)?;
            note(version, vec![swapped]);
            Ok(Applied::Done { tray: vec![exe.into(), "tray".into()] })
        }
    }
}

/// Runs the installer quietly; kadr quits right after (the installer stops
/// what's left of it and starts the tray again).
pub fn run_installer(setup: &Path) -> bool {
    crate::spawn(&[setup.as_os_str(), "/VERYSILENT".as_ref(), "/SUPPRESSMSGBOXES".as_ref(), "/NORESTART".as_ref()]).is_some()
}

/// Runs `command`, its error output as the error.
fn run(command: &mut Command) -> Result<(), String> {
    let out = command.stdin(Stdio::null()).output().map_err(|e| format!("{:?}: {e}", command.get_program()))?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
}

/// The new program starts and says it's `version`, or it isn't put in.
fn answers(exe: &Path, version: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755));
    }
    let mut command = Command::new(exe);
    command.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|e| format!("the new kadr doesn't start: {e}"))?;
    let started = Instant::now();
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if started.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            return Err("the new kadr doesn't answer".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let said = String::from_utf8_lossy(&out.stdout);
    if said.split_whitespace().any(|w| w == version) { Ok(()) } else { Err(format!("the new kadr says {:?}", said.trim())) }
}

/// The old file (or app) beside `at`, kept until the new one settles.
fn prev_of(at: &Path) -> PathBuf {
    let name = at.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if at.extension().is_some_and(|e| e == "app") { at.with_file_name(format!(".{name}.prev")) } else { at.with_file_name(format!("{name}.prev")) }
}

/// Where a version that failed to start is moved when the old one comes back.
fn failed_of(at: &Path) -> PathBuf {
    at.with_file_name(format!(".{}.failed", at.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()))
}

/// Puts `new` at `at`, the old one moved aside: renames only, so a running
/// program keeps its file and nothing is ever half-written at `at`.
fn swap(new: &Path, at: &Path) -> Result<(PathBuf, PathBuf), String> {
    let prev = prev_of(at);
    remove(&prev);
    remove(&failed_of(at));
    // Beside `at` first, so the last step is a rename on one disk.
    let staged = at.with_file_name(format!(".{}.next", at.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
    remove(&staged);
    if std::fs::rename(new, &staged).is_err() {
        copy_all(new, &staged)?;
    }
    if at.exists() {
        std::fs::rename(at, &prev).map_err(|e| format!("{}: {e}", at.display()))?;
    }
    if let Err(e) = std::fs::rename(&staged, at) {
        let _ = std::fs::rename(&prev, at);
        return Err(format!("{}: {e}", at.display()));
    }
    Ok((at.to_owned(), prev))
}

fn copy_all(from: &Path, to: &Path) -> Result<(), String> {
    if from.is_dir() {
        run(Command::new("cp").arg("-R").arg(from).arg(to))
    } else {
        std::fs::copy(from, to).map(|_| ()).map_err(|e| e.to_string())
    }
}

fn remove(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

fn note(version: &str, files: Vec<(PathBuf, PathBuf)>) {
    Pending { version: version.to_owned(), starts: 0, files }.save();
}

/// At start, before anything that could fail: counts a start of a version
/// just put in, and after `TRIES` that never settled puts the old files
/// back. Returns the program to start again with (the old one, now at the
/// same place) when it did.
pub fn started() -> Option<PathBuf> {
    let mut pending = Pending::load()?;
    match pending.start(kadr_core::update::CURRENT) {
        Start::Nothing => None,
        Start::Counted => {
            pending.save();
            None
        }
        Start::RollBack => {
            let exe = std::env::current_exe().ok();
            for (at, prev) in &pending.files {
                if prev.exists() {
                    let bad = failed_of(at);
                    remove(&bad);
                    let _ = std::fs::rename(at, &bad);
                    let _ = std::fs::rename(prev, at);
                    // Gone at once where nothing runs it (Windows keeps a
                    // running exe; the next update's swap clears it).
                    remove(&bad);
                }
            }
            eprintln!("kadr: {} didn't start {} times: the previous version is back", pending.version, kadr_core::update::TRIES);
            Pending::clear();
            exe
        }
    }
}

/// The new version is up: the old files go, and what was downloaded (an
/// installer that ran, a release unpacked).
pub fn settled() {
    if let Some(pending) = Pending::load() {
        if kadr_core::update::compare(&pending.version, kadr_core::update::CURRENT) != Some(std::cmp::Ordering::Equal) {
            return;
        }
        for (_, prev) in &pending.files {
            remove(prev);
        }
        Pending::clear();
    }
    let _ = std::fs::remove_dir_all(folder());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_is_told_by_where_kadr_is() {
        assert_eq!(install_at(Path::new("/home/u/src/kadr/target/release/kadr")), Install::Built);
        if cfg!(target_os = "linux") {
            assert_eq!(install_at(Path::new("/home/u/.local/bin/kadr")), Install::Own(Layout::Dir("/home/u/.local/bin".into())));
            assert!(matches!(install_at(Path::new("/usr/bin/kadr")), Install::Managed(_)));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_release_replaces_kadr_and_its_tray_and_can_be_undone() {
        let root = std::env::temp_dir().join(format!("kadr-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (bin, release) = (root.join("bin"), root.join("release/kadr-9.9.9-x86_64-linux"));
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&release).unwrap();
        let script = |path: &Path, says: &str| {
            std::fs::write(path, format!("#!/bin/sh\necho {says}\n")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        script(&bin.join("kadr"), "kadr 1.0.0");
        script(&bin.join("kadr-tray"), "tray 1.0.0");
        script(&release.join("kadr"), "kadr 9.9.9");
        script(&release.join("kadr-tray"), "tray 9.9.9");
        let archive = root.join("kadr.tar.xz");
        run(Command::new("tar").arg("-C").arg(root.join("release")).arg("-cJf").arg(&archive).arg("kadr-9.9.9-x86_64-linux")).unwrap();

        // (kadr's folders are under the temporary one in tests.)
        // One that says another version isn't put in.
        assert!(apply(&Layout::Dir(bin.clone()), &archive, "9.9.8").is_err());
        assert!(std::fs::read_to_string(bin.join("kadr")).unwrap().contains("1.0.0"));

        let Applied::Done { tray } = apply(&Layout::Dir(bin.clone()), &archive, "9.9.9").unwrap() else { panic!("not done") };
        assert_eq!(tray, vec![OsString::from(bin.join("kadr-tray"))]);
        assert!(std::fs::read_to_string(bin.join("kadr")).unwrap().contains("9.9.9"));
        assert!(std::fs::read_to_string(bin.join("kadr-tray.prev")).unwrap().contains("1.0.0"));
        let pending = Pending::load().unwrap();
        assert_eq!((pending.version.as_str(), pending.files.len()), ("9.9.9", 2));

        // Three starts that never settle put the old ones back.
        let mut p = pending.clone();
        assert_eq!(p.start("9.9.9"), Start::Counted);
        p.starts = kadr_core::update::TRIES - 1;
        assert_eq!(p.start("9.9.9"), Start::RollBack);
        for (at, prev) in &pending.files {
            std::fs::rename(prev, at).unwrap();
        }
        assert!(std::fs::read_to_string(bin.join("kadr")).unwrap().contains("1.0.0"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
