//! `.kadr` project files (the format is `kadr_core::bundle`): opening one
//! in the editor or the studio, and the Linux desktop's file type for them.

use std::path::PathBuf;

use gpui::App;
pub use kadr_core::bundle::*;

use crate::i18n::t;
use crate::util;

/// Open a `.kadr` in the editor or the studio, whichever it was made in.
pub fn open(path: PathBuf, cx: &mut App) {
    match kind(&path) {
        Ok(Kind::Shot) => crate::editor::open(path, cx),
        Ok(Kind::Studio) => crate::studio_editor::open(path, cx),
        Err(e) => util::notify(t("bundle.couldnt_open"), &format!("{}: {e}", path.display())),
    }
}
