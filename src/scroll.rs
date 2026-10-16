//! Scrolling screenshot, like CleanShot's: pick an area, scroll it yourself,
//! and kadr keeps grabbing it and stitches the new rows on at the bottom by
//! matching the rows it has already seen. "scroll.done" saves one tall picture.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use crate::kit::{Anchor, KeyboardInteractivity};
use gpui::{App, Context, Global, Window, WindowHandle, div, px};
use crate::ui::{Tooltip, prelude::*};

use crate::kit::{self, Hold, InputRegion, LayerSpec};
use crate::settings::Settings;
use crate::util::{self, Mon};
use crate::capture;
use crate::i18n::t;

/// Rows taller than this stop growing the picture.
const MAX_HEIGHT: usize = 40_000;
/// The scrollbar and its fade on the right change while scrolling.
const IGNORE_RIGHT: usize = 28;

/// Brightness samples per row: rows are compared by these, not exactly, since
/// with fractional scaling a row that scrolled lands on slightly different
/// pixels and never matches byte for byte while the page moves.
const BINS: usize = 32;
/// Rows that didn't move come out identical; this only absorbs rounding.
const SAME_ROW: f32 = 0.1;
/// The best shift has to line rows up at least this well.
const MATCH: f32 = 6.0;

/// One row, as average brightness across `BINS` strips.
type Row = [f32; BINS];

/// Stitches frames of a scrolling area into one tall picture.
pub struct Stitcher {
    width: usize,
    height: usize,
    /// RGB rows of the whole picture so far.
    pixels: Vec<u8>,
    /// Rows of the last frame, None for flat (single-colour) rows, which
    /// match anything and can't tell scroll positions apart.
    last: Vec<Option<Row>>,
}

fn rows(frame: &image::RgbImage) -> Vec<Option<Row>> {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let used = w.saturating_sub(IGNORE_RIGHT).max(BINS);
    let used = used.min(w);
    (0..h)
        .map(|y| {
            let line = &frame.as_raw()[y * w * 3..y * w * 3 + used * 3];
            let mut row = [0f32; BINS];
            for (i, bin) in row.iter_mut().enumerate() {
                let (a, b) = (i * used / BINS, ((i + 1) * used / BINS).max(i * used / BINS + 1));
                let sum: u32 = line[a * 3..b * 3].iter().map(|&v| v as u32).sum();
                *bin = sum as f32 / ((b - a) * 3) as f32;
            }
            let first = &line[..3];
            let flat = line.chunks_exact(3).all(|p| p.iter().zip(first).all(|(&a, &b)| a.abs_diff(b) <= 2));
            (!flat).then_some(row)
        })
        .collect()
}

fn distance(a: &Row, b: &Row) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / BINS as f32
}

impl Stitcher {
    pub fn new(first: image::RgbImage) -> Stitcher {
        let last = rows(&first);
        Stitcher { width: first.width() as usize, height: first.height() as usize, pixels: first.into_raw(), last }
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// How far `next` scrolled past the last frame, in rows: the shift under
    /// which their rows line up best. None when they don't line up.
    fn offset(&self, next: &[Option<Row>]) -> Option<usize> {
        let h = next.len().min(self.last.len());
        // Rows that stayed put (a sticky header, a frozen column) say nothing
        // about the scroll; leave them out. Nothing moved at all: no scroll.
        let fixed: Vec<bool> = (0..h).map(|r| matches!((&next[r], &self.last[r]), (Some(a), Some(b)) if distance(a, b) < SAME_ROW)).collect();
        if (0..h).all(|r| fixed[r] || next[r].is_none()) {
            return Some(0);
        }
        let mut best: Option<(usize, f32)> = None;
        for dy in 1..h {
            let (mut total, mut seen) = (0f32, 0usize);
            for r in 0..h - dy {
                if fixed[r] {
                    continue;
                }
                if let (Some(a), Some(b)) = (&self.last[r + dy], &next[r]) {
                    seen += 1;
                    total += distance(a, b);
                }
            }
            // Enough of the overlap has to be real content: a few rows line up
            // with something by chance. Frames come often, so a real scroll
            // leaves plenty of overlap.
            if seen < 12 || (h - dy) * 3 < h {
                continue;
            }
            let score = total / seen as f32;
            if best.is_none_or(|(_, s)| score < s) {
                best = Some((dy, score));
            }
        }
        best.filter(|&(_, score)| score < MATCH).map(|(dy, _)| dy)
    }

    /// Add a frame; true when it brought new rows.
    pub fn push(&mut self, frame: image::RgbImage) -> bool {
        if frame.width() as usize != self.width || self.height >= MAX_HEIGHT {
            return false;
        }
        let next = rows(&frame);
        let Some(dy) = self.offset(&next) else { return false };
        let h = frame.height() as usize;
        self.last = next;
        if dy == 0 {
            return false;
        }
        // The bottom `dy` rows of the new frame are new.
        let rows = &frame.as_raw()[(h - dy) * self.width * 3..];
        self.pixels.extend_from_slice(rows);
        self.height += dy;
        true
    }

    pub fn image(self) -> Option<image::RgbImage> {
        image::RgbImage::from_raw(self.width as u32, self.height as u32, self.pixels)
    }
}

/// Grab the area as an image (grim writes PPM to stdout).
fn grab(geometry: &str) -> Option<image::RgbImage> {
    let out = std::process::Command::new("grim").args(["-g", geometry, "-t", "ppm", "-"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    image::load_from_memory_with_format(&out.stdout, image::ImageFormat::Pnm).ok().map(|i| i.to_rgb8())
}

/// The running capture: shared with the grabbing thread.
struct Shared {
    stop: AtomicBool,
    cancelled: AtomicBool,
    height: AtomicU32,
}

#[derive(Default)]
struct Session(Option<(Arc<Shared>, Option<WindowHandle<ScrollBar>>)>);

impl Global for Session {}

/// `kadr shot scroll`: start, or finish when already running (the same hotkey).
/// The area comes from the picker unless given as "X,Y WxH".
pub fn start(area: Option<String>, cx: &mut App) {
    if let Some((shared, _)) = cx.try_global::<Session>().and_then(|s| s.0.as_ref()) {
        shared.stop.store(true, Ordering::Relaxed);
        return;
    }
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let area = match area {
            Some(area) => area,
            None => match crate::picker::pick(true, true, cx).await {
                Some(picked) => picked.geometry(),
                None => return,
            },
        };
        let Some((x, y, w, h)) = parse(area.trim()) else { return };
        let geometry = format!("{x},{y} {w}x{h}");
        let shared = Arc::new(Shared { stop: AtomicBool::new(false), cancelled: AtomicBool::new(false), height: AtomicU32::new(0) });

        let mon = util::monitors().into_iter().find(|m| m.contains(x, y));
        let window = cx.update(|cx| open_bar(cx, mon, (x, y, w, h), shared.clone(), kit::hold(cx)));
        let _ = cx.update(|cx| cx.set_global(Session(Some((shared.clone(), window)))));

        // Grab and stitch off the UI thread until "scroll.done" or "common.cancel".
        let (tx, rx) = futures::channel::oneshot::channel();
        let worker = shared.clone();
        std::thread::spawn(move || {
            let mut stitcher = None::<Stitcher>;
            while !worker.stop.load(Ordering::Relaxed) {
                if let Some(frame) = grab(&geometry) {
                    match stitcher.as_mut() {
                        Some(s) => {
                            s.push(frame);
                        }
                        None => stitcher = Some(Stitcher::new(frame)),
                    }
                    if let Some(s) = &stitcher {
                        worker.height.store(s.height() as u32, Ordering::Relaxed);
                    }
                }
                // Grab again right away: the more frames overlap, the faster
                // one can scroll without losing the thread.
                std::thread::sleep(Duration::from_millis(15));
            }
            let _ = tx.send(stitcher.and_then(Stitcher::image));
        });
        let picture = rx.await.ok().flatten();

        let _ = cx.update(|cx| {
            if let Some(w) = cx.try_global::<Session>().and_then(|s| s.0.as_ref()).and_then(|(_, w)| *w) {
                let _ = w.update(cx, |_, window, _| window.remove_window());
            }
            cx.set_global(Session(None));
        });
        if shared.cancelled.load(Ordering::Relaxed) {
            return;
        }
        let Some(picture) = picture else {
            return util::notify(t("scroll.couldnt_capture_area"), t("scroll.grim_returned_nothing"));
        };
        let settings = Settings::load();
        let dir = settings.screenshot_dir();
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("screenshot-{}.png", util::timestamp()));
        if let Err(e) = picture.save(&path) {
            return util::notify(t("common.couldnt_save"), &e.to_string());
        }
        let _ = cx.update(|cx| capture::deliver(path, &settings, cx));
        drop(hold);
    })
    .detach();
}

/// "X,Y WxH".
fn parse(area: &str) -> Option<(i32, i32, i32, i32)> {
    let (pos, size) = area.split_once(' ')?;
    let (x, y) = pos.split_once(',')?;
    let (w, h) = size.split_once('x')?;
    Some((x.parse().ok()?, y.parse().ok()?, w.parse().ok()?, h.parse().ok()?))
}

fn open_bar(cx: &mut App, mon: Option<Mon>, area: (i32, i32, i32, i32), shared: Arc<Shared>, hold: Hold) -> Option<WindowHandle<ScrollBar>> {
    let mon = mon?;
    let spec = LayerSpec {
        namespace: "kadr-scroll",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mon.w as f32, mon.h as f32),
        // The app being scrolled keeps the keyboard and the pointer.
        keyboard: KeyboardInteractivity::None,
        ignore_zones: true,
        monitor: Some(mon.clone()),
    };
    let local = (area.0 - mon.x, area.1 - mon.y, area.2, area.3);
    let screen = (mon.w, mon.h);
    kit::open_layer(cx, spec, move |window, cx| {
        cx.new(|cx| {
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(200)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            })
            .detach();
            ScrollBar { area: local, screen, shared, region: InputRegion::default(), _hold: hold }
        })
    })
}

/// The frame around the area and the bar with the stitched height and the
/// buttons; only the bar takes clicks, so scrolling reaches the app.
pub struct ScrollBar {
    area: (i32, i32, i32, i32),
    screen: (i32, i32),
    shared: Arc<Shared>,
    region: InputRegion,
    _hold: Hold,
}

impl Render for ScrollBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.region.reset();
        let (x, y, w, h) = (self.area.0 as f32, self.area.1 as f32, self.area.2 as f32, self.area.3 as f32);
        let accent = cx.theme().colors().text_accent;
        let height = self.shared.height.load(Ordering::Relaxed);
        let bar_h = 52. * kit::SCALE;
        // Below the area, or above it when there's no room, or inside.
        let below = y + h + 12.;
        let top = if below + bar_h < self.screen.1 as f32 - 8. { below } else if y - 12. - bar_h > 8. { y - 12. - bar_h } else { y + h - bar_h - 12. };
        let (stop, cancel) = (self.shared.clone(), self.shared.clone());
        kit::root(cx)
            .relative()
            // The frame sits just outside the area so it isn't captured.
            .child(div().absolute().left(px(x - 3.)).top(px(y - 3.)).w(px(w + 6.)).h(px(h + 6.)).border_2().border_color(accent).rounded(px(4.)))
            .child(
                // Centred under the area, kept on screen.
                div().absolute().left(px((x - 300.).max(0.))).w(px((w + 600.).min(self.screen.0 as f32 - (x - 300.).max(0.)))).top(px(top)).flex().justify_center().child(
                    h_flex()
                        .gap_2()
                        .p_1()
                        .pl_3()
                        .elevation_3(cx)
                        .child(Label::new(t("scroll.scroll_kadr_stitches")).color(Color::Muted))
                        .child(div().w(px(90.)).font_family(kit::MONO_FONT).text_color(cx.theme().colors().text_muted).child(format!("{height} px")))
                        .child(
                            Button::new("cancel", t("common.cancel"))
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Large)
                                .on_click(move |_, _, _| {
                                    cancel.cancelled.store(true, Ordering::Relaxed);
                                    cancel.stop.store(true, Ordering::Relaxed);
                                }),
                        )
                        .child(
                            Button::new("done", t("scroll.done"))
                                .style(ButtonStyle::Accent)
                                .size(ButtonSize::Large)
                                .tooltip(Tooltip::text(t("scroll.press_hotkey_again")))
                                .on_click(move |_, _, _| stop.stop.store(true, Ordering::Relaxed)),
                        )
                        .child(self.region.mark()),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tall page with distinct rows, seen through a window that scrolls.
    fn page(height: u32) -> image::RgbImage {
        image::RgbImage::from_fn(80, height, |x, y| image::Rgb([(y % 251) as u8, (y / 251) as u8 * 40 + (x % 7) as u8, (x * 3) as u8]))
    }

    fn view(page: &image::RgbImage, top: u32, h: u32) -> image::RgbImage {
        image::imageops::crop_imm(page, 0, top, page.width(), h).to_image()
    }

    #[test]
    fn stitches_a_scrolled_page_back_together() {
        let full = page(900);
        let mut s = Stitcher::new(view(&full, 0, 300));
        assert!(!s.push(view(&full, 0, 300)), "no scroll, nothing new");
        for top in [70, 190, 350, 520, 600] {
            assert!(s.push(view(&full, top, 300)));
        }
        let out = s.image().unwrap();
        assert_eq!(out.height(), 900);
        assert_eq!(out, full);
    }

    /// With fractional scaling a page that scrolls lands between pixels: each
    /// frame is a blend of two neighbouring rows, never an exact copy.
    #[test]
    fn stitches_frames_that_land_between_pixels() {
        // Smooth like rendered content, so a blend of two rows sits between them.
        let full = image::RgbImage::from_fn(240, 1200, |x, y| {
            let (y, x) = (y as f32, x as f32);
            let v = 128. + 60. * (y * 0.071).sin() + 60. * (y * 0.0173 + x * 0.05).sin();
            image::Rgb([v as u8, (v * 0.5) as u8, (x as u32 % 256) as u8])
        });
        let half = |top: u32| {
            let (a, b) = (view(&full, top, 300), view(&full, top + 1, 300));
            image::RgbImage::from_fn(a.width(), a.height(), |x, y| {
                let (p, q) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
                image::Rgb([0, 1, 2].map(|i| ((p[i] as u16 + q[i] as u16) / 2) as u8))
            })
        };
        let mut s = Stitcher::new(view(&full, 0, 300));
        let mut tops = Vec::new();
        for top in [40, 95, 170, 260, 330, 420, 515, 600, 700, 820] {
            if top % 2 == 0 { s.push(half(top)) } else { s.push(view(&full, top, 300)) };
            tops.push(top);
        }
        let end = *tops.last().unwrap() as i64 + 300;
        assert!((s.height() as i64 - end).abs() <= 2, "height {} vs {end}", s.height());
    }

    /// KADR_TEST_PAGE=<tall png>: a real page with a sticky header and a
    /// scrollbar that moves, scrolled by uneven steps.
    #[test]
    fn stitches_a_real_page() {
        let Ok(path) = std::env::var("KADR_TEST_PAGE") else { return };
        let full = image::open(path).unwrap().to_rgb8();
        let (w, h) = (full.width(), 700);
        let header = image::imageops::crop_imm(&full, 0, 0, w, 90).to_image();
        let shot = |top: u32| {
            let mut v = view(&full, top, h);
            image::imageops::replace(&mut v, &header, 0, 0);
            // The scrollbar thumb moves with the page.
            for y in 0..h {
                for x in w - 12..w {
                    let on = y >= top * h / full.height() && y < top * h / full.height() + 60;
                    v.put_pixel(x, y, if on { image::Rgb([200, 200, 200]) } else { image::Rgb([40, 40, 40]) });
                }
            }
            v
        };
        let mut s = Stitcher::new(shot(0));
        let mut top = 0;
        for step in [37, 120, 260, 95, 300, 180, 410] {
            top = (top + step).min(full.height() - h);
            s.push(shot(top));
        }
        let out = s.image().unwrap();
        assert_eq!(out.height(), top + h, "stitched to the bottom of the page");
        let a = image::imageops::crop_imm(&out, 0, 300, w - 30, 600).to_image();
        let b = image::imageops::crop_imm(&full, 0, 300, w - 30, 600).to_image();
        assert!(a == b, "content matches the page");
    }
}
