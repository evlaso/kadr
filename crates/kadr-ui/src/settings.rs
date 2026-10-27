//! The settings window's state: the section shown, type-to-search over all
//! of them, editing the file name template, and recording a shortcut. Every
//! change is saved on the spot. What only the app can do (hand the hotkeys
//! to the system, pause them while one is recorded, close the window) comes
//! back from `key` as an `Effect`.

use kadr_core::hotkeys;
use kadr_core::i18n::t;
use kadr_core::settings::Settings;

use crate::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    General,
    Screenshots,
    Recording,
    Studio,
    Text,
    Shortcuts,
    Storage,
    About,
}

impl Tab {
    pub const ALL: [Tab; 8] = [Tab::General, Tab::Screenshots, Tab::Recording, Tab::Studio, Tab::Text, Tab::Shortcuts, Tab::Storage, Tab::About];

    pub fn label(self) -> &'static str {
        match self {
            Tab::General => t("settings.general"),
            Tab::Screenshots => t("common.screenshots"),
            Tab::Recording => t("common.recording"),
            Tab::Studio => t("common.studio"),
            Tab::Text => t("common.text"),
            Tab::Shortcuts => t("settings.shortcuts"),
            Tab::Storage => t("settings.storage"),
            Tab::About => t("settings.about_kadr"),
        }
    }
}

/// What the app does after a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Nothing,
    /// Redraw (something changed).
    Changed,
    /// The search changed: back to the top of the list.
    ScrollTop,
    /// The shortcuts changed: hand them to the system and let the paused
    /// ones work again.
    KeysChanged,
    /// Recording a shortcut stopped without one: let the paused ones work.
    RecordingStopped,
    Close,
}

pub struct SettingsVm {
    pub settings: Settings,
    pub tab: Tab,
    /// Type-to-search over every section, like System Settings.
    pub query: String,
    /// Typing goes into the file name template instead of the search.
    pub naming: bool,
    /// Key specs for each of `hotkeys::ACTIONS`.
    pub keys: Vec<Vec<String>>,
    /// The action a new shortcut is being recorded for.
    pub recording: Option<usize>,
}

impl SettingsVm {
    pub fn new(settings: Settings, keys: Vec<Vec<String>>) -> SettingsVm {
        SettingsVm { settings, tab: Tab::General, query: String::new(), naming: false, keys, recording: None }
    }

    /// Change a setting; it's saved at once. On the file as it is now, so
    /// what was changed in it by hand meanwhile stays.
    pub fn change(&mut self, f: impl FnOnce(&mut Settings)) {
        self.settings = Settings::load();
        f(&mut self.settings);
        self.settings.save();
    }

    pub fn select(&mut self, tab: Tab) {
        self.tab = tab;
        self.query.clear();
    }

    /// Start recording a shortcut for action `i`; the app pauses the
    /// system's binds while it does.
    pub fn start_recording(&mut self, i: usize) {
        self.recording = Some(i.min(hotkeys::ACTIONS.len() - 1));
        self.naming = false;
    }

    /// Stop recording; true when it was (the binds can work again).
    pub fn stop_recording(&mut self) -> bool {
        self.recording.take().is_some()
    }

    /// Drop a shortcut from action `i`.
    pub fn remove_key(&mut self, i: usize, spec: &str) {
        if let Some(keys) = self.keys.get_mut(i) {
            keys.retain(|s| s != spec);
        }
    }

    pub fn searching(&self) -> bool {
        !self.query.is_empty()
    }

    /// The sections listed: the chosen one, or every one while searching.
    pub fn tabs_shown(&self) -> Vec<Tab> {
        if self.searching() { Tab::ALL.to_vec() } else { vec![self.tab] }
    }

    /// Whether a row titled `text` in `tab` shows: always, unless a search
    /// matches neither it nor its section.
    pub fn row_shown(&self, tab: Tab, text: &str) -> bool {
        let query = self.query.to_lowercase();
        query.is_empty() || tab.label().to_lowercase().contains(&query) || text.to_lowercase().contains(&query)
    }

    /// A key pressed in the window.
    pub fn key(&mut self, k: &Key) -> Effect {
        if self.recording.is_some() {
            return self.record_key(k);
        }
        if self.naming {
            return self.name_key(k);
        }
        let i = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        let n = Tab::ALL.len();
        match k.key.as_str() {
            "escape" if self.searching() => {
                self.query.clear();
                Effect::ScrollTop
            }
            "escape" => Effect::Close,
            "tab" | "down" => {
                self.tab = Tab::ALL[(i + 1) % n];
                Effect::Changed
            }
            "up" => {
                self.tab = Tab::ALL[(i + n - 1) % n];
                Effect::Changed
            }
            "backspace" => {
                self.query.pop();
                Effect::ScrollTop
            }
            // Typing anywhere searches.
            _ => match k.text() {
                Some(ch) => {
                    self.query.push_str(ch);
                    Effect::ScrollTop
                }
                None => Effect::Nothing,
            },
        }
    }

    /// Typing into the file name template.
    fn name_key(&mut self, k: &Key) -> Effect {
        match k.key.as_str() {
            "escape" | "enter" | "tab" => self.naming = false,
            "backspace" => self.change(|s| {
                s.name_template.pop();
            }),
            _ => match k.text() {
                Some(ch) => {
                    let ch = ch.to_owned();
                    self.change(|s| s.name_template.push_str(&ch));
                }
                None => return Effect::Nothing,
            },
        }
        Effect::Changed
    }

    /// The shortcut being recorded: Escape alone cancels, modifiers alone
    /// wait for the key, and a shortcut belongs to one action.
    fn record_key(&mut self, k: &Key) -> Effect {
        let Some(i) = self.recording else { return Effect::Nothing };
        if k.key == "escape" && !k.any_modifier() {
            self.recording = None;
            return Effect::RecordingStopped;
        }
        let Some(spec) = hotkeys::spec(&k.key, k.ctrl, k.alt, k.shift, k.logo) else { return Effect::Nothing };
        for keys in &mut self.keys {
            keys.retain(|s| *s != spec);
        }
        self.keys[i].push(spec);
        self.recording = None;
        Effect::KeysChanged
    }

    /// For `kadr dev state`.
    pub fn dev_state(&self) -> serde_json::Value {
        serde_json::json!({
            "tab": format!("{:?}", self.tab).to_lowercase(),
            "query": self.query,
            "naming": self.naming,
            "name_template": self.settings.name_template,
            "recording_shortcut": self.recording,
            "keys": self.keys,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm() -> SettingsVm {
        SettingsVm::new(Settings::default(), vec![Vec::new(); hotkeys::ACTIONS.len()])
    }

    #[test]
    fn typing_searches_and_escape_clears_then_closes() {
        let mut vm = vm();
        assert_eq!(vm.key(&Key::char("t")), Effect::ScrollTop);
        vm.key(&Key::char("r"));
        vm.key(&Key::char("a"));
        assert_eq!(vm.query, "tra");
        assert_eq!(vm.tabs_shown().len(), Tab::ALL.len(), "a search looks through every section");
        assert!(vm.row_shown(Tab::General, "Tray icon"));
        assert!(!vm.row_shown(Tab::General, "Language"));
        assert_eq!(vm.key(&Key::named("backspace")), Effect::ScrollTop);
        assert_eq!(vm.query, "tr");
        assert_eq!(vm.key(&Key::named("escape")), Effect::ScrollTop);
        assert!(vm.query.is_empty());
        assert_eq!(vm.key(&Key::named("escape")), Effect::Close);
    }

    #[test]
    fn arrows_and_tab_walk_the_sections() {
        let mut vm = vm();
        vm.key(&Key::named("down"));
        assert_eq!(vm.tab, Tab::Screenshots);
        vm.key(&Key::named("up"));
        vm.key(&Key::named("up"));
        assert_eq!(vm.tab, Tab::About, "round from the first to the last");
        vm.key(&Key::named("tab"));
        assert_eq!(vm.tab, Tab::General);
    }

    #[test]
    fn a_shortcut_goes_to_one_action() {
        let mut vm = vm();
        vm.start_recording(1);
        assert_eq!(vm.key(&Key { ctrl: true, ..Key::named("shift") }), Effect::Nothing, "modifiers alone wait");
        let ctrl_shift_1 = Key { ctrl: true, shift: true, ..Key::char("1") };
        assert_eq!(vm.key(&ctrl_shift_1), Effect::KeysChanged);
        assert_eq!(vm.keys[1], ["CTRL + SHIFT + code:10"]);
        assert!(vm.recording.is_none());
        // The same keys for another action move there.
        vm.start_recording(4);
        vm.key(&ctrl_shift_1);
        assert!(vm.keys[1].is_empty());
        assert_eq!(vm.keys[4], ["CTRL + SHIFT + code:10"]);
        // Escape alone stops without one.
        vm.start_recording(2);
        assert_eq!(vm.key(&Key::named("escape")), Effect::RecordingStopped);
        assert!(vm.keys[2].is_empty() && vm.recording.is_none());
    }

    #[test]
    fn the_name_template_takes_typing_and_saves() {
        let mut vm = vm();
        vm.naming = true;
        vm.change(|s| s.name_template = "shot".into());
        vm.key(&Key::named("backspace"));
        vm.key(&Key::char("!"));
        assert_eq!(vm.settings.name_template, "sho!");
        assert_eq!(Settings::load().name_template, "sho!", "saved on the spot");
        vm.key(&Key::named("enter"));
        assert!(!vm.naming);
        assert!(vm.query.is_empty(), "the typing didn't search");
    }
}
