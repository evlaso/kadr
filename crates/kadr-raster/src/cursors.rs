//! The drawn cursor's shapes in a macOS-like look: one SVG per shape on a
//! 32-unit canvas, named like CSS cursors (`kadr_core::document::CURSORS`).
//! A recording logs which shape the pointer had, the studio draws it, and
//! the editor stamps it. Where the app sets a theme (`set_theme`, the user's
//! own cursors), its pictures are drawn instead.

use std::sync::OnceLock;

use image::RgbaImage;
use kadr_core::document::{CURSORS, cursor};

/// Picture and hotspot in canvas units, in `CURSORS`' order.
const SHAPES: [(&str, (f32, f32)); 13] = [
    (include_str!("../../../data/cursors/arrow.svg"), (5., 3.)),
    (include_str!("../../../data/cursors/pointer.svg"), (12.9, 4.4)),
    (include_str!("../../../data/cursors/text.svg"), (16., 16.)),
    (include_str!("../../../data/cursors/grab.svg"), (15., 15.)),
    (include_str!("../../../data/cursors/grabbing.svg"), (15., 17.)),
    (include_str!("../../../data/cursors/crosshair.svg"), (16., 16.)),
    (include_str!("../../../data/cursors/move.svg"), (16., 16.)),
    (include_str!("../../../data/cursors/not-allowed.svg"), (16., 16.)),
    (include_str!("../../../data/cursors/wait.svg"), (16., 16.)),
    (include_str!("../../../data/cursors/ew-resize.svg"), (16., 16.)),
    (include_str!("../../../data/cursors/ns-resize.svg"), (16., 16.)),
    (include_str!("../../../data/cursors/nwse-resize.svg"), (16., 16.)),
    (include_str!("../../../data/cursors/nesw-resize.svg"), (16., 16.)),
];

/// The arrow's height in canvas units: a shape drawn at `height` pixels has
/// the arrow that tall, and the others in proportion.
const ARROW_UNITS: f32 = 22.4;

/// The shapes the editor's cursor tool offers, in its toolbar's order.
/// (No "move": macOS themes draw it as the closed hand.)
pub const STAMPS: [&str; 8] = ["arrow", "pointer", "text", "grab", "grabbing", "crosshair", "not-allowed", "wait"];

/// The user's cursor for a shape at an arrow height: its picture and hotspot.
pub type Theme = fn(&str, f32) -> Option<(RgbaImage, (f64, f64))>;

static THEME: OnceLock<Theme> = OnceLock::new();

/// Draw the user's own cursors where `theme` has them. Set once, at start.
pub fn set_theme(theme: Theme) {
    let _ = THEME.set(theme);
}

/// A shape's hotspot and picture size per pixel of arrow height, as
/// `render` draws it (the theme's pictures scale evenly with the arrow).
fn proportions(name: &str) -> ((f32, f32), (f32, f32)) {
    static SEEN: std::sync::Mutex<Vec<(&'static str, ((f32, f32), (f32, f32)))>> = std::sync::Mutex::new(Vec::new());
    let name = cursor(name);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, p)) = seen.iter().find(|(n, _)| *n == name) {
        return *p;
    }
    const AT: f32 = 100.;
    let p = render(name, AT).map_or(((0., 0.), (1., 1.)), |(pm, (hx, hy))| {
        ((hx as f32 / AT, hy as f32 / AT), (pm.width() as f32 / AT, pm.height() as f32 / AT))
    });
    seen.push((name, p));
    p
}

/// Where a shape drawn with the arrow `height` tall has its hotspot, and
/// its picture's size, both in the same units as `height`.
pub fn geometry(name: &str, height: f32) -> ((f32, f32), (f32, f32)) {
    let ((hx, hy), (w, h)) = proportions(name);
    ((hx * height, hy * height), (w * height, h * height))
}

/// The arrow height at which a shape's picture is `side` across, its
/// longer way.
pub fn height_for(name: &str, side: f32) -> f32 {
    let (_, (w, h)) = proportions(name);
    side / w.max(h).max(1e-3)
}

/// Draw the shape (as `render` has it) onto `pm` with the arrow `height`
/// pixels tall and its hotspot at `at`.
pub fn draw(name: &str, height: f32, at: (f32, f32), pm: &mut tiny_skia::PixmapMut) {
    let Some((picture, (hx, hy))) = render(name, height) else { return };
    let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
    pm.draw_pixmap(0, 0, picture.as_ref(), &paint, tiny_skia::Transform::from_translate(at.0 - hx as f32, at.1 - hy as f32), None);
}

/// The shape as a picture with the arrow `height` pixels tall, and its
/// hotspot in pixels: the theme's where there is one, else kadr's.
pub fn render(name: &str, height: f32) -> Option<(tiny_skia::Pixmap, (f64, f64))> {
    let height = height.max(8.);
    if let Some((image, hot)) = THEME.get().and_then(|theme| theme(cursor(name), height)) {
        let size = tiny_skia::IntSize::from_wh(image.width(), image.height())?;
        return Some((tiny_skia::Pixmap::from_vec(image.into_raw(), size)?, hot));
    }
    own(name, height)
}

/// kadr's own picture of the shape.
fn own(name: &str, height: f32) -> Option<(tiny_skia::Pixmap, (f64, f64))> {
    let i = CURSORS.iter().position(|&n| n == name).unwrap_or(0);
    let (svg, (hx, hy)) = SHAPES[i];
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).ok()?;
    let k = height.max(8.) / ARROW_UNITS;
    let side = (32. * k).ceil() as u32;
    let mut pm = tiny_skia::Pixmap::new(side, side)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(k, k), &mut pm.as_mut());
    Some((pm, ((hx * k) as f64, (hy * k) as f64)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_shape_draws() {
        for name in kadr_core::document::CURSORS {
            let (pm, _) = super::own(name, 40.).unwrap();
            assert!(pm.pixels().iter().any(|p| p.alpha() == 255), "{name} is empty");
        }
        assert_eq!(kadr_core::document::cursor("help"), "arrow");
    }
}
