//! kadr's screens as plain state, without a UI framework: what a screen
//! holds, what its commands do, undo. The GPUI screens (the app) only draw
//! it and turn input into these calls, so the behaviour is tested without a
//! window and another framework could draw the same thing.

pub mod cards;
pub mod editor;
pub mod history;
pub mod settings;
pub mod text;

/// A key as the screens get it, without a UI framework's types: its name
/// ("a", "escape", "down"), the text it types, and the modifiers held.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Key {
    pub key: String,
    pub ch: Option<String>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Super, Command, the Windows key.
    pub logo: bool,
}

impl Key {
    /// A key that types nothing, by name.
    pub fn named(key: &str) -> Key {
        Key { key: key.to_owned(), ..Key::default() }
    }

    /// A key that types `text`.
    pub fn char(text: &str) -> Key {
        Key { key: text.to_lowercase(), ch: Some(text.to_owned()), ..Key::default() }
    }

    pub fn any_modifier(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.logo
    }

    /// What it types, when it types something (not a shortcut).
    pub fn text(&self) -> Option<&str> {
        if self.ctrl || self.alt || self.logo {
            return None;
        }
        self.ch.as_deref().filter(|c| !c.chars().any(char::is_control))
    }
}
