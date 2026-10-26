//! Test hooks, so a script (or an agent) can drive kadr without a person:
//! `kadr dev <command>` asks the running kadr and prints its JSON answer.
//! They're off unless that kadr was started with `KADR_DEV=1`.
//!
//! - `windows`: kadr's windows: id, kind, title, bounds.
//! - `dump [window]`: the buttons and switches drawn in a window: id, kind,
//!   text, bounds, state.
//! - `find <id> [window]`: one of them.
//! - `click <id> [window] [--at dx,dy]`: a click in its middle (or offset).
//! - `drag <id|x,y> <dx,dy> [window]`: press on an element's middle (or a
//!   point in the window), move by dx,dy and let go.
//! - `key <combo>… [--window id]`: keystrokes, as GPUI writes them
//!   ("ctrl-z", "escape", "a").
//! - `type <text> [--window id]`: text, one key per character.
//! - `state [window]`: what the window's view holds (the editor's drawing,
//!   tool and undo depth; the settings' tab).
//! - `shot <path> [window]`: the window's pixels as a PNG.
//! - `grab-window <id> [block]`: the system's window `id` (kadr-shot's)
//!   taken by itself on another thread, as a window shot is, within 8 s;
//!   with `block` the main thread waits for it meanwhile, as a stuck one
//!   would: whether it came, its size and how long it took.
//! - `mem`: the process's memory, and every live allocation of a megabyte
//!   or more with where it was made (`Counting`, kadr's allocator): how many
//!   copies of a picture are held, and by whom. The frames need line tables:
//!   build with `CARGO_PROFILE_FAST_DEBUG=line-tables-only
//!   CARGO_PROFILE_FAST_STRIP=false`; `KADR_DEV_RAW=1` gives whole stacks.
//!
//! A window is the id from `windows`; by default the active one, or the
//! newest.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::sync::OnceLock;
use std::time::Duration;

use gpui::{AnyWindowHandle, App, Bounds, ElementId, IntoElement, Modifiers, MouseButton, Pixels, PlatformInput, Styled, Window, point, px};
use serde_json::{Value, json};

use crate::platform::ipc::Stream;

#[cfg(feature = "dev")]
mod counting {
    use std::collections::HashMap;

    use serde_json::{Value, json};

    /// The global allocator: the system's, noting each block of `BIG` bytes or
    /// more and its call stack while the dev hooks are on. Pictures are what's
    /// that big, so this counts the copies of them kadr holds.
    pub struct Counting;

    const BIG: usize = 1 << 20;

    static TRACKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static BIG_ONES: std::sync::Mutex<Option<HashMap<usize, (usize, std::backtrace::Backtrace)>>> = std::sync::Mutex::new(None);

    thread_local! {
        /// Set while noting a block, so the notes' own allocations aren't noted.
        static NOTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    fn note(ptr: *mut u8, size: usize) {
        if NOTING.with(|n| n.replace(true)) {
            return;
        }
        let trace = std::backtrace::Backtrace::force_capture();
        if let Ok(mut big) = BIG_ONES.lock() {
            big.get_or_insert_with(HashMap::new).insert(ptr as usize, (size, trace));
        }
        NOTING.with(|n| n.set(false));
    }

    fn forget(ptr: *mut u8) {
        if NOTING.with(|n| n.replace(true)) {
            return;
        }
        if let Ok(mut big) = BIG_ONES.lock()
            && let Some(big) = big.as_mut()
        {
            big.remove(&(ptr as usize));
        }
        NOTING.with(|n| n.set(false));
    }

    // SAFETY: every call goes straight to the system allocator; the notes are
    // kept on the side and never touch the blocks.
    unsafe impl std::alloc::GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
            let ptr = unsafe { std::alloc::System.alloc(layout) };
            if layout.size() >= BIG && TRACKING.load(std::sync::atomic::Ordering::Relaxed) && !ptr.is_null() {
                note(ptr, layout.size());
            }
            ptr
        }

        unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
            let ptr = unsafe { std::alloc::System.alloc_zeroed(layout) };
            if layout.size() >= BIG && TRACKING.load(std::sync::atomic::Ordering::Relaxed) && !ptr.is_null() {
                note(ptr, layout.size());
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
            if layout.size() >= BIG && TRACKING.load(std::sync::atomic::Ordering::Relaxed) {
                forget(ptr);
            }
            unsafe { std::alloc::System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
            let tracking = TRACKING.load(std::sync::atomic::Ordering::Relaxed);
            if tracking && layout.size() >= BIG {
                forget(ptr);
            }
            let new = unsafe { std::alloc::System.realloc(ptr, layout, new_size) };
            if tracking && new_size >= BIG && !new.is_null() {
                note(new, new_size);
            }
            new
        }
    }

    /// Start noting big blocks (when the dev hooks are on).
    pub fn track() {
        if super::on() {
            TRACKING.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// The live big blocks, largest first: size and the frames in kadr's own
    /// code that made it.
    pub(super) fn big_blocks() -> Vec<Value> {
        NOTING.with(|n| n.set(true));
        let mut out: Vec<(usize, Vec<String>)> = BIG_ONES
            .lock()
            .ok()
            .and_then(|big| big.as_ref().map(|big| big.values().map(|(size, trace)| (*size, ours(trace))).collect()))
            .unwrap_or_default();
        NOTING.with(|n| n.set(false));
        out.sort_by(|a, b| b.0.cmp(&a.0));
        out.into_iter().map(|(size, frames)| json!({ "mb": (size as f64 / 1048576. * 10.).round() / 10., "made_by": frames })).collect()
    }

    /// A stack's frames in kadr's own code (`src/`, `crates/`, `vendor/`),
    /// innermost first, as "function @ file:line". Needs line tables in the
    /// build: `CARGO_PROFILE_FAST_DEBUG=line-tables-only CARGO_PROFILE_FAST_STRIP=false`.
    fn ours(trace: &std::backtrace::Backtrace) -> Vec<String> {
        let text = trace.to_string();
        let mut frames = Vec::new();
        let mut name = "";
        for line in text.lines() {
            let line = line.trim();
            if let Some(at) = line.strip_prefix("at ./") {
                if !at.starts_with("src/dev.rs") && ["src/", "crates/", "vendor/"].iter().any(|p| at.starts_with(p)) {
                    let at = at.rsplit_once(':').map_or(at, |(file_line, _)| file_line);
                    frames.push(format!("{name} @ {at}"));
                    if frames.len() == 4 {
                        break;
                    }
                }
            } else if let Some((_, n)) = line.split_once(": ") {
                name = n;
            }
        }
        frames
    }
}

#[cfg(feature = "dev")]
pub use counting::{Counting, track};
#[cfg(feature = "dev")]
use counting::big_blocks;

/// Without `--features dev` there's nothing to note.
#[cfg(not(feature = "dev"))]
pub fn track() {}

#[cfg(not(feature = "dev"))]
fn big_blocks() -> Vec<Value> {
    Vec::new()
}

fn rss_mb() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: f64 = status.lines().find(|l| l.starts_with("VmRSS:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some((kb / 1024. * 10.).round() / 10.)
}

/// When kadr started (`prepare`, first thing in `main`).
static STARTED: OnceLock<std::time::Instant> = OnceLock::new();

/// A step of kadr's start, timed from `prepare`, in KADR_DEV's log:
/// "kadr start: kit=41ms". Only the first of each name counts.
pub fn mark(step: &str) {
    let at = STARTED.get_or_init(std::time::Instant::now).elapsed().as_millis();
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap();
    if on() && !seen.iter().any(|s| s == step) {
        seen.push(step.to_owned());
        eprintln!("kadr start: {step}={at}ms");
    }
}

pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("KADR_DEV").is_some_and(|v| v != "0"))
}

/// GPUI's own log (which GPU it took, why a window couldn't get one) on
/// stderr, for a kadr under test: without a logger it goes nowhere.
pub fn log_to_stderr() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata) -> bool {
            // GPUI's notes, everyone's warnings (zbus's notes are noise).
            m.level() <= log::Level::Warn || m.level() <= log::Level::Info && m.target().starts_with("gpui")
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("{} {}: {}", r.level(), r.target(), r.args());
            }
        }
        fn flush(&self) {}
    }
    if on() && log::set_logger(&Stderr).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// An interactive element as the last frame drew it.
#[derive(Clone)]
struct Node {
    id: String,
    kind: &'static str,
    text: String,
    bounds: Bounds<Pixels>,
    disabled: bool,
    selected: bool,
    hovered: bool,
}

impl Node {
    fn to_json(&self) -> Value {
        let b = self.bounds;
        json!({
            "id": self.id,
            "kind": self.kind,
            "text": self.text,
            "bounds": [f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)],
            "disabled": self.disabled,
            "selected": self.selected,
            "hovered": self.hovered,
        })
    }
}

thread_local! {
    static NODES: RefCell<HashMap<u64, Vec<Node>>> = RefCell::new(HashMap::new());
    static KINDS: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new());
}

/// What a window shows, by its app id or layer namespace ("kadr-editor").
pub fn name_window(window: AnyWindowHandle, kind: &str) {
    if on() {
        KINDS.with_borrow_mut(|k| k.insert(window.window_id().as_u64(), kind.to_owned()));
    }
}

/// Put inside a button or switch (only while `on`): notes where it was drawn.
pub fn probe(id: &ElementId, kind: &'static str, text: String, disabled: bool, selected: bool) -> impl IntoElement + use<> {
    let id = id.to_string();
    gpui::canvas(
        move |bounds, window: &mut Window, _| {
            let hovered = window.is_window_hovered() && bounds.contains(&window.mouse_position());
            let node = Node { id, kind, text, bounds, disabled, selected, hovered };
            NODES.with_borrow_mut(|n| n.entry(window.window_handle().window_id().as_u64()).or_default().push(node));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// Run one `kadr dev …` command and answer on `reply`.
pub fn handle(args: Vec<String>, reply: Stream, cx: &mut App) {
    if !on() {
        return answer(reply, Err("dev hooks are off: start kadr with KADR_DEV=1".into()));
    }
    let (mut words, mut window, mut at) = (Vec::new(), None, None);
    let mut rest = args.iter().skip(2);
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--window" => window = rest.next().and_then(|w| w.parse::<u64>().ok()),
            "--at" => at = rest.next().and_then(|a| pair(a)),
            _ => words.push(arg.clone()),
        }
    }
    let command = args.get(1).cloned().unwrap_or_default();
    // A trailing number names the window, where the command takes no more words.
    let takes = match command.as_str() {
        "dump" | "state" | "windows" | "mem" => 0,
        "find" | "click" | "shot" => 1,
        "drag" => 2,
        _ => usize::MAX,
    };
    if words.len() > takes
        && let Some(id) = words.last().and_then(|w| w.parse::<u64>().ok())
    {
        window = Some(id);
        words.pop();
    }
    if command == "windows" {
        return answer(reply, Ok(windows(cx)));
    }
    if command == "grab-window" {
        let Some(id) = words.first().and_then(|w| w.parse::<u64>().ok()) else {
            return answer(reply, Err("dev grab-window <id> [block]".into()));
        };
        let block = words.iter().any(|w| w == "block");
        let (tx, rx) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let _ = tx.send(crate::platform::grab_window(id).map(|i| i.dimensions()));
        });
        let wait = move || {
            let got = rx.recv_timeout(std::time::Duration::from_secs(8));
            let ms = started.elapsed().as_millis() as u64;
            Ok(json!({ "block": block, "came": got.is_ok(), "size": got.ok().flatten().map(|(w, h)| [w, h]), "ms": ms }))
        };
        if block {
            return answer(reply, wait());
        }
        std::thread::spawn(move || answer(reply, wait()));
        return;
    }
    if command == "mem" {
        let blocks = big_blocks();
        let total: f64 = blocks.iter().filter_map(|b| b["mb"].as_f64()).sum();
        let note = if cfg!(feature = "dev") { "" } else { "build with --features dev to list the big blocks" };
        return answer(reply, Ok(json!({ "rss_mb": rss_mb(), "big_mb": (total * 10.).round() / 10., "big": blocks, "note": note })));
    }
    let Some(handle) = pick(window, cx) else {
        return answer(reply, Err("no such window".into()));
    };
    match command.as_str() {
        "dump" | "find" | "click" => {
            let id = words.first().cloned();
            if command != "dump" && id.is_none() {
                return answer(reply, Err(format!("dev {command} <id>")));
            }
            fresh_nodes(handle, cx, move |nodes, cx| {
                let found = || id.as_ref().and_then(|id| nodes.iter().find(|n| &n.id == id)).cloned();
                let result = match command.as_str() {
                    "dump" => Ok(json!(nodes.iter().map(Node::to_json).collect::<Vec<_>>())),
                    "find" => found().map(|n| n.to_json()).ok_or_else(|| "not found".to_owned()),
                    _ => match found() {
                        Some(node) => click(handle, &node, at, cx).map(|_| node.to_json()),
                        None => Err("not found".to_owned()),
                    },
                };
                answer(reply, result);
            });
        }
        "drag" => {
            let (Some(from), Some(by)) = (words.first().cloned(), words.get(1).and_then(|w| pair(w))) else {
                return answer(reply, Err("dev drag <id|x,y> <dx,dy>".into()));
            };
            fresh_nodes(handle, cx, move |nodes, cx| {
                let start = match pair(&from) {
                    Some((x, y)) => Some(point(px(x), px(y))),
                    None => nodes.iter().find(|n| n.id == from).map(|n| n.bounds.center()),
                };
                let result = match start {
                    Some(start) => drag(handle, start, by, cx).map(|_| json!({ "from": [f32::from(start.x), f32::from(start.y)], "by": [by.0, by.1] })),
                    None => Err("not found".to_owned()),
                };
                answer(reply, result);
            });
        }
        "key" | "type" => {
            let strokes: Result<Vec<gpui::Keystroke>, String> = if command == "key" {
                words.iter().map(|w| gpui::Keystroke::parse(w).map_err(|e| format!("{w}: {e}"))).collect()
            } else {
                Ok(words.join(" ").chars().map(|c| gpui::Keystroke { modifiers: Modifiers::default(), key: c.to_lowercase().to_string(), key_char: Some(c.to_string()) }).collect())
            };
            let result = strokes.and_then(|strokes| {
                handle
                    .update(cx, |_, window, cx| {
                        // GPUI's own "handled" only counts actions, not key
                        // handlers, so it says nothing useful here.
                        let sent = strokes.len();
                        for k in strokes {
                            window.dispatch_keystroke(k, cx);
                        }
                        json!({ "sent": sent })
                    })
                    .map_err(|e| e.to_string())
            });
            answer(reply, result);
        }
        "state" => answer(reply, state(handle, cx)),
        "shot" => {
            let Some(path) = words.first().cloned() else {
                return answer(reply, Err("dev shot <path>".into()));
            };
            let title = handle.update(cx, |_, window, _| window.window_title()).unwrap_or_default();
            let result = crate::platform::own_window_shot(&title, &kind_of(handle))
                .ok_or_else(|| "couldn't capture the window".to_owned())
                .and_then(|img| img.save(&path).map_err(|e| e.to_string()).map(|_| json!({ "path": path, "size": [img.width(), img.height()] })));
            answer(reply, result);
        }
        other => answer(reply, Err(format!("unknown dev command: {other}"))),
    }
}

fn answer(mut reply: Stream, result: Result<Value, String>) {
    let v = match result {
        Ok(result) => json!({ "ok": true, "result": result }),
        Err(error) => json!({ "ok": false, "error": error }),
    };
    let _ = writeln!(reply, "{v}");
}

fn kind_of(window: AnyWindowHandle) -> String {
    KINDS.with_borrow(|k| k.get(&window.window_id().as_u64()).cloned()).unwrap_or_default()
}

fn windows(cx: &mut App) -> Value {
    let active = cx.active_window().map(|w| w.window_id().as_u64());
    let list: Vec<Value> = cx
        .windows()
        .into_iter()
        .filter_map(|w| {
            let id = w.window_id().as_u64();
            let (title, b, scale, gpu) = w.update(cx, |_, window, _| (window.window_title(), window.bounds(), window.scale_factor(), window.gpu_specs())).ok()?;
            Some(json!({
                "id": id,
                "kind": kind_of(w),
                "title": title,
                "bounds": [f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)],
                "scale": scale,
                "active": active == Some(id),
                // What draws it: on a machine without a GPU, a software one
                // (WARP on Windows, llvmpipe on Linux) or none.
                "gpu": gpu.map(|g| json!({ "device": g.device_name, "driver": g.driver_name, "software": g.is_software_emulated })),
            }))
        })
        .collect();
    json!(list)
}

fn pick(id: Option<u64>, cx: &mut App) -> Option<AnyWindowHandle> {
    let all = cx.windows();
    match id {
        Some(id) => all.into_iter().find(|w| w.window_id().as_u64() == id),
        None => cx.active_window().or_else(|| all.last().copied()),
    }
}

/// Redraw the window, then hand over what that frame drew.
fn fresh_nodes(window: AnyWindowHandle, cx: &mut App, then: impl FnOnce(Vec<Node>, &mut App) + 'static) {
    let id = window.window_id().as_u64();
    NODES.with_borrow_mut(|n| n.remove(&id));
    let _ = window.update(cx, |_, window, _| window.refresh());
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_millis(120)).await;
        let nodes = NODES.with_borrow(|n| n.get(&id).cloned()).unwrap_or_default();
        let _ = cx.update(|cx| then(nodes, cx));
    })
    .detach();
}

/// "12,-5" → (12, -5).
fn pair(s: &str) -> Option<(f32, f32)> {
    let (x, y) = s.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Press at `start`, move by `by` in a few steps, let go.
fn drag(window: AnyWindowHandle, start: gpui::Point<Pixels>, by: (f32, f32), cx: &mut App) -> Result<(), String> {
    let modifiers = Modifiers::default();
    let button = MouseButton::Left;
    window
        .update(cx, |_, window, cx| {
            window.dispatch_event(PlatformInput::MouseMove(gpui::MouseMoveEvent { position: start, pressed_button: None, modifiers }), cx);
            window.dispatch_event(PlatformInput::MouseDown(gpui::MouseDownEvent { button, position: start, modifiers, click_count: 1, first_mouse: false }), cx);
            let steps = 12;
            let mut position = start;
            for i in 1..=steps {
                let t = i as f32 / steps as f32;
                position = point(start.x + px(by.0 * t), start.y + px(by.1 * t));
                window.dispatch_event(PlatformInput::MouseMove(gpui::MouseMoveEvent { position, pressed_button: Some(button), modifiers }), cx);
            }
            window.dispatch_event(PlatformInput::MouseUp(gpui::MouseUpEvent { button, position, modifiers, click_count: 1 }), cx);
        })
        .map_err(|e| e.to_string())
}

/// The pointer moves over the element, presses and lets go.
fn click(window: AnyWindowHandle, node: &Node, at: Option<(f32, f32)>, cx: &mut App) -> Result<(), String> {
    let b = node.bounds;
    let position = match at {
        Some((dx, dy)) => point(b.origin.x + px(dx), b.origin.y + px(dy)),
        None => b.center(),
    };
    let modifiers = Modifiers::default();
    window
        .update(cx, |_, window, cx| {
            window.dispatch_event(PlatformInput::MouseMove(gpui::MouseMoveEvent { position, pressed_button: None, modifiers }), cx);
            window.dispatch_event(PlatformInput::MouseDown(gpui::MouseDownEvent { button: MouseButton::Left, position, modifiers, click_count: 1, first_mouse: false }), cx);
            window.dispatch_event(PlatformInput::MouseUp(gpui::MouseUpEvent { button: MouseButton::Left, position, modifiers, click_count: 1 }), cx);
        })
        .map_err(|e| e.to_string())
}

fn state(window: AnyWindowHandle, cx: &mut App) -> Result<Value, String> {
    let kind = kind_of(window);
    window
        .update(cx, |_, window, cx| {
            let view = if let Some(Some(editor)) = window.root::<crate::editor::Editor>() {
                editor.read(cx).dev_state()
            } else if let Some(Some(settings)) = window.root::<crate::settings_view::SettingsView>() {
                settings.read(cx).dev_state()
            } else if let Some(Some(stack)) = window.root::<crate::cards::CardStack>() {
                stack.read(cx).dev_state()
            } else {
                Value::Null
            };
            json!({ "kind": kind, "title": window.window_title(), "view": view })
        })
        .map_err(|e| e.to_string())
}
