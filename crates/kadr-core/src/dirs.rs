//! Where kadr keeps things. The app knows each system's folders and hands
//! them over once at start (`set`); core only joins paths onto them. Until
//! then (in tests) everything goes under the temporary folder.

use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct Dirs {
    /// Settings and the like (~/.config/kadr).
    pub config: PathBuf,
    /// What can be made again: previews, click logs, unpacked projects.
    pub cache: PathBuf,
    /// What keeps edits editable (~/.local/share/kadr).
    pub data: PathBuf,
    /// Screenshots and recordings by default.
    pub pictures: PathBuf,
    pub videos: PathBuf,
    /// Scratch files for this session.
    pub runtime: PathBuf,
}

static DIRS: OnceLock<Dirs> = OnceLock::new();

/// Once, before anything reads or writes.
pub fn set(dirs: Dirs) {
    let _ = DIRS.set(dirs);
}

fn get() -> &'static Dirs {
    DIRS.get_or_init(|| {
        let tmp = std::env::temp_dir().join(format!("kadr-{}", std::process::id()));
        Dirs { config: tmp.join("config"), cache: tmp.join("cache"), data: tmp.join("data"), pictures: tmp.join("pictures"), videos: tmp.join("videos"), runtime: tmp.join("run") }
    })
}

pub fn config() -> PathBuf {
    get().config.clone()
}

pub fn cache() -> PathBuf {
    get().cache.clone()
}

pub fn data() -> PathBuf {
    get().data.clone()
}

pub fn pictures() -> PathBuf {
    get().pictures.clone()
}

pub fn videos() -> PathBuf {
    get().videos.clone()
}

pub fn runtime() -> PathBuf {
    get().runtime.clone()
}
