//! `.kadr` project files (the format is `kadr_core::bundle`): opening one
//! in the editor or the studio, and the Linux desktop's file type for them.

use std::path::{Path, PathBuf};

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

    #[test]
    fn desktop_files_land_under_the_given_folder() {
        let dir = std::env::temp_dir().join(format!("kadr-desktop-{}", std::process::id()));
        install_desktop(&dir, Path::new("/opt/kadr")).unwrap();
        assert!(std::fs::read_to_string(dir.join("applications/kadr.desktop")).unwrap().contains("Exec=/opt/kadr open %f"));
        assert!(dir.join("mime/packages/kadr.xml").exists());
        assert!(dir.join("icons/hicolor/scalable/mimetypes/application-x-kadr-project.svg").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
