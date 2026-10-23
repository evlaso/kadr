//! User settings, `settings.json` in the config folder (~/.config/kadr). Read where they're used (kadr
//! runs briefly, so there's nothing to keep in sync) and written by the
//! settings window on every change.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::dirs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// gpu-screen-recorder's quality presets.
pub const QUALITIES: [(&str, &str); 4] = [("medium", "common.medium"), ("high", "common.high"), ("very_high", "settings.very_high"), ("ultra", "settings.maximum")];
pub const CARD_SECONDS: [u32; 4] = [5, 10, 20, 0];
pub const MAX_CARDS: [usize; 3] = [2, 4, 6];
/// Recording frame rates; 0 is the screen's own refresh rate.
pub const FPS: [u32; 4] = [0, 30, 60, 120];

/// What happens right after a capture; any number of them at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct After {
    /// The thumbnail card in the screen corner.
    pub card: bool,
    pub copy: bool,
    /// Keep the file in the captures folder; off, it goes to a temporary one.
    pub save: bool,
    /// Open the editor (screenshots) or the studio (recordings).
    pub edit: bool,
    /// Pin the screenshot above all windows.
    pub pin: bool,
}

impl After {
    fn load(v: &Value, defaults: After) -> After {
        let flag = |key: &str, default: bool| v[key].as_bool().unwrap_or(default);
        After {
            card: flag("card", defaults.card),
            copy: flag("copy", defaults.copy),
            save: flag("save", defaults.save),
            edit: flag("edit", defaults.edit),
            pin: flag("pin", defaults.pin),
        }
    }

    fn json(&self) -> Value {
        json!({ "card": self.card, "copy": self.copy, "save": self.save, "edit": self.edit, "pin": self.pin })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// None keeps Omarchy's screenshot folder.
    pub screenshot_dir: Option<PathBuf>,
    pub after_shot: After,
    pub after_video: After,
    /// File names for new captures, with {date}, {time}, {type} and {n}.
    pub name_template: String,
    /// Window captures keep the window's shadow on a transparent background.
    pub window_shadow: bool,
    /// Screenshots are scaled down to logical pixels (1×), so a pixel of the
    /// picture is a pixel of the design; off keeps every screen pixel.
    pub logical_size: bool,
    /// 0 keeps cards until closed.
    pub card_seconds: u32,
    pub card_side: Side,
    /// The `kadr tray` icon.
    pub tray: bool,
    /// The tray keeps the area overlay running, warm, so it shows at once
    /// (SPEC 3.6): on by default on macOS, where starting it cold is slow.
    pub keep_warm: bool,
    pub max_cards: usize,
    /// None keeps the XDG videos folder.
    pub video_dir: Option<PathBuf>,
    pub fps: u32,
    /// Index into `QUALITIES`.
    pub quality: usize,
    pub region_frame: bool,
    /// The screenshot editor opens as an ordinary window; false puts it over
    /// everything (a layer) on Linux, the old way.
    pub editor_window: bool,
    /// Delete kadr's captures older than this many days; 0 keeps them.
    pub cleanup_days: u32,
    pub theme: ThemeMode,
    pub language: crate::i18n::Lang,
    pub studio_zoom: f64,
    pub studio_backdrop: usize,
    pub studio_padding: usize,
}

/// kadr's colours: following the system's appearance, or always one set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Dark,
    Light,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            screenshot_dir: None,
            after_shot: After { card: true, copy: true, save: true, edit: false, pin: false },
            after_video: After { card: true, copy: true, save: true, edit: false, pin: false },
            name_template: crate::storage::DEFAULT_NAME.into(),
            window_shadow: true,
            logical_size: true,
            card_seconds: 10,
            card_side: Side::Left,
            tray: true,
            keep_warm: cfg!(target_os = "macos"),
            max_cards: 4,
            video_dir: None,
            fps: 0,
            quality: 2,
            region_frame: true,
            editor_window: true,
            cleanup_days: 0,
            theme: ThemeMode::System,
            language: crate::i18n::Lang::En,
            studio_zoom: 1.8,
            studio_backdrop: 0,
            studio_padding: 2,
        }
    }
}

fn path() -> PathBuf {
    dirs::config().join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        let d = Settings::default();
        let Some(v) = std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
            return d;
        };
        let dir = |key: &str| v[key].as_str().filter(|s| !s.is_empty()).map(PathBuf::from);
        let bool_or = |key: &str, default: bool| v[key].as_bool().unwrap_or(default);
        let num_or = |key: &str, default: u64| v[key].as_u64().unwrap_or(default);
        Settings {
            screenshot_dir: dir("screenshot_dir"),
            // Before the after-capture choices there were two switches.
            after_shot: After::load(&v["after_shot"], After { card: bool_or("show_card", d.after_shot.card), copy: bool_or("copy_screenshot", d.after_shot.copy), ..d.after_shot }),
            after_video: After::load(&v["after_video"], After { card: bool_or("show_card", d.after_video.card), ..d.after_video }),
            name_template: v["name_template"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or(&d.name_template).to_owned(),
            window_shadow: bool_or("window_shadow", d.window_shadow),
            logical_size: bool_or("logical_size", d.logical_size),
            card_seconds: num_or("card_seconds", d.card_seconds as u64) as u32,
            card_side: if v["card_side"].as_str() == Some("right") { Side::Right } else { Side::Left },
            tray: bool_or("tray", d.tray),
            keep_warm: bool_or("keep_warm", d.keep_warm),
            max_cards: (num_or("max_cards", d.max_cards as u64) as usize).clamp(1, 8),
            video_dir: dir("video_dir"),
            fps: num_or("fps", d.fps as u64) as u32,
            quality: (num_or("quality", d.quality as u64) as usize).min(QUALITIES.len() - 1),
            region_frame: bool_or("region_frame", d.region_frame),
            editor_window: bool_or("editor_window", d.editor_window),
            cleanup_days: num_or("cleanup_days", d.cleanup_days as u64) as u32,
            language: crate::i18n::Lang::from_code(v["language"].as_str().unwrap_or("en")),
            theme: match v["theme"].as_str() {
                Some("dark") => ThemeMode::Dark,
                Some("light") => ThemeMode::Light,
                _ => ThemeMode::System,
            },
            studio_zoom: v["studio_zoom"].as_f64().unwrap_or(d.studio_zoom).clamp(1.0, 4.0),
            studio_backdrop: num_or("studio_backdrop", d.studio_backdrop as u64) as usize,
            studio_padding: num_or("studio_padding", d.studio_padding as u64) as usize,
        }
    }

    pub fn save(&self) {
        let dir = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
        let v = json!({
            "screenshot_dir": dir(&self.screenshot_dir),
            "after_shot": self.after_shot.json(),
            "after_video": self.after_video.json(),
            "name_template": self.name_template,
            "window_shadow": self.window_shadow,
            "logical_size": self.logical_size,
            "card_seconds": self.card_seconds,
            "card_side": if self.card_side == Side::Right { "right" } else { "left" },
            "tray": self.tray,
            "keep_warm": self.keep_warm,
            "max_cards": self.max_cards,
            "video_dir": dir(&self.video_dir),
            "fps": self.fps,
            "quality": self.quality,
            "region_frame": self.region_frame,
            "editor_window": self.editor_window,
            "cleanup_days": self.cleanup_days,
            "language": self.language.code(),
            "theme": match self.theme {
                ThemeMode::System => "system",
                ThemeMode::Dark => "dark",
                ThemeMode::Light => "light",
            },
            "studio_zoom": self.studio_zoom,
            "studio_backdrop": self.studio_backdrop,
            "studio_padding": self.studio_padding,
        });
        let _ = std::fs::create_dir_all(dirs::config());
        let _ = std::fs::write(path(), serde_json::to_string_pretty(&v).unwrap_or_default());
    }

    pub fn video_dir(&self) -> PathBuf {
        self.video_dir.clone().unwrap_or_else(dirs::videos)
    }

    /// The system's screenshots folder (Omarchy's) unless overridden.
    pub fn screenshot_dir(&self) -> PathBuf {
        self.screenshot_dir.clone().unwrap_or_else(dirs::pictures)
    }
}
