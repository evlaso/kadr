//! The cursor's shapes, named like CSS cursors (`kadr_core::document::CURSORS`):
//! a recording logs which shape the pointer had, the studio draws it, and
//! the editor stamps it. The pictures are the system's own (`set_theme`:
//! the XCursor theme on Linux, NSCursor on macOS, the system cursors on
//! Windows). Where it has none, and always in the golden tests (no theme is
//! set there), the bundled set is drawn: the Adwaita cursors (GNOME, CC BY-SA
//! 3.0, `data/cursors/LICENSE`), at 96 pixels, scaled to size.

use std::sync::OnceLock;

use image::RgbaImage;
use kadr_core::document::cursor;

/// The shapes the editor's cursor tool offers, in its toolbar's order.
/// (No "move": macOS themes draw it as the closed hand.)
pub const STAMPS: [&str; 8] = ["arrow", "pointer", "text", "grab", "grabbing", "crosshair", "not-allowed", "wait"];

/// The bundled pictures (96 pixels square) and their hotspots there, in
/// `CURSORS`' order.
const BUNDLED: [(&str, &[u8], (f32, f32)); 13] = [
    ("arrow", include_bytes!("../../../data/cursors/arrow.png"), (12., 4.)),
    ("pointer", include_bytes!("../../../data/cursors/pointer.png"), (28., 20.)),
    ("text", include_bytes!("../../../data/cursors/text.png"), (44., 48.)),
    ("grab", include_bytes!("../../../data/cursors/grab.png"), (44., 8.)),
    ("grabbing", include_bytes!("../../../data/cursors/grabbing.png"), (36., 20.)),
    ("crosshair", include_bytes!("../../../data/cursors/crosshair.png"), (44., 44.)),
    ("move", include_bytes!("../../../data/cursors/move.png"), (12., 4.)),
    ("not-allowed", include_bytes!("../../../data/cursors/not-allowed.png"), (48., 48.)),
    ("wait", include_bytes!("../../../data/cursors/wait.png"), (44., 44.)),
    ("ew-resize", include_bytes!("../../../data/cursors/ew-resize.png"), (48., 48.)),
    ("ns-resize", include_bytes!("../../../data/cursors/ns-resize.png"), (48., 52.)),
    ("nwse-resize", include_bytes!("../../../data/cursors/nwse-resize.png"), (44., 44.)),
    ("nesw-resize", include_bytes!("../../../data/cursors/nesw-resize.png"), (44., 44.)),
];

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
/// hotspot in pixels: the system's where it has one, else kadr's own.
pub fn render(name: &str, height: f32) -> Option<(tiny_skia::Pixmap, (f64, f64))> {
    let height = height.max(8.);
    if let Some((image, hot)) = THEME.get().and_then(|theme| theme(cursor(name), height)) {
        let size = tiny_skia::IntSize::from_wh(image.width(), image.height())?;
        return Some((tiny_skia::Pixmap::from_vec(image.into_raw(), size)?, hot));
    }
    own(cursor(name), height)
}

/// The bundled picture of the shape (the arrow for a name it lacks), scaled
/// so the arrow is `height` tall: premultiplied, as the canvas wants.
fn own(name: &str, height: f32) -> Option<(tiny_skia::Pixmap, (f64, f64))> {
    /// The bundled arrow's visible height, its margin aside.
    fn arrow_height() -> f32 {
        static H: OnceLock<f32> = OnceLock::new();
        *H.get_or_init(|| {
            let Ok(arrow) = image::load_from_memory(BUNDLED[0].1) else { return 96. };
            let arrow = arrow.to_rgba8();
            let rows: Vec<u32> = (0..arrow.height()).filter(|&y| (0..arrow.width()).any(|x| arrow.get_pixel(x, y)[3] > 0)).collect();
            rows.first().zip(rows.last()).map_or(96., |(a, b)| (b - a + 1) as f32)
        })
    }
    let (_, png, (hx, hy)) = BUNDLED.iter().find(|(n, _, _)| *n == name).unwrap_or(&BUNDLED[0]);
    let picture = image::load_from_memory(png).ok()?.to_rgba8();
    let k = height / arrow_height();
    let (w, h) = (((picture.width() as f32 * k).round() as u32).max(1), ((picture.height() as f32 * k).round() as u32).max(1));
    let scaled = image::imageops::resize(&picture, w, h, image::imageops::FilterType::CatmullRom);
    let mut pm = tiny_skia::Pixmap::new(w, h)?;
    for (dst, p) in pm.pixels_mut().iter_mut().zip(scaled.pixels()) {
        let [r, g, b, a] = p.0;
        *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    Some((pm, ((hx * k) as f64, (hy * k) as f64)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_bundled_set_draws() {
        for (name, _, _) in super::BUNDLED {
            let (pm, _) = super::own(name, 40.).unwrap();
            assert!(pm.pixels().iter().any(|p| p.alpha() == 255), "{name} is empty");
        }
        assert_eq!(super::BUNDLED.map(|b| b.0), kadr_core::document::CURSORS, "one picture per shape, in order");
        // What it has no picture for falls back to the arrow.
        assert_eq!(super::own("help", 40.).unwrap().1, super::own("arrow", 40.).unwrap().1);
    }

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
