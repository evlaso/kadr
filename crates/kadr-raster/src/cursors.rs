//! The cursor's shapes, named like CSS cursors (`kadr_core::document::CURSORS`):
//! a recording logs which shape the pointer had, the studio draws it, and
//! the editor stamps it. The pictures are the system's own (`set_theme`:
//! the XCursor theme on Linux, NSCursor on macOS, the system cursors on
//! Windows); where it has none, nothing is drawn.

use std::sync::OnceLock;

use image::RgbaImage;
use kadr_core::document::cursor;

/// The shapes the editor's cursor tool offers, in its toolbar's order.
/// (No "move": macOS themes draw it as the closed hand.)
pub const STAMPS: [&str; 8] = ["arrow", "pointer", "text", "grab", "grabbing", "crosshair", "not-allowed", "wait"];

/// The system's cursor for a shape at an arrow height: its picture and hotspot.
pub type Theme = fn(&str, f32) -> Option<(RgbaImage, (f64, f64))>;

static THEME: OnceLock<Theme> = OnceLock::new();

/// Where the pictures come from. Set once, at start.
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
/// hotspot in pixels: the system's, if it has one.
pub fn render(name: &str, height: f32) -> Option<(tiny_skia::Pixmap, (f64, f64))> {
    let height = height.max(8.);
    let (image, hot) = THEME.get().and_then(|theme| theme(cursor(name), height))?;
    let size = tiny_skia::IntSize::from_wh(image.width(), image.height())?;
    Some((tiny_skia::Pixmap::from_vec(image.into_raw(), size)?, hot))
}

#[cfg(test)]
mod tests {
    /// A stand-in for the system: a box per shape, the arrow 20 tall at
    /// the size asked for.
    fn boxes(name: &str, height: f32) -> Option<(image::RgbaImage, (f64, f64))> {
        let side = (height * if name == "arrow" { 1. } else { 1.5 }).round() as u32;
        Some((image::RgbaImage::from_pixel(side, side, image::Rgba([0, 0, 0, 255])), (1., 2.)))
    }

    #[test]
    fn shapes_come_from_the_system_and_scale_with_the_arrow() {
        super::set_theme(boxes);
        let (pm, hot) = super::render("pointer", 40.).unwrap();
        assert_eq!((pm.width(), hot), (60, (1., 2.)));
        let ((hx, _), (w, _)) = super::geometry("pointer", 20.);
        assert!((w - 30.).abs() < 0.5 && hx < 1.);
        assert_eq!(kadr_core::document::cursor("help"), "arrow");
    }
}
