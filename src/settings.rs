//! User settings, ~/.config/kadr/settings.json. Read where they're used (kadr
//! runs briefly, so there's nothing to keep in sync) and written by the
//! settings window on every change.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::util;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// gpu-screen-recorder's quality presets.
pub const QUALITIES: [(&str, &str); 4] = [("medium", "Среднее"), ("high", "Высокое"), ("very_high", "Очень высокое"), ("ultra", "Максимальное")];
pub const CARD_SECONDS: [u32; 4] = [5, 10, 20, 0];
pub const MAX_CARDS: [usize; 3] = [2, 4, 6];
pub const FPS: [u32; 2] = [30, 60];

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// None keeps Omarchy's screenshot folder.
    pub screenshot_dir: Option<PathBuf>,
    pub copy_screenshot: bool,
    pub show_card: bool,
    /// 0 keeps cards until closed.
    pub card_seconds: u32,
    pub card_side: Side,
    pub max_cards: usize,
    /// None keeps the XDG videos folder.
    pub video_dir: Option<PathBuf>,
    pub fps: u32,
    /// Index into `QUALITIES`.
    pub quality: usize,
    pub region_frame: bool,
    pub studio_zoom: f64,
    pub studio_backdrop: usize,
    pub studio_padding: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            screenshot_dir: None,
            copy_screenshot: true,
            show_card: true,
            card_seconds: 10,
            card_side: Side::Left,
            max_cards: 4,
            video_dir: None,
            fps: 60,
            quality: 2,
            region_frame: true,
            studio_zoom: 1.8,
            studio_backdrop: 0,
            studio_padding: 2,
        }
    }
}

fn path() -> PathBuf {
    util::config_dir().join("settings.json")
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
            copy_screenshot: bool_or("copy_screenshot", d.copy_screenshot),
            show_card: bool_or("show_card", d.show_card),
            card_seconds: num_or("card_seconds", d.card_seconds as u64) as u32,
            card_side: if v["card_side"].as_str() == Some("right") { Side::Right } else { Side::Left },
            max_cards: (num_or("max_cards", d.max_cards as u64) as usize).clamp(1, 8),
            video_dir: dir("video_dir"),
            fps: num_or("fps", d.fps as u64) as u32,
            quality: (num_or("quality", d.quality as u64) as usize).min(QUALITIES.len() - 1),
            region_frame: bool_or("region_frame", d.region_frame),
            studio_zoom: v["studio_zoom"].as_f64().unwrap_or(d.studio_zoom).clamp(1.0, 4.0),
            studio_backdrop: num_or("studio_backdrop", d.studio_backdrop as u64) as usize,
            studio_padding: num_or("studio_padding", d.studio_padding as u64) as usize,
        }
    }

    pub fn save(&self) {
        let dir = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
        let v = json!({
            "screenshot_dir": dir(&self.screenshot_dir),
            "copy_screenshot": self.copy_screenshot,
            "show_card": self.show_card,
            "card_seconds": self.card_seconds,
            "card_side": if self.card_side == Side::Right { "right" } else { "left" },
            "max_cards": self.max_cards,
            "video_dir": dir(&self.video_dir),
            "fps": self.fps,
            "quality": self.quality,
            "region_frame": self.region_frame,
            "studio_zoom": self.studio_zoom,
            "studio_backdrop": self.studio_backdrop,
            "studio_padding": self.studio_padding,
        });
        let _ = std::fs::create_dir_all(util::config_dir());
        let _ = std::fs::write(path(), serde_json::to_string_pretty(&v).unwrap_or_default());
    }

    pub fn video_dir(&self) -> PathBuf {
        self.video_dir.clone().unwrap_or_else(|| util::user_dir("VIDEOS", "Videos"))
    }

    /// Where Omarchy saves screenshots unless overridden.
    pub fn screenshot_dir(&self) -> PathBuf {
        self.screenshot_dir.clone().unwrap_or_else(|| {
            std::env::var_os("OMARCHY_SCREENSHOT_DIR").map(PathBuf::from).unwrap_or_else(|| util::user_dir("PICTURES", "Pictures"))
        })
    }
}
