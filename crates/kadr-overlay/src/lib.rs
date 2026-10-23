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
pub mod spec;
#[cfg(target_os = "linux")]
mod wayland;

pub use model::{Outcome, Rect};

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
