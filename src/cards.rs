//! Quick-access overlay: a stack of thumbnails in the bottom-left corner of
//! the focused monitor, like CleanShot's floating preview. One layer surface
//! holds the whole stack; only the cards themselves take input.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt, AnyElement, App, ClickEvent, Context, Global, Image, ImageFormat,
    MouseButton, MouseDownEvent, ObjectFit, SharedString, Window, WindowHandle, div, ease_out_quint,
    img, layer_shell::*, px, rgba,
};
use ui::{IconButtonShape, TintColor, Tooltip, prelude::*};

use crate::kit::{self, InputRegion, LayerSpec};
use crate::{pin, studio, track, util};

const CARD_WIDTH: f32 = 300.;
const MAX_CARDS: usize = 4;
const GAP: f32 = 14.;
/// Room above and to the right of the stack so card shadows aren't cut off by
/// the overlay's edge. Below and to the left is the screen edge anyway.
const PAD: f32 = 56.;
/// Distance from the screen corner.
const EDGE: f32 = 10.;
const AUTO_DISMISS: Duration = Duration::from_secs(10);

#[derive(Default)]
struct CardsWindow(Option<WindowHandle<CardStack>>);

impl Global for CardsWindow {}

struct Card {
    id: usize,
    path: PathBuf,
    image: Arc<Image>,
    height: f32,
    /// Clip duration in seconds for recordings.
    video: Option<f64>,
    deadline: Instant,
    hovered: bool,
}

pub struct CardStack {
    cards: Vec<Card>,
    next_id: usize,
    hidden: bool,
    region: InputRegion,
}

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
                .map(|(image, height)| (image, height, None))
        };
        match loaded {
            Some((image, height, video)) => {
                let _ = cx.update(|cx| add(path, image, height, video, cx));
            }
            None => util::notify("Не удалось открыть файл", &path.display().to_string()),
        }
    })
    .detach();
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

fn add(path: PathBuf, image: Arc<Image>, height: f32, video: Option<f64>, cx: &mut App) {
    let mut slot = Some(Card { id: 0, path, image, height, video, deadline: Instant::now() + AUTO_DISMISS, hovered: false });

    if let Some(handle) = cx.try_global::<CardsWindow>().and_then(|w| w.0) {
        let pushed = handle.update(cx, |stack, _, cx| {
            let mut card = slot.take().expect("card");
            card.id = stack.next_id;
            stack.next_id += 1;
            stack.cards.push(card);
            if stack.cards.len() > MAX_CARDS {
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
        anchor: Anchor::BOTTOM | Anchor::LEFT,
        margin: (0., 0., 0., 0.),
        size: (CARD_WIDTH + EDGE + PAD, MAX_CARDS as f32 * (240. + GAP) + EDGE + PAD),
        keyboard: KeyboardInteractivity::None,
        ignore_zones: false,
        monitor: util::focused_monitor(),
    };
    let handle = kit::open_layer(cx, spec, |window, cx| {
        cx.new(|cx| {
            CardStack::start_ticking(window, cx);
            CardStack { cards: vec![card], next_id: 1, hidden: false, region: InputRegion::default() }
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
                    stack.cards.retain(|c| c.hovered || now < c.deadline);
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

    fn set_hover(&mut self, id: usize, hovered: bool) {
        if let Some(card) = self.cards.iter_mut().find(|c| c.id == id) {
            card.hovered = hovered;
            if !hovered {
                card.deadline = Instant::now() + AUTO_DISMISS;
            }
        }
    }

    fn render_card(&self, card: &Card, cx: &mut Context<Self>) -> AnyElement {
        let id = card.id;
        let group: SharedString = format!("card-{id}").into();
        let path = card.path.clone();
        let video = card.video.is_some();
        let has_track = video && track::path_for(&path).exists();

        let act = |action: fn(&mut CardStack, &Path, usize, &mut Window, &mut Context<CardStack>)| {
            let path = path.clone();
            cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                action(this, &path, id, window, cx)
            })
        };
        let big = |id: (&'static str, usize), label: &'static str, icon: IconName| {
            Button::new(id, label)
                .style(ButtonStyle::Filled)
                .size(ButtonSize::Large)
                .start_icon(Icon::new(icon).size(IconSize::Small))
                .width(px(170.))
        };
        let corner = |id: (&'static str, usize), icon: IconName, tip: &'static str| {
            IconButton::new(id, icon).style(ButtonStyle::Filled).shape(IconButtonShape::Square).tooltip(Tooltip::text(tip))
        };

        let mut center = v_flex().items_center().gap_1p5();
        center = center.child(big(("copy", id), "Копировать", IconName::Copy).on_click(act(|this, path, id, window, cx| {
            if is_video(path) {
                util::copy_file(path)
            } else {
                util::copy_image(path)
            }
            this.remove(id, window, cx);
        })));
        center = center.child(
            big(("edit", id), if video { "Открыть" } else { "Редактировать" }, if video { IconName::PlayFilled } else { IconName::Pencil })
                .on_click(act(|this, path, id, window, cx| {
                    this.remove(id, window, cx);
                    edit(path.to_owned(), cx);
                })),
        );
        if has_track {
            // Recordings made by kadr carry a cursor track and can get the studio treatment.
            center = center.child(
                big(("studio", id), "Улучшить", IconName::Sparkle).style(ButtonStyle::Tinted(TintColor::Accent)).on_click(act(
                    |this, path, id, window, cx| {
                        this.remove(id, window, cx);
                        studio::render(path.to_owned(), cx);
                    },
                )),
            );
        }

        let at = |el: IconButton| div().absolute().m_1p5().child(el);
        let mut hover = div()
            .absolute()
            .inset_0()
            .rounded(px(kit::RADIUS))
            .bg(rgba(0x00000099))
            .opacity(0.)
            .group_hover(group.clone(), |s| s.opacity(1.))
            .flex()
            .items_center()
            .justify_center()
            .child(center)
            .child(at(corner(("close", id), IconName::Close, "Закрыть").on_click(act(|this, _, id, window, cx| this.remove(id, window, cx)))).top_0().left_0())
            .child(
                at(corner(("trash", id), IconName::Trash, "Удалить файл").on_click(act(|this, path, id, window, cx| {
                    util::trash(path);
                    this.remove(id, window, cx);
                })))
                .bottom_0()
                .left_0(),
            )
            .child(at(corner(("folder", id), IconName::Folder, "Открыть папку").on_click(act(|_, path, _, _, _| util::open_folder(path)))).bottom_0().right_0());
        if !video {
            hover = hover.child(
                at(corner(("pin", id), IconName::Pin, "Закрепить поверх окон").on_click(act(|this, path, id, window, cx| {
                    pin::open(path.to_owned(), cx);
                    this.remove(id, window, cx);
                })))
                .top_0()
                .right_0(),
            );
        }

        let badge = card.video.map(|secs| {
            h_flex()
                .absolute()
                .bottom_2()
                .left_0()
                .right_0()
                .justify_center()
                .group_hover(group.clone(), |s| s.opacity(0.))
                .child(
                    h_flex()
                        .gap_1()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .bg(rgba(0x000000b3))
                        .child(Icon::new(IconName::PlayFilled).size(IconSize::XSmall).color(Color::Custom(gpui::white())))
                        .child(Label::new(util::format_duration(secs)).size(LabelSize::Small).color(Color::Custom(gpui::white()))),
                )
        });

        // Double-click opens the editor (or the player).
        let double_click = {
            let path = path.clone();
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                if e.click_count == 2 {
                    this.remove(id, window, cx);
                    edit(path.clone(), cx);
                }
            })
        };

        let colors = cx.theme().colors();
        div()
            .id(("card", id))
            .group(group)
            .relative()
            .flex_none()
            .w(px(CARD_WIDTH))
            .h(px(card.height))
            .rounded(px(kit::RADIUS))
            .overflow_hidden()
            .bg(colors.elevated_surface_background)
            .shadow(kit::shadow())
            // GPUI clips children to a rectangle, so the image rounds its own
            // corners; it is pre-cropped to the card so those are the visible ones.
            .child(img(card.image.clone()).size_full().object_fit(ObjectFit::Fill).rounded(px(kit::RADIUS)))
            .children(badge)
            .child(hover)
            .child(self.region.mark())
            .on_hover(cx.listener(move |this, hovered: &bool, _, _| this.set_hover(id, *hovered)))
            .on_mouse_down(MouseButton::Left, double_click)
            // Drag the file out to a chat, browser or file manager.
            .on_drag(path, |_: &PathBuf, _, _, cx| cx.new(|_| DragPreview))
            .external_drag_payload(|path: &PathBuf, _, _| {
                Some(gpui::ExternalDragPayload::Files(gpui::FileDragPaths::new([(path.clone(), false)])))
            })
            .with_animation(("appear", id), Animation::new(Duration::from_millis(240)).with_easing(ease_out_quint()), |el, d| {
                el.opacity(d).mb(px((1. - d) * -24.))
            })
            .into_any_element()
    }
}

impl Render for CardStack {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.region.reset();
        if self.hidden {
            window.set_input_region(Some(&[]));
            return kit::root(cx);
        }
        let cards: Vec<_> = self.cards.iter().map(|c| self.render_card(c, cx)).collect();
        kit::root(cx)
            .flex()
            .flex_col()
            .justify_end()
            .pt(px(PAD))
            .pr(px(PAD))
            .pb(px(EDGE))
            .pl(px(EDGE))
            .gap(px(GAP))
            .children(cards)
    }
}

/// The platform draws its own icon for outbound file drags.
struct DragPreview;

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Screenshots open in Tensaku (Omarchy's annotator), recordings in the player.
fn edit(path: PathBuf, cx: &mut App) {
    if is_video(&path) {
        return util::open_file(&path);
    }
    let before = util::mtime(&path);
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        let _ = async_process::Command::new("tensaku-edit").arg(&path).status().await;
        // The editor saves in place; bring the result back if it changed.
        if util::mtime(&path) != before {
            util::copy_image(&path);
            let _ = cx.update(|cx| show(path, cx));
        }
    })
    .detach();
}

pub fn is_video(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("mp4" | "mkv" | "webm" | "mov")
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

/// Loaded from bytes rather than by path, so an edited file isn't served from cache.
pub fn image_preview(path: &Path) -> Option<(Arc<Image>, u32, u32)> {
    let bytes = std::fs::read(path).ok()?;
    let (w, h) = png_size(&bytes)?;
    Some((Arc::new(Image::from_bytes(ImageFormat::Png, bytes)), w, h))
}

/// Card-sized thumbnail: center-cropped to the card's aspect and scaled to
/// twice its width, so corners round correctly and a 4K screenshot isn't kept
/// in memory. Returns the image and the card height.
fn card_thumbnail(bytes: &[u8]) -> Option<(Arc<Image>, f32)> {
    let src = image::load_from_memory(bytes).ok()?;
    let (w, h) = (src.width().max(1), src.height().max(1));
    let height = (CARD_WIDTH * h as f32 / w as f32).clamp(90., 240.);
    let aspect = CARD_WIDTH / height;
    let (cw, ch) = if (w as f32 / h as f32) > aspect {
        ((h as f32 * aspect).round() as u32, h)
    } else {
        (w, (w as f32 / aspect).round() as u32)
    };
    let cropped = src.crop_imm((w - cw.min(w)) / 2, (h - ch.min(h)) / 2, cw.min(w), ch.min(h));
    let thumb = cropped.resize_exact((CARD_WIDTH * 2.) as u32, (height * 2.) as u32, image::imageops::FilterType::Triangle);
    let mut png = Vec::new();
    thumb.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some((Arc::new(Image::from_bytes(ImageFormat::Png, png)), height))
}

/// First frame (scaled down) and duration in seconds.
async fn video_preview(path: &Path, cx: &mut gpui::AsyncApp) -> Option<(Arc<Image>, f32, Option<f64>)> {
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
    let (image, height) = cx.background_executor().spawn(async move { card_thumbnail(&bytes?) }).await?;
    Some((image, height, Some(duration.trim().parse().unwrap_or(0.0))))
}
