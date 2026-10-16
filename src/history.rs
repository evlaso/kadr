//! Capture history, CleanShot-style, laid out like Zed's palettes: every
//! screenshot, recording and GIF kadr (or Omarchy) made, newest first, with
//! type-to-search, filters, keyboard navigation and the card actions.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use crate::kit::{Anchor, KeyboardInteractivity};
use gpui::{
    AnyElement, App, BackgroundExecutor, ClickEvent, Context, FocusHandle, Image, ImageFormat, KeyDownEvent, MouseButton,
    MouseDownEvent, ObjectFit, ScrollHandle, SharedString, Window, div, img, px,
    rgba,
};
use crate::ui::{ButtonLike, IconButtonShape, Tooltip, prelude::*};

use crate::kit::{self, LayerSpec};
use crate::{editor, pin, trim, util};

const ITEM_WIDTH: f32 = 208.;
const THUMB: (u32, u32) = (208, 128);
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
    const ALL: [(Filter, &'static str); 4] = [(Filter::All, "Все"), (Filter::Shots, "Скриншоты"), (Filter::Videos, "Записи"), (Filter::Gifs, "GIF")];

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
    thumb: Option<Arc<Image>>,
}

pub struct History {
    items: Vec<Item>,
    query: String,
    filter: Filter,
    /// Index into `visible()`.
    selected: usize,
    /// Items per row, from the last layout.
    columns: usize,
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
async fn thumbnail(path: PathBuf, modified: SystemTime, video: bool, executor: BackgroundExecutor) -> Option<Arc<Image>> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    modified.hash(&mut h);
    let cached = util::cache_dir().join("thumbs").join(format!("{:016x}.png", h.finish()));
    if let Ok(bytes) = std::fs::read(&cached) {
        return Some(Arc::new(Image::from_bytes(ImageFormat::Png, bytes)));
    }
    let _ = std::fs::create_dir_all(cached.parent()?);
    let source = if video {
        let frame = util::runtime_dir().join(format!("kadr-history-{:016x}.png", h.finish()));
        let (ok, _) = util::output::<&std::ffi::OsStr>(&[
            "ffmpeg".as_ref(), "-y".as_ref(), "-loglevel".as_ref(), "error".as_ref(), "-i".as_ref(), path.as_os_str(),
            "-frames:v".as_ref(), "1".as_ref(), "-vf".as_ref(), "scale=640:-2".as_ref(), frame.as_os_str(),
        ])
        .await?;
        let bytes = ok.then(|| std::fs::read(&frame).ok()).flatten();
        let _ = std::fs::remove_file(&frame);
        bytes?
    } else {
        std::fs::read(&path).ok()?
    };
    let png = executor.spawn(async move { crop_thumb(&source) }).await?;
    let _ = std::fs::write(&cached, &png);
    Some(Arc::new(Image::from_bytes(ImageFormat::Png, png)))
}

fn crop_thumb(bytes: &[u8]) -> Option<Vec<u8>> {
    let src = image::load_from_memory(bytes).ok()?;
    let (tw, th) = (THUMB.0 * 2, THUMB.1 * 2);
    let thumb = src.resize_to_fill(tw, th, image::imageops::FilterType::Triangle);
    let mut png = Vec::new();
    thumb.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

pub fn open(cx: &mut App) {
    let Some(mon) = util::focused_monitor() else { return };
    let spec = LayerSpec {
        namespace: "kadr-history",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mon.w as f32, mon.h as f32),
        keyboard: KeyboardInteractivity::Exclusive,
        ignore_zones: true,
        monitor: Some(mon),
    };
    kit::open_layer(cx, spec, |window, cx| {
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
            History { items, query: String::new(), filter: Filter::All, selected: 0, columns: 4, scroll: ScrollHandle::new(), focus }
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
        let tool = |key: &'static str, icon: IconName, tip: &'static str| {
            IconButton::new((key, index), icon).style(ButtonStyle::Filled).shape(IconButtonShape::Square).icon_size(IconSize::XSmall).tooltip(Tooltip::text(tip))
        };
        let meta = match item.kind {
            Kind::Shot => "PNG",
            Kind::Video => "Видео",
            Kind::Gif => "GIF",
        };
        let actions = h_flex()
            .absolute()
            .bottom_1p5()
            .right_1p5()
            .gap_0p5()
            .opacity(0.)
            .group_hover(group.clone(), |s| s.opacity(1.))
            .child(tool("copy", IconName::Copy, "Копировать (Ctrl+C)").on_click(cx.listener(move |this, _: &ClickEvent, window, _| {
                this.copy(index);
                window.remove_window();
            })))
            .when(item.kind != Kind::Gif, |row| {
                row.child(
                    tool("edit", if item.kind == Kind::Shot { IconName::Pencil } else { IconName::Scissors }, if item.kind == Kind::Shot { "Редактировать (E)" } else { "Обрезать (E)" })
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_item(index, true, window, cx))),
                )
            })
            .child(tool("folder", IconName::Folder, "Показать в папке").on_click({
                let path = path.clone();
                move |_, _, _| util::open_folder(&path)
            }))
            .child(tool("trash", IconName::Trash, "Удалить (Del)").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.delete(index, cx))));

        let thumb = div()
            .relative()
            .w(px(THUMB.0 as f32))
            .h(px(THUMB.1 as f32))
            .rounded_md()
            .bg(colors.element_background)
            .children(item.thumb.clone().map(|t| img(t).size_full().object_fit(ObjectFit::Fill).rounded_md()))
            .when(item.kind != Kind::Shot, |t| {
                t.child(
                    div()
                        .absolute()
                        .top_1p5()
                        .left_1p5()
                        .px_1()
                        .rounded_sm()
                        .bg(rgba(0x000000a6))
                        .text_color(gpui::white())
                        .text_xs()
                        .child(meta),
                )
            })
            .child(actions);

        v_flex()
            .id(("item", index))
            .group(group)
            .w(px(ITEM_WIDTH))
            .gap_1()
            .p_1()
            .rounded_lg()
            .border_2()
            .border_color(if selected { colors.border_focused } else { gpui::transparent_black() })
            .hover(|s| s.bg(colors.ghost_element_hover))
            .cursor_pointer()
            .child(thumb)
            .child(
                h_flex()
                    .px_0p5()
                    .gap_1()
                    .justify_between()
                    .child(Label::new(when(item.modified)).size(LabelSize::Small))
                    .child(Label::new(meta).size(LabelSize::XSmall).color(Color::Muted)),
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
        let width = (f32::from(viewport.width) * 0.8).min(1180.);
        let height = f32::from(viewport.height) * 0.78;
        self.columns = (((width - 24.) + GAP) / (ITEM_WIDTH + GAP)).floor().max(1.) as usize;

        let visible = self.visible();
        let filters: Vec<_> = Filter::ALL.iter().map(|&(filter, label)| {
            ButtonLike::new(SharedString::from(format!("filter-{label}")))
                .style(if self.filter == filter { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                .toggle_state(self.filter == filter)
                .child(div().px_1p5().child(Label::new(label).size(LabelSize::Small)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.filter = filter;
                    this.selected = 0;
                    cx.notify();
                }))
        }).collect();
        let search = h_flex()
            .flex_1()
            .gap_2()
            .child(Icon::new(IconName::MagnifyingGlass).size(IconSize::Small).color(Color::Muted))
            .child(if self.query.is_empty() {
                Label::new("Поиск по имени или дате…").color(Color::Placeholder).into_any_element()
            } else {
                Label::new(format!("{}▏", self.query)).into_any_element()
            });

        let grid: Vec<AnyElement> = visible.iter().enumerate().map(|(pos, &i)| self.render_item(i, pos, cx)).collect();
        let empty = grid.is_empty();
        let body = div()
            .id("grid")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p_3()
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
                        .child(Label::new(if self.items.is_empty() { "Пока ничего не снято" } else { "Ничего не найдено" }).color(Color::Muted)),
                )
            });

        let hint = |keys: &str, label: &'static str| h_flex().gap_1().child(kit::keys(keys)).child(Label::new(label).size(LabelSize::XSmall).color(Color::Muted));

        kit::root(cx)
            .bg(rgba(0x00000099))
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.remove_window())
            .child(
                v_flex()
                    .id("history")
                    .w(px(width))
                    .h(px(height))
                    .elevation_3(cx)
                    .overflow_hidden()
                    // Clicks inside don't reach the backdrop's close handler.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .child(search)
                            .child(h_flex().gap_0p5().children(filters))
                            .child(Label::new(format!("{}", visible.len())).size(LabelSize::Small).color(Color::Muted))
                            .child(
                                IconButton::new("settings", IconName::Settings)
                                    .shape(IconButtonShape::Square)
                                    .tooltip(Tooltip::text("Настройки"))
                                    .on_click(|_, window, cx| {
                                        window.remove_window();
                                        crate::settings_view::open(cx);
                                    }),
                            ),
                    )
                    .child(body)
                    .child(
                        h_flex()
                            .gap_4()
                            .px_3()
                            .py_1p5()
                            .child(hint("enter", "Открыть"))
                            .child(hint("e", "Редактор / обрезка"))
                            .child(hint("ctrl-c", "Копировать"))
                            .child(hint("delete", "Удалить"))
                            .child(hint("tab", "Фильтр"))
                            .child(div().flex_1())
                            .child(hint("escape", "Закрыть")),
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
    let months = ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];
    let date = t.date_naive();
    let time = t.format("%H:%M");
    if date == today {
        format!("сегодня {time}")
    } else if Some(date) == today.pred_opt() {
        format!("вчера {time}")
    } else if date.year() == today.year() {
        format!("{} {} {time}", date.day(), months[date.month0() as usize])
    } else {
        format!("{} {} {}", date.day(), months[date.month0() as usize], date.year())
    }
}
