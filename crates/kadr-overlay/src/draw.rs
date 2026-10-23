//! The overlay drawn on the CPU, onto the window's shared-memory pixels
//! (four bytes each, B G R A as Wayland's ARGB8888 lays them out): the
//! frozen screen dimmed outside what would be taken, its frame and
//! handles, the guides, the size, the magnifier and the keys. It looks as
//! the GPUI picker did.

use kadr_core::color::Rgba;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, PixmapMut, Rect as SkRect, Stroke, Transform};

use crate::model::{ASPECTS, Drag, Model, handles};

/// The frozen screen in the buffer's byte order, as is and dimmed: a frame
/// starts as a copy of these.
pub struct Frozen {
    pub width: u32,
    pub height: u32,
    pub bright: Vec<u8>,
    pub dim: Vec<u8>,
}

/// What lies outside the selection is darkened by this.
const SHADE: f32 = 0.42;
/// How many screen pixels the magnifier shows across (odd, so one is in the
/// middle), and about how big each is, in logical pixels.
const CELLS: u32 = 13;
const CELL: f32 = 11.;

impl Frozen {
    /// From the screen as RGBA in physical pixels.
    pub fn new(rgba: &image::RgbaImage) -> Frozen {
        let (width, height) = rgba.dimensions();
        let mut bright = Vec::with_capacity(rgba.len());
        let mut dim = Vec::with_capacity(rgba.len());
        let keep = 1. - SHADE;
        for p in rgba.pixels() {
            let [r, g, b, _] = p.0;
            bright.extend_from_slice(&[b, g, r, 255]);
            dim.extend_from_slice(&[(b as f32 * keep) as u8, (g as f32 * keep) as u8, (r as f32 * keep) as u8, 255]);
        }
        Frozen { width, height, bright, dim }
    }

    /// The pixel at (x, y), as RGB.
    pub fn rgb(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        (x < self.width && y < self.height).then(|| {
            let i = ((y * self.width + x) * 4) as usize;
            [self.bright[i + 2], self.bright[i + 1], self.bright[i]]
        })
    }
}

/// The words around the picker, in the app's language.
pub struct Words {
    /// What the keys do now: the key, and what it does.
    pub keys: Vec<(String, String)>,
    /// The aspect ratio's name when it's free ("Free").
    pub free: String,
}

/// A colour in the buffer's order: red and blue swap places, as tiny-skia
/// writes R G B A and the buffer reads B G R A.
fn sk(c: Rgba) -> Color {
    Color::from_rgba(c.b, c.g, c.r, c.a).unwrap_or(Color::BLACK)
}

fn text_rgba(c: Rgba) -> Rgba {
    Rgba { r: c.b, g: c.g, b: c.r, a: c.a }
}

fn rgba(r: f32, g: f32, b: f32, a: f32) -> Rgba {
    Rgba { r, g, b, a }
}

fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.).min(h / 2.);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

fn fill(pm: &mut PixmapMut, path: Option<tiny_skia::Path>, c: Rgba) {
    if let Some(path) = path {
        let mut paint = Paint::default();
        paint.set_color(sk(c));
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn rect(pm: &mut PixmapMut, (x, y, w, h): (f32, f32, f32, f32), c: Rgba) {
    if let Some(r) = SkRect::from_xywh(x, y, w.max(0.), h.max(0.)) {
        let mut paint = Paint::default();
        paint.set_color(sk(c));
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }
}

fn circle(pm: &mut PixmapMut, (x, y): (f32, f32), r: f32, c: Rgba) {
    fill(pm, PathBuilder::from_circle(x, y, r), c);
}

/// A dark plate with a line of text on it, its top left at (x, y), in
/// physical pixels; gives back its size.
fn plate(buf: &mut [u8], size: (u32, u32), (x, y): (f32, f32), text: &str, font: f32, s: f32) -> (f32, f32) {
    let (padx, pady) = (8. * s, 2.5 * s);
    let m = kadr_raster::text::metrics(font);
    let (w, h) = (kadr_raster::text::text_width(text, font) + 2. * padx, m.line + 2. * pady);
    if let Some(mut pm) = PixmapMut::from_bytes(buf, size.0, size.1) {
        fill(&mut pm, rounded(x, y, w, h, 6. * s), rgba(0., 0., 0., 0.72));
    }
    let baseline = y + pady + (m.line - (m.ascent - m.descent)) / 2. + m.ascent;
    kadr_raster::text::draw_line_into(buf, size, text, (x + padx, baseline), font, text_rgba(Rgba::WHITE));
    (w, h)
}

/// One frame: `out` is the buffer, `scale` physical pixels per logical one,
/// `accent` the frame and handles' colour.
pub fn frame(out: &mut [u8], frozen: &Frozen, m: &Model, accent: Rgba, words: &Words) {
    let s = m.scale;
    let size = (frozen.width, frozen.height);
    let (sw, sh) = (frozen.width as f32, frozen.height as f32);
    let px = |v: f32| v * s;

    let editable = m.editable();
    let hovered = if m.selection.is_none() && (m.snap || m.windows_only) && m.drag.is_none() { m.hovered_window() } else { None };
    let lit = m.selection.or(hovered);
    let aiming = (!editable && !m.windows_only) || matches!(m.drag, Some(Drag::Resize { .. }));
    let guides = m.cursor.filter(|_| m.drag.is_none() && !editable && !m.windows_only);

    // The dimmed screen, the lit area bright again.
    out.copy_from_slice(&frozen.dim);
    if let Some((x, y, w, h)) = lit {
        let (x0, y0) = (px(x).round().max(0.) as usize, px(y).round().max(0.) as usize);
        let (x1, y1) = ((px(x + w).round() as usize).min(frozen.width as usize), (px(y + h).round() as usize).min(frozen.height as usize));
        let stride = frozen.width as usize * 4;
        for row in y0..y1 {
            let (a, b) = (row * stride + x0 * 4, row * stride + x1 * 4);
            if a < b {
                out[a..b].copy_from_slice(&frozen.bright[a..b]);
            }
        }
    }

    let Some(mut pm) = PixmapMut::from_bytes(out, size.0, size.1) else { return };
    if let Some((x, y, w, h)) = lit {
        let color = if m.selection.is_some() { Rgba::WHITE } else { accent };
        let mut pb = PathBuilder::new();
        pb.push_rect(SkRect::from_xywh(px(x) - 0.5 * s, px(y) - 0.5 * s, px(w) + s, px(h) + s).unwrap_or(SkRect::from_xywh(0., 0., 1., 1.).unwrap()));
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(sk(color));
            pm.stroke_path(&path, &paint, &Stroke { width: s, ..Default::default() }, Transform::identity(), None);
        }
    }
    if let (Some(sel), true) = (m.selection, editable) {
        for (_, (hx, hy)) in handles(sel) {
            let c = (px(hx), px(hy) + 0.);
            // A knob: an accent dot in a white ring with a soft shadow.
            circle(&mut pm, (c.0, c.1 + 1.5 * s), 11.5 * s, rgba(0., 0., 0., 0.1));
            circle(&mut pm, (c.0, c.1 + s), 10.5 * s, rgba(0., 0., 0., 0.18));
            circle(&mut pm, c, 9.5 * s, Rgba::WHITE);
            circle(&mut pm, c, 7. * s, accent);
        }
    }
    if let Some((gx, gy)) = guides {
        let guide = rgba(1., 1., 1., 0.35);
        rect(&mut pm, (0., px(gy.floor()), sw, s.max(1.)), guide);
        rect(&mut pm, (px(gx.floor()), 0., s.max(1.), sh), guide);
    }
    drop(pm);

    // The size in real pixels under the selection's corner, with the ratio.
    if let Some((x, y, w, h)) = m.selection {
        let text = match &m.typing {
            Some((tw, th, on_h)) => format!("{}{} × {}{}", tw, if *on_h { "" } else { "|" }, th, if *on_h { "|" } else { "" }),
            None => format!("{} × {}", (w * s).round(), (h * s).round()),
        };
        let (label, ratio) = ASPECTS[m.aspect];
        let aspect = (editable || ratio.is_some()).then(|| if ratio.is_some() { label.to_owned() } else { words.free.clone() });
        let font = 13. * s;
        let fw = |t: &str| kadr_raster::text::text_width(t, font) + 16. * s;
        let total = fw(&text) + aspect.as_deref().map_or(0., |a| fw(a) + 4. * s);
        let lx = (px(x + w) - total).max(4. * s);
        let ly = if px(y + h) + 34. * s < sh { px(y + h) + 8. * s } else { (px(y) - 30. * s).max(4. * s) };
        let (w0, _) = plate(out, size, (lx, ly), &text, font, s);
        if let Some(a) = aspect {
            plate(out, size, (lx + w0 + 4. * s, ly), &a, font, s);
        }
    }

    // The magnifier by the pointer, away from the edges.
    if let Some((cx, cy)) = m.cursor.filter(|_| aiming) {
        let cell = (CELL * s).round().max(2.);
        let side = CELLS as f32 * cell;
        let edge = (2. * s).round();
        let outer = side + 2. * edge;
        let (lx, ly) = (if px(cx) + 24. * s + outer < sw { px(cx) + 24. * s } else { px(cx) - 24. * s - outer }, if px(cy) + 24. * s + outer + 40. * s < sh { px(cy) + 24. * s } else { px(cy) - 24. * s - outer - 40. * s });
        let (lx, ly) = (lx.round(), ly.round());
        if let Some(mut pm) = PixmapMut::from_bytes(out, size.0, size.1) {
            fill(&mut pm, rounded(lx, ly + 6. * s, outer, outer, 12. * s + edge), rgba(0., 0., 0., 0.22));
            fill(&mut pm, rounded(lx, ly, outer, outer, 12. * s + edge), Rgba::WHITE);
            let (ix, iy) = (px(cx).max(0.) as i64, px(cy).max(0.) as i64);
            let half = (CELLS / 2) as i64;
            for v in 0..CELLS as i64 {
                for u in 0..CELLS as i64 {
                    let (sx, sy) = (ix + u - half, iy + v - half);
                    let c = if sx < 0 || sy < 0 { None } else { frozen.rgb(sx as u32, sy as u32) };
                    let [r, g, b] = c.unwrap_or([0, 0, 0]);
                    let at = (lx + edge + u as f32 * cell, ly + edge + v as f32 * cell);
                    rect(&mut pm, (at.0, at.1, cell, cell), rgba(r as f32 / 255., g as f32 / 255., b as f32 / 255., 1.));
                }
            }
            // A light grid between the pixels, the middle one outlined.
            let grid = rgba(0x9C as f32 / 255., 0xD0 as f32 / 255., 1., 0.35);
            for k in 1..CELLS {
                let o = k as f32 * cell;
                rect(&mut pm, (lx + edge + o, ly + edge, 1., side), grid);
                rect(&mut pm, (lx + edge, ly + edge + o, side, 1.), grid);
            }
            let mid = (CELLS / 2) as f32 * cell;
            let mut pb = PathBuilder::new();
            pb.push_rect(SkRect::from_xywh(lx + edge + mid, ly + edge + mid, cell, cell).unwrap_or(SkRect::from_xywh(0., 0., 1., 1.).unwrap()));
            if let Some(path) = pb.finish() {
                let mut paint = Paint::default();
                paint.set_color(sk(Rgba::WHITE));
                pm.stroke_path(&path, &paint, &Stroke { width: 1.5 * s, ..Default::default() }, Transform::identity(), None);
            }
        }
        let hex = frozen.rgb(px(cx) as u32, px(cy) as u32).map(|[r, g, b]| format!("#{r:02X}{g:02X}{b:02X}")).unwrap_or_default();
        let label = format!("{}, {}  {}", px(cx).round(), px(cy).round(), hex);
        let font = 11. * s;
        let w = kadr_raster::text::text_width(&label, font) + 16. * s;
        plate(out, size, (lx + (outer - w) / 2., ly + outer + 4. * s), &label, font, s);
    }

    // What the keys do now, at the bottom, like a tooltip.
    if !words.keys.is_empty() {
        let font = 12. * s;
        let (gap, inner) = (12. * s, 4. * s);
        let widths: Vec<(f32, f32)> = words.keys.iter().map(|(k, w)| (kadr_raster::text::text_width(k, font), kadr_raster::text::text_width(w, font))).collect();
        let total: f32 = widths.iter().map(|(a, b)| a + inner + b).sum::<f32>() + gap * (widths.len() as f32 - 1.) + 2. * 6. * s;
        let m_ = kadr_raster::text::metrics(font);
        let h = m_.line + 8. * s;
        let (x0, y0) = ((sw - total) / 2., sh - 28. * s - h);
        if let Some(mut pm) = PixmapMut::from_bytes(out, size.0, size.1) {
            fill(&mut pm, rounded(x0 - 1., y0 - 1., total + 2., h + 2., 7. * s), rgba(1., 1., 1., 0.14));
            fill(&mut pm, rounded(x0, y0, total, h, 6. * s), rgba(0., 0., 0., 1.));
        }
        let baseline = y0 + 4. * s + (m_.line - (m_.ascent - m_.descent)) / 2. + m_.ascent;
        let mut x = x0 + 6. * s;
        for ((k, w), (kw, ww)) in words.keys.iter().zip(widths) {
            kadr_raster::text::draw_line_into(out, size, k, (x, baseline), font, text_rgba(rgba(1., 1., 1., 0.5)));
            x += kw + inner;
            kadr_raster::text::draw_line_into(out, size, w, (x, baseline), font, text_rgba(Rgba::WHITE));
            x += ww + gap;
        }
    }
}

/// What the keys do now, for the hint bar: as the GPUI picker listed them.
pub fn keys(m: &Model, t: impl Fn(&'static str) -> String) -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = Vec::new();
    let mut add = |k: &str, w: String| keys.push((k.to_owned(), w));
    if m.typing.is_some() {
        add("Enter", t("picker.set_size"));
        add("X", t("picker.height"));
        add("Esc", t("common.cancel"));
    } else if m.editable() {
        add("Enter", t("picker.take"));
        add(&t("picker.arrows"), t("picker.move"));
        add(&t("picker.alt_arrows"), t("picker.resize"));
        add("A", t("picker.aspect"));
        add("0–9", t("picker.size"));
        add("Esc", t("common.cancel"));
    } else if m.windows_only {
        add(&t("picker.click"), t("picker.take_window"));
        add(&t("picker.space"), t("picker.area"));
        add("Esc", t("common.cancel"));
    } else if m.drag.is_none() {
        add(&t("picker.space"), t("common.window"));
        if m.previous.is_some() {
            add("P", t("picker.previous_area"));
            add("Enter", t("picker.take_previous"));
        }
        let aspect = ASPECTS[m.aspect];
        add("A", if aspect.1.is_some() { aspect.0.to_owned() } else { t("picker.aspect") });
        add("Esc", t("common.cancel"));
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_dims_all_but_the_selection() {
        let screen = image::RgbaImage::from_pixel(1000, 600, image::Rgba([200, 100, 50, 255]));
        let frozen = Frozen::new(&screen);
        let mut m = Model::new((500., 300.), 2.);
        m.selection = Some((100., 50., 100., 60.));
        let mut out = vec![0; frozen.bright.len()];
        frame(&mut out, &frozen, &m, rgba(0.2, 0.5, 1., 1.), &Words { keys: vec![], free: "Free".into() });
        let at = |x: usize, y: usize| &out[(y * 1000 + x) * 4..(y * 1000 + x) * 4 + 4];
        // Inside: bright, in B G R A.
        assert_eq!(at(300, 150), &[50, 100, 200, 255]);
        // Outside: dimmed.
        assert_eq!(at(900, 500), &[29, 58, 116, 255]);
        // The frame: white round the selection's edge.
        assert_eq!(at(199, 150)[..3], [255, 255, 255]);
    }
}
