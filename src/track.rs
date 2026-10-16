//! Cursor, click and key activity captured during a recording, saved next to
//! the video (in the cache) so the studio render can zoom to where things happen.
//!
//! Clicks and key presses come from a small Hyprland Lua hook (installed as
//! ~/.config/hypr/kadr.lua) that appends to `input_log()` only while that
//! file exists, i.e. while kadr is recording.

use std::cell::{Cell, RefCell};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{Context, Task};
use serde_json::json;

use crate::util;

/// finalize() drops the first 0.1 s of the recording.
const TRIM: f64 = 0.1;

/// While this file exists the Lua hook logs shortcut key codes too.
pub fn keys_flag() -> PathBuf {
    util::runtime_dir().join("kadr-keys.on")
}

pub fn input_log() -> PathBuf {
    util::runtime_dir().join("kadr-input.log")
}

pub fn path_for(video: &Path) -> PathBuf {
    let name = video.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    util::cache_dir().join("tracks").join(format!("{name}.json"))
}

#[derive(Default, Debug, Clone)]
pub struct Track {
    /// Recorded area in Hyprland's logical layout coordinates.
    pub rect: (f64, f64, f64, f64),
    /// (t, x, y) cursor samples; t is seconds into the final video.
    pub cursor: Vec<(f64, f64, f64)>,
    pub clicks: Vec<(f64, f64, f64)>,
    pub keys: Vec<f64>,
    /// Shortcuts pressed, as shown on screen: (t, "Ctrl + C").
    pub keystrokes: Vec<(f64, String)>,
    /// Recorded without the system cursor: the studio draws one.
    pub cursor_hidden: bool,
}

impl Track {
    pub fn load(video: &Path) -> Option<Track> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path_for(video)).ok()?).ok()?;
        let r = v["rect"].as_array()?;
        let f = |x: &serde_json::Value| x.as_f64().unwrap_or(0.0);
        let triples = |key: &str| -> Vec<(f64, f64, f64)> {
            v[key].as_array().into_iter().flatten().filter_map(|p| {
                let p = p.as_array()?;
                Some((f(p.first()?), f(p.get(1)?), f(p.get(2)?)))
            }).collect()
        };
        Some(Track {
            rect: (f(&r[0]), f(&r[1]), f(&r[2]), f(&r[3])),
            cursor: triples("cursor"),
            clicks: triples("clicks"),
            keys: v["keys"].as_array().into_iter().flatten().map(f).collect(),
            keystrokes: v["keystrokes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|k| Some((k[0].as_f64()?, k[1].as_str()?.to_owned())))
                .collect(),
            cursor_hidden: v["cursor_hidden"].as_bool().unwrap_or(false),
        })
    }

    /// Cursor position at time t (linear between samples).
    pub fn cursor_at(&self, t: f64) -> Option<(f64, f64)> {
        let i = self.cursor.partition_point(|s| s.0 <= t);
        match (i.checked_sub(1).map(|j| self.cursor[j]), self.cursor.get(i)) {
            (Some(a), Some(b)) if b.0 > a.0 => {
                let k = (t - a.0) / (b.0 - a.0);
                Some((a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k))
            }
            (Some(a), _) => Some((a.1, a.2)),
            (None, Some(b)) => Some((b.1, b.2)),
            (None, None) => None,
        }
    }
}

pub struct Tracker {
    data: Rc<RefCell<Track>>,
    _task: Task<()>,
}

impl Tracker {
    /// Samples at ~60 Hz on the entity's context. `clock` returns seconds of
    /// recorded video so far, or None while paused.
    pub fn start<T: 'static>(cx: &mut Context<T>, show_keys: bool, clock: impl Fn(&T) -> Option<f64> + 'static) -> Tracker {
        let log = input_log();
        let _ = std::fs::write(&log, b"");
        if show_keys {
            let _ = std::fs::write(keys_flag(), b"");
        }
        let data = Rc::new(RefCell::new(Track::default()));
        let shared = data.clone();
        let task = cx.spawn(async move |this, cx| {
            let offset = Cell::new(0u64);
            let mut last_key = f64::NEG_INFINITY;
            let mut held: Vec<u32> = Vec::new();
            loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let Ok(t) = this.update(cx, |owner, _| clock(owner)) else { break };
                let Some(t) = t else { continue };
                let t = t - TRIM;
                let mut d = shared.borrow_mut();
                if let Some((x, y)) = util::cursor_pos() {
                    let moved = d.cursor.last().is_none_or(|l| l.1 != x || l.2 != y || t - l.0 > 0.5);
                    if moved {
                        d.cursor.push((t, x, y));
                    }
                }
                for line in read_new(&log, &offset).lines() {
                    let mut parts = line.split_whitespace();
                    let kind = parts.next();
                    let mut num = || parts.next().and_then(|s| s.parse::<f64>().ok());
                    let (x, y) = (num(), num());
                    match kind {
                        Some("c") => {
                            let pos = x.zip(y).or_else(|| d.cursor.last().map(|l| (l.1, l.2)));
                            if let Some((x, y)) = pos {
                                d.clicks.push((t, x, y));
                            }
                        }
                        // Hyprland reports press and release; one per key is enough.
                        Some("k") if t - last_key > 0.05 => {
                            d.keys.push(t);
                            last_key = t;
                        }
                        // x CODE p|r: modifiers and shortcut keys (show keys on).
                        Some("x") => {
                            let (Some(code), pressed) = (x.map(|c| c as u32), line.ends_with(" p")) else { continue };
                            if is_modifier(code) {
                                held.retain(|&m| m != code);
                                if pressed {
                                    held.push(code);
                                }
                            } else if pressed {
                                d.keys.push(t);
                                last_key = t;
                                if let Some(label) = shortcut_label(&held, code) {
                                    d.keystrokes.push((t, label));
                                }
                            }
                        }
                        Some("?") => eprintln!("kadr: key hook: {line}"),
                        _ => {}
                    }
                }
            }
        });
        Tracker { data, _task: task }
    }

    pub fn save(self, rect: (i32, i32, i32, i32), cursor_hidden: bool, video: &Path) {
        let d = self.data.borrow();
        let r = |v: f64| (v * 1000.0).round() / 1000.0;
        let v = json!({
            "rect": [rect.0, rect.1, rect.2, rect.3],
            "cursor": d.cursor.iter().map(|&(t, x, y)| [r(t), r(x), r(y)]).collect::<Vec<_>>(),
            "clicks": d.clicks.iter().map(|&(t, x, y)| [r(t), r(x), r(y)]).collect::<Vec<_>>(),
            "keys": d.keys.iter().map(|&t| r(t)).collect::<Vec<_>>(),
            "keystrokes": d.keystrokes.iter().map(|(t, k)| json!([r(*t), k])).collect::<Vec<_>>(),
            "cursor_hidden": cursor_hidden,
        });
        let path = path_for(video);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, v.to_string()) {
            eprintln!("kadr: {}: {e}", path.display());
        }
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        // Stops the Lua hook from logging; the task is cancelled with `_task`.
        let _ = std::fs::remove_file(input_log());
        let _ = std::fs::remove_file(keys_flag());
    }
}

fn read_new(path: &Path, offset: &Cell<u64>) -> String {
    let mut out = String::new();
    if let Ok(mut f) = std::fs::File::open(path) {
        if f.seek(SeekFrom::Start(offset.get())).is_ok() && f.read_to_string(&mut out).is_ok() {
            // Keep a trailing partial line for the next round.
            let complete = out.rfind('\n').map_or(0, |i| i + 1);
            out.truncate(complete);
            offset.set(offset.get() + complete as u64);
        }
    }
    out
}

/// Carry a recording's track over to a trimmed copy covering `start..end`
/// seconds of the original.
pub fn trim(original: &Path, trimmed: &Path, start: f64, end: f64) {
    let Ok(text) = std::fs::read_to_string(path_for(original)) else { return };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    let keep = |t: f64| t >= start && t <= end;
    for key in ["cursor", "clicks"] {
        if let Some(list) = v[key].as_array_mut() {
            list.retain(|p| p[0].as_f64().is_some_and(keep));
            for p in list.iter_mut() {
                p[0] = json!(p[0].as_f64().unwrap_or(0.) - start);
            }
        }
    }
    if let Some(list) = v["keystrokes"].as_array_mut() {
        list.retain(|k| k[0].as_f64().is_some_and(keep));
        for k in list.iter_mut() {
            k[0] = json!(k[0].as_f64().unwrap_or(0.) - start);
        }
    }
    if let Some(keys) = v["keys"].as_array_mut() {
        keys.retain(|t| t.as_f64().is_some_and(keep));
        for t in keys.iter_mut() {
            *t = json!(t.as_f64().unwrap_or(0.) - start);
        }
    }
    let _ = std::fs::write(path_for(trimmed), v.to_string());
}

/// evdev key codes.
fn is_modifier(code: u32) -> bool {
    matches!(code, 29 | 97 | 42 | 54 | 56 | 100 | 125 | 126)
}

/// "Ctrl + Shift + T" for a key pressed with `held` modifiers; None for keys
/// that shouldn't be shown on their own.
fn shortcut_label(held: &[u32], code: u32) -> Option<String> {
    let has = |codes: &[u32]| held.iter().any(|m| codes.contains(m));
    let mut parts: Vec<&str> = Vec::new();
    if has(&[125, 126]) {
        parts.push("Super");
    }
    if has(&[29, 97]) {
        parts.push("Ctrl");
    }
    if has(&[56, 100]) {
        parts.push("Alt");
    }
    if has(&[42, 54]) {
        parts.push("Shift");
    }
    let key = key_name(code)?;
    let shortcut = parts.iter().any(|p| *p != "Shift");
    let special = !(key.chars().count() == 1 && key.chars().all(|c| c.is_alphanumeric()));
    // A bare letter (or Shift+letter) is typing, not a shortcut.
    if !shortcut && !special {
        return None;
    }
    parts.push(key);
    Some(parts.join(" + "))
}

fn key_name(code: u32) -> Option<&'static str> {
    const ROW1: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
    const QWERTY: [&str; 10] = ["Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P"];
    const ASDF: [&str; 9] = ["A", "S", "D", "F", "G", "H", "J", "K", "L"];
    const ZXCV: [&str; 7] = ["Z", "X", "C", "V", "B", "N", "M"];
    const FKEYS: [&str; 10] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10"];
    Some(match code {
        1 => "Esc",
        2..=11 => ROW1[(code - 2) as usize],
        12 => "-",
        13 => "=",
        14 => "⌫",
        15 => "Tab",
        16..=25 => QWERTY[(code - 16) as usize],
        26 => "[",
        27 => "]",
        28 | 96 => "Enter",
        30..=38 => ASDF[(code - 30) as usize],
        39 => ";",
        40 => "'",
        41 => "`",
        43 => "\\",
        44..=50 => ZXCV[(code - 44) as usize],
        51 => ",",
        52 => ".",
        53 => "/",
        57 => "Space",
        59..=68 => FKEYS[(code - 59) as usize],
        87 => "F11",
        88 => "F12",
        102 => "Home",
        103 => "↑",
        104 => "PgUp",
        105 => "←",
        106 => "→",
        107 => "End",
        108 => "↓",
        109 => "PgDn",
        110 => "Ins",
        111 => "Del",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_labels() {
        assert_eq!(shortcut_label(&[29], 46).as_deref(), Some("Ctrl + C"));
        assert_eq!(shortcut_label(&[29, 42], 20).as_deref(), Some("Ctrl + Shift + T"));
        assert_eq!(shortcut_label(&[125], 28).as_deref(), Some("Super + Enter"));
        assert_eq!(shortcut_label(&[], 1).as_deref(), Some("Esc"));
        assert_eq!(shortcut_label(&[42], 103).as_deref(), Some("Shift + ↑"));
        assert_eq!(shortcut_label(&[], 30), None, "plain typing stays hidden");
        assert_eq!(shortcut_label(&[42], 30), None, "Shift+letter is typing too");
    }
}
