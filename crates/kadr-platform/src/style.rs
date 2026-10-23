//! Style constants that follow the system (SPEC §3.10): the window's corner
//! radius, which control stands for an on/off setting, and where a pair of
//! buttons puts the one that confirms. kadr draws its own UI the same on
//! every system; these are the edges where it meets the system's own.

use crate::keymap::Os;

/// An on/off control.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Toggle {
    Switch,
    Checkbox,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Style {
    /// The window's corners: the system's own on macOS and Windows, kadr's
    /// Hyprland rule on Linux (`data/hypr/kadr.lua`).
    pub window_radius: f32,
    /// What a setting that's on or off is drawn as.
    pub toggle: Toggle,
    /// A pair like Cancel / Save puts the confirming one last (at the
    /// right), as macOS and GNOME do; Windows puts it first.
    pub confirm_last: bool,
}

pub const fn style(os: Os) -> Style {
    match os {
        Os::MacOs => Style { window_radius: 12., toggle: Toggle::Switch, confirm_last: true },
        Os::Windows => Style { window_radius: 8., toggle: Toggle::Switch, confirm_last: false },
        Os::Linux => Style { window_radius: 13., toggle: Toggle::Switch, confirm_last: true },
    }
}

/// This system's style.
pub const fn current() -> Style {
    style(Os::current())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_confirming_button_goes_where_the_system_puts_it() {
        assert!(style(Os::MacOs).confirm_last);
        assert!(!style(Os::Windows).confirm_last);
        assert_eq!(style(Os::Windows).window_radius, 8.);
        assert!((10. ..=12.).contains(&style(Os::MacOs).window_radius));
    }
}
