//! User settings, `settings.toml` in the config folder (~/.config/kadr), to
//! change in the settings window or by hand. Read where they're used (kadr
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
/// The self-timer's choices, in seconds.
pub const TIMER_SECONDS: [u32; 3] = [3, 5, 10];
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
    /// The editor's saved look (by name) window captures are put on, with
    /// its background, margin and shadow; None keeps them as they are.
    pub window_backdrop: Option<String>,
    /// Screenshots are scaled down to logical pixels (1×), so a pixel of the
    /// picture is a pixel of the design; off keeps every screen pixel.
    pub logical_size: bool,
    /// Screenshots have the pointer in them.
    pub shot_cursor: bool,
    /// How long the self-timer counts down (`kadr shot timer`), one of
    /// `TIMER_SECONDS`.
    pub timer_seconds: u32,
    /// 0 keeps cards until closed.
    pub card_seconds: u32,
    pub card_side: Side,
    /// The `kadr tray` icon.
    pub tray: bool,
    /// The tray keeps the area overlay running, warm, so it shows at once
    /// (SPEC 3.6): on by default on macOS, where starting it cold is slow.
    pub keep_warm: bool,
    /// The tray looks for a newer release (`update`) and kadr shows a card.
    pub check_updates: bool,
    /// Which releases it looks for; None follows this build (a pre-release
    /// is on Beta).
    pub update_channel: Option<crate::update::Channel>,
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
            window_backdrop: None,
            logical_size: true,
            timer_seconds: 3,
            shot_cursor: false,
            card_seconds: 10,
            card_side: Side::Left,
            tray: true,
            keep_warm: cfg!(target_os = "macos"),
            check_updates: true,
            update_channel: None,
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

/// The settings file. TOML, so it reads and edits well by hand; the
/// settings window keeps what a person wrote in it (comments, order) and
/// changes only the values it changed.
pub fn path() -> PathBuf {
    dirs::config().join("settings.toml")
}

/// Where settings were kept before 0.4, moved to `path()` on first read.
fn old_path() -> PathBuf {
    dirs::config().join("settings.json")
}

/// The file's version, for migrations to come (SPEC §5).
const VERSION: u64 = 1;

/// What the file starts with when kadr writes it.
const INTRO: &str = "\
# kadr's settings. Change them here or in kadr's settings window: either way
# kadr picks them up the next time it needs them. A setting left out keeps
# its default; one kadr can't read is ignored.
";

/// The file's settings in the order kadr writes them, each with what it
/// says above it and, for one that's off by default, an example. An empty
/// key starts a group.
const KEYS: &[(&str, &str, &str)] = &[
    ("version", "The file's format.", ""),
    ("", "General", ""),
    ("language", "\"en\" or \"ru\".", ""),
    ("theme", "\"system\", \"dark\" or \"light\".", ""),
    ("tray", "The tray icon.", ""),
    ("keep_warm", "Keep the area picker running so it shows at once (uses some memory).", ""),
    ("check_updates", "Look for new releases.", ""),
    ("update_channel", "\"stable\" or \"beta\"; left out, the channel of this build.", "\"beta\""),
    ("", "Captures", ""),
    ("screenshot_dir", "Where screenshots go; left out, the system's pictures folder.", "\"~/Pictures/Screenshots\""),
    ("video_dir", "Where recordings go; left out, the system's videos folder.", "\"~/Videos\""),
    ("name_template", "New files' names: {type}, {date}, {time} and {n} are filled in.", ""),
    ("card_seconds", "How long the card after a capture stays, in seconds; 0 keeps it.", ""),
    ("card_side", "Where the cards show: \"left\" or \"right\".", ""),
    ("max_cards", "How many cards show at once.", ""),
    ("cleanup_days", "Delete captures older than this many days; 0 keeps them.", ""),
    ("", "Screenshots", ""),
    ("logical_size", "Save at 1x: a pixel of the picture is a pixel of the design.", ""),
    ("shot_cursor", "Put the pointer in screenshots.", ""),
    ("window_shadow", "Window shots keep the window's shadow, on a transparent background.", ""),
    ("window_backdrop", "A look saved in the editor, by name, for window shots to go on.", "\"Ocean\""),
    ("timer_seconds", "The self-timer: 3, 5 or 10 seconds.", ""),
    ("editor_window", "The editor opens as an ordinary window; false puts it over everything (Linux).", ""),
    ("", "Recording", ""),
    ("fps", "Frames a second; 0 is the screen's own rate.", ""),
    ("quality", "\"medium\", \"high\", \"very_high\" or \"ultra\".", ""),
    ("region_frame", "Outline the recorded area (the outline isn't recorded).", ""),
    ("", "Studio", ""),
    ("studio_zoom", "The zoom placed on clicks, 1 to 4.", ""),
    ("studio_backdrop", "The background, by its place in the studio's list (from 0).", ""),
    ("studio_padding", "How much background shows around: 0 (none) to 3.", ""),
];

/// The two tables at the end: what happens after a capture.
const AFTER: [(&str, &str); 2] = [("after_shot", "a screenshot"), ("after_video", "a recording")];

/// Their switches, each with what it does.
const AFTER_KEYS: [(&str, &str); 5] = [
    ("card", "the card in the screen's corner"),
    ("copy", "copy it to the clipboard"),
    ("save", "keep the file (off: only a temporary one)"),
    ("edit", "open it in the editor or the studio"),
    ("pin", "pin a screenshot above every window"),
];

/// A path from the file, with `~/` for the home folder.
fn expand(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).map_or_else(|| PathBuf::from(path), |home| home.join(rest)),
        None => PathBuf::from(path),
    }
}

/// The file as JSON, which `from_json` reads: TOML's tables and arrays as
/// objects and arrays, dates as strings.
fn item_json(item: &toml_edit::Item) -> Value {
    match item {
        toml_edit::Item::Value(v) => value_json(v),
        toml_edit::Item::Table(t) => Value::Object(t.iter().map(|(k, i)| (k.to_owned(), item_json(i))).collect()),
        toml_edit::Item::ArrayOfTables(a) => Value::Array(a.iter().map(|t| Value::Object(t.iter().map(|(k, i)| (k.to_owned(), item_json(i))).collect())).collect()),
        toml_edit::Item::None => Value::Null,
    }
}

fn value_json(v: &toml_edit::Value) -> Value {
    use toml_edit::Value as T;
    match v {
        T::String(s) => Value::String(s.value().clone()),
        T::Integer(i) => json!(*i.value()),
        T::Float(f) => json!(*f.value()),
        T::Boolean(b) => Value::Bool(*b.value()),
        T::Datetime(d) => Value::String(d.value().to_string()),
        T::Array(a) => Value::Array(a.iter().map(value_json).collect()),
        T::InlineTable(t) => Value::Object(t.iter().map(|(k, v)| (k.to_owned(), value_json(v))).collect()),
    }
}

/// A setting's value as TOML; None for one that's left out (null).
fn toml_value(v: &Value) -> Option<toml_edit::Value> {
    Some(match v {
        Value::Null | Value::Object(_) => return None,
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64()?.into(),
        },
        Value::String(s) => s.as_str().into(),
        Value::Array(a) => toml_edit::Value::Array(a.iter().filter_map(toml_value).collect()),
    })
}

/// Why the settings file can't be read, in a line or two; None when it can
/// (or there's none yet).
pub fn problem() -> Option<String> {
    read().err()
}

/// The settings file's contents, or why it can't be read. Settings kept
/// the old way are moved over first.
fn read() -> Result<Option<Value>, String> {
    match std::fs::read_to_string(path()) {
        Ok(text) => {
            let doc = text.parse::<toml_edit::DocumentMut>().map_err(|e| {
                // Its first line says where, its last what's wrong.
                let e = e.to_string();
                let lines: Vec<&str> = e.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
                match (lines.first(), lines.last()) {
                    (Some(a), Some(b)) if a != b => format!("{a}: {b}"),
                    _ => e.trim().to_owned(),
                }
            })?;
            Ok(Some(item_json(doc.as_item())))
        }
        Err(_) => {
            let Some(v) = std::fs::read_to_string(old_path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
                return Ok(None);
            };
            let settings = Settings::from_json(&v);
            if write(&template(&settings.to_json())).is_ok() {
                let _ = std::fs::rename(old_path(), dirs::config().join("settings.json.old"));
            }
            Ok(Some(settings.to_json()))
        }
    }
}

/// Put the file in place whole, so nothing reads it half written.
fn write(text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dirs::config())?;
    let tmp = dirs::config().join(format!(".settings.toml.{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path())
}

/// The file as kadr writes it the first time: every setting, each with what
/// it does.
fn template(v: &Value) -> String {
    let mut out = String::from(INTRO);
    for &(key, about, example) in KEYS {
        if key.is_empty() {
            out += &format!("\n# --- {about} ---\n");
            continue;
        }
        out += &format!("\n# {about}\n");
        match toml_value(&v[key]) {
            Some(value) => out += &format!("{key} = {}\n", value.to_string().trim()),
            None => out += &format!("# {key} = {example}\n"),
        }
    }
    for (table, what) in AFTER {
        out += &format!("\n# --- What happens after {what}, any of these at once ---\n[{table}]\n");
        for (k, about) in AFTER_KEYS {
            if let Some(value) = toml_value(&v[table][k]) {
                let line = format!("{k} = {}", value.to_string().trim());
                out += &format!("{line:<14}# {about}\n");
            }
        }
    }
    out
}

/// Write into `table` the settings that differ between `old` and `new`,
/// keeping the comments around them.
fn merge(table: &mut dyn toml_edit::TableLike, old: &Value, new: &Value) {
    for (key, value) in new.as_object().into_iter().flatten() {
        if old.get(key) == Some(value) {
            continue;
        }
        if value.is_object() {
            if !table.get(key).is_some_and(|i| i.is_table_like()) {
                table.insert(key, toml_edit::Item::Table(toml_edit::Table::new()));
            }
            if let Some(sub) = table.get_mut(key).and_then(|i| i.as_table_like_mut()) {
                merge(sub, &old[key], value);
            }
            continue;
        }
        match (toml_value(value), table.get_mut(key)) {
            (None, _) => drop(table.remove(key)),
            (Some(mut value), Some(item)) => {
                if let Some(was) = item.as_value() {
                    *value.decor_mut() = was.decor().clone();
                }
                *item = toml_edit::Item::Value(value);
            }
            (Some(value), None) => drop(table.insert(key, toml_edit::Item::Value(value))),
        }
    }
}

impl Settings {
    pub fn load() -> Settings {
        match read() {
            Ok(Some(v)) => Settings::from_json(&v),
            _ => Settings::default(),
        }
    }

    fn from_json(v: &Value) -> Settings {
        let d = Settings::default();
        let dir = |key: &str| v[key].as_str().filter(|s| !s.is_empty()).map(expand);
        let bool_or = |key: &str, default: bool| v[key].as_bool().unwrap_or(default);
        let num_or = |key: &str, default: u64| v[key].as_u64().unwrap_or(default);
        Settings {
            screenshot_dir: dir("screenshot_dir"),
            // Before the after-capture choices there were two switches.
            after_shot: After::load(&v["after_shot"], After { card: bool_or("show_card", d.after_shot.card), copy: bool_or("copy_screenshot", d.after_shot.copy), ..d.after_shot }),
            after_video: After::load(&v["after_video"], After { card: bool_or("show_card", d.after_video.card), ..d.after_video }),
            name_template: v["name_template"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or(&d.name_template).to_owned(),
            window_shadow: bool_or("window_shadow", d.window_shadow),
            window_backdrop: v["window_backdrop"].as_str().filter(|s| !s.is_empty()).map(str::to_owned),
            logical_size: bool_or("logical_size", d.logical_size),
            shot_cursor: bool_or("shot_cursor", d.shot_cursor),
            timer_seconds: Some(num_or("timer_seconds", d.timer_seconds as u64) as u32).filter(|n| TIMER_SECONDS.contains(n)).unwrap_or(d.timer_seconds),
            card_seconds: num_or("card_seconds", d.card_seconds as u64) as u32,
            card_side: if v["card_side"].as_str() == Some("right") { Side::Right } else { Side::Left },
            tray: bool_or("tray", d.tray),
            keep_warm: bool_or("keep_warm", d.keep_warm),
            check_updates: bool_or("check_updates", d.check_updates),
            update_channel: v["update_channel"].as_str().and_then(crate::update::Channel::from_key),
            max_cards: (num_or("max_cards", d.max_cards as u64) as usize).clamp(1, 8),
            video_dir: dir("video_dir"),
            fps: num_or("fps", d.fps as u64) as u32,
            // By name; settings.json kept its place in the list.
            quality: match &v["quality"] {
                Value::String(name) => QUALITIES.iter().position(|(q, _)| q == name).unwrap_or(d.quality),
                q => (q.as_u64().unwrap_or(d.quality as u64) as usize).min(QUALITIES.len() - 1),
            },
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

    fn to_json(&self) -> Value {
        let dir = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string());
        json!({
            "version": VERSION,
            "screenshot_dir": dir(&self.screenshot_dir),
            "after_shot": self.after_shot.json(),
            "after_video": self.after_video.json(),
            "name_template": self.name_template,
            "window_shadow": self.window_shadow,
            "window_backdrop": self.window_backdrop,
            "logical_size": self.logical_size,
            "timer_seconds": self.timer_seconds,
            "shot_cursor": self.shot_cursor,
            "card_seconds": self.card_seconds,
            "card_side": if self.card_side == Side::Right { "right" } else { "left" },
            "tray": self.tray,
            "keep_warm": self.keep_warm,
            "check_updates": self.check_updates,
            "update_channel": self.update_channel.map(crate::update::Channel::key),
            "max_cards": self.max_cards,
            "video_dir": dir(&self.video_dir),
            "fps": self.fps,
            "quality": QUALITIES[self.quality.min(QUALITIES.len() - 1)].0,
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
        })
    }

    /// Write the settings: the whole file the first time, after that only
    /// what changed from what the file says, so a person's edits and
    /// comments stay. A file kadr can't read is left alone (`problem` says
    /// why).
    pub fn save(&self) {
        let new = self.to_json();
        let Ok(text) = std::fs::read_to_string(path()) else {
            let _ = write(&template(&new));
            return;
        };
        let Ok(mut doc) = text.parse::<toml_edit::DocumentMut>() else { return };
        let old = Settings::from_json(&item_json(doc.as_item())).to_json();
        merge(doc.as_table_mut(), &old, &new);
        let _ = write(&doc.to_string());
    }

    pub fn video_dir(&self) -> PathBuf {
        self.video_dir.clone().unwrap_or_else(dirs::videos)
    }

    /// The system's screenshots folder (Omarchy's) unless overridden.
    pub fn screenshot_dir(&self) -> PathBuf {
        self.screenshot_dir.clone().unwrap_or_else(dirs::pictures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test, since they'd share the file: the old settings moved over,
    /// edits by hand read and kept through a save, a broken file left alone.
    #[test]
    fn the_settings_file_is_kept_as_a_person_wrote_it() {
        let _ = std::fs::remove_file(path());
        std::fs::create_dir_all(dirs::config()).unwrap();
        std::fs::write(old_path(), r#"{"fps": 60, "quality": 3, "theme": "dark", "screenshot_dir": null}"#).unwrap();
        let s = Settings::load();
        assert_eq!((s.fps, s.quality, s.theme), (60, 3, ThemeMode::Dark), "settings.json read");
        assert!(path().exists() && !old_path().exists(), "moved to settings.toml");
        let text = std::fs::read_to_string(path()).unwrap();
        assert!(text.contains("fps = 60") && text.contains("quality = \"ultra\"") && text.contains("# screenshot_dir = "), "{text}");
        assert_eq!(Settings::load(), s, "the template reads back the same");

        // A person's edit and comments.
        let edited = text.replace("fps = 60", "fps = 30 # smoother files").replace("# screenshot_dir = \"~/Pictures/Screenshots\"", "screenshot_dir = \"~/shots\"") + "\n# mine\n";
        std::fs::write(path(), &edited).unwrap();
        let mut s = Settings::load();
        assert_eq!(s.fps, 30);
        assert!(s.screenshot_dir.as_ref().is_some_and(|d| d.ends_with("shots") && d.is_absolute()), "{:?}", s.screenshot_dir);
        s.theme = ThemeMode::Light;
        s.after_shot.pin = true;
        s.save();
        let saved = std::fs::read_to_string(path()).unwrap();
        assert!(saved.contains("fps = 30 # smoother files") && saved.contains("screenshot_dir = \"~/shots\"") && saved.contains("# mine"), "{saved}");
        assert!(saved.contains("theme = \"light\""), "{saved}");
        assert!(Settings::load().after_shot.pin);

        // Broken: the defaults, the reason, and the file as it was.
        std::fs::write(path(), "fps = = 3\n").unwrap();
        assert!(problem().is_some_and(|p| p.contains("line 1")), "{:?}", problem());
        assert_eq!(Settings::load(), Settings::default());
        Settings::default().save();
        assert_eq!(std::fs::read_to_string(path()).unwrap(), "fps = = 3\n");
        let _ = std::fs::remove_file(path());
    }
}
