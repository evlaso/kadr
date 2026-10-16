//! Capture history, CleanShot-style, laid out like Zed's palettes: every
//! screenshot, recording and GIF kadr (or Omarchy) made, newest first, with
//! type-to-search, filters, keyboard navigation and the card actions.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use gpui::{
    AnyElement, App, BackgroundExecutor, ClickEvent, Context, FocusHandle, Image, ImageFormat, KeyDownEvent, MouseButton,
    MouseDownEvent, ObjectFit, ScrollHandle, SharedString, Window, div, img, px,
    rgba,
};
use crate::ui::{ButtonLike, IconButtonShape, Tooltip, prelude::*};

use crate::kit;
use crate::{editor, pin, trim, util};
use crate::i18n::t;

/// Tiles are at least this wide; a row stretches them to fill the window.
const MIN_ITEM: f32 = 210.;
/// Thumbnail shape, width over height.
const THUMB_ASPECT: f32 = 1.6;
/// Stored thumbnail size, sharp up to twice the smallest tile.
const THUMB: (u32, u32) = (420, 262);
const GAP: f32 = 12.;
/// Older captures are left out to keep the window snappy.
const LIMIT: usize = 300;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Shots,
    Videos,
    Gifs,
}

impl Filter {
    const ALL: [(Filter, &'static str); 4] = [(Filter::All, "history.all"), (Filter::Shots, "common.screenshots"), (Filter::Videos, "common.recordings"), (Filter::Gifs, "GIF")];

    fn accepts(self, kind: Kind) -> bool {
        match self {
            Filter::All => true,
            Filter::Shots => kind == Kind::Shot,
            Filter::Videos => kind == Kind::Video,
            Filter::Gifs => kind == Kind::Gif,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Shot,
    Video,
    Gif,
}

struct Item {
    path: PathBuf,
    name: SharedString,
    kind: Kind,
    modified: SystemTime,
    thumb: Option<Thumb>,
}

/// A tile's picture, a blurred copy for under the hover buttons, and a
/// recording's length.
#[derive(Clone)]
struct Thumb {
    sharp: Arc<Image>,
    blurred: Arc<Image>,
    seconds: Option<f64>,
}

pub struct History {
    items: Vec<Item>,
    query: String,
    filter: Filter,
    /// Index into `visible()`.
    selected: usize,
    /// Items per row and their width, from the last layout.
    columns: usize,
    item_width: f32,
    scroll: ScrollHandle,
    focus: FocusHandle,
}

/// Files kadr and Omarchy save, newest first.
fn scan() -> Vec<Item> {
    let settings = crate::settings::Settings::load();
    let mut items: Vec<Item> = [(settings.screenshot_dir(), "screenshot-"), (settings.video_dir(), "screenrecording-")]
        .into_iter()
        .flat_map(|(dir, prefix)| {
            std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(move |entry| {
                let path = entry.path();
                let name = path.file_name()?.to_string_lossy().into_owned();
                let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
                let kind = match ext.as_str() {
                    "png" => Kind::Shot,
                    "gif" => Kind::Gif,
                    "mp4" | "mkv" | "webm" | "mov" => Kind::Video,
                    _ => return None,
                };
                if !name.starts_with(prefix) {
                    return None;
                }
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some(Item { path, name: name.into(), kind, modified, thumb: None })
            })
        })
        .collect();
    items.sort_by(|a, b| b.modified.cmp(&a.modified));
    items.truncate(LIMIT);
    items
}

/// Cached, card-shaped thumbnail; generated (image crate or ffmpeg) on a miss.
async fn thumbnail(path: PathBuf, modified: SystemTime, video: bool, executor: BackgroundExecutor) -> Option<Thumb> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    modified.hash(&mut h);
    THUMB.hash(&mut h);
    let dir = util::cache_dir().join("thumbs");
    let (sharp_file, blur_file, length_file) = (dir.join(format!("{:016x}.png", h.finish())), dir.join(format!("{:016x}-blur.png", h.finish())), dir.join(format!("{:016x}.len", h.finish())));
    let image = |bytes: Vec<u8>| Arc::new(Image::from_bytes(ImageFormat::Png, bytes));
    if let (Ok(sharp), Ok(blurred)) = (std::fs::read(&sharp_file), std::fs::read(&blur_file)) {
        let seconds = std::fs::read_to_string(&length_file).ok().and_then(|s| s.trim().parse().ok());
        return Some(Thumb { sharp: image(sharp), blurred: image(blurred), seconds });
    }
    let _ = std::fs::create_dir_all(&dir);
    let (source, seconds) = if video {
        let frame = util::runtime_dir().join(format!("kadr-history-{:016x}.png", h.finish()));
        let (ok, _) = util::output::<&std::ffi::OsStr>(&[
            "ffmpeg".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(), "-i".as_ref(), path.as_os_str(),
            "-frames:v".as_ref(), "1".as_ref(), "-vf".as_ref(), "scale=840:-2".as_ref(), frame.as_os_str(),
        ])
        .await?;
        let bytes = ok.then(|| std::fs::read(&frame).ok()).flatten();
        let _ = std::fs::remove_file(&frame);
        let (_, length) = util::output::<&std::ffi::OsStr>(&[
            "ffprobe".as_ref(), "-v".as_ref(), "error".as_ref(), "-show_entries".as_ref(), "format=duration".as_ref(), "-of".as_ref(), "csv=p=0".as_ref(), path.as_os_str(),
        ])
        .await
        .unwrap_or_default();
        (bytes?, length.trim().parse::<f64>().ok())
    } else {
        (std::fs::read(&path).ok()?, None)
    };
    let (sharp, blurred) = executor.spawn(async move { crop_thumb(&source) }).await?;
    let _ = std::fs::write(&sharp_file, &sharp);
    let _ = std::fs::write(&blur_file, &blurred);
    if let Some(s) = seconds {
        let _ = std::fs::write(&length_file, s.to_string());
    }
    Some(Thumb { sharp: image(sharp), blurred: image(blurred), seconds })
}

/// The tile picture and its blurred copy, as PNGs.
fn crop_thumb(bytes: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let src = image::load_from_memory(bytes).ok()?;
    let thumb = src.resize_to_fill(THUMB.0, THUMB.1, image::imageops::FilterType::Triangle);
    let blurred = thumb.resize_exact(THUMB.0 / 4, THUMB.1 / 4, image::imageops::FilterType::Triangle).blur(3.).resize_exact(THUMB.0, THUMB.1, image::imageops::FilterType::Triangle);
    let encode = |img: &image::DynamicImage| {
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(png)
    };
    Some((encode(&thumb)?, encode(&blurred)?))
}

pub fn open(cx: &mut App) {
    // One history window at a time.
    if let Some(window) = cx.windows().into_iter().find_map(|w| w.downcast::<History>()) {
        let _ = window.update(cx, |_, window, _| window.activate());
        return;
    }
    let mon = util::focused_monitor();
    kit::open_window(cx, "kadr-history", mon.as_ref(), (1100., 760.), |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let items = scan();
            // Thumbnails stream in, newest first.
            let jobs: Vec<(usize, PathBuf, SystemTime, bool)> = items.iter().enumerate().map(|(i, it)| (i, it.path.clone(), it.modified, it.kind != Kind::Shot)).collect();
            cx.spawn(async move |this, cx| {
                for (i, path, modified, video) in jobs {
                    let thumb = thumbnail(path.clone(), modified, video, cx.background_executor().clone()).await;
                    let alive = this.update(cx, |history: &mut History, cx| {
                        if let Some(item) = history.items.get_mut(i).filter(|it| it.path == path) {
                            item.thumb = thumb;
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            })
            .detach();
            History { items, query: String::new(), filter: Filter::All, selected: 0, columns: 4, item_width: MIN_ITEM, scroll: ScrollHandle::new(), focus }
        })
    });
}

impl History {
    fn visible(&self) -> Vec<usize> {
        let q = self.query.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| self.filter.accepts(it.kind) && (q.is_empty() || it.name.to_lowercase().contains(&q) || when(it.modified).to_lowercase().contains(&q)))
            .map(|(i, _)| i)
            .collect()
    }

    fn current(&self) -> Option<usize> {
        self.visible().get(self.selected).copied()
    }

    fn select(&mut self, index: usize) {
        let count = self.visible().len();
        if count > 0 {
            self.selected = index.min(count - 1);
            self.scroll.scroll_to_item(self.selected);
        }
    }

    /// Open: the editor for screenshots, the player for videos and GIFs.
    fn open_item(&mut self, index: usize, edit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(index) else { return };
        let path = item.path.clone();
        match (item.kind, edit) {
            (Kind::Shot, _) => {
                window.remove_window();
                editor::open(path, cx);
            }
            (Kind::Video, true) => {
                window.remove_window();
                trim::open(path, cx);
            }
            _ => util::open_file(&path),
        }
    }

    fn copy(&self, index: usize) {
        if let Some(item) = self.items.get(index) {
            match item.kind {
                Kind::Shot => util::copy_image(&item.path),
                _ => util::copy_file(&item.path),
            }
        }
    }

    fn delete(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.items.len() {
            util::trash(&self.items[index].path);
            self.items.remove(index);
            let count = self.visible().len();
            self.selected = self.selected.min(count.saturating_sub(1));
            cx.notify();
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        let current = self.current();
        let cols = self.columns.max(1);
        match (k.key.as_str(), ctrl) {
            ("escape", _) if !self.query.is_empty() => {
                self.query.clear();
                self.selected = 0;
            }
            ("escape", _) => return window.remove_window(),
            ("left", _) => self.select(self.selected.saturating_sub(1)),
            ("right", _) => self.select(self.selected + 1),
            ("up", _) => self.select(self.selected.saturating_sub(cols)),
            ("down", _) => self.select(self.selected + cols),
            ("enter", _) => {
                if let Some(i) = current {
                    return self.open_item(i, false, window, cx);
                }
            }
            ("c", true) => {
                if let Some(i) = current {
                    self.copy(i);
                    return window.remove_window();
                }
            }
            ("tab", _) => {
                let i = Filter::ALL.iter().position(|(f, _)| *f == self.filter).unwrap_or(0);
                self.filter = Filter::ALL[(i + 1) % Filter::ALL.len()].0;
                self.selected = 0;
            }
            ("delete", _) => {
                if let Some(i) = current {
                    self.delete(i, cx);
                }
            }
            ("backspace", _) => {
                self.query.pop();
                self.selected = 0;
            }
            // Single-letter shortcuts only when not typing a search.
            ("e", false) if self.query.is_empty() => {
                if let Some(i) = current {
                    return self.open_item(i, true, window, cx);
                }
            }
            ("p", false) if self.query.is_empty() => {
                if let Some(i) = current.filter(|&i| self.items[i].kind == Kind::Shot) {
                    window.remove_window();
                    return pin::open(self.items[i].path.clone(), cx);
                }
            }
            (_, false) => {
                if let Some(ch) = k.key_char.as_deref().filter(|c| !c.chars().any(char::is_control)) {
                    self.query.push_str(ch);
                    self.selected = 0;
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn render_item(&self, index: usize, position: usize, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.items[index];
        let selected = position == self.selected;
        let colors = cx.theme().colors();
        let group: SharedString = format!("item-{index}").into();
        let path = item.path.clone();
        let (width, height) = (self.item_width, self.item_width / THUMB_ASPECT);
        // Hover looks like the thumbnail card: the picture blurs, white
        // pills on top.
        let ink = gpui::hsla(0., 0., 0.12, 1.);
        let pill = |el: gpui::Stateful<Div>| {
            el.flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(gpui::hsla(0., 0., 1., 0.92))
                .text_color(ink)
                .cursor_pointer()
                .hover(|s| s.bg(gpui::hsla(0., 0., 1., 1.)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let corner = |id: &'static str, icon: IconName, tip: &'static str| {
            pill(div().id((id, index))).size(px(26.)).child(Icon::new(icon).size(IconSize::XSmall).color(Color::Custom(ink))).tooltip(Tooltip::text(tip))
        };
        let wide = |id: &'static str, label: &'static str| pill(div().id((id, index))).h(px(28.)).px_3().child(Label::new(label).size(LabelSize::Small).color(Color::Custom(ink)));
        let overlay = div()
            .absolute()
            .inset_0()
            .opacity(0.)
            .group_hover(group.clone(), |s| s.opacity(1.))
            .children(item.thumb.as_ref().map(|t| img(t.blurred.clone()).absolute().inset_0().size_full().object_fit(ObjectFit::Fill)))
            .child(div().absolute().inset_0().bg(rgba(0x00000052)))
            .child(
                v_flex()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .gap_1p5()
                    .child(wide("copy", t("common.copy")).tooltip(Tooltip::text(t("history.copy_ctrl_c"))).on_click(cx.listener(move |this, _: &ClickEvent, window, _| {
                        this.copy(index);
                        window.remove_window();
                    })))
                    .when(item.kind != Kind::Gif, |col| {
                        col.child(
                            wide(if item.kind == Kind::Shot { "edit" } else { "trim" }, if item.kind == Kind::Shot { t("card.edit") } else { t("common.trim") })
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_item(index, true, window, cx))),
                        )
                    }),
            )
            .child(div().absolute().top_1p5().right_1p5().child(corner("folder", IconName::Folder, t("common.show_in_folder")).on_click({
                let path = path.clone();
                move |_, _, _| util::open_folder(&path)
            })))
            .child(div().absolute().bottom_1p5().right_1p5().child(corner("trash", IconName::Trash, t("history.delete_del")).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.delete(index, cx)))));

        // A recording's length (or "GIF") on a frosted pill.
        let badge = match item.kind {
            Kind::Shot => None,
            Kind::Video => item.thumb.as_ref().and_then(|t| t.seconds).map(util::format_duration),
            Kind::Gif => Some("GIF".to_string()),
        }
        .map(|text| {
            h_flex()
                .absolute()
                .top_1p5()
                .left_1p5()
                .gap_1()
                .px_2()
                .py_0p5()
                .rounded_full()
                .bg(rgba(0x00000066))
                .backdrop_blur(px(8.))
                .when(item.kind == Kind::Video, |d| d.child(Icon::new(IconName::PlayFilled).size(IconSize::XSmall).color(Color::Custom(gpui::white()))))
                .child(Label::new(text).size(LabelSize::Small).color(Color::Custom(gpui::white())))
        });

        let thumb = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .rounded(px(8.))
            .overflow_hidden()
            .bg(colors.element_background)
            .children(item.thumb.as_ref().map(|t| img(t.sharp.clone()).size_full().object_fit(ObjectFit::Cover).rounded(px(8.))))
            .children(badge)
            .child(overlay);
        // The selected thumbnail gets an accent ring, like Finder's.
        let ring = div().p(px(3.)).rounded(px(11.)).border_2().border_color(if selected { colors.text_accent } else { gpui::transparent_black() }).child(thumb);
        let kind = match item.kind {
            Kind::Shot => "PNG",
            Kind::Video => "MP4",
            Kind::Gif => "GIF",
        };
        v_flex()
            .id(("item", index))
            .group(group)
            .w(px(width + 10.))
            .gap_1()
            .cursor_pointer()
            .child(ring)
            .child(
                h_flex()
                    .px_1p5()
                    .gap_1()
                    .justify_between()
                    .child(Label::new(when(item.modified)).size(LabelSize::Small))
                    .child(Label::new(kind).size(LabelSize::XSmall).color(Color::Muted)),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                this.selected = position;
                if e.click_count == 2 {
                    this.open_item(index, false, window, cx);
                }
                cx.notify();
            }))
            // Drag the file out to a chat, browser or file manager.
            .on_drag(path, |_: &PathBuf, _, _, cx| cx.new(|_| DragPreview))
            .external_drag_payload(|path: &PathBuf, _, _| Some(gpui::ExternalDragPayload::Files(gpui::FileDragPaths::new([(path.clone(), false)]))))
            .into_any_element()
    }
}

impl Render for History {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        // As many columns as fit, stretched to fill the row.
        let room = f32::from(viewport.width) - 2. * 16. - 6.;
        let tile = MIN_ITEM + 10.;
        self.columns = ((room + GAP) / (tile + GAP)).floor().max(1.) as usize;
        self.item_width = (room - GAP * (self.columns as f32 - 1.)) / self.columns as f32 - 10.;

        let visible = self.visible();
        let filters: Vec<_> = Filter::ALL.iter().map(|&(filter, label)| {
            ButtonLike::new(SharedString::from(format!("filter-{label}")))
                .style(if self.filter == filter { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                .size(ButtonSize::Large)
                .toggle_state(self.filter == filter)
                .child(div().px_2().child(Label::new(t(label))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.filter = filter;
                    this.selected = 0;
                    cx.notify();
                }))
        }).collect();
        // Big and bold, like Raycast's search.
        let search = h_flex()
            .flex_1()
            .gap_3()
            .text_size(crate::ui::rems_from_px(19.))
            .child(Icon::new(IconName::MagnifyingGlass).size(IconSize::Large).color(Color::Muted))
            .child(if self.query.is_empty() {
                div().text_color(cx.theme().colors().text_placeholder).child(t("history.search_by_name_date")).into_any_element()
            } else {
                div().child(format!("{}▏", self.query)).into_any_element()
            });

        let grid: Vec<AnyElement> = visible.iter().enumerate().map(|(pos, &i)| self.render_item(i, pos, cx)).collect();
        let empty = grid.is_empty();
        let body = div()
            .id("grid")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px_4()
            .py_3()
            .flex()
            .flex_wrap()
            .content_start()
            .gap(px(GAP))
            .children(grid)
            .when(empty, |b| {
                b.child(
                    div()
                        .w_full()
                        .pt_10()
                        .flex()
                        .justify_center()
                        .child(Label::new(if self.items.is_empty() { t("history.nothing_captured_yet") } else { t("history.nothing_found") }).color(Color::Muted)),
                )
            });

        // The key, then what it does, on one line at one size.
        let hint = |keys: &str, label: &'static str| h_flex().gap_1p5().child(kit::keys(keys)).child(Label::new(label).size(LabelSize::Small));

        kit::root(cx)
            .bg(cx.theme().colors().panel_background)
            .flex()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .child(
                v_flex()
                    .id("history")
                    .size_full()
                    .child(
                        h_flex()
                            .gap_2()
                            .pl_4()
                            .pr_2()
                            .py_2()
                            .border_b_1()
                            .border_color(cx.theme().colors().border_variant)
                            .child(search)
                            .child(h_flex().gap_0p5().children(filters))
                            .child(Label::new(format!("{}", visible.len())).size(LabelSize::Small).color(Color::Muted))
                            .child(
                                IconButton::new("settings", IconName::Settings)
                                    .shape(IconButtonShape::Square)
                                    .size(ButtonSize::Large)
                                    .tooltip(Tooltip::text(t("common.settings")))
                                    .on_click(|_, window, cx| {
                                        window.remove_window();
                                        crate::settings_view::open(cx);
                                    }),
                            ),
                    )
                    .child(body)
                    .child(
                        h_flex()
                            .gap_5()
                            .px_4()
                            .py_2()
                            .border_t_1()
                            .border_color(cx.theme().colors().border_variant)
                            .child(hint("enter", t("common.open")))
                            .child(hint("e", t("history.editor_trim")))
                            .child(hint("ctrl-c", t("common.copy")))
                            .child(hint("delete", t("history.delete_selected")))
                            .child(hint("tab", t("history.filter")))
                            .child(div().flex_1())
                            .child(hint("escape", t("common.close"))),
                    ),
            )
    }
}

/// The platform draws its own icon for outbound file drags.
struct DragPreview;

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// "сегодня 14:32", "вчера 09:10", "12 окт 18:05", "3 мар 2025".
fn when(t: SystemTime) -> String {
    use chrono::{Datelike, Local};
    let t: chrono::DateTime<Local> = t.into();
    let today = Local::now().date_naive();
    let months = [crate::i18n::t("history.jan"), crate::i18n::t("history.feb"), crate::i18n::t("history.mar"), crate::i18n::t("history.apr"), crate::i18n::t("history.may"), crate::i18n::t("history.jun"), crate::i18n::t("history.jul"), crate::i18n::t("history.aug"), crate::i18n::t("history.sep"), crate::i18n::t("history.oct"), crate::i18n::t("history.nov"), crate::i18n::t("history.dec")];
    let date = t.date_naive();
    let time = t.format("%H:%M");
    if date == today {
        tr!("history.today", time)
    } else if Some(date) == today.pred_opt() {
        tr!("history.yesterday", time)
    } else if date.year() == today.year() {
        format!("{} {} {time}", date.day(), months[date.month0() as usize])
    } else {
        format!("{} {} {}", date.day(), months[date.month0() as usize], date.year())
    }
}
