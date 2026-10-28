//! The one way to get pixels from a kadr drawing: `render` draws a `Doc` and
//! its `Frame` onto the picture, the same for Save, copying, dragging out and
//! the golden tests. The editor's canvas draws with the same geometry
//! (`shapes`) and text metrics (`text`), so screen and file match. `stitch`
//! makes the scrolling screenshot's picture from the frames it grabbed.
//!
//! Deterministic: the bundled IBM Plex only, no system fonts, no clock. The
//! one thing a caller can swap in is the cursor stamps' pictures
//! (`cursors::set_theme`, the user's cursor theme), which the tests leave out.

pub mod cursors;
mod frame;
mod render;
pub mod shapes;
pub mod stitch;
pub mod text;

pub use frame::{backdrops, blurred_pixels, framed};
pub use render::{Format, burn_redactions, encode, extent, hide, rasterize, render, render_as, render_png};
pub use shapes::{Bounds, on_color};
