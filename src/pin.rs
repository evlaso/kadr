//! A screenshot pinned above all windows: drag to move, scroll to zoom,
//! Esc / right click / double click to close.
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
use crate::ui::{IconButtonShape, Tooltip, prelude::*};

use crate::cards::image_preview;
use crate::kit::{self, InputRegion, LayerSpec};
use crate::util;

pub struct Pin {
    image: Arc<Image>,
    /// Size at zoom 1, in logical pixels.
    base: (f32, f32),
    zoom: f32,
    pos: Point<Pixels>,
    /// Pointer offset from the picture's corner while dragging.
    grab: Option<Point<Pixels>>,
    focus: FocusHandle,
    region: InputRegion,
}

pub fn open(path: PathBuf, cx: &mut App) {
    let Some((image, w, h)) = image_preview(&path) else {
        return util::notify("Не удалось открыть картинку", &path.display().to_string());
    };
    let Some(mon) = util::focused_monitor() else { return };
    let (mw, mh) = (mon.w as f32, mon.h as f32);

    // Screenshots are captured in physical pixels; show them at their real size.
    let scale = mon.scale as f32;
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

impl Render for Pin {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.region.reset();
        let (w, h) = self.size();
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
            .child(img(self.image.clone()).size_full().rounded(px(kit::RADIUS)))
            .child(
                div().absolute().top_1p5().left_1p5().opacity(0.).group_hover("pin", |s| s.opacity(1.)).child(
                    IconButton::new("close", IconName::Close)
                        .style(ButtonStyle::Filled)
                        .shape(IconButtonShape::Square)
                        .tooltip(Tooltip::text("Закрыть (Esc)"))
                        .on_click(|_, window, _| window.remove_window()),
                ),
            )
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
                this.set_zoom(this.zoom * 1.1f32.powf(dy / 20.));
                cx.notify();
            }));

        kit::root(cx)
            .track_focus(&self.focus)
            .on_key_down(|e: &KeyDownEvent, window, _| {
                if e.keystroke.key == "escape" {
                    window.remove_window();
                }
            })
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
