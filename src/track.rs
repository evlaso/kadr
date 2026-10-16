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

use gtk4 as gtk;
use gtk::glib;
use serde_json::json;

use crate::util;

/// finalize() drops the first 0.1 s of the recording.
const TRIM: f64 = 0.1;

pub fn input_log() -> PathBuf {
    glib::user_runtime_dir().join("kadr-input.log")
}

pub fn path_for(video: &Path) -> PathBuf {
    let name = video.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    glib::user_cache_dir().join("kadr").join("tracks").join(format!("{name}.json"))
}

#[derive(Default, Debug, Clone)]
pub struct Track {
    /// Recorded area in Hyprland's logical layout coordinates.
    pub rect: (f64, f64, f64, f64),
    /// (t, x, y) cursor samples; t is seconds into the final video.
    pub cursor: Vec<(f64, f64, f64)>,
    pub clicks: Vec<(f64, f64, f64)>,
    pub keys: Vec<f64>,
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
    source: Option<glib::SourceId>,
}

impl Tracker {
    /// Samples at ~60 Hz. `clock` returns seconds of recorded video so far, or
    /// None while paused.
    pub fn start(clock: impl Fn() -> Option<f64> + 'static) -> Tracker {
        let log = input_log();
        let _ = std::fs::write(&log, b"");
        let data = Rc::new(RefCell::new(Track::default()));
        let offset = Cell::new(0u64);
        let last_key = Cell::new(f64::NEG_INFINITY);
        let source = glib::timeout_add_local(Duration::from_millis(16), glib::clone!(#[strong] data, move || {
            let Some(t) = clock() else { return glib::ControlFlow::Continue };
            let t = t - TRIM;
            let mut d = data.borrow_mut();
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
                    Some("k") if t - last_key.get() > 0.05 => {
                        d.keys.push(t);
                        last_key.set(t);
                    }
                    _ => {}
                }
            }
            glib::ControlFlow::Continue
        }));
        Tracker { data, source: Some(source) }
    }

    fn stop(&mut self) {
        if let Some(id) = self.source.take() {
            id.remove();
        }
        let _ = std::fs::remove_file(input_log());
    }

    pub fn save(mut self, rect: (i32, i32, i32, i32), video: &Path) {
        self.stop();
        let d = self.data.borrow();
        let r = |v: f64| (v * 1000.0).round() / 1000.0;
        let v = json!({
            "rect": [rect.0, rect.1, rect.2, rect.3],
            "cursor": d.cursor.iter().map(|&(t, x, y)| [r(t), r(x), r(y)]).collect::<Vec<_>>(),
            "clicks": d.clicks.iter().map(|&(t, x, y)| [r(t), r(x), r(y)]).collect::<Vec<_>>(),
            "keys": d.keys.iter().map(|&t| r(t)).collect::<Vec<_>>(),
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
        self.stop();
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
