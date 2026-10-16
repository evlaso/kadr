//! Quick-access overlay: a stack of screenshot thumbnails in the bottom-left
//! corner of the focused monitor, like CleanShot's floating preview.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::{pin, studio, track, util};

const CARD_WIDTH: i32 = 300;
const MAX_CARDS: usize = 5;
const AUTO_DISMISS: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct Cards {
    ui: RefCell<Option<Ui>>,
    hidden: Cell<bool>,
}

struct Ui {
    window: gtk::Window,
    list: gtk::Box,
    cards: Vec<Rc<Card>>,
}

struct Card {
    revealer: gtk::Revealer,
    timer: RefCell<Option<glib::SourceId>>,
}

impl Card {
    fn stop_timer(&self) {
        if let Some(id) = self.timer.take() {
            id.remove();
        }
    }
}

impl Cards {
    /// Hide or restore the overlay; returns whether anything was on screen.
    pub fn set_hidden(&self, hidden: bool) -> bool {
        self.hidden.set(hidden);
        match &*self.ui.borrow() {
            Some(ui) => {
                let was_visible = ui.window.is_visible();
                ui.window.set_visible(!hidden);
                was_visible
            }
            None => false,
        }
    }

    pub fn show(self: &Rc<Self>, app: &gtk::Application, path: PathBuf) {
        if is_video(&path) {
            let hold = app.hold();
            glib::spawn_future_local(glib::clone!(#[weak(rename_to = cards)] self, #[weak] app, async move {
                let _hold = hold;
                match video_preview(&path).await {
                    Some((texture, duration)) => cards.show_texture(&app, path, &texture, Some(duration)),
                    None => util::notify("Не удалось открыть видео", &path.display().to_string()),
                }
            }));
            return;
        }
        match gdk::Texture::from_filename(&path) {
            Ok(texture) => self.show_texture(app, path, &texture, None),
            Err(e) => util::notify("Не удалось открыть картинку", &e.to_string()),
        }
    }

    /// `video` carries the clip duration in seconds for recordings.
    fn show_texture(self: &Rc<Self>, app: &gtk::Application, path: PathBuf, texture: &gdk::Texture, video: Option<f64>) {
        self.ensure_ui(app);

        let oldest = {
            let ui = self.ui.borrow();
            (ui.as_ref().unwrap().cards.len() >= MAX_CARDS).then(|| ui.as_ref().unwrap().cards[0].clone())
        };
        if let Some(card) = oldest {
            self.remove(&card);
        }

        let card = self.build_card(app, path, texture, video);
        {
            let mut ui = self.ui.borrow_mut();
            let ui = ui.as_mut().unwrap();
            ui.list.append(&card.revealer);
            ui.cards.push(card.clone());
            if let Some(monitor) = util::focused_monitor() {
                ui.window.set_monitor(Some(&monitor));
            }
            ui.window.set_visible(!self.hidden.get());
        }
        // Reveal after the first frame so the slide-in animation plays.
        let revealer = card.revealer.clone();
        glib::idle_add_local_once(move || revealer.set_reveal_child(true));
        self.arm_timer(&card);
    }

    fn ensure_ui(&self, app: &gtk::Application) {
        if self.ui.borrow().is_some() {
            return;
        }
        let window = gtk::Window::builder()
            .application(app)
            .decorated(false)
            .resizable(false)
            .css_classes(["kadr-overlay"])
            .build();
        window.init_layer_shell();
        window.set_namespace(Some("kadr"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Left, true);
        window.set_margin(Edge::Bottom, 14);
        window.set_margin(Edge::Left, 14);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.set_valign(gtk::Align::End);
        window.set_child(Some(&list));
        *self.ui.borrow_mut() = Some(Ui { window, list, cards: Vec::new() });
    }

    fn build_card(self: &Rc<Self>, app: &gtk::Application, path: PathBuf, texture: &gdk::Texture, video: Option<f64>) -> Rc<Card> {
        let height = (CARD_WIDTH * texture.height() / texture.width().max(1)).clamp(90, 240);

        let picture = gtk::Picture::for_paintable(&util::scaled_paintable(texture, CARD_WIDTH as f64, height as f64));

        let copy = pill_button("Копировать");
        let edit = pill_button(if video.is_some() { "Открыть" } else { "Редактировать" });
        // Recordings made by kadr carry a cursor track and can get the studio treatment.
        let studio = pill_button("✨ Улучшить");
        studio.set_visible(video.is_some() && track::path_for(&path).exists());
        let center = gtk::Box::new(gtk::Orientation::Vertical, 8);
        center.set_halign(gtk::Align::Center);
        center.set_valign(gtk::Align::Center);
        center.append(&copy);
        center.append(&edit);
        center.append(&studio);

        let close = round_button("kadr-close-symbolic", "Закрыть", gtk::Align::Start, gtk::Align::Start);
        let pin_btn = round_button("kadr-pin-symbolic", "Закрепить поверх окон", gtk::Align::End, gtk::Align::Start);
        let trash = round_button("kadr-trash-symbolic", "Удалить файл", gtk::Align::Start, gtk::Align::End);
        let folder = round_button("kadr-folder-symbolic", "Открыть папку", gtk::Align::End, gtk::Align::End);

        let hover = gtk::Overlay::new();
        hover.add_css_class("kadr-hover");
        hover.set_child(Some(&center));
        for b in [&close, &pin_btn, &trash, &folder] {
            hover.add_overlay(b);
        }

        let frame = gtk::Overlay::new();
        frame.add_css_class("kadr-card");
        frame.set_overflow(gtk::Overflow::Hidden);
        frame.set_child(Some(&picture));
        if let Some(seconds) = video {
            pin_btn.set_visible(false);
            let badge = gtk::Label::new(Some(&format!("▶ {}", util::format_duration(seconds))));
            badge.add_css_class("kadr-badge");
            badge.set_halign(gtk::Align::Center);
            badge.set_valign(gtk::Align::End);
            frame.add_overlay(&badge);
        }
        frame.add_overlay(&hover);

        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .transition_duration(220)
            .child(&frame)
            .build();
        let card = Rc::new(Card { revealer, timer: RefCell::new(None) });

        // Pause auto-dismiss while the pointer is over the card.
        let motion = gtk::EventControllerMotion::new();
        motion.connect_enter(glib::clone!(#[strong] card, move |_, _, _| card.stop_timer()));
        motion.connect_leave(glib::clone!(#[weak(rename_to = cards)] self, #[strong] card, move |_| cards.arm_timer(&card)));
        frame.add_controller(motion);

        self.attach_drag(&frame, &picture, &path, &card);

        let on = |button: &gtk::Button, action: fn(&Rc<Cards>, &gtk::Application, &Path, &Rc<Card>)| {
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = cards)] self,
                #[weak] app,
                #[strong] path,
                #[strong] card,
                move |_| action(&cards, &app, &path, &card)
            ));
        };
        on(&copy, |cards, _, path, card| {
            if is_video(path) {
                util::copy_file(path);
            } else {
                util::copy_image(path);
            }
            cards.remove(card);
        });
        on(&edit, |cards, app, path, card| {
            cards.remove(card);
            cards.edit(app, path);
        });
        on(&close, |cards, _, _, card| cards.remove(card));
        on(&studio, |cards, app, path, card| {
            cards.remove(card);
            studio::render(app, cards, path.to_owned());
        });
        on(&pin_btn, |cards, app, path, card| {
            pin::open(app, path);
            cards.remove(card);
        });
        on(&trash, |cards, _, path, card| {
            if let Err(e) = gio::File::for_path(path).trash(None::<&gio::Cancellable>) {
                util::notify("Не удалось удалить файл", &e.to_string());
            }
            cards.remove(card);
        });
        on(&folder, |_, _, path, _| util::open_folder(path));

        // Double-click on the thumbnail opens the editor (or player).
        let click = gtk::GestureClick::new();
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = cards)] self,
            #[weak] app,
            #[strong] path,
            #[strong] card,
            move |_, n, _, _| {
                if n == 2 {
                    cards.remove(&card);
                    cards.edit(&app, &path);
                }
            }
        ));
        picture.add_controller(click);

        card
    }

    fn attach_drag(self: &Rc<Self>, frame: &gtk::Overlay, picture: &gtk::Picture, path: &Path, card: &Rc<Card>) {
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::COPY);
        let file = gio::File::for_path(path);
        drag.connect_prepare(move |_, _, _| {
            let files = gdk::FileList::from_array(&[file.clone()]);
            Some(gdk::ContentProvider::for_value(&files.to_value()))
        });
        let icon = gtk::WidgetPaintable::new(Some(picture));
        drag.connect_drag_begin(move |src, _| src.set_icon(Some(&icon), CARD_WIDTH / 2, 20));
        let cancelled = Rc::new(Cell::new(false));
        drag.connect_drag_cancel(glib::clone!(#[strong] cancelled, move |_, _, _| {
            cancelled.set(true);
            false
        }));
        // A file dropped somewhere has done its job, like in CleanShot.
        drag.connect_drag_end(glib::clone!(#[weak(rename_to = cards)] self, #[strong] card, move |_, _, _| {
            if !cancelled.replace(false) {
                cards.remove(&card);
            }
        }));
        frame.add_controller(drag);
    }

    fn arm_timer(self: &Rc<Self>, card: &Rc<Card>) {
        card.stop_timer();
        let id = glib::timeout_add_local_once(AUTO_DISMISS, glib::clone!(
            #[weak(rename_to = cards)] self,
            #[strong] card,
            move || {
                // The source is finished; drop the id without removing it.
                card.timer.take();
                cards.remove(&card);
            }
        ));
        card.timer.replace(Some(id));
    }

    fn remove(self: &Rc<Self>, card: &Rc<Card>) {
        card.stop_timer();
        {
            let mut ui = self.ui.borrow_mut();
            let Some(ui) = ui.as_mut() else { return };
            let Some(i) = ui.cards.iter().position(|c| Rc::ptr_eq(c, card)) else { return };
            ui.cards.remove(i);
        }
        card.revealer.connect_child_revealed_notify(glib::clone!(#[weak(rename_to = cards)] self, move |revealer| {
            if !revealer.is_child_revealed() {
                cards.detach(revealer);
            }
        }));
        card.revealer.set_reveal_child(false);
        // Hidden overlay: no animation will run, so detach right away.
        if !card.revealer.is_mapped() {
            self.detach(&card.revealer);
        }
    }

    fn detach(&self, revealer: &gtk::Revealer) {
        let mut slot = self.ui.borrow_mut();
        let Some(ui) = slot.as_ref() else { return };
        if revealer.parent().as_ref() == Some(ui.list.upcast_ref()) {
            ui.list.remove(revealer);
        }
        if ui.list.first_child().is_none() {
            // Closing the last window lets the app exit when nothing else holds it.
            let ui = slot.take().unwrap();
            drop(slot);
            ui.window.close();
        }
    }

    fn edit(self: &Rc<Self>, app: &gtk::Application, path: &Path) {
        if is_video(path) {
            return util::open_file(path);
        }
        let Some(proc) = util::spawn(&[Path::new("tensaku-edit"), path]) else { return };
        let before = util::mtime(path);
        let hold = app.hold();
        let path = path.to_owned();
        glib::spawn_future_local(glib::clone!(#[weak(rename_to = cards)] self, #[weak] app, async move {
            let _hold = hold;
            let _ = proc.wait_future().await;
            // The editor saves in place; bring the result back if it changed.
            if util::mtime(&path) != before {
                util::copy_image(&path);
                cards.show(&app, path);
            }
        }));
    }
}

fn is_video(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("mp4" | "mkv" | "webm" | "mov")
    )
}

/// First frame (scaled down) and duration in seconds.
async fn video_preview(path: &Path) -> Option<(gdk::Texture, f64)> {
    let thumb = glib::user_runtime_dir().join(format!("kadr-thumb-{}.png", std::process::id()));
    let (ok, _) = util::output(&[
        "ffmpeg".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(), "-i".as_ref(), path.as_os_str(),
        "-vframes".as_ref(), "1".as_ref(), "-vf".as_ref(), "scale=640:-2".as_ref(), thumb.as_os_str(),
    ])
    .await?;
    let texture = ok.then(|| gdk::Texture::from_filename(&thumb).ok()).flatten();
    let _ = std::fs::remove_file(&thumb);
    let (_, duration) = util::output(&[
        "ffprobe".as_ref(), "-v".as_ref(), "error".as_ref(), "-show_entries".as_ref(), "format=duration".as_ref(),
        "-of".as_ref(), "csv=p=0".as_ref(), path.as_os_str(),
    ])
    .await?;
    Some((texture?, duration.trim().parse().unwrap_or(0.0)))
}

fn pill_button(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("kadr-pill");
    b
}

fn round_button(icon: &str, tooltip: &str, halign: gtk::Align, valign: gtk::Align) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("kadr-round");
    b.set_tooltip_text(Some(tooltip));
    b.set_halign(halign);
    b.set_valign(valign);
    b
}
