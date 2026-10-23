//! The system's own file dialogs (SPEC §3.10): open, save as, choose a
//! folder. kadr draws none of its own. Each takes the window it belongs to,
//! so it opens over it (and as its sheet on macOS), and gives back a future
//! for the app's executor; nothing blocks while it's open.

use std::future::Future;
use std::path::{Path, PathBuf};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use rfd::AsyncFileDialog;

/// Files a dialog offers: a name and the extensions it means.
pub struct Filter<'a> {
    pub name: &'a str,
    pub extensions: &'a [&'a str],
}

/// Pictures kadr can open.
pub const PICTURES: &[&str] = &["png", "jpg", "jpeg", "webp"];

fn dialog<W: HasWindowHandle + HasDisplayHandle>(parent: Option<&W>, title: &str, filters: &[Filter]) -> AsyncFileDialog {
    let mut d = AsyncFileDialog::new().set_title(title);
    if let Some(parent) = parent {
        d = d.set_parent(parent);
    }
    for f in filters {
        d = d.add_filter(f.name, f.extensions);
    }
    d
}

/// One file to open, or several; None when cancelled.
pub fn open<W: HasWindowHandle + HasDisplayHandle>(parent: Option<&W>, title: &str, filters: &[Filter], multiple: bool) -> impl Future<Output = Option<Vec<PathBuf>>> + use<W> {
    let d = dialog(parent, title, filters);
    async move {
        let picked = if multiple { d.pick_files().await? } else { vec![d.pick_file().await?] };
        Some(picked.into_iter().map(|f| f.path().to_path_buf()).collect())
    }
}

/// A folder; None when cancelled.
pub fn folder<W: HasWindowHandle + HasDisplayHandle>(parent: Option<&W>, title: &str) -> impl Future<Output = Option<PathBuf>> + use<W> {
    let d = dialog(parent, title, &[]);
    async move { Some(d.pick_folder().await?.path().to_path_buf()) }
}

/// Where to save, starting in `dir` with `name`; None when cancelled.
pub fn save<W: HasWindowHandle + HasDisplayHandle>(parent: Option<&W>, title: &str, dir: &Path, name: Option<&str>, filters: &[Filter]) -> impl Future<Output = Option<PathBuf>> + use<W> {
    let mut d = dialog(parent, title, filters).set_directory(dir);
    if let Some(name) = name {
        d = d.set_file_name(name);
    }
    async move { Some(d.save_file().await?.path().to_path_buf()) }
}
