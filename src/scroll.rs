//! Scrolling screenshot, like CleanShot's: pick an area, scroll it yourself,
//! and kadr keeps grabbing it and stitches the new rows on at the bottom by
//! matching the rows it has already seen. "Готово" saves one tall picture.

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

/// Rows taller than this stop growing the picture.
const MAX_HEIGHT: usize = 40_000;
/// The scrollbar and its fade on the right change while scrolling.
const IGNORE_RIGHT: usize = 28;

/// Stitches frames of a scrolling area into one tall picture.
pub struct Stitcher {
    width: usize,
    height: usize,
    /// RGB rows of the whole picture so far.
    pixels: Vec<u8>,
    /// Row signatures of the last frame, None for flat (single-colour) rows,
    /// which match anything and can't tell scroll positions apart.
    last: Vec<Option<u64>>,
}

fn signatures(frame: &image::RgbImage) -> Vec<Option<u64>> {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let used = w.saturating_sub(IGNORE_RIGHT).max(1);
    (0..h)
        .map(|y| {
            let row = &frame.as_raw()[y * w * 3..y * w * 3 + used * 3];
            let flat = row.chunks_exact(3).all(|p| p == &row[..3]);
            (!flat).then(|| row.iter().fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3)))
        })
        .collect()
}

impl Stitcher {
    pub fn new(first: image::RgbImage) -> Stitcher {
        let last = signatures(&first);
        Stitcher { width: first.width() as usize, height: first.height() as usize, pixels: first.into_raw(), last }
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// How far `next` scrolled past the last frame, in rows: the shift under
    /// which most of their distinct rows agree. None when they don't line up.
    fn offset(&self, next: &[Option<u64>]) -> Option<usize> {
        let h = next.len().min(self.last.len());
        // Rows that stayed put (a sticky header, a frozen column) say nothing
        // about the scroll; leave them out. Nothing moved at all: no scroll.
        let fixed: Vec<bool> = (0..h).map(|r| next[r].is_some() && next[r] == self.last[r]).collect();
        if (0..h).all(|r| fixed[r] || next[r].is_none()) {
            return Some(0);
        }
        let mut best: Option<(usize, usize)> = None;
        for dy in 1..h {
            let (mut same, mut seen) = (0, 0);
            for r in 0..h - dy {
                if fixed[r] {
                    continue;
                }
                if let (Some(a), Some(b)) = (self.last[r + dy], next[r]) {
                    seen += 1;
                    if a == b {
                        same += 1;
                    }
                }
            }
            // Enough of the overlap has to be real content and agree.
            if seen >= 12 && same * 100 >= seen * 85 && best.is_none_or(|(_, s)| same > s) {
                best = Some((dy, same));
            }
        }
        best.map(|(dy, _)| dy)
    }

    /// Add a frame; true when it brought new rows.
    pub fn push(&mut self, frame: image::RgbImage) -> bool {
        if frame.width() as usize != self.width || self.height >= MAX_HEIGHT {
            return false;
        }
        let next = signatures(&frame);
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

        // Grab and stitch off the UI thread until "Готово" or "Отмена".
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
                std::thread::sleep(Duration::from_millis(120));
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
            return util::notify("Не удалось снять область", "grim ничего не вернул");
        };
        let settings = Settings::load();
        let dir = settings.screenshot_dir();
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("screenshot-{}.png", util::timestamp()));
        if let Err(e) = picture.save(&path) {
            return util::notify("Не удалось сохранить", &e.to_string());
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
                        .child(Label::new("Прокручивай — kadr склеит").color(Color::Muted))
                        .child(div().w(px(90.)).font_family(kit::MONO_FONT).text_color(cx.theme().colors().text_muted).child(format!("{height} px")))
                        .child(
                            Button::new("cancel", "Отмена")
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Large)
                                .on_click(move |_, _, _| {
                                    cancel.cancelled.store(true, Ordering::Relaxed);
                                    cancel.stop.store(true, Ordering::Relaxed);
                                }),
                        )
                        .child(
                            Button::new("done", "Готово")
                                .style(ButtonStyle::Accent)
                                .size(ButtonSize::Large)
                                .tooltip(Tooltip::text("Или нажми горячую клавишу ещё раз"))
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
        for top in [70, 190, 400, 600] {
            assert!(s.push(view(&full, top, 300)));
        }
        let out = s.image().unwrap();
        assert_eq!(out.height(), 900);
        assert_eq!(out, full);
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
