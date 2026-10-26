//! The area-selection overlay (SPEC §3.8): the selection and what the
//! pointer and keys do to it (`model`), its drawing on the CPU (`draw`),
//! and its own window, without a GPU or a UI framework: a layer-shell
//! surface on Wayland (`wayland`, on a thread of the app), a borderless
//! window above everything through winit on macOS and Windows (`desktop`,
//! in the `kadr-overlay` program; `spec` is what passes between them).

use std::sync::Arc;

#[cfg(not(target_os = "linux"))]
mod desktop;
pub mod draw;
pub mod model;
#[cfg(target_os = "macos")]
mod sck;
pub mod spec;
#[cfg(target_os = "linux")]
mod wayland;

pub use model::{Outcome, Rect};

/// When the program started, for its timings (`timing`).
static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Marks the program's start.
pub fn started() {
    let _ = STARTED.set(std::time::Instant::now());
}

/// Milliseconds since `started`, for the timing line the program prints.
pub fn since_start() -> u128 {
    STARTED.get().map_or(0, |t| t.elapsed().as_millis())
}

/// The monitor to pick on, frozen.
pub struct Screen {
    /// The compositor's name for it ("DP-1").
    pub output: String,
    /// Its size in logical pixels, and physical pixels per logical one.
    pub logical: (u32, u32),
    pub scale: f64,
    /// What was on it, in physical pixels.
    pub frozen: Arc<image::RgbaImage>,
    /// Windows to snap to, in its logical pixels.
    pub windows: Vec<Rect>,
    /// The last area picked on it.
    pub previous: Option<Rect>,
    /// Its top left in the desktop's logical layout.
    pub origin: (f64, f64),
}

pub struct Options {
    /// Hovering highlights windows and a plain click takes one.
    pub snap: bool,
    /// A click on nothing picks the colour under it.
    pub color: bool,
    /// The frame and handles' colour.
    pub accent: kadr_core::color::Rgba,
    /// "Free", for the aspect ratio, in the app's language.
    pub free: String,
    /// The app's words, by key.
    pub t: fn(&'static str) -> String,
    /// Where the pointer is on the desktop, and putting it somewhere: while
    /// dragging, the pointer is kept on the monitor (the overlay only hears
    /// it over itself), and a selection pushed past an edge takes it in.
    pub pointer: Option<(fn() -> Option<(f64, f64)>, fn(f64, f64))>,
}

/// Shows the overlay and runs it to its end, on the calling thread (on
/// macOS the main one): what it ended in (None: cancelled), or Err(())
/// where it can't show itself (not Wayland, no layer shell), for the app to
/// pick its own way.
pub fn pick(screen: &Screen, options: &Options) -> Result<Option<Outcome>, ()> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return wayland::pick(screen, options);
        }
        let _ = (screen, options);
        Err(())
    }
    #[cfg(not(target_os = "linux"))]
    desktop::pick(screen, options)
}

/// Keeps the overlay ready (macOS: its event loop and AppKit started) and
/// takes picks over `socket`, for as long as the process lives (`keep_warm`,
/// SPEC 3.6). The program's `--serve`.
#[cfg(all(unix, not(target_os = "linux")))]
pub fn serve(socket: &std::path::Path) -> Result<(), ()> {
    desktop::serve(socket)
}

/// The `kadr-overlay` program with its arguments (`kadr overlay …` runs it
/// too, for a kadr that's one file): see main.rs.
pub fn program(args: Vec<std::ffi::OsString>) -> ! {
    started();
    let Some(path) = args.first().cloned() else {
        eprintln!("usage: kadr-overlay SPEC.json | --serve SOCKET");
        std::process::exit(2);
    };
    if path == "--serve" {
        #[cfg(all(unix, not(target_os = "linux")))]
        if let Some(socket) = args.get(1) {
            let _ = serve(std::path::Path::new(&socket));
        }
        eprintln!("kadr-overlay: --serve needs a socket, and macOS");
        std::process::exit(2);
    }
    let Some((screen, options, lang)) = spec::read(std::path::Path::new(&path)) else {
        eprintln!("kadr-overlay: can't read {}", std::path::Path::new(&path).display());
        std::process::exit(2);
    };
    let read_ms = since_start();
    kadr_core::i18n::set(kadr_core::i18n::Lang::from_code(&lang));
    eprintln!("kadr-overlay timing: read={read_ms}ms");
    // The words were read before the language was set.
    let options = Options { free: kadr_core::i18n::t("picker.free").to_owned(), ..options };
    match pick(&screen, &options) {
        Ok(outcome) => {
            println!("{}", spec::outcome_line(outcome));
            std::process::exit(0);
        }
        Err(()) => {
            eprintln!("kadr-overlay: couldn't show the overlay here");
            std::process::exit(2);
        }
    }
}
