//! Cursor, click and key activity captured during a recording, saved next to
//! the video (in the cache) so the studio render can zoom to where things happen.
//!
//! Clicks and key presses come from a small Hyprland Lua hook (installed as
//! ~/.config/hypr/kadr.lua) that appends to `input_log()` only while that
//! file exists, i.e. while kadr is recording.

use std::cell::{Cell, RefCell};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui::{Context, Task};
use serde_json::json;

use crate::util;
pub use kadr_media::track::*;

/// finalize() drops the first 0.1 s of the recording.
const TRIM: f64 = 0.1;

pub struct Tracker {
    data: Rc<RefCell<Track>>,
    _task: Task<()>,
}

impl Tracker {
    /// Samples at ~60 Hz on the entity's context. `clock` returns seconds of
    /// recorded video so far, or None while paused. With `shapes` it also
    /// notes the pointer's shape, for the cursor the studio draws.
    pub fn start<T: 'static>(cx: &mut Context<T>, show_keys: bool, shapes: bool, clock: impl Fn(&T) -> Option<f64> + 'static) -> Tracker {
        let log = input_log();
        let _ = std::fs::write(&log, b"");
        if show_keys {
            let _ = std::fs::write(keys_flag(), b"");
        }
        let data = Rc::new(RefCell::new(Track::default()));
        let shared = data.clone();
        let watch = shapes.then(crate::platform::CursorShapes::start).flatten();
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
                // Stamped with when it was seen, not when it's read here.
                if let Some((shape, ago)) = watch.as_ref().and_then(|w| w.current()) {
                    if d.shapes.last().is_none_or(|l| l.1 != shape) {
                        let seen = (t - ago).max(d.shapes.last().map_or(0., |l| l.0)).max(0.);
                        d.shapes.push((seen, shape.to_owned()));
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
            "shapes": d.shapes.iter().map(|(t, s)| json!([r(*t), s])).collect::<Vec<_>>(),
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
