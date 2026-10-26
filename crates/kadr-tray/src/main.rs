// A window app on Windows: no console opens with it.
#![cfg_attr(windows, windows_subsystem = "windows")]

//! `kadr-tray`: kadr's tray icon and, on macOS and Windows, its global
//! hotkeys, without the rest of kadr loaded (`kadr tray` does the same).

fn main() {
    kadr_platform::set_dirs();
    kadr_core::i18n::set(kadr_core::settings::Settings::load().language);
    kadr_tray::run();
}
