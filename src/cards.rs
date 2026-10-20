//! Quick-access overlay: a stack of thumbnails in the bottom-left corner of
//! the focused monitor, like CleanShot's floating preview. One layer surface
//! holds the whole stack; only the cards themselves take input.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::kit::{Anchor, KeyboardInteractivity};
use gpui::{
    Animation, AnimationExt, AnyElement, App, ClickEvent, Context, Global, Image, ImageFormat,
    MouseButton, MouseDownEvent, MouseUpEvent, ObjectFit, Stateful, hsla, Window, WindowHandle, div, ease_out_quint,
    img, px, rgba,
};
use crate::ui::{Tooltip, prelude::*};

use crate::kit::{self, InputRegion, LayerSpec};
use crate::settings::{Settings, Side};
use crate::storage::Capture;
use crate::{editor, job, ocr, pin, studio_editor, track, trim, util};
use crate::i18n::t;

const CARD_WIDTH: f32 = 180. * kit::SCALE;
/// The height of the text buttons in the middle of a hovered card; their
/// width is the label's and some padding, so it suits every language.
const PILL_H: f32 = 26. * kit::SCALE;
/// How far in the corner buttons sit; as circles of radius
/// `RADIUS - INSET` they stay concentric with the card's corners.
const INSET: f32 = 5.;
/// The card's corners, rounder than a small card can carry `kit::RADIUS`.
const RADIUS: f32 = 17.;
const CORNER: f32 = 2. * (RADIUS - INSET);

/// A hairline round the card, like the desktop's window borders (Omarchy's
/// 1px white at 18%), so a screenshot doesn't blend into what's behind it.
/// On top of the picture, which would cover a border of the card itself.
fn edge() -> gpui::Div {
    div().absolute().inset_0().rounded(px(RADIUS)).border_1().border_color(hsla(0., 0., 1., 0.18))
}
const THUMB_WIDTH: f32 = CARD_WIDTH;
/// Cards are always 4:3; the picture is cropped to fill them, centred.
const THUMB_MAX: f32 = THUMB_WIDTH * 3. / 4.;
const CHROME: f32 = 0.;
const GAP: f32 = 14.;
/// Room above and to the right of the stack so card shadows aren't cut off by
/// the overlay's edge. Below and to the left is the screen edge anyway.
const PAD: f32 = 56.;
/// Distance from the screen corner.
const EDGE: f32 = 24.;

/// When an untouched card goes away; "never" is a long way off.
fn dismiss_at(settings: &Settings) -> Instant {
    let secs = if settings.card_seconds == 0 { 365 * 24 * 3600 } else { settings.card_seconds as u64 };
    Instant::now() + Duration::from_secs(secs)
}

#[derive(Default)]
struct CardsWindow(Option<WindowHandle<CardStack>>);

impl Global for CardsWindow {}

/// A card's picture, and a blurred copy shown under the hover buttons.
/// Blurring ahead of time covers the whole picture; a backdrop blur thins out
/// toward its edges and lets the sharp picture show there.
struct Thumb {
    sharp: Arc<Image>,
    blurred: Arc<Image>,
}

struct Card {
    id: usize,
    path: PathBuf,
    image: Option<Thumb>,
    /// A text card (recognized text) instead of a file: the text, or None
    /// when nothing was found.
    text: Option<Option<String>>,
    /// A link in that text (or a QR code), to open from the card.
    link: Option<String>,
    /// A job still working on this video: its progress and title. The card
    /// shows the video blurred until the job hands over its result.
    job: Option<(Rc<job::Job>, &'static str)>,
    /// Thumbnail height.
    height: f32,
    /// Clip duration in seconds for recordings.
    video: Option<f64>,
    deadline: Instant,
    hovered: bool,
}

/// How long a card takes to slide in from the screen edge.
const SLIDE: Duration = Duration::from_millis(280);

pub struct CardStack {
    cards: Vec<Card>,
    settings: Settings,
    hidden: bool,
    region: InputRegion,
    /// Whether the pointer is over this layer. GPUI keeps the last pointer
    /// position after it leaves, so hover on its own would stick.
    inside: bool,
    /// A card being swiped toward the screen edge: its id, where the press
    /// was, and how far it has moved. Swiped far enough, it goes away;
    /// dragged the other way, it's a file drag as before.
    swipe: Option<(usize, f32, f32)>,
}

/// How far a card has to be swiped to go away.
const SWIPE_AWAY: f32 = CARD_WIDTH * 0.3;

/// Show a card for a screenshot or recording.
pub fn show(path: PathBuf, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        let loaded = if is_video(&path) {
            video_preview(&path, cx).await
        } else {
            let file = path.clone();
            cx.background_executor()
                .spawn(async move { card_thumbnail(&std::fs::read(&file).ok()?) })
                .await
                .map(|(image, height, _)| (image, height, None))
        };
        match loaded {
            Some((image, height, video)) => {
                let _ = cx.update(|cx| add(Card { id: 0, path, image: Some(image), text: None, link: None, job: None, height, video, deadline: Instant::now(), hovered: false }, cx));
            }
            None => util::notify(t("card.couldnt_open_file"), &path.display().to_string()),
        }
    })
    .detach();
}

/// What happens to a fresh capture, as set in the settings: copied, shown in
/// a card, opened in the editor or the studio, pinned; kept in the captures
/// folder or moved to a temporary one.
pub fn after_capture(path: PathBuf, kind: Capture, cx: &mut App) {
    let settings = Settings::load();
    let after = match kind {
        Capture::Screenshot => settings.after_shot,
        Capture::Recording => settings.after_video,
    };
    let path = if after.save { path } else { unsaved(&path).unwrap_or(path) };
    if after.copy {
        match kind {
            Capture::Screenshot => util::copy_image(&path),
            Capture::Recording => util::copy_file(&path),
        }
    }
    if after.card {
        show(path.clone(), cx);
    }
    match kind {
        Capture::Screenshot => {
            if after.pin {
                pin::open(path.clone(), cx);
            }
            if after.edit {
                editor::open_from_card(path, cx);
            }
        }
        Capture::Recording if after.edit => studio_editor::open_from_card(path, cx),
        Capture::Recording => {}
    }
}

/// Move a capture out of the captures folder into a temporary one, under the
/// same name (so a recording's click log still finds it).
fn unsaved(path: &Path) -> Option<PathBuf> {
    let dir = util::runtime_dir().join("kadr-unsaved");
    std::fs::create_dir_all(&dir).ok()?;
    let to = dir.join(path.file_name()?);
    if std::fs::rename(path, &to).is_err() {
        std::fs::copy(path, &to).ok()?;
        let _ = std::fs::remove_file(path);
    }
    Some(to)
}

/// Hide the overlay while a screenshot is taken. Returns whether it was visible.
pub fn set_hidden(hidden: bool, cx: &mut App) -> bool {
    let Some(handle) = cx.try_global::<CardsWindow>().and_then(|w| w.0) else { return false };
    handle
        .update(cx, |stack, _, cx| {
            let was = !stack.hidden && !stack.cards.is_empty();
            stack.hidden = hidden;
            cx.notify();
            was
        })
        .unwrap_or(false)
}

/// A card with recognized text (copied already), or with "nothing found".
pub fn show_text(text: Option<String>, link: Option<String>, cx: &mut App) {
    add(Card { id: 0, path: PathBuf::new(), image: None, text: Some(text), link, job: None, height: THUMB_MAX, video: None, deadline: Instant::now(), hovered: false }, cx);
}

/// A card for a job on `source` (an export, a trim, a GIF): the video's
/// first frame, blurred, with the job's progress. Returns its id for
/// `complete`.
pub fn show_job(job: Rc<job::Job>, title: &'static str, source: PathBuf, cx: &mut App) -> usize {
    let id = NEXT_CARD.fetch_add(1, Ordering::Relaxed);
    add(Card { id, path: source.clone(), image: None, text: None, link: None, job: Some((job, title)), height: THUMB_MAX, video: None, deadline: Instant::now(), hovered: false }, cx);
    cx.spawn(async move |cx| {
        if let Some((image, _, _)) = video_preview(&source, cx).await {
            let _ = cx.update(|cx| with_card(id, cx, |card| card.image = Some(image)));
        }
        // Keep the progress moving while the job runs.
        loop {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let running = cx.update(|cx| {
                let mut running = false;
                with_card(id, cx, |card| running = card.job.is_some());
                running
            });
            if !running {
                break;
            }
        }
    })
    .detach();
    id
}

/// The job behind card `id` is over: the card becomes `result`'s (the
/// blur lifts), or goes away without one.
pub fn complete(id: usize, result: Option<PathBuf>, cx: &mut App) {
    let Some(out) = result else {
        if let Some(handle) = cx.try_global::<CardsWindow>().and_then(|w| w.0) {
            let _ = handle.update(cx, |stack, window, cx| stack.remove(id, window, cx));
        }
        return;
    };
    cx.spawn(async move |cx| {
        let loaded = if is_video(&out) {
            video_preview(&out, cx).await
        } else {
            let file = out.clone();
            cx.background_executor().spawn(async move { card_thumbnail(&std::fs::read(&file).ok()?) }).await.map(|(image, height, _)| (image, height, None))
        };
        let _ = cx.update(|cx| {
            let settings = Settings::load();
            with_card(id, cx, |card| {
                card.path = out.clone();
                card.job = None;
                card.deadline = dismiss_at(&settings);
                if let Some((image, height, video)) = loaded {
                    card.image = Some(image);
                    card.height = height;
                    card.video = video;
                }
            });
        });
    })
    .detach();
}

/// Change a card in place and redraw the stack.
fn with_card(id: usize, cx: &mut App, change: impl FnOnce(&mut Card)) {
    if let Some(handle) = cx.try_global::<CardsWindow>().and_then(|w| w.0) {
        let _ = handle.update(cx, |stack, _, cx| {
            if let Some(card) = stack.cards.iter_mut().find(|c| c.id == id) {
                change(card);
            }
            cx.notify();
        });
    }
}

/// Card ids, unique for the run.
static NEXT_CARD: AtomicUsize = AtomicUsize::new(1);

fn add(card: Card, cx: &mut App) {
    let settings = Settings::load();
    // A job's card stays until the job is done.
    let deadline = if card.job.is_some() { Instant::now() + Duration::from_secs(365 * 24 * 3600) } else { dismiss_at(&settings) };
    let mut card = Card { deadline, ..card };
    if card.id == 0 {
        card.id = NEXT_CARD.fetch_add(1, Ordering::Relaxed);
    }
    let mut slot = Some(card);

    if let Some(handle) = cx.try_global::<CardsWindow>().and_then(|w| w.0) {
        let pushed = handle.update(cx, |stack, _, cx| {
            let card = slot.take().expect("card");
            // The same file's card already showing (back from the editor, or
            // saved over): it takes the new picture in its place.
            if !card.path.as_os_str().is_empty()
                && let Some(shown) = stack.cards.iter_mut().find(|c| c.path == card.path)
            {
                *shown = Card { id: shown.id, ..card };
                return cx.notify();
            }
            stack.cards.push(card);
            if stack.cards.len() > stack.settings.max_cards {
                stack.cards.remove(0);
            }
            cx.notify();
        });
        if pushed.is_ok() {
            return;
        }
    }
    // No stack on screen (or it just closed): open a fresh one.
    let Some(card) = slot else { return };
    let spec = LayerSpec {
        namespace: "kadr",
        anchor: Anchor::BOTTOM | if settings.card_side == Side::Right { Anchor::RIGHT } else { Anchor::LEFT },
        margin: (0., 0., 0., 0.),
        size: (CARD_WIDTH + EDGE + PAD, settings.max_cards as f32 * (THUMB_MAX + CHROME + GAP) + EDGE + PAD),
        keyboard: KeyboardInteractivity::None,
        ignore_zones: false,
        monitor: util::focused_monitor(),
    };
    let handle = kit::open_layer(cx, spec, |window, cx| {
        cx.new(|cx| {
            CardStack::start_ticking(window, cx);
            CardStack { cards: vec![card], settings, hidden: false, region: InputRegion::default(), inside: false, swipe: None }
        })
    });
    cx.set_global(CardsWindow(handle));
}

impl CardStack {
    /// Auto-dismiss: cards expire unless the pointer rests on them.
    fn start_ticking(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let alive = this.update_in(cx, |stack, window, cx| {
                    let now = Instant::now();
                    let before = stack.cards.len();
                    let inside = window.is_window_hovered();
                    stack.cards.retain(|c| (c.hovered && inside) || now < c.deadline);
                    if stack.cards.len() != before {
                        cx.notify();
                    }
                    if stack.cards.is_empty() {
                        window.remove_window();
                        false
                    } else {
                        true
                    }
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn remove(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.cards.retain(|c| c.id != id);
        if self.cards.is_empty() {
            window.remove_window();
        }
        cx.notify();
    }

    fn swipe_offset(&self, id: usize) -> f32 {
        self.swipe.filter(|s| s.0 == id).map_or(0., |s| s.2)
    }

    /// Follow the pointer toward the screen edge, never away from it.
    fn swipe_move(&mut self, x: f32, cx: &mut Context<Self>) {
        let side = self.settings.card_side;
        if let Some((_, start, offset)) = &mut self.swipe {
            let dx = x - *start;
            *offset = if side == Side::Right { dx.max(0.) } else { dx.min(0.) };
            cx.notify();
        }
    }

    fn swipe_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((id, _, offset)) = self.swipe.take() {
            if offset.abs() > SWIPE_AWAY {
                self.remove(id, window, cx);
            }
            cx.notify();
        }
    }

    fn set_hover(&mut self, id: usize, hovered: bool) {
        if let Some(card) = self.cards.iter_mut().find(|c| c.id == id) {
            card.hovered = hovered;
            if !hovered {
                card.deadline = dismiss_at(&self.settings);
            }
        }
    }

    /// CleanShot's Quick Access look in Zed's components: just the picture;
    /// hovering shows the main actions in the middle and the rest in the
    /// corners. Drag it out as a file; double-click to edit.
    fn render_card(&self, card: &Card, inside: bool, cx: &mut Context<Self>) -> AnyElement {
        if let Some(text) = &card.text {
            return self.render_text_card(card, text.as_deref(), inside, cx);
        }
        if let Some((job, title)) = &card.job {
            return self.render_job_card(card, job.clone(), title, cx);
        }
        let shown = card.hovered && inside;
        // Off-screen offset toward the card's edge, to come in from.
        let away = (CARD_WIDTH + EDGE + PAD) * if self.settings.card_side == Side::Right { 1. } else { -1. };
        let id = card.id;
        let path = card.path.clone();
        let video = card.video.is_some();
        let gif = is_gif(&path);
        let has_track = video && track::path_for(&path).exists();

        let act = |action: fn(&mut CardStack, &Path, usize, &mut Window, &mut Context<CardStack>)| {
            let path = path.clone();
            cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                action(this, &path, id, window, cx)
            })
        };
        // Solid white pills with dark ink, like CleanShot's overlay buttons.
        let ink = hsla(0., 0., 0.12, 1.);
        let pill = |el: Stateful<Div>| {
            el.flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(hsla(0., 0., 1., 0.92))
                .text_color(ink)
                .cursor_pointer()
                .hover(|s| s.bg(hsla(0., 0., 1., 1.)))
                .active(|s| s.bg(hsla(0., 0., 0.88, 1.)))
        };
        let big = |key: &'static str, label: &'static str| {
            pill(div().id((key, id)))
                .px_3()
                .h(px(PILL_H))
                .child(Label::new(label).size(LabelSize::Small).color(Color::Custom(ink)))
        };
        let corner = |key: &'static str, icon: IconName, tip: &'static str| {
            pill(div().id((key, id)))
                .size(px(CORNER))
                .child(Icon::new(icon).size(IconSize::XSmall).color(Color::Custom(ink)))
                .tip(Tooltip::text(tip))
        };

        // As wide as the longest label, the shorter ones stretched to it.
        let mut center = v_flex().items_stretch().gap_1().child(big("copy", t("common.copy")).on_click(act(|this, path, id, window, cx| {
            if is_video(path) {
                util::copy_file(path)
            } else {
                util::copy_image(path)
            }
            this.remove(id, window, cx);
        })));
        center = if video && !gif {
            center.child(
                big("studio", t("common.studio"))
                    .tip(Tooltip::text(if has_track { t("card.zooms_on_clicks_background_clicks") } else { t("card.background_padding_zooms_by_hand") }))
                    .on_click(act(|this, path, id, window, cx| {
                        this.remove(id, window, cx);
                        studio_editor::open_from_card(path.to_owned(), cx);
                    })),
            )
        } else {
            center.child(big("edit", if gif { t("common.open") } else { t("card.edit") }).on_click(act(|this, path, id, window, cx| {
                this.remove(id, window, cx);
                edit(path.to_owned(), cx);
            })))
        };

        // Top right: pin for pictures, trim for recordings.
        let top_right = if video {
            (!gif).then(|| {
                corner("trim", IconName::Scissors, t("common.trim")).on_click(act(|this, path, id, window, cx| {
                    this.remove(id, window, cx);
                    trim::open(path.to_owned(), cx);
                }))
            })
        } else {
            Some(corner("pin", IconName::Pin, t("card.pin_on_top")).on_click(act(|this, path, id, window, cx| {
                pin::open(path.to_owned(), cx);
                this.remove(id, window, cx);
            })))
        };
        // Bottom right: text recognition for pictures, GIF for recordings, the folder for both.
        let extra = if video {
            (!gif).then(|| {
                pill(div().id(("gif", id)))
                    .h(px(CORNER))
                    .px_2()
                    .child(Label::new("GIF").size(LabelSize::XSmall).color(Color::Custom(ink)))
                    .tip(Tooltip::text(t("card.save_as_gif_720px_15")))
                    .on_click(act(|_, path, _, _, cx| trim::quick_gif(path.to_owned(), cx)))
                    .into_any_element()
            })
        } else {
            Some(corner("ocr", IconName::CaseSensitive, t("common.recognize_text")).on_click(act(|_, path, _, _, cx| ocr::image(path.to_owned(), cx))).into_any_element())
        };

        let at = |el: AnyElement| div().absolute().m(px(INSET)).child(el);
        let hover = div()
            .absolute()
            .inset_0()
            .rounded(px(RADIUS))
            .opacity(if shown { 1. } else { 0. })
            // The picture blurs and dims behind the buttons.
            .when_some(card.image.as_ref().filter(|_| shown), |d, image| d.child(img(image.blurred.clone()).absolute().inset_0().size_full().object_fit(ObjectFit::Fill).rounded(px(RADIUS))))
            .child(div().absolute().inset_0().rounded(px(RADIUS)).bg(rgba(0x00000052)))
            .flex()
            .items_center()
            .justify_center()
            .child(center)
            .child(at(corner("close", IconName::Close, t("common.close")).on_click(act(|this, _, id, window, cx| this.remove(id, window, cx))).into_any_element()).top_0().left_0())
            .children(top_right.map(|b| at(b.into_any_element()).top_0().right_0()))
            .child(
                at(corner("trash", IconName::Trash, t("card.delete_file"))
                    .on_click(act(|this, path, id, window, cx| {
                        util::trash(path);
                        this.remove(id, window, cx);
                    }))
                    .into_any_element())
                .bottom_0()
                .left_0(),
            )
            .child(
                div().absolute().bottom_0().right_0().m(px(INSET)).child(
                    h_flex()
                        .gap_1()
                        .children(extra)
                        .child(corner("folder", IconName::Folder, t("common.show_in_folder")).on_click(act(|_, path, _, _, _| util::open_folder(path)))),
                ),
            );

        // The length in the corner, like in the history.
        let badge = card.video.map(|secs| {
            div()
                .absolute()
                .top(px(INSET))
                .left(px(INSET))
                .when(shown, |s| s.opacity(0.))
                .child(
                    h_flex()
                        .h(px(CORNER))
                        .gap_1()
                        .px_2p5()
                        .rounded_full()
                        // Frosted, like the hover overlay under the buttons.
                        .bg(rgba(0x00000066))
                        .backdrop_blur(px(8.))
                        .child(Icon::new(IconName::PlayFilled).size(IconSize::XSmall).color(Color::Custom(gpui::white())))
                        .child(Label::new(util::format_duration(secs)).size(LabelSize::Small).color(Color::Custom(gpui::white()))),
                )
        });

        // Double-click opens the editor (or the player), once the button is
        // up: closing on the press hands the rest of the click to the window
        // below, and a terminal there selects text onto the clipboard.
        let press = cx.listener(move |this, e: &MouseDownEvent, _, _| {
            if e.click_count != 2 {
                this.swipe = Some((id, f32::from(e.position.x), 0.));
            }
        });
        let double_click = {
            let path = path.clone();
            cx.listener(move |this, e: &MouseUpEvent, window, cx| {
                if e.click_count == 2 {
                    this.remove(id, window, cx);
                    edit(path.clone(), cx);
                }
            })
        };

        let card_el = div()
            .id(("card", id))
            .relative()
            .flex_none()
            .w(px(CARD_WIDTH))
            .h(px(card.height))
            .rounded(px(RADIUS))
            .shadow(kit::shadow())
            .bg(cx.theme().colors().elevated_surface_background)
            // GPUI clips children to a rectangle, so the image rounds its own
            // corners; it is pre-cropped to the card so those are the visible ones.
            .children(card.image.as_ref().map(|image| img(image.sharp.clone()).size_full().object_fit(ObjectFit::Fill).rounded(px(RADIUS))))
            .children(badge)
            .child(hover)
            .child(edge())
            .child(self.region.mark())
            .on_hover(cx.listener(move |this, hovered: &bool, _, _| this.set_hover(id, *hovered)))
            .on_mouse_down(MouseButton::Left, press)
            .on_mouse_up(MouseButton::Left, double_click)
            // Drag the file out to a chat, browser or file manager.
            .on_drag(path, |_: &PathBuf, _, _, cx| cx.new(|_| DragPreview))
            .external_drag_payload(|path: &PathBuf, _, _| {
                Some(gpui::ExternalDragPayload::Files(gpui::FileDragPaths::new([(path.clone(), false)])))
            });
        // In from the screen edge the cards sit on.
        let swiped = self.swipe_offset(id);
        let card_el = card_el.opacity(1. - (swiped.abs() / CARD_WIDTH).min(1.) * 0.8);
        card_el.with_animation(("appear", id), Animation::new(SLIDE).with_easing(ease_out_quint()), move |el, d| el.left(px(away * (1. - d) + swiped))).into_any_element()
    }
}

impl CardStack {
    /// A job at work on a video: its first frame blurred, the title, how
    /// far along, and a way to stop it.
    fn render_job_card(&self, card: &Card, job: Rc<job::Job>, title: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let away = (CARD_WIDTH + EDGE + PAD) * if self.settings.card_side == Side::Right { 1. } else { -1. };
        let id = card.id;
        let fraction = job.fraction();
        let colors = cx.theme().colors();
        let ink = hsla(0., 0., 0.12, 1.);
        let card_el = div()
            .id(("card", id))
            .relative()
            .flex_none()
            .w(px(CARD_WIDTH))
            .h(px(card.height))
            .rounded(px(RADIUS))
            .shadow(kit::shadow())
            .bg(colors.elevated_surface_background)
            .children(card.image.as_ref().map(|image| img(image.blurred.clone()).size_full().object_fit(ObjectFit::Fill).rounded(px(RADIUS))))
            .child(div().absolute().inset_0().rounded(px(RADIUS)).bg(rgba(0x00000066)))
            .child(
                v_flex()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .px_4()
                    .child(Label::new(title).size(LabelSize::Small).color(Color::Custom(gpui::white())))
                    .child(Label::new(format!("{}%", (fraction * 100.) as u32)).color(Color::Custom(gpui::white())))
                    .child(div().w_full().h(px(4.)).rounded_full().bg(hsla(0., 0., 1., 0.25)).child(div().h_full().w(gpui::relative(fraction)).rounded_full().bg(gpui::white()))),
            )
            .child(
                div().absolute().top_0().left_0().m(px(INSET)).child(
                    div()
                        .id(("cancel-job", id))
                        .size(px(CORNER))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(hsla(0., 0., 1., 0.92))
                        .cursor_pointer()
                        .hover(|s| s.bg(hsla(0., 0., 1., 1.)))
                        .child(Icon::new(IconName::Close).size(IconSize::XSmall).color(Color::Custom(ink)))
                        .tip(Tooltip::text(t("job.cancel")))
                        .on_click(move |_, _, _| job.cancel()),
                ),
            )
            .child(edge())
            .child(self.region.mark());
        card_el.with_animation(("appear", id), Animation::new(SLIDE).with_easing(ease_out_quint()), move |el, d| el.left(px(away * (1. - d)))).into_any_element()
    }

    /// Recognized text, as a card like the others: the text fading out at
    /// the bottom, Copy again on hover.
    fn render_text_card(&self, card: &Card, text: Option<&str>, inside: bool, cx: &mut Context<Self>) -> AnyElement {
        let shown = card.hovered && inside;
        let away = (CARD_WIDTH + EDGE + PAD) * if self.settings.card_side == Side::Right { 1. } else { -1. };
        let id = card.id;
        let colors = cx.theme().colors();
        let surface = colors.elevated_surface_background;
        let (title, body): (&str, String) = match text {
            Some(text) => (t("ocr.text_copied"), text.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n")),
            None if !ocr::has_russian() => (t("ocr.no_text_found"), t("ocr.for_russian_text_install_tesseract").into()),
            None => (t("ocr.no_text_found"), t("ocr.try_selecting_larger_area").into()),
        };
        let chars = text.map(|t| t.chars().count());
        let ink = hsla(0., 0., 0.12, 1.);
        let pill = |el: Stateful<Div>| {
            el.flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(hsla(0., 0., 1., 0.92))
                .cursor_pointer()
                .hover(|s| s.bg(hsla(0., 0., 1., 1.)))
                .active(|s| s.bg(hsla(0., 0., 0.88, 1.)))
        };
        let copy = text.map(|t| t.to_owned());
        let link = card.link.clone();
        let hover = div()
            .absolute()
            .inset_0()
            .rounded(px(RADIUS))
            .opacity(if shown { 1. } else { 0. })
            .bg(rgba(0x00000080))
            .flex()
            .flex_col()
            .gap_1p5()
            .items_center()
            .justify_center()
            .when_some(copy, |d, text| {
                d.child(
                    pill(div().id(("again", id)))
                        .w(px(CARD_WIDTH - 2. * INSET))
                        .h(px(PILL_H))
                        .child(Label::new(t("ocr.copy_again")).size(LabelSize::XSmall).color(Color::Custom(ink)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            crate::platform::copy_text(&text);
                            this.remove(id, window, cx);
                        })),
                )
            })
            .when_some(link.clone(), |d, link| {
                d.child(
                    pill(div().id(("link", id)))
                        .w(px(CARD_WIDTH - 2. * INSET))
                        .h(px(PILL_H))
                        .gap_1()
                        .child(Icon::from_path("icons/kadr/link.svg").size(IconSize::Small).color(Color::Custom(ink)))
                        .child(Label::new(t("ocr.open_link")).size(LabelSize::XSmall).color(Color::Custom(ink)))
                        .tip(Tooltip::text(link.clone()))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            util::open_file(Path::new(&link));
                            this.remove(id, window, cx);
                        })),
                )
            })
            .child(
                div().absolute().top_0().left_0().m(px(INSET)).child(
                    pill(div().id(("close", id)))
                        .size(px(CORNER))
                        .child(Icon::new(IconName::Close).size(IconSize::XSmall).color(Color::Custom(ink)))
                        .tip(Tooltip::text(t("common.close")))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.remove(id, window, cx);
                        })),
                ),
            );
        let mut clear = surface;
        clear.alpha = 0.;
        let card_el = div()
            .id(("card", id))
            .relative()
            .flex_none()
            .w(px(CARD_WIDTH))
            .h(px(card.height))
            .rounded(px(RADIUS))
            .shadow(kit::shadow())
            .bg(surface)
            .child(
                v_flex()
                    .size_full()
                    .gap_1()
                    .p_2()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(if link.is_some() { Icon::from_path("icons/kadr/link.svg") } else { Icon::new(IconName::CaseSensitive) }.size(IconSize::Small).color(if text.is_some() { Color::Accent } else { Color::Muted }))
                            .child(div().flex_1().child(Label::new(title).size(LabelSize::Small)))
                            .when_some(chars, |row, n| row.child(Label::new(tr!("ocr.chars", n)).size(LabelSize::XSmall).color(Color::Muted))),
                    )
                    .child(div().flex_1().min_h_0().overflow_hidden().text_xs().text_color(colors.text_muted).child(body)),
            )
            // Text that doesn't fit fades out instead of being cut.
            .child(div().absolute().left_0().right_0().bottom_0().h(px(36.)).rounded_b(px(RADIUS)).bg(gpui::linear_gradient(180., gpui::linear_color_stop(clear, 0.), gpui::linear_color_stop(surface, 0.9))))
            .child(hover)
            .child(edge())
            .child(self.region.mark())
            .on_hover(cx.listener(move |this, hovered: &bool, _, _| this.set_hover(id, *hovered)))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, _| this.swipe = Some((id, f32::from(e.position.x), 0.))));
        let swiped = self.swipe_offset(id);
        let card_el = card_el.opacity(1. - (swiped.abs() / CARD_WIDTH).min(1.) * 0.8);
        card_el.with_animation(("appear", id), Animation::new(SLIDE).with_easing(ease_out_quint()), move |el, d| el.left(px(away * (1. - d)))).into_any_element()
    }
}

impl Render for CardStack {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.region.reset();
        if self.hidden {
            window.set_input_region(Some(&[]));
            return kit::root(cx);
        }
        let inside = window.is_window_hovered();
        if self.inside && !inside {
            let settings = self.settings.clone();
            self.cards.iter_mut().filter(|c| c.hovered).for_each(|c| c.deadline = dismiss_at(&settings));
        }
        self.inside = inside;
        let cards: Vec<_> = self.cards.iter().map(|c| self.render_card(c, inside, cx)).collect();
        kit::root(cx)
            .flex()
            .flex_col()
            .justify_end()
            .pt(px(PAD))
            .pb(px(EDGE))
            // The screen edge side needs no room for the shadow.
            .when(self.settings.card_side == Side::Right, |d| d.items_end().pr(px(EDGE)).pl(px(PAD)))
            .when(self.settings.card_side == Side::Left, |d| d.pr(px(PAD)).pl(px(EDGE)))
            .gap(px(GAP))
            .children(cards)
            // The pointer stays with this surface while the button is held.
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
                if e.pressed_button == Some(MouseButton::Left) {
                    this.swipe_move(f32::from(e.position.x), cx);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, window, cx| this.swipe_end(window, cx)))
    }
}

/// The platform draws its own icon for outbound file drags.
struct DragPreview;

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Screenshots open in kadr's editor, recordings in the player.
fn edit(path: PathBuf, cx: &mut App) {
    if is_video(&path) {
        return util::open_file(&path);
    }
    editor::open_from_card(path, cx);
}

fn is_gif(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

pub fn is_video(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("mp4" | "mkv" | "webm" | "mov" | "gif")
    )
}

/// Pixel size from a PNG's IHDR chunk.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[1..4] != b"PNG" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    Some((be(16), be(20)))
}

/// One meter in inches over CSS's 96 pixels per inch: pHYs pixels per
/// meter for a picture at 1×.
const PPM_1X: f64 = 96. / 0.0254;

/// Picture pixels per logical pixel, from a PNG's pHYs chunk: 1 for a 1×
/// shot, the screen's scale for one in screen pixels. None without it.
pub fn png_density(bytes: &[u8]) -> Option<f64> {
    let mut i = 8;
    while i + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?) as usize;
        let kind = &bytes[i + 4..i + 8];
        if kind == b"pHYs" && len == 9 && i + 17 <= bytes.len() && bytes[i + 16] == 1 {
            let ppm = u32::from_be_bytes(bytes[i + 8..i + 12].try_into().ok()?) as f64;
            return (ppm > 0.).then(|| (ppm / PPM_1X * 100.).round() / 100.);
        }
        if kind == b"IDAT" || kind == b"IEND" {
            return None;
        }
        i += 12 + len;
    }
    None
}

/// The PNG with a pHYs chunk saying `density` picture pixels per logical
/// pixel, right after IHDR. Viewers that read it (macOS Preview, browsers'
/// image-set) show the picture at its real size; kadr's pin and editor too.
pub fn with_density(png: Vec<u8>, density: f64) -> Vec<u8> {
    if png.len() < 33 || &png[12..16] != b"IHDR" || png_density(&png).is_some() {
        return png;
    }
    let ppm = ((PPM_1X * density).round() as u32).to_be_bytes();
    let mut chunk = Vec::with_capacity(21);
    chunk.extend_from_slice(&9u32.to_be_bytes());
    chunk.extend_from_slice(b"pHYs");
    chunk.extend_from_slice(&ppm);
    chunk.extend_from_slice(&ppm);
    chunk.push(1);
    let crc = crc32fast::hash(&chunk[4..]);
    chunk.extend_from_slice(&crc.to_be_bytes());
    let mut out = png;
    out.splice(33..33, chunk);
    out
}

/// Loaded from bytes rather than by path, so an edited file isn't served from cache.
pub fn image_preview(path: &Path) -> Option<(Arc<Image>, u32, u32)> {
    let bytes = std::fs::read(path).ok()?;
    let (w, h) = png_size(&bytes)?;
    Some((Arc::new(Image::from_bytes(ImageFormat::Png, bytes)), w, h))
}

/// Card-sized thumbnail: center-cropped to the card's aspect and scaled to
/// twice its width, so corners round correctly and a 4K screenshot isn't kept
/// in memory. Returns its pictures, the card height and the source size.
fn card_thumbnail(bytes: &[u8]) -> Option<(Thumb, f32, (u32, u32))> {
    let src = image::load_from_memory(bytes).ok()?;
    let (w, h) = (src.width().max(1), src.height().max(1));
    let height = THUMB_MAX;
    let aspect = THUMB_WIDTH / height;
    let (cw, ch) = if (w as f32 / h as f32) > aspect {
        ((h as f32 * aspect).round() as u32, h)
    } else {
        (w, (w as f32 / aspect).round() as u32)
    };
    let cropped = src.crop_imm((w - cw.min(w)) / 2, (h - ch.min(h)) / 2, cw.min(w), ch.min(h));
    let thumb = cropped.resize_exact((THUMB_WIDTH * 2.) as u32, (height * 2.) as u32, image::imageops::FilterType::Triangle);
    let encode = |img: &image::DynamicImage| {
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(Arc::new(Image::from_bytes(ImageFormat::Png, png)))
    };
    // Blurred small and scaled back up: cheap, and just as soft.
    let (tw, th) = (thumb.width(), thumb.height());
    let small = thumb.resize_exact(tw / 4, th / 4, image::imageops::FilterType::Triangle).blur(3.);
    let blurred = small.resize_exact(tw, th, image::imageops::FilterType::Triangle);
    Some((Thumb { sharp: encode(&thumb)?, blurred: encode(&blurred)? }, height, (w, h)))
}

/// First frame (scaled down) and duration in seconds.
async fn video_preview(path: &Path, cx: &mut gpui::AsyncApp) -> Option<(Thumb, f32, Option<f64>)> {
    use std::ffi::OsStr;
    let thumb = util::runtime_dir().join(format!("kadr-thumb-{}.png", std::process::id()));
    let (ok, _) = util::output::<&OsStr>(&[
        "ffmpeg".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(), "-i".as_ref(), path.as_os_str(),
        "-vframes".as_ref(), "1".as_ref(), "-vf".as_ref(), "scale=640:-2".as_ref(), thumb.as_os_str(),
    ])
    .await?;
    let bytes = ok.then(|| std::fs::read(&thumb).ok()).flatten();
    let _ = std::fs::remove_file(&thumb);
    let (_, duration) = util::output::<&OsStr>(&[
        "ffprobe".as_ref(), "-v".as_ref(), "error".as_ref(), "-show_entries".as_ref(), "format=duration".as_ref(),
        "-of".as_ref(), "csv=p=0".as_ref(), path.as_os_str(),
    ])
    .await?;
    let (image, height, _) = cx.background_executor().spawn(async move { card_thumbnail(&bytes?) }).await?;
    let seconds: f64 = duration.trim().parse().unwrap_or(0.0);
    Some((image, height, Some(seconds)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_are_always_four_by_three() {
        for (w, h) in [(1600u32, 400u32), (400, 1600), (800, 600)] {
            let mut png = Vec::new();
            image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]))
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();
            let (_, height, _) = card_thumbnail(&png).unwrap();
            assert!((height / THUMB_WIDTH - 0.75).abs() < 1e-4, "{w}x{h}: {height}");
        }
    }
}
