//! A drawing onto its picture at full resolution.

use image::RgbaImage;
use kadr_core::document::{Doc, Frame, Hide, Kind, MAX_REACH, P, SPOT_SHADE, Shape, TextStyle, hide_strength, rect, work_margin_at};

use crate::frame::framed;
use crate::shapes::{Bounds, draw_vector, on_color, paint_for, rounded_rect};
use crate::text::{draw_line, metrics, text_width};

/// The result of a drawing: `base` as it was taken, turned as `doc` says,
/// drawn on, cropped and framed. What Save writes.
pub fn render(base: &RgbaImage, doc: &Doc, frame: &Frame) -> Option<RgbaImage> {
    if doc.view.is_upright() {
        compose(base, doc, frame)
    } else {
        compose(&doc.view.apply(base), doc, frame)
    }
}

/// `render` for a picture already turned as `doc.view` says, as PNG bytes.
pub fn render_png(turned: &RgbaImage, doc: &Doc, frame: &Frame) -> Option<Vec<u8>> {
    let out = compose(turned, doc, frame)?;
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// What the result covers, in image pixels (whole, within the work area):
/// the crop, or else the picture and, with `grow`, everything drawn round it.
pub fn extent(doc: &Doc, size: (u32, u32), grow: bool) -> (f32, f32, f32, f32) {
    let (w, h) = (size.0 as f32, size.1 as f32);
    // A crop may reach as far as the crop mode lets it.
    let m = work_margin_at(size, MAX_REACH);
    let (l, t, r, b) = match doc.crop {
        Some((a, b)) => rect(a, b),
        None if !grow => (0., 0., w, h),
        None => doc.shapes.iter().map(Shape::bounds).fold((0f32, 0f32, w, h), |e, s| (e.0.min(s.0), e.1.min(s.1), e.2.max(s.2), e.3.max(s.3))),
    };
    let (l, t) = (l.round().clamp(-m, w + m), t.round().clamp(-m, h + m));
    let (r, b) = (r.round().clamp(l + 1., w + m + 1.), b.round().clamp(t + 1., h + m + 1.));
    (l, t, r, b)
}

/// Annotated, cropped, optionally framed picture, from one already turned.
fn compose(src: &RgbaImage, doc: &Doc, frame: &Frame) -> Option<RgbaImage> {
    // The picture on a transparent canvas the size of the result, and the
    // shapes moved along with it.
    let (l, t, r, b) = extent(doc, src.dimensions(), !frame.clips());
    let (ew, eh) = ((r - l) as u32, (b - t) as u32);
    let canvas = match doc.fill {
        Some(fill) => {
            let mut canvas = RgbaImage::from_pixel(ew, eh, image::Rgba(fill));
            image::imageops::overlay(&mut canvas, src, -l as i64, -t as i64);
            canvas
        }
        None => {
            let mut canvas = RgbaImage::new(ew, eh);
            image::imageops::replace(&mut canvas, src, -l as i64, -t as i64);
            canvas
        }
    };
    let shapes: Vec<Shape> = doc
        .shapes
        .iter()
        .map(|sh| {
            let mut sh = sh.clone();
            sh.translate((-l, -t));
            sh
        })
        .collect();
    let mut out = rasterize(&canvas, &shapes)?;
    if frame.on() {
        // The picture gets the rounded corners and the shadow; what's drawn
        // past its edges sits on the backdrop as it is.
        let drawn = rasterize(&RgbaImage::new(ew, eh), &shapes)?;
        let (w, h) = (src.width() as f32, src.height() as f32);
        // A filled canvas is all picture: it gets the corners and shadow.
        let inner = if doc.fill.is_some() { (0., 0., ew as f32, eh as f32) } else { ((-l).max(0.), (-t).max(0.), (w - l).min(ew as f32), (h - t).min(eh as f32)) };
        out = framed(&out, &drawn, inner, frame);
    }
    // The crop mode's image size: the whole result, smaller.
    if doc.scale < 0.999 {
        let (w, h) = out.dimensions();
        let (nw, nh) = (((w as f32 * doc.scale).round() as u32).max(1), ((h as f32 * doc.scale).round() as u32).max(1));
        out = image::imageops::resize(&out, nw, nh, image::imageops::FilterType::Lanczos3);
    }
    Some(out)
}

/// The a–b area of `src` pixelated or blurred, placed at the area's top
/// left corner clamped to the picture, as strong as `size` says (see
/// `hide_strength`). Screen and export share it.
pub fn hide(src: &RgbaImage, a: P, b: P, how: Hide, size: f32) -> Option<RgbaImage> {
    let (l, t, r, bt) = rect(a, b);
    let (x, y) = (l.floor().max(0.) as u32, t.floor().max(0.) as u32);
    let w = (r.ceil() as u32).min(src.width()).saturating_sub(x);
    let h = (bt.ceil() as u32).min(src.height()).saturating_sub(y);
    if w < 2 || h < 2 {
        return None;
    }
    // Blocks and blur scale with the screenshot so text stays unreadable at any size.
    let long = src.width().max(src.height());
    let k = hide_strength(size);
    match how {
        Hide::Pixelate => {
            let block = (((long / 120).clamp(8, 32) as f32 * k).round() as u32).max(3);
            let region = image::imageops::crop_imm(src, x, y, w, h).to_image();
            let small = image::imageops::resize(&region, (w / block).max(1), (h / block).max(1), image::imageops::FilterType::Triangle);
            Some(image::imageops::resize(&small, w, h, image::imageops::FilterType::Nearest))
        }
        Hide::Blur => {
            // Blurred with what's around it, so the edges don't sharpen up.
            let sigma = (long as f32 / 140.).clamp(6., 24.) * k;
            let reach = (sigma * 2.).ceil() as u32;
            let (bx, by) = (x.saturating_sub(reach), y.saturating_sub(reach));
            let (bw, bh) = ((x + w + reach).min(src.width()) - bx, (y + h + reach).min(src.height()) - by);
            let region = image::imageops::crop_imm(src, bx, by, bw, bh).to_image();
            let blurred = image::imageops::fast_blur(&region, sigma);
            Some(image::imageops::crop_imm(&blurred, x - bx, y - by, w, h).to_image())
        }
    }
}

/// Draw everything onto the picture at full resolution.
pub fn rasterize(src: &RgbaImage, shapes: &[Shape]) -> Option<RgbaImage> {
    use tiny_skia::{FillRule, PathBuilder, Pixmap, Transform};

    // Hiding edits the pixels themselves, beneath every other shape.
    let mut base = src.clone();
    for shape in shapes {
        if let Kind::Blur(how) = shape.kind
            && let Some(patch) = hide(src, shape.a, shape.b, how, shape.size)
        {
            let (l, t, _, _) = rect(shape.a, shape.b);
            image::imageops::replace(&mut base, &patch, l.floor().max(0.) as i64, t.floor().max(0.) as i64);
        }
    }
    let spots: Vec<(f32, f32, f32, f32)> = shapes.iter().filter(|sh| matches!(sh.kind, Kind::Spotlight)).map(|sh| rect(sh.a, sh.b)).collect();
    if !spots.is_empty() {
        let keep = 1. - SPOT_SHADE;
        for (x, y, px) in base.enumerate_pixels_mut() {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if !spots.iter().any(|&(l, t, r, b)| fx >= l && fx < r && fy >= t && fy < b) {
                for c in &mut px.0[..3] {
                    *c = (*c as f32 * keep) as u8;
                }
            }
        }
    }
    // Pictures on top of the screenshot, under the drawing.
    for shape in shapes {
        if let Kind::Picture(picture) = &shape.kind {
            let (l, t, r, b) = rect(shape.a, shape.b);
            let (w, h) = ((r - l).round().max(1.) as u32, (b - t).round().max(1.) as u32);
            let scaled = image::imageops::resize(picture.as_ref(), w, h, image::imageops::FilterType::Lanczos3);
            image::imageops::overlay(&mut base, &scaled, l.round() as i64, t.round() as i64);
        }
    }
    let (w, h) = base.dimensions();
    let mut data = base.into_raw();
    // tiny-skia works on premultiplied pixels.
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = (*c as u32 * a / 255) as u8;
        }
    }
    let mut pm = Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h)?)?;

    for shape in shapes {
        if draw_vector(&mut pm, shape, Transform::identity()) {
            continue;
        }
        let (a, c, size) = (shape.a, shape.color, shape.size);
        let solid = paint_for(c, 1.);
        match &shape.kind {
            Kind::Cursor(name) => crate::cursors::draw(name, size, a, &mut pm.as_mut()),
            Kind::Counter(n) => {
                if let Some(path) = PathBuilder::from_circle(a.0, a.1, size / 2.) {
                    pm.fill_path(&path, &solid, FillRule::Winding, Transform::identity(), None);
                }
                let label = n.to_string();
                let fs = size * 0.55;
                let m = metrics(fs);
                let width = text_width(&label, fs);
                draw_line(&mut pm, &label, (a.0 - width / 2., a.1 - (m.ascent - m.descent) / 2. + m.ascent), fs, on_color(c));
            }
            Kind::Text(text) => {
                let m = metrics(size);
                if shape.text == TextStyle::Plate {
                    let (l, t, r, bt) = shape.bounds();
                    let (_, _, radius) = TextStyle::plate(size);
                    if let Some(path) = rounded_rect(l, t, r, bt, radius) {
                        pm.fill_path(&path, &solid, FillRule::Winding, Transform::identity(), None);
                    }
                }
                let ink = if shape.text == TextStyle::Plate { on_color(c) } else { c };
                for (i, line) in text.split('\n').enumerate() {
                    let top = a.1 + i as f32 * m.line;
                    let baseline = top + (m.line - (m.ascent - m.descent)) / 2. + m.ascent;
                    if shape.text == TextStyle::Outline {
                        let o = TextStyle::outline(size);
                        for k in 0..16 {
                            let th = k as f32 / 16. * std::f32::consts::TAU;
                            draw_line(&mut pm, line, (a.0 + o * th.cos(), baseline + o * th.sin()), size, on_color(c));
                        }
                    }
                    draw_line(&mut pm, line, (a.0, baseline), size, ink);
                }
            }
            _ => {}
        }
    }
    let mut data = pm.take();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = (*c as u32 * 255 / a).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(w, h, data)
}
