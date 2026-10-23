use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AsyncApp};

use crate::settings::Settings;
use crate::util::Mon;
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

/// How long taking the picture and writing it may take.
const SAVING: Duration = Duration::from_secs(15);

/// Freeze the screen, pick (or take the whole monitor), save the picked part.
async fn take(mode: &str, settings: &Settings, cx: &mut AsyncApp) -> Option<PathBuf> {
    if mode == "all" {
        return all_screens(settings, cx).await;
    }
    let picked = match mode {
        "fullscreen" => picker::whole_monitor(cx).await?,
        // The self-timer: counting down over the screen, then all of it.
        "timer" => {
            let mons = util::monitors();
            let name = mons.iter().find(|m| m.focused)?.name.clone();
            if !crate::record::countdown(&crate::record::Target::Monitor(name), settings.timer_seconds, &mons, cx).await {
                return None;
            }
            picker::whole_monitor(cx).await?
        }
        // The last area again, as it is now.
        "previous" => picker::previous(cx).await?,
        // Free selection; the others also snap to windows.
        "region" => picker::pick(false, true, cx).await?,
        _ => picker::pick(true, true, cx).await?,
    };
    eprintln!("kadr: shot {mode}: {:?}{}", picked.area, if picked.window { " (a window)" } else { "" });
    let dir = settings.screenshot_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = crate::storage::capture_path(&dir, crate::storage::Capture::Screenshot, "png");
    let target = path.clone();
    // A clicked window gets its rounded corners and a soft shadow on a
    // transparent background, like a macOS window shot.
    let shadow = (picked.window && window_shadow()).then(|| (crate::platform::window_rounding(), picked.mon.scale));
    let (scale, logical) = (picked.mon.scale, settings.logical_size);
    // Encoding a big PNG takes a moment; keep the UI free.
    // On a thread of its own, within a limit: taking a window by itself
    // asks the system, which may not answer.
    let saved = util::off_main(cx, SAVING, move || {
        // A clicked window by itself where the system gives it, else cut
        // from the frozen screen.
        let alone = picked.window_alone();
        if picked.window {
            eprintln!("kadr: window shot {}", if alone.is_some() { "by itself" } else { "cut from the screen" });
        }
        let image = alone.unwrap_or_else(|| picked.crop());
        let image = match shadow {
            Some((rounding, scale)) => with_shadow(&image, rounding, scale),
            None => image,
        };
        save_png(image, scale, logical, &target)
    })
    .await;
    let Some(saved) = saved else {
        util::capture_timed_out("the window shot", cx);
        return None;
    };
    eprintln!("kadr: shot {mode} saved: {}", saved.is_ok());
    match saved {
        Ok(()) => Some(path),
        Err(e) => {
            util::notify(t("common.couldnt_save"), &e.to_string());
            None
        }
    }
}

/// Every monitor at once, laid out as the desktop has them.
async fn all_screens(settings: &Settings, cx: &mut AsyncApp) -> Option<PathBuf> {
    let dir = settings.screenshot_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = crate::storage::capture_path(&dir, crate::storage::Capture::Screenshot, "png");
    let (target, logical) = (path.clone(), settings.logical_size);
    let saved = util::off_main(cx, SAVING, move || {
        let shots: Vec<(Mon, image::RgbaImage)> = util::monitors().into_iter().filter_map(|m| Some((m.clone(), picker::grab(&m)?))).collect();
        let (image, scale) = side_by_side(&shots).ok_or_else(|| std::io::Error::other(t("common.couldnt_save")))?;
        save_png(image, scale, logical, &target).map_err(std::io::Error::other)
    })
    .await;
    let Some(saved) = saved else {
        util::capture_timed_out("every screen", cx);
        return None;
    };
    match saved {
        Ok(()) => Some(path),
        Err(e) => {
            util::notify(t("common.couldnt_save"), &e.to_string());
            None
        }
    }
}

/// Monitors' pictures put where the monitors are, at the sharpest one's
/// scale (the others scaled up to it); the gaps between them transparent.
fn side_by_side(shots: &[(Mon, image::RgbaImage)]) -> Option<(image::RgbaImage, f64)> {
    let scale = shots.iter().map(|(m, _)| m.scale).fold(0., f64::max);
    let left = shots.iter().map(|(m, _)| m.x).min()?;
    let top = shots.iter().map(|(m, _)| m.y).min()?;
    let right = shots.iter().map(|(m, _)| m.x + m.w).max()?;
    let bottom = shots.iter().map(|(m, _)| m.y + m.h).max()?;
    let px = |v: i32| (v as f64 * scale).round() as u32;
    let mut out = image::RgbaImage::new(px(right - left).max(1), px(bottom - top).max(1));
    for (m, shot) in shots {
        let (w, h) = (px(m.w), px(m.h));
        let shot = if shot.dimensions() == (w, h) { shot.clone() } else { image::imageops::resize(shot, w, h, image::imageops::FilterType::Lanczos3) };
        image::imageops::replace(&mut out, &shot, px(m.x - left) as i64, px(m.y - top) as i64);
    }
    Some((out, scale))
}

/// Save a screenshot taken in screen pixels at `scale`: scaled down to 1×
/// with `logical`, and with its density in the PNG either way.
pub fn save_png(image: impl Into<image::DynamicImage>, scale: f64, logical: bool, path: &std::path::Path) -> image::ImageResult<()> {
    let image: image::DynamicImage = image.into();
    let (image, density) = if logical && (scale - 1.).abs() > 1e-3 {
        let size = |n: u32| ((n as f64 / scale).round() as u32).max(1);
        (image.resize_exact(size(image.width()), size(image.height()), image::imageops::FilterType::Lanczos3), 1.)
    } else {
        (image, if logical { 1. } else { scale })
    };
    let mut png = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    std::fs::write(path, cards::with_density(png, density)).map_err(image::ImageError::IoError)
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

/// How far (x, y) lies outside a w × h rectangle with corners rounded by
/// `r`, in pixels; 0 inside.
fn outside(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let r = r.min(w / 2.).min(h / 2.);
    let (dx, dy) = ((r - x).max(x - (w - r)).max(0.), (r - y).max(y - (h - r)).max(0.));
    ((dx * dx + dy * dy).sqrt() - r).max(0.)
}

/// A window shot with its corners rounded by `rounding` (logical pixels)
/// and a shadow under it like a macOS window's, on a transparent background:
/// a wide faint shadow cast downwards, a little ambient one all around and a
/// thin dark rim right at the edge, so the window doesn't sit in a grey haze.
pub fn with_shadow(image: &image::RgbaImage, rounding: f32, scale: f64) -> image::RgbaImage {
    use image::imageops;

    let s = scale as f32;
    let (w, h) = image.dimensions();
    let (fw, fh) = (w as f32, h as f32);
    let radius = rounding * s;
    // (blur sigma, drop, opacity) of the cast and the ambient shadow.
    let layers = [(20. * s, 18. * s, 0.22), (4. * s, 1. * s, 0.10)];
    let (rim, rim_opacity) = (1. * s, 0.18);
    let margin = (20. * s * 2.5 + 18. * s).ceil() as u32;

    let mut window = image.clone();
    if radius > 0.5 {
        for (x, y, p) in window.enumerate_pixels_mut() {
            p.0[3] = (p.0[3] as f32 * coverage(x as f32 + 0.5, y as f32 + 0.5, fw, fh, radius)).round() as u8;
        }
    }

    // Each shadow is the window's shape moved down and blurred, worked out at
    // a quarter of the size: the blur is slow and comes out smooth anyway.
    const STEP: u32 = 4;
    let (cw, ch) = (w + 2 * margin, h + 2 * margin);
    let masks: Vec<_> = layers
        .iter()
        .map(|&(blur, drop, _)| {
            let mask = image::GrayImage::from_fn(cw.div_ceil(STEP), ch.div_ceil(STEP), |u, v| {
                let x = (u * STEP) as f32 + STEP as f32 / 2. - margin as f32;
                let y = (v * STEP) as f32 + STEP as f32 / 2. - margin as f32 - drop;
                image::Luma([(255. * coverage(x, y, fw, fh, radius)).round() as u8])
            });
            imageops::resize(&imageops::blur(&mask, blur / STEP as f32), cw, ch, imageops::FilterType::Triangle)
        })
        .collect();
    let mut out = image::RgbaImage::from_fn(cw, ch, |x, y| {
        let mut clear = 1.;
        for (mask, &(_, _, opacity)) in masks.iter().zip(&layers) {
            clear *= 1. - mask.get_pixel(x, y).0[0] as f32 / 255. * opacity;
        }
        let d = outside(x as f32 + 0.5 - margin as f32, y as f32 + 0.5 - margin as f32, fw, fh, radius);
        clear *= 1. - rim_opacity * (-(d / rim).powi(2) / 2.).exp();
        image::Rgba([0, 0, 0, (255. * (1. - clear)).round() as u8])
    });
    imageops::overlay(&mut out, &window, margin as i64, margin as i64);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_screen_goes_where_it_is_at_the_sharpest_scale() {
        let mon = |name: &str, x, y, w, h, scale| Mon { name: name.into(), x, y, w, h, phys: ((w as f64 * scale) as i64, (h as f64 * scale) as i64), scale, hz: 60., focused: false };
        let plain = |w, h, v| image::RgbaImage::from_pixel(w, h, image::Rgba([v, v, v, 255]));
        // A 2× laptop at the left, a 1× monitor right of it and lower.
        let shots = [(mon("eDP-1", 0, 0, 100, 50, 2.), plain(200, 100, 10)), (mon("DP-1", 100, 20, 80, 40, 1.), plain(80, 40, 200))];
        let (out, scale) = side_by_side(&shots).unwrap();
        assert_eq!((out.dimensions(), scale), ((360, 120), 2.));
        assert_eq!(out.get_pixel(10, 10).0, [10, 10, 10, 255]);
        // The 1× one scaled up to 2×, placed at (200, 40).
        assert_eq!(out.get_pixel(210, 50).0, [200, 200, 200, 255]);
        assert_eq!(out.get_pixel(359, 119).0, [200, 200, 200, 255]);
        // Under the laptop and above the monitor: nothing.
        assert_eq!(out.get_pixel(300, 10).0[3], 0);
    }

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
