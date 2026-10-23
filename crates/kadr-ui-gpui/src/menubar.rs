//! The app's commands with their system keys (SPEC 3.10), and macOS's menu
//! bar: kadr ▸ About, Settings… ⌘, and Quit ⌘Q; Window ▸ Close ⌘W. The
//! keys come from `kadr_platform::keymap`; the menus are GPUI's, which are
//! the system's own NSMenu there and nothing elsewhere.

use gpui::{App, KeyBinding, Menu, MenuItem, actions};
use kadr_platform::keymap::{self, Command};

use crate::i18n::t;

actions!(kadr, [About, OpenSettings, Quit, CloseWindow]);

pub fn init(cx: &mut App) {
    cx.on_action(|_: &About, cx| crate::settings_view::open_about(cx));
    cx.on_action(|_: &OpenSettings, cx| crate::settings_view::open(cx));
    cx.on_action(|_: &CloseWindow, cx| {
        // After the key's dispatch, which has the window in use.
        if let Some(window) = cx.active_window() {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            });
        }
    });
    cx.on_action(|_: &Quit, cx| cx.quit());

    let mut keys = Vec::new();
    keys.extend(keymap::key(Command::Settings).map(|k| KeyBinding::new(k, OpenSettings, None)));
    // Windows closes a window with Alt+F4 itself.
    keys.extend(keymap::keys(keymap::Os::current(), Command::CloseWindow).iter().filter(|k| **k != "alt-f4").map(|k| KeyBinding::new(k, CloseWindow, None)));
    // Quitting by key is macOS's; elsewhere kadr quits once idle.
    if keymap::Os::current() == keymap::Os::MacOs {
        keys.extend(keymap::key(Command::Quit).map(|k| KeyBinding::new(k, Quit, None)));
    }
    cx.bind_keys(keys);

    cx.set_menus([
        Menu::new("kadr").items([
            MenuItem::action(t("settings.about_kadr"), About),
            MenuItem::separator(),
            MenuItem::action(t("menu.settings"), OpenSettings),
            MenuItem::separator(),
            MenuItem::action(t("menu.quit"), Quit),
        ]),
        Menu::new(t("menu.window")).items([MenuItem::action(t("common.close"), CloseWindow)]),
    ]);
}
