use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AsyncApp};

use crate::settings::Settings;
use crate::{cards, kit, picker, util};
use crate::i18n::t;

/// Take a screenshot with kadr's own picker, then copy it and show its card.
pub fn shot(mode: String, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        // Keep existing cards out of the frozen frame; give the compositor
        // a moment to present the emptied overlay.
        if cx.update(|cx| cards::set_hidden(true, cx)) {
            cx.background_executor().timer(Duration::from_millis(120)).await;
        }
        let settings = Settings::load();
        let result = take(&mode, &settings, cx).await;
        let _ = cx.update(|cx| {
            cards::set_hidden(false, cx);
            // A cancelled pick gives nothing.
            if let Some(path) = result {
                deliver(path, &settings, cx);
            }
        });
    })
    .detach();
}

/// What happens to a fresh screenshot: as set in the settings.
pub fn deliver(path: PathBuf, _settings: &Settings, cx: &mut App) {
    cards::after_capture(path, crate::storage::Capture::Screenshot, cx);
}

/// Freeze the screen, pick (or take the whole monitor), save the picked part.
async fn take(mode: &str, settings: &Settings, cx: &mut AsyncApp) -> Option<PathBuf> {
    let picked = match mode {
        "fullscreen" => picker::whole_monitor().await?,
        // The self-timer: 3, 2, 1 over the screen, then all of it.
        "timer" => {
            let mons = util::monitors();
            let name = mons.iter().find(|m| m.focused)?.name.clone();
            if !crate::record::countdown(&crate::record::Target::Monitor(name), 3, &mons, cx).await {
                return None;
            }
            picker::whole_monitor().await?
        }
        // The last area again, as it is now.
        "previous" => picker::previous()?,
        // Free selection; the others also snap to windows.
        "region" => picker::pick(false, true, cx).await?,
        _ => picker::pick(true, true, cx).await?,
    };
    let dir = settings.screenshot_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = crate::storage::capture_path(&dir, crate::storage::Capture::Screenshot, "png");
    let target = path.clone();
    // A clicked window gets its rounded corners and a soft shadow on a
    // transparent background, like a macOS window shot.
    let shadow = (picked.window && window_shadow()).then(|| (crate::platform::window_rounding(), picked.mon.scale));
    // Encoding a big PNG takes a moment; keep the UI free.
    let saved = cx
        .background_executor()
        .spawn(async move {
            let image = picked.crop();
            match shadow {
                Some((rounding, scale)) => with_shadow(&image, rounding, scale).save(&target),
                None => image.save(&target),
            }
        })
        .await;
    match saved {
        Ok(()) => Some(path),
        Err(e) => {
            util::notify(t("common.couldnt_save"), &e.to_string());
            None
        }
    }
}

/// Window shots get a shadow (the `window_shadow` setting, on by default).
/// Read straight from settings.json, so it works before the settings window
/// shows it.
pub fn window_shadow() -> bool {
    std::fs::read_to_string(util::config_dir().join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["window_shadow"].as_bool())
        .unwrap_or(true)
}

/// How much of the pixel at (x, y) lies inside a w × h rectangle with
/// corners rounded by `r`, all in pixels: 0 to 1, smoothed over a pixel.
fn coverage(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    if x < 0. || y < 0. || x > w || y > h {
        return 0.;
    }
    let r = r.min(w / 2.).min(h / 2.);
    let (cx, cy) = (x.clamp(r, w - r), y.clamp(r, h - r));
    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
    (r - d + 0.5).clamp(0., 1.)
}

/// A window shot with its corners rounded by `rounding` (logical pixels)
/// and a soft shadow under it, on a transparent background.
pub fn with_shadow(image: &image::RgbaImage, rounding: f32, scale: f64) -> image::RgbaImage {
    use image::imageops;

    let s = scale as f32;
    let (w, h) = image.dimensions();
    let (fw, fh) = (w as f32, h as f32);
    let radius = rounding * s;
    let (blur, drop, opacity) = (20. * s, 10. * s, 0.5);
    let margin = (blur * 2.5 + drop).ceil() as u32;

    let mut window = image.clone();
    if radius > 0.5 {
        for (x, y, p) in window.enumerate_pixels_mut() {
            p.0[3] = (p.0[3] as f32 * coverage(x as f32 + 0.5, y as f32 + 0.5, fw, fh, radius)).round() as u8;
        }
    }

    // The shadow is the window's shape moved down and blurred, worked out at
    // a quarter of the size: the blur is slow and comes out smooth anyway.
    const STEP: u32 = 4;
    let (cw, ch) = (w + 2 * margin, h + 2 * margin);
    let mask = image::GrayImage::from_fn(cw.div_ceil(STEP), ch.div_ceil(STEP), |u, v| {
        let x = (u * STEP) as f32 + STEP as f32 / 2. - margin as f32;
        let y = (v * STEP) as f32 + STEP as f32 / 2. - margin as f32 - drop;
        image::Luma([(255. * coverage(x, y, fw, fh, radius)).round() as u8])
    });
    let mask = imageops::blur(&mask, blur / STEP as f32);
    let mask = imageops::resize(&mask, cw, ch, imageops::FilterType::Triangle);
    let mut out = image::RgbaImage::from_fn(cw, ch, |x, y| image::Rgba([0, 0, 0, (mask.get_pixel(x, y).0[0] as f32 * opacity) as u8]));
    imageops::overlay(&mut out, &window, margin as i64, margin as i64);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_shots_get_round_corners_and_a_shadow_around() {
        let shot = image::RgbaImage::from_pixel(200, 100, image::Rgba([255, 255, 255, 255]));
        let out = with_shadow(&shot, 10., 1.);
        let margin = (out.width() - 200) / 2;
        assert_eq!(out.height(), 100 + 2 * margin);
        // The window itself stays opaque; its corner pixel is cut away.
        assert_eq!(out.get_pixel(margin + 100, margin + 50).0, [255, 255, 255, 255]);
        assert!(out.get_pixel(margin, margin).0[3] < 128);
        // The shadow falls below, darker there than above, and fades out.
        let below = out.get_pixel(margin + 100, margin + 100 + 8).0[3];
        let above = out.get_pixel(margin + 100, margin - 8).0[3];
        assert!(below > above && below > 20, "below {below}, above {above}");
        assert_eq!(out.get_pixel(0, 0).0[3], 0);
    }
}
