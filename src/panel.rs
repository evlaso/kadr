//! Pre-recording toolbar: audio/camera toggles and the capture mode, like
//! CleanShot's recording bar. Enter picks a region, Esc cancels.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::record::{self, Opts, Recorder};
use crate::util;

pub fn show(app: &gtk::Application, recorder: &Rc<Recorder>) -> gtk::Window {
    let opts = Opts::load();
    let mic = toggle("kadr-mic-symbolic", "Микрофон", opts.mic);
    let device = Rc::new(RefCell::new(opts.mic_device.clone()));
    let mic_menu = mic_picker(&mic, &device);
    let mic_group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    mic_group.add_css_class("kadr-group");
    mic_group.append(&mic);
    mic_group.append(&mic_menu);
    let audio = toggle("kadr-speaker-symbolic", "Звук системы", opts.audio);
    let webcam = toggle("kadr-camera-symbolic", "Камера", opts.webcam);

    let region = labeled_button("kadr-region-symbolic", "Область");
    region.add_css_class("kadr-primary");
    region.set_tooltip_text(Some("Выбрать область (Enter)"));
    let full = labeled_button("kadr-screen-symbolic", "Весь экран");
    let close = gtk::Button::from_icon_name("kadr-close-symbolic");
    close.set_tooltip_text(Some("Отмена (Esc)"));

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bar.add_css_class("kadr-panel");
    for w in [mic_group.upcast_ref::<gtk::Widget>(), audio.upcast_ref(), webcam.upcast_ref()] {
        bar.append(w);
    }
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    for w in [region.upcast_ref::<gtk::Widget>(), full.upcast_ref(), close.upcast_ref()] {
        bar.append(w);
    }

    let window = gtk::Window::builder()
        .application(app)
        .decorated(false)
        .resizable(false)
        .css_classes(["kadr-overlay"])
        .child(&bar)
        .build();
    window.init_layer_shell();
    window.set_namespace(Some("kadr-panel"));
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_anchor(Edge::Top, true);
    window.set_margin(Edge::Top, 40);
    window.set_monitor(util::focused_monitor().as_ref());

    let read = glib::clone!(#[strong] mic, #[strong] audio, #[strong] webcam, #[strong] device, move || Opts {
        mic: mic.is_active(),
        mic_device: device.borrow().clone(),
        audio: audio.is_active(),
        webcam: webcam.is_active(),
    });
    let choose = Rc::new(glib::clone!(#[weak] recorder, move |mode: Option<&str>| {
        recorder.panel_done(mode.map(|m| (m, read())));
    }));

    region.connect_clicked(glib::clone!(#[strong] choose, move |_| choose(Some("region"))));
    full.connect_clicked(glib::clone!(#[strong] choose, move |_| choose(Some("fullscreen"))));
    close.connect_clicked(glib::clone!(#[strong] choose, move |_| choose(None)));

    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(glib::clone!(#[strong] choose, move |_, key, _, _| {
        match key {
            gdk::Key::Escape => choose(None),
            gdk::Key::Return | gdk::Key::KP_Enter => choose(Some("region")),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }));
    window.add_controller(keys);

    window.present();
    region.grab_focus();
    window
}

/// Dropdown next to the mic toggle listing input devices; picking one also
/// turns the microphone on.
fn mic_picker(mic: &gtk::ToggleButton, device: &Rc<RefCell<String>>) -> gtk::MenuButton {
    let mics = record::microphones();
    // A remembered device that is gone falls back to the system default.
    if !mics.iter().any(|(name, _)| *name == *device.borrow()) {
        device.borrow_mut().clear();
    }
    let default_name = std::process::Command::new("pactl")
        .arg("get-default-source")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    let default_desc = mics
        .iter()
        .find(|(name, _)| *name == default_name)
        .map(|(_, d)| d.clone())
        .unwrap_or_else(|| "системный".into());

    let mut entries = vec![(String::new(), format!("По умолчанию — {default_desc}"))];
    entries.extend(mics);

    let describe = |name: &str, entries: &[(String, String)]| {
        entries.iter().find(|(n, _)| n == name).map(|(_, d)| d.clone()).unwrap_or_default()
    };
    mic.set_tooltip_text(Some(&describe(&device.borrow(), &entries)));

    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    list.add_css_class("kadr-menu");
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.add_css_class("kadr-popover");
    popover.set_child(Some(&list));

    let checks: Rc<RefCell<Vec<(String, gtk::Image)>>> = Rc::default();
    for (name, desc) in &entries {
        let check = gtk::Image::from_icon_name("kadr-check-symbolic");
        check.set_opacity(if *device.borrow() == *name { 1.0 } else { 0.0 });
        let label = gtk::Label::new(Some(desc));
        label.set_xalign(0.0);
        let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row_box.append(&check);
        row_box.append(&label);
        let row = gtk::Button::new();
        row.set_child(Some(&row_box));
        checks.borrow_mut().push((name.clone(), check));
        row.connect_clicked(glib::clone!(
            #[strong] device, #[strong] checks, #[weak] mic, #[weak] popover, #[strong] name, #[strong] desc,
            move |_| {
                *device.borrow_mut() = name.clone();
                for (n, img) in checks.borrow().iter() {
                    img.set_opacity(if *n == name { 1.0 } else { 0.0 });
                }
                mic.set_active(true);
                mic.set_tooltip_text(Some(&desc));
                popover.popdown();
            }
        ));
        list.append(&row);
    }

    let button = gtk::MenuButton::new();
    button.set_icon_name("kadr-down-symbolic");
    button.set_tooltip_text(Some("Выбрать микрофон"));
    button.add_css_class("kadr-chevron");
    button.set_popover(Some(&popover));
    button
}

fn content(icon: &str, label: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    b.append(&gtk::Image::from_icon_name(icon));
    b.append(&gtk::Label::new(Some(label)));
    b
}

fn toggle(icon: &str, label: &str, active: bool) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_child(Some(&content(icon, label)));
    b.set_active(active);
    b.add_css_class("kadr-toggle");
    b
}

fn labeled_button(icon: &str, label: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_child(Some(&content(icon, label)));
    b
}
