//! A screenshot pinned above all windows: drag to move, scroll to zoom,
//! Esc / right click / double click to close.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::util;

/// Room around the image for the drop shadow.
const PAD: f64 = 16.0;

struct Pin {
    window: gtk::Window,
    picture: gtk::Picture,
    texture: gdk::Texture,
    base: (f64, f64),
    zoom: Cell<f64>,
    pos: Cell<(f64, f64)>,
}

impl Pin {
    fn apply_pos(&self) {
        let (x, y) = self.pos.get();
        self.window.set_margin(Edge::Left, x.round() as i32);
        self.window.set_margin(Edge::Top, y.round() as i32);
    }

    fn apply_size(&self) {
        let z = self.zoom.get();
        let (w, h) = ((self.base.0 * z).round(), (self.base.1 * z).round());
        self.picture.set_paintable(Some(&util::scaled_paintable(&self.texture, w, h)));
        // Size the layer surface explicitly so it also shrinks when zooming out.
        let pad = 2.0 * PAD;
        self.window.set_default_size((w + pad) as i32, (h + pad) as i32);
    }

    /// Zoom around the image center so it doesn't jump towards a corner.
    fn set_zoom(&self, zoom: f64) {
        let old = self.zoom.get();
        let zoom = zoom.clamp(0.1, 4.0);
        let (x, y) = self.pos.get();
        let dw = self.base.0 * (zoom - old);
        let dh = self.base.1 * (zoom - old);
        self.zoom.set(zoom);
        self.pos.set((x - dw / 2.0, y - dh / 2.0));
        self.apply_size();
        self.apply_pos();
    }
}

pub fn open(app: &gtk::Application, path: &Path) {
    let texture = match gdk::Texture::from_filename(path) {
        Ok(t) => t,
        Err(e) => return util::notify("Не удалось открыть картинку", &e.to_string()),
    };
    let monitor = util::focused_monitor();
    let (mw, mh, scale) = monitor
        .as_ref()
        .map(|m| (m.geometry().width() as f64, m.geometry().height() as f64, m.scale()))
        .unwrap_or((1920.0, 1080.0, 1.0));

    // Screenshots are captured in physical pixels; show them at their real size.
    let (mut w, mut h) = (texture.width() as f64 / scale, texture.height() as f64 / scale);
    let fit = (0.8 * mw / w).min(0.8 * mh / h).min(1.0);
    w *= fit;
    h *= fit;

    let picture = gtk::Picture::new();
    picture.set_can_shrink(false);

    let close = gtk::Button::from_icon_name("kadr-close-symbolic");
    close.add_css_class("kadr-round");
    close.set_halign(gtk::Align::Start);
    close.set_valign(gtk::Align::Start);
    close.set_margin_start(6);
    close.set_margin_top(6);

    let frame = gtk::Overlay::new();
    frame.add_css_class("kadr-pin-frame");
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.set_halign(gtk::Align::Start);
    frame.set_valign(gtk::Align::Start);
    frame.set_child(Some(&picture));
    frame.add_overlay(&close);

    let window = gtk::Window::builder()
        .application(app)
        .decorated(false)
        .resizable(false)
        .css_classes(["kadr-pin"])
        .child(&frame)
        .build();
    window.init_layer_shell();
    window.set_namespace(Some("kadr-pin"));
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_monitor(monitor.as_ref());

    let pin = Rc::new(Pin {
        window: window.clone(),
        picture,
        texture,
        base: (w, h),
        zoom: Cell::new(1.0),
        pos: Cell::new(((mw - w) / 2.0 - PAD, (mh - h) / 2.0 - PAD)),
    });
    pin.apply_size();
    pin.apply_pos();

    close.connect_clicked(glib::clone!(#[weak] window, move |_| window.close()));

    // Moving a layer surface shifts the pointer's surface-relative coordinates,
    // so track the absolute cursor position from Hyprland instead.
    let drag = gtk::GestureDrag::new();
    let start = Rc::new(Cell::new(None::<((f64, f64), (f64, f64))>));
    drag.connect_drag_begin(glib::clone!(#[strong] pin, #[strong] start, move |_, _, _| {
        start.set(util::cursor_pos().map(|c| (c, pin.pos.get())));
    }));
    drag.connect_drag_update(glib::clone!(#[strong] pin, #[strong] start, move |_, _, _| {
        if let (Some(((cx, cy), (px, py))), Some((x, y))) = (start.get(), util::cursor_pos()) {
            pin.pos.set((px + x - cx, py + y - cy));
            pin.apply_pos();
        }
    }));
    drag.connect_drag_end(glib::clone!(#[strong] start, move |_, _, _| start.set(None)));
    frame.add_controller(drag);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll(glib::clone!(#[strong] pin, move |_, _, dy| {
        pin.set_zoom(pin.zoom.get() * 1.1f64.powf(-dy));
        glib::Propagation::Stop
    }));
    frame.add_controller(scroll);

    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.connect_pressed(glib::clone!(#[weak] window, move |g, n, _, _| {
        if g.current_button() == gdk::BUTTON_SECONDARY || n == 2 {
            window.close();
        }
    }));
    frame.add_controller(click);

    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(#[weak] window, #[upgrade_or] glib::Propagation::Proceed, move |_, key, _, _| {
        if key == gdk::Key::Escape {
            window.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    }));
    window.add_controller(keys);

    window.present();
}
