//! Text in the bundled IBM Plex Sans SemiBold, measured and drawn the same
//! way everywhere.

use std::sync::OnceLock;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use kadr_core::color::Rgba;
use kadr_core::document::P;

pub fn font() -> &'static FontRef<'static> {
    static FONT: OnceLock<FontRef<'static>> = OnceLock::new();
    FONT.get_or_init(|| FontRef::try_from_slice(include_bytes!("../../../data/fonts/IBMPlexSans-SemiBold.ttf")).expect("bundled font"))
}

/// ab_glyph scales by the font's whole height (ascender to descender), GPUI
/// and everyone else by the em; this turns an em size into ab_glyph's scale
/// so text measures and draws the size it shows on screen.
pub fn em(size: f32) -> PxScale {
    let f = font();
    let height = f.ascent_unscaled() - f.descent_unscaled();
    PxScale::from(size * height / f.units_per_em().unwrap_or(1000.))
}

pub struct Metrics {
    pub ascent: f32,
    pub descent: f32,
    /// Line box height; text sits centered in it, as in GPUI.
    pub line: f32,
}

pub fn metrics(size: f32) -> Metrics {
    let f = font().as_scaled(em(size));
    let (ascent, descent) = (f.ascent(), f.descent());
    Metrics { ascent, descent, line: ((ascent - descent) * 1.2).ceil() }
}

pub fn text_width(text: &str, size: f32) -> f32 {
    let f = font().as_scaled(em(size));
    let mut prev = None;
    let mut w = 0.;
    for ch in text.chars() {
        let id = font().glyph_id(ch);
        if let Some(p) = prev {
            w += f.kern(p, id);
        }
        w += f.h_advance(id);
        prev = Some(id);
    }
    w
}

/// One line of text with its baseline at `origin.1`, onto premultiplied pixels.
pub fn draw_line(pm: &mut tiny_skia::Pixmap, text: &str, origin: P, size: f32, color: Rgba) {
    let font = font();
    let scale = em(size);
    let f = font.as_scaled(scale);
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    let mut x = origin.0;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            x += f.kern(p, id);
        }
        let glyph = id.with_scale_and_position(scale, ab_glyph::point(x, origin.1));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bb = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let (px, py) = (bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32);
                if px < 0 || py < 0 || px >= w || py >= h {
                    return;
                }
                let i = ((py * w + px) * 4) as usize;
                let k = coverage.clamp(0., 1.) * color.a;
                for (ch, v) in [color.r, color.g, color.b].into_iter().enumerate() {
                    data[i + ch] = (data[i + ch] as f32 * (1. - k) + v * 255. * k) as u8;
                }
                data[i + 3] = (data[i + 3] as f32 * (1. - k) + 255. * k) as u8;
            });
        }
        x += f.h_advance(id);
        prev = Some(id);
    }
}
