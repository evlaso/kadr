//! kadr's data and the logic over it, with no OS and no UI underneath: what
//! an annotated screenshot is (`document`) and how it's kept (`project`,
//! `bundle`), the settings, the texts in each language (`i18n`), capture
//! names and clearing them out (`storage`). Folders come from the app
//! (`dirs`).

pub mod bundle;
pub mod color;
pub mod dirs;
pub mod document;
pub mod hotkeys;
#[macro_use]
pub mod i18n;
pub mod project;
pub mod settings;
pub mod storage;
