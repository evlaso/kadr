//! A screenshot pinned above all windows: drag or arrow keys to move,
//! scroll to zoom, Ctrl or Alt + scroll for opacity, L to let clicks
//! through to the windows below; Esc / right click / double click to close.
//!
//! GPUI can't move a layer surface after creation, so each pin gets a
//! transparent full-monitor layer and the picture moves inside it; only the
//! picture itself takes input.

use std::path::PathBuf;
use std::sync::Arc;

use crate::kit::{Anchor, KeyboardInteractivity};
use gpui::{
    App, Context, FocusHandle, Image, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    Pixels, Point, ScrollWheelEvent, Window, div, img, point, px,
};
use crate::ui::{ButtonLike, IconButtonShape, Tooltip, prelude::*};

use crate::cards::image_preview;
use crate::kit::{self, InputRegion, LayerSpec};
use crate::util;
use crate::i18n::t;

pub struct Pin {
    image: Arc<Image>,
    /// Size at zoom 1, in logical pixels.
    base: (f32, f32),
    zoom: f32,
    pos: Point<Pixels>,
    /// Pointer offset from the picture's corner while dragging.
    grab: Option<Point<Pixels>>,
    opacity: f32,
    /// Clicks go through the picture to the windows below; only the lock
    /// in its corner still takes them.
    locked: bool,
    focus: FocusHandle,
    region: InputRegion,
}

pub fn open(path: PathBuf, cx: &mut App) {
    let Some((image, w, h)) = image_preview(&path) else {
        return util::notify(t("common.couldnt_open_image"), &path.display().to_string());
    };
    let Some(mon) = util::focused_monitor() else { return };
    let (mw, mh) = (mon.w as f32, mon.h as f32);

    // Show screenshots at their real size: a 1× shot as it is, one in screen
    // pixels (or without a density, like older ones) shrunk by the scale.
    let scale = std::fs::read(&path).ok().and_then(|b| crate::cards::png_density(&b)).unwrap_or(mon.scale) as f32;
    let (mut bw, mut bh) = (w as f32 / scale, h as f32 / scale);
    let fit = (0.8 * mw / bw).min(0.8 * mh / bh).min(1.0);
    bw *= fit;
    bh *= fit;

    let spec = LayerSpec {
        namespace: "kadr-pin",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mw, mh),
        keyboard: KeyboardInteractivity::OnDemand,
        ignore_zones: true,
        monitor: Some(mon),
    };
    kit::open_layer(cx, spec, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            Pin {
                image,
                base: (bw, bh),
                zoom: 1.0,
                pos: point(px((mw - bw) / 2.), px((mh - bh) / 2.)),
                grab: None,
                opacity: 1.0,
                locked: false,
                focus,
                region: InputRegion::default(),
            }
        })
    });
}

impl Pin {
    fn size(&self) -> (f32, f32) {
        (self.base.0 * self.zoom, self.base.1 * self.zoom)
    }

    /// Zoom around the picture's center so it doesn't jump towards a corner.
    fn set_zoom(&mut self, zoom: f32) {
        let (w0, h0) = self.size();
        self.zoom = zoom.clamp(0.1, 4.0);
        let (w1, h1) = self.size();
        self.pos = point(self.pos.x - px((w1 - w0) / 2.), self.pos.y - px((h1 - h0) / 2.));
    }
}

impl Pin {
    fn toggle_lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.locked = !self.locked;
        self.grab = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }
}

impl Render for Pin {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.region.reset();
        let (w, h) = self.size();
        let locked = self.locked;
        let lock = |id: &'static str| {
            ButtonLike::new(id)
                .square()
                .style(ButtonStyle::Filled)
                .child(Icon::from_path(if locked { "icons/kadr/lock.svg" } else { "icons/kadr/unlock.svg" }).size(IconSize::Small))
                .tooltip(Tooltip::text(if locked { t("pin.catch_clicks_again") } else { t("pin.let_clicks_through") }))
                .on_click(cx.listener(|this, _, window, cx| this.toggle_lock(window, cx)))
        };
        // While clicks go through, the picture can't be hovered: the lock
        // stays on show, and is the one thing that takes clicks.
        if locked {
            let handle = div().absolute().left(self.pos.x + px(6.)).top(self.pos.y + px(6.)).child(div().relative().child(lock("unlock")).child(self.region.mark()));
            let picture = div()
                .absolute()
                .left(self.pos.x)
                .top(self.pos.y)
                .w(px(w))
                .h(px(h))
                .rounded(px(kit::RADIUS))
                .overflow_hidden()
                .shadow(kit::shadow())
                .opacity(self.opacity)
                .child(img(self.image.clone()).size_full().rounded(px(kit::RADIUS)));
            return kit::root(cx).track_focus(&self.focus).on_key_down(cx.listener(Self::key_down)).child(picture).child(handle);
        }
        let percent = (self.opacity < 1.).then(|| {
            div()
                .absolute()
                .bottom_1p5()
                .right_1p5()
                .px_2()
                .py_0p5()
                .rounded_full()
                .bg(gpui::rgba(0x00000099))
                .opacity(0.)
                .group_hover("pin", |s| s.opacity(1.))
                .child(Label::new(format!("{}%", (self.opacity * 100.).round())).size(LabelSize::Small).color(Color::Custom(gpui::white())))
        });
        let picture = div()
            .id("pin")
            .group("pin")
            .absolute()
            .left(self.pos.x)
            .top(self.pos.y)
            .w(px(w))
            .h(px(h))
            .rounded(px(kit::RADIUS))
            .overflow_hidden()
            .shadow(kit::shadow())
            .cursor_grab()
            .child(img(self.image.clone()).size_full().rounded(px(kit::RADIUS)).opacity(self.opacity))
            .child(
                h_flex().gap_1().absolute().top_1p5().left_1p5().opacity(0.).group_hover("pin", |s| s.opacity(1.)).child(
                    IconButton::new("close", IconName::Close)
                        .style(ButtonStyle::Filled)
                        .shape(IconButtonShape::Square)
                        .tooltip(Tooltip::text(t("pin.close_esc")))
                        .on_click(|_, window, _| window.remove_window()),
                )
                .child(lock("lock")),
            )
            .children(percent)
            .child(self.region.mark())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, _| {
                    if e.click_count == 2 {
                        window.remove_window();
                    } else {
                        this.grab = Some(e.position - this.pos);
                    }
                }),
            )
            .on_mouse_down(MouseButton::Right, |_, window, _| window.remove_window())
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| {
                let dy = f32::from(e.delta.pixel_delta(px(20.)).y);
                if e.modifiers.control || e.modifiers.alt {
                    // Never quite invisible: it has to be found again.
                    this.opacity = (this.opacity + dy / 20. * 0.05).clamp(0.15, 1.);
                } else {
                    this.set_zoom(this.zoom * 1.1f32.powf(dy / 20.));
                }
                cx.notify();
            }));

        kit::root(cx)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            // The pointer stays with this surface while the button is held,
            // even outside the input region.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if let (Some(grab), Some(MouseButton::Left)) = (this.grab, e.pressed_button) {
                    this.pos = e.position - grab;
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.grab = None))
            .child(picture)
    }
}

impl Pin {
    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // A screen pixel, or ten with Shift.
        let step = px(if e.keystroke.modifiers.shift { 10. } else { 1. });
        match e.keystroke.key.as_str() {
            "escape" => return window.remove_window(),
            "l" => return self.toggle_lock(window, cx),
            "left" => self.pos.x -= step,
            "right" => self.pos.x += step,
            "up" => self.pos.y -= step,
            "down" => self.pos.y += step,
            _ => return,
        }
        cx.notify();
    }
}
