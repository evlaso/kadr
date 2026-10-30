//! A local text band for the highlighter, without language models or OCR.

use image::RgbaImage;
use kadr_core::document::P;

/// Centre and ink height in image pixels. Only short, separated strokes on
/// a mostly uniform background qualify; solid controls and rules do not.
pub fn at(image: &RgbaImage, p: P) -> Option<(f32, f32)> {
    let (w, h) = image.dimensions();
    if !p.0.is_finite() || !p.1.is_finite() || p.0 < 0. || p.1 < 0. || p.0 >= w as f32 || p.1 >= h as f32 {
        return None;
    }
    let (x, y) = (p.0 as u32, p.1 as u32);
    let (l, r) = (x.saturating_sub(80), x.saturating_add(81).min(w));
    let (t, b) = (y.saturating_sub(64), y.saturating_add(65).min(h));
    // A quantised mode tolerates antialiasing, dark themes and coloured
    // backgrounds without assuming that the pointer starts on a blank pixel.
    let mut bins = [0u32; 4096];
    let bin = |c: &image::Rgba<u8>| ((c[0] as usize >> 4) << 8) | ((c[1] as usize >> 4) << 4) | (c[2] as usize >> 4);
    for yy in t..b {
        for xx in l..r {
            let c = image.get_pixel(xx, yy);
            if c[3] >= 240 {
                bins[bin(c)] += 1;
            }
        }
    }
    let (mode, count) = bins.iter().enumerate().max_by_key(|(_, n)| *n)?;
    if *count < (r - l) * (b - t) / 2 {
        return None;
    }
    let bg = [((mode >> 8) * 16 + 8) as i32, (((mode >> 4) & 15) * 16 + 8) as i32, ((mode & 15) * 16 + 8) as i32];
    let ink = |xx, yy| {
        let c = image.get_pixel(xx, yy);
        c[3] >= 240 && (0..3).map(|i| (c[i] as i32 - bg[i]).abs()).max().unwrap_or(0) >= 48
    };
    // Build row bands, including dots and antialiased extremities. Check
    // their text-like structure over the whole band rather than each row:
    // a capital's horizontal bar may join several of its vertical strokes.
    let rows: Vec<bool> = (t..b).map(|yy| (l..r).filter(|&xx| ink(xx, yy)).count() >= 2).collect();
    let mut bands = Vec::new();
    let mut start = None;
    let mut last = 0;
    for (i, &active) in rows.iter().enumerate() {
        if active {
            start.get_or_insert(i);
            last = i;
        }
        if let Some(s) = start
            && (i > last + 2 || i + 1 == rows.len())
        {
            bands.push((t + s as u32, t + last as u32 + 1));
            start = None;
        }
    }
    bands.into_iter().filter_map(|(top, bottom)| {
        let height = bottom - top;
        let centre = (top + bottom) as f32 / 2.;
        if !(5..=64).contains(&height) || (p.1 - centre).abs() > height as f32 / 2. + 3. || top == t || bottom == b {
            return None;
        }
        let mut short = 0;
        let mut total = 0;
        let mut nearest = u32::MAX;
        for yy in top..bottom {
            let mut run = 0;
            for xx in l..=r {
                if xx < r && ink(xx, yy) {
                    run += 1;
                    total += 1;
                    nearest = nearest.min(xx.abs_diff(x));
                } else if run > 0 {
                    if run <= height {
                        short += 1;
                    }
                    run = 0;
                }
            }
        }
        let area = (r - l) * height;
        (short >= height && total < area / 2 && nearest <= height.max(8))
            .then_some((centre, height as f32))
    }).min_by(|a, b| (a.0 - p.1).abs().total_cmp(&(b.0 - p.1).abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(size: f32, dark: bool) -> RgbaImage {
        let (bg, fg) = if dark { ([24, 24, 24, 255], [235, 235, 235, 255]) } else { ([255; 4], [20, 20, 20, 255]) };
        let mut image = RgbaImage::from_pixel(400, 240, image::Rgba(bg));
        crate::text::draw_line_into(image.as_mut(), (400, 240), "The quick brown fox", (20., 90.), size, kadr_core::color::Rgba::new(fg[0] as f32 / 255., fg[1] as f32 / 255., fg[2] as f32 / 255., 1.));
        image
    }

    #[test]
    fn finds_actual_glyph_height_in_both_themes() {
        for dark in [false, true] {
            for size in [14., 24., 48.] {
                let image = picture(size, dark);
                let (centre, height) = at(&image, (60., 90. - size * 0.3)).expect("text band");
                assert!(height > size * 0.5 && height <= size * 1.1, "{size}: {height}");
                assert!(centre < 90. && centre > 90. - size);
                assert!(at(&image, (60., 150.)).is_none());
            }
        }
    }

    #[test]
    fn ignores_blank_pictures_rules_and_solid_controls() {
        let mut image = RgbaImage::from_pixel(400, 240, image::Rgba([255; 4]));
        assert!(at(&image, (50., 50.)).is_none());
        for y in 40..60 {
            for x in 10..390 {
                image.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        assert!(at(&image, (50., 50.)).is_none());
        assert!(at(&image, (-1., 50.)).is_none());
    }

    #[test]
    fn keeps_neighbouring_lines_separate_and_rejects_texture() {
        let mut image = picture(24., false);
        crate::text::draw_line_into(image.as_mut(), (400, 240), "The quick brown fox", (20., 124.), 24., kadr_core::color::Rgba::hex(0x141414));
        let upper = at(&image, (60., 82.)).unwrap();
        let lower = at(&image, (60., 116.)).unwrap();
        assert!((lower.0 - upper.0 - 34.).abs() < 2.);
        assert!(at(&image, (60., 100.)).is_none());
        let noise = RgbaImage::from_fn(400, 240, |x, y| image::Rgba([(x * 71 + y * 31) as u8, (x * 19 + y * 73) as u8, (x * 41 + y * 17) as u8, 255]));
        assert!(at(&noise, (60., 80.)).is_none());
    }
}
