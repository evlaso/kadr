//! Scrolling screenshot: pick an area, scroll it yourself (down or to the
//! right), and kadr keeps grabbing it and stitches what comes into view onto
//! one long picture (`kadr_raster::stitch`). "scroll.done" saves it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use crate::kit::{Anchor, KeyboardInteractivity};
use gpui::{AnyWindowHandle, App, Context, Global, Window, div, px};
use kadr_raster::stitch::Stitcher;
use crate::ui::{Tooltip, prelude::*};

use crate::kit::{self, Hold, InputRegion, LayerSpec};
use crate::settings::Settings;
use crate::util::{self, Mon};
use crate::capture;
use crate::i18n::t;

/// The frame's line, and its gap to the area (so it isn't captured).
const LINE: f32 = 2.;
const GAP: f32 = 1.;

/// The running capture: shared with the grabbing thread.
struct Shared {
    stop: AtomicBool,
    cancelled: AtomicBool,
    /// The picture's length so far along the scroll.
    length: AtomicU32,
}

/// The running capture and its windows.
#[derive(Default)]
struct Session(Option<(Arc<Shared>, Vec<AnyWindowHandle>)>);

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
        let shared = Arc::new(Shared { stop: AtomicBool::new(false), cancelled: AtomicBool::new(false), length: AtomicU32::new(0) });

        let mon = util::monitors().into_iter().find(|m| m.contains(x, y));
        let mon_scale = mon.as_ref().map(|m| m.scale);
        let windows = cx.update(|cx| open_windows(cx, mon, (x, y, w, h), shared.clone()));
        let _ = cx.update(|cx| cx.set_global(Session(Some((shared.clone(), windows)))));

        // Grab and stitch off the UI thread until "scroll.done" or "common.cancel".
        let (tx, rx) = futures::channel::oneshot::channel();
        let worker = shared.clone();
        std::thread::spawn(move || {
            let mut stitcher = None::<Stitcher>;
            while !worker.stop.load(Ordering::Relaxed) {
                if let Some(frame) = crate::platform::grab_area(x, y, w, h) {
                    match stitcher.as_mut() {
                        Some(s) => {
                            s.push(frame);
                        }
                        None => stitcher = Some(Stitcher::new(frame)),
                    }
                    if let Some(s) = &stitcher {
                        let (w, h) = s.size();
                        let length = if s.axis() == Some(kadr_raster::stitch::Axis::Right) { w } else { h };
                        worker.length.store(length as u32, Ordering::Relaxed);
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
            let windows = cx.try_global::<Session>().and_then(|s| s.0.as_ref()).map(|(_, w)| w.clone()).unwrap_or_default();
            for w in windows {
                let _ = w.update(cx, |_, window, _| window.remove_window());
            }
            cx.set_global(Session(None));
        });
        if shared.cancelled.load(Ordering::Relaxed) {
            return;
        }
        let Some(picture) = picture else {
            return util::notify(t("scroll.couldnt_capture_area"), t("scroll.nothing_grabbed"));
        };
        let settings = Settings::load();
        let dir = settings.screenshot_dir();
        let _ = std::fs::create_dir_all(&dir);
        let path = crate::storage::capture_path(&dir, crate::storage::Capture::Screenshot, "png");
        let scale = mon_scale.unwrap_or(1.);
        if let Err(e) = capture::save_png(picture, scale, settings.logical_size, &path) {
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

/// The frame around the area and the bar. On Wayland one layer over the
/// monitor, clicks passing through all but the bar; elsewhere a window can't
/// let clicks through parts of itself, so the frame is four thin windows
/// beside the area and the bar a window of its own.
fn open_windows(cx: &mut App, mon: Option<Mon>, area: (i32, i32, i32, i32), shared: Arc<Shared>) -> Vec<AnyWindowHandle> {
    let Some(mon) = mon else { return Vec::new() };
    let local = (area.0 - mon.x, area.1 - mon.y, area.2, area.3);
    let screen = (mon.w, mon.h);
    let layer = |cx: &mut App, at: (f32, f32), size: (f32, f32), build: Box<dyn FnOnce(&mut Window, &mut App) -> gpui::AnyView>| {
        let spec = LayerSpec {
            namespace: "kadr-scroll",
            anchor: Anchor::TOP | Anchor::LEFT,
            margin: (at.1, 0., 0., at.0),
            size,
            // The app being scrolled keeps the keyboard and the pointer.
            keyboard: KeyboardInteractivity::None,
            ignore_zones: true,
            monitor: Some(mon.clone()),
        };
        kit::open_layer(cx, spec, move |window, cx| {
            let view = build(window, cx);
            cx.new(|_| Strip(view))
        })
        .map(|w| w.into())
    };
    let bar = move |frame: bool, cx: &mut App| -> Box<dyn FnOnce(&mut Window, &mut App) -> gpui::AnyView> {
        let hold = kit::hold(cx);
        let shared = shared.clone();
        Box::new(move |window, cx| {
            cx.new(|cx| {
                // The stitched length, as it grows.
                cx.spawn_in(window, async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_millis(200)).await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                })
                .detach();
                ScrollBar { area: local, screen, frame, shared, region: InputRegion::default(), _hold: hold }
            })
            .into()
        })
    };
    if cfg!(target_os = "linux") {
        let build = bar(true, cx);
        return layer(cx, (0., 0.), (mon.w as f32, mon.h as f32), build).into_iter().collect();
    }
    let (x, y, w, h) = (local.0 as f32, local.1 as f32, local.2 as f32, local.3 as f32);
    let out = GAP + LINE;
    let mut windows = Vec::new();
    for (at, size) in [
        ((x - out, y - out), (w + 2. * out, LINE)),
        ((x - out, y + h + GAP), (w + 2. * out, LINE)),
        ((x - out, y - GAP), (LINE, h + 2. * GAP)),
        ((x + w + GAP, y - GAP), (LINE, h + 2. * GAP)),
    ] {
        let line: Box<dyn FnOnce(&mut Window, &mut App) -> gpui::AnyView> = Box::new(|_, cx| cx.new(|_| Line).into());
        windows.extend(layer(cx, at, size, line));
    }
    let size = (BAR_WIDTH.min(mon.w as f32 - 16.), bar_height() + 16.);
    let top = bar_top(local, screen) - 8.;
    let left = (x + w / 2. - size.0 / 2.).clamp(8., (mon.w as f32 - size.0 - 8.).max(8.));
    let build = bar(false, cx);
    windows.extend(layer(cx, (left, top), size, build));
    windows
}

/// The bar's window where it's a window of its own: room for the longest
/// labels; the bar sits in its middle.
const BAR_WIDTH: f32 = 640.;

fn bar_height() -> f32 {
    52. * kit::SCALE
}

/// Below the area, or above it when there's no room, or inside.
fn bar_top((_, y, _, h): (i32, i32, i32, i32), screen: (i32, i32)) -> f32 {
    let (y, h, bar_h) = (y as f32, h as f32, bar_height());
    let below = y + h + 12.;
    if below + bar_h < screen.1 as f32 - 8. {
        below
    } else if y - 12. - bar_h > 8. {
        y - 12. - bar_h
    } else {
        y + h - bar_h - 12.
    }
}

/// A window holding another view (the layers take one type).
struct Strip(gpui::AnyView);

impl Render for Strip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}

/// One side of the frame, where it's a window of its own.
struct Line;

impl Render for Line {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(cx.theme().colors().text_accent)
    }
}

/// The bar with the stitched length and the buttons, and the frame around
/// the area when it's drawn on the same layer; only the bar takes clicks
/// there, so scrolling reaches the app.
pub struct ScrollBar {
    area: (i32, i32, i32, i32),
    screen: (i32, i32),
    /// Drawn over the whole monitor, frame included.
    frame: bool,
    shared: Arc<Shared>,
    region: InputRegion,
    _hold: Hold,
}

impl Render for ScrollBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.region.reset();
        let accent = cx.theme().colors().text_accent;
        let length = self.shared.length.load(Ordering::Relaxed);
        let (stop, cancel) = (self.shared.clone(), self.shared.clone());
        let bar = h_flex()
            .gap_2()
            .p_1()
            .pl_3()
            .elevation_3(cx)
            .child(Label::new(t("scroll.scroll_kadr_stitches")).color(Color::Muted))
            .child(div().w(px(90.)).font_family(kit::MONO_FONT).text_color(cx.theme().colors().text_muted).child(format!("{length} px")))
            .children(crate::ui::in_system_order(vec![
                Button::new("cancel", t("common.cancel"))
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Large)
                    .on_click(move |_, _, _| {
                        cancel.cancelled.store(true, Ordering::Relaxed);
                        cancel.stop.store(true, Ordering::Relaxed);
                    })
                    .into_any_element(),
                Button::new("done", t("scroll.done"))
                    .style(ButtonStyle::Accent)
                    .size(ButtonSize::Large)
                    .tip(Tooltip::text(t("scroll.press_hotkey_again")))
                    .on_click(move |_, _, _| stop.stop.store(true, Ordering::Relaxed))
                    .into_any_element(),
            ]))
            .child(self.region.mark());
        if !self.frame {
            return kit::root(cx).flex().items_center().justify_center().child(bar);
        }
        let (x, y, w, h) = (self.area.0 as f32, self.area.1 as f32, self.area.2 as f32, self.area.3 as f32);
        let top = bar_top(self.area, self.screen);
        let out = GAP + LINE;
        kit::root(cx)
            .relative()
            // The frame sits just outside the area so it isn't captured.
            .child(div().absolute().left(px(x - out)).top(px(y - out)).w(px(w + 2. * out)).h(px(h + 2. * out)).border(px(LINE)).border_color(accent).rounded(px(4.)))
            .child(
                // Centred under the area, kept on screen.
                div().absolute().left(px((x - 300.).max(0.))).w(px((w + 600.).min(self.screen.0 as f32 - (x - 300.).max(0.)))).top(px(top)).flex().justify_center().child(bar),
            )
    }
}
