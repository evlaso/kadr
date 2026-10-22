//! The background a picture is put on: a gradient, a colour, a wallpaper or
//! the picture itself blurred, with rounded corners and a soft shadow.

use image::RgbaImage;
use kadr_core::color::Rgba;
use kadr_core::document::{Backdrop, Frame};

/// Gradient backdrops shared by the screenshot editor and the studio.
/// Gradients first, then plain colours (a gradient from a colour to itself),
/// then more gradients, so saved indices keep meaning what they did.
pub fn backdrops() -> Vec<(Rgba, Rgba)> {
    let hex = Rgba::hex;
    // Flexoki's blue, magenta, orange, red and green.
    let (blue, magenta, orange, red, green) = (hex(0x4385BE), hex(0xCE5D97), hex(0xDA702C), hex(0xD14D41), hex(0x879A39));
    let mut all = vec![(blue, magenta), (orange, red), (green, blue), (Rgba::hsl(0., 0., 0.22, 1.), Rgba::hsl(0., 0., 0.08, 1.))];
    all.extend([0xFFFFFF, 0x000000, 0xE5E5EA, 0x8E8E93, 0x007AFF, 0x34C759, 0xFFCC00, 0xFF3B30, 0xAF52DE].map(|c| (hex(c), hex(c))));
    all.extend(
        [
            (0xFF5F6D, 0xFFC371),
            (0x7F7FD5, 0x91EAE4),
            (0x654EA3, 0xEAAFC8),
            (0x00C6FF, 0x0072FF),
            (0xF7797D, 0xC471ED),
            (0x11998E, 0x38EF7D),
            (0xFC466B, 0x3F5EFB),
            (0x0F2027, 0x2C5364),
            (0xEECDA3, 0xEF629F),
            (0x8E2DE2, 0x4A00E0),
        ]
        .map(|(a, b)| (hex(a), hex(b))),
    );
    all
}

/// The backdrop picture at `w`×`h`: the wallpaper cropped to fill, or the
/// gradient.
fn backdrop_pixels(backdrop: &Backdrop, picture: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    if *backdrop == Backdrop::Blurred {
        return blurred_pixels(picture, w, h);
    }
    if let Backdrop::Wallpaper(path) = backdrop
        && let Ok(img) = image::open(path)
    {
        return img.resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    }
    let all = backdrops();
    let (a, b) = match backdrop {
        Backdrop::Gradient(i) => all[(*i).min(all.len() - 1)],
        _ => all[0],
    };
    RgbaImage::from_fn(w, h, |x, y| {
        let g = ((x as f32 / w as f32) * 0.6 + (y as f32 / h as f32) * 0.4).clamp(0., 1.);
        let mix = |p: f32, q: f32| ((p + (q - p) * g) * 255.).round() as u8;
        image::Rgba([mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), 255])
    })
}

/// `picture` blurred into a `w`×`h` fill: shrunk a lot, softened, grown
/// back, which is quick even for big outputs.
pub fn blurred_pixels(picture: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    use image::imageops::{FilterType, blur, resize};
    let (w, h) = (w.max(1), h.max(1));
    let k = (w.max(h) as f32 / 48.).max(1.);
    let (sw, sh) = (((w as f32 / k).round() as u32).max(2), ((h as f32 / k).round() as u32).max(2));
    let small = image::DynamicImage::ImageRgba8(picture.clone()).resize_to_fill(sw, sh, FilterType::Triangle).to_rgba8();
    let mut out = resize(&blur(&small, 1.6), w, h, FilterType::CatmullRom);
    for p in out.pixels_mut() {
        p.0[3] = 255;
    }
    out
}

/// `img` on the backdrop: the `inner` part (the picture, in `img` pixels)
/// rounded and shadowed, and `drawn` (the shapes alone) wherever it reaches
/// past that.
pub fn framed(img: &RgbaImage, drawn: &RgbaImage, inner: (f32, f32, f32, f32), frame: &Frame) -> RgbaImage {
    let ((ow, oh), (left, top)) = frame.layout(img.width() as f32, img.height() as f32);
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let (w, h) = ((inner.2 - inner.0).max(1.), (inner.3 - inner.1).max(1.));
    let radius = w.max(h) * frame.radius;
    let blur = (w.max(h) * frame.padding * 0.45).max(w.max(h) * 0.02);
    let strength = frame.shadow.clamp(0., 1.);
    let clear = frame.backdrop == Backdrop::Transparent;
    let mut out = if clear { RgbaImage::new(ow as u32, oh as u32) } else { backdrop_pixels(&frame.backdrop, img, ow as u32, oh as u32) };
    // Signed distance to a rounded rectangle centered at (cx, cy).
    let sdf = |x: f32, y: f32, cx: f32, cy: f32| {
        let qx = (x - cx).abs() - (w / 2. - radius);
        let qy = (y - cy).abs() - (h / 2. - radius);
        qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - radius
    };
    let (cx, cy) = (left + inner.0 + w / 2., top + inner.1 + h / 2.);
    for (x, y, px) in out.enumerate_pixels_mut() {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        // Premultiplied colour and coverage: an opaque background stays
        // opaque, a transparent one picks up the shadow as dark alpha.
        let mut a = if clear { 0. } else { 1. };
        let mut c = [px.0[0] as f32 / 255. * a, px.0[1] as f32 / 255. * a, px.0[2] as f32 / 255. * a];
        if strength > 0. {
            let d = sdf(fx, fy, cx, cy + blur * 0.35);
            let u = ((d + blur * 0.4) / (blur * 1.4)).clamp(0., 1.);
            let k = u * u * (3. - 2. * u);
            let dark = 0.9 * strength * (1. - k);
            c.iter_mut().for_each(|v| *v *= 1. - dark);
            a = a * (1. - dark) + dark;
        }
        // The picture itself, antialiased along its rounded edge.
        let cover = (0.5 - sdf(fx, fy, cx, cy)).clamp(0., 1.);
        let (ix, iy) = (fx - left, fy - top);
        if ix >= 0. && iy >= 0. && ix < iw && iy < ih {
            let (ix, iy) = (ix as u32, iy as u32);
            // The picture with its drawing inside the rounded edge, the
            // drawing alone outside it.
            let mut over = |p: [u8; 4], k: f32| {
                let k = k * p[3] as f32 / 255.;
                for (v, s) in c.iter_mut().zip(p) {
                    *v = *v * (1. - k) + s as f32 / 255. * k;
                }
                a = a * (1. - k) + k;
            };
            if cover < 1. {
                over(drawn.get_pixel(ix, iy).0, 1.);
            }
            if cover > 0. {
                over(img.get_pixel(ix, iy).0, cover);
            }
        }
        let un = |v: f32| if a > 0. { (v / a * 255.).round().clamp(0., 255.) as u8 } else { 0 };
        *px = image::Rgba([un(c[0]), un(c[1]), un(c[2]), (a * 255.).round() as u8]);
    }
    out
}
