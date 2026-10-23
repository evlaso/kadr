//! Capture history, laid out as a palette: every
//! screenshot, recording and GIF kadr (or Omarchy) made, newest first, with
//! type-to-search, filters, keyboard navigation and the card actions.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, KeyDownEvent, MouseButton,
    MouseDownEvent, ObjectFit, RenderImage, ScrollHandle, SharedString, Window, div, img, px,
    rgba,
};
use crate::ui::{ButtonLike, IconButtonShape, Tooltip, prelude::*};

use crate::kit;
use crate::{editor, pin, util};
use crate::i18n::t;

/// Tiles are at least this wide; a row stretches them to fill the window.
const MIN_ITEM: f32 = 210.;
/// Thumbnail shape, width over height.
const THUMB_ASPECT: f32 = 1.6;
/// Stored thumbnail size, sharp up to twice the smallest tile.
const THUMB: (u32, u32) = (420, 262);
const GAP: f32 = 12.;
/// The grid's padding above and below.
const PAD_Y: f32 = 12.;
/// Thumbnail corners, and how far in its round buttons and badge sit:
/// circles of radius `RADIUS - INSET` stay concentric with the corner.
const RADIUS: f32 = 14.;
const INSET: f32 = 4.;
const BUTTON: f32 = 2. * (RADIUS - INSET);
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
    sharp: Arc<RenderImage>,
    blurred: Arc<RenderImage>,
    seconds: Option<f64>,
}

pub struct History {
    items: Vec<Item>,
    query: String,
    filter: Filter,
    /// Index into `visible()`: the current tile, and where a Shift-click
    /// range starts.
    selected: usize,
    /// Tiles picked with Ctrl or Shift (indices into `items`); the actions
    /// work on all of them. Empty: just the current one.
    picked: std::collections::BTreeSet<usize>,
    /// Items per row and their width, from the last layout.
    columns: usize,
    item_width: f32,
    scroll: ScrollHandle,
    /// Where the scrollbar thumb was grabbed, while it is dragged.
    grab: std::rc::Rc<std::cell::Cell<Option<f32>>>,
    focus: FocusHandle,
    /// The search's caret is showing: it blinks while the window is active.
    caret: bool,
    /// The tile under the pointer: only it carries the hover buttons.
    hovered: Option<usize>,
    /// Only the rows in view are laid out, between two spacers: the
    /// distance from one row to the next, measured, and which child of the
    /// grid was the first tile last time.
    pitch: Option<f32>,
    first_child: usize,
    /// A narrow window shows the filters as a dropdown, and fewer key hints.
    filter_fit: crate::ui::Fit,
    hint_fit: crate::ui::Fit,
    filter_open: bool,
    filter_menu: crate::ui::MenuBounds,
}

/// Files kadr and Omarchy save, newest first.
fn scan() -> Vec<Item> {
    let settings = crate::settings::Settings::load();
    let mut dirs = vec![settings.screenshot_dir(), settings.video_dir()];
    dirs.dedup();
    let template = &settings.name_template;
    let mut items: Vec<Item> = dirs
        .into_iter()
        .flat_map(|dir| {
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
                if !crate::storage::is_capture(&path, template) {
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

/// Cached, card-shaped thumbnail; generated (image crate or ffmpeg) on a
/// miss. Decoded here, off the main thread, so a tile shows it at once.
async fn thumbnail(path: PathBuf, modified: SystemTime, video: bool) -> Option<Thumb> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    modified.hash(&mut h);
    THUMB.hash(&mut h);
    let dir = util::cache_dir().join("thumbs");
    let (sharp_file, length_file) = (dir.join(format!("{:016x}.png", h.finish())), dir.join(format!("{:016x}.len", h.finish())));
    if let Some(sharp) = std::fs::read(&sharp_file).ok().and_then(|b| image::load_from_memory(&b).ok()) {
        let seconds = std::fs::read_to_string(&length_file).ok().and_then(|s| s.trim().parse().ok());
        return Some(Thumb::new(sharp, seconds));
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
    let sharp = image::load_from_memory(&source).ok()?.resize_to_fill(THUMB.0, THUMB.1, image::imageops::FilterType::Triangle);
    let mut png = Vec::new();
    if sharp.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_ok() {
        let _ = std::fs::write(&sharp_file, &png);
    }
    if let Some(s) = seconds {
        let _ = std::fs::write(&length_file, s.to_string());
    }
    Some(Thumb::new(sharp, seconds))
}

impl Thumb {
    /// The blurred copy stays small: the GPU stretches it over the tile,
    /// which a blur hides.
    fn new(sharp: image::DynamicImage, seconds: Option<f64>) -> Thumb {
        let blurred = sharp.resize_exact(THUMB.0 / 4, THUMB.1 / 4, image::imageops::FilterType::Triangle).blur(3.);
        let render = |img: image::DynamicImage| {
            let mut img = img.to_rgba8();
            for px in img.pixels_mut() {
                px.0.swap(0, 2);
            }
            Arc::new(RenderImage::new([image::Frame::new(img)]))
        };
        Thumb { sharp: render(sharp), blurred: render(blurred), seconds }
    }
}

/// How long the search's caret stays on and off.
const CARET_BLINK: std::time::Duration = std::time::Duration::from_millis(530);

pub fn open(cx: &mut App) {
    // One history window at a time.
    if let Some(window) = cx.windows().into_iter().find_map(|w| w.downcast::<History>()) {
        let _ = window.update(cx, |_, window, _| window.activate());
        return;
    }
    let mon = util::focused_monitor();
    kit::open_window(cx, "kadr-history", mon.as_ref(), (1100., 760.), (560., 400.), |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let items = scan();
            // Thumbnails stream in, newest first, made by a few workers at
            // once and handed to the window in batches.
            let jobs: std::collections::VecDeque<(usize, PathBuf, SystemTime, bool)> =
                items.iter().enumerate().map(|(i, it)| (i, it.path.clone(), it.modified, it.kind != Kind::Shot)).collect();
            let jobs = Arc::new(std::sync::Mutex::new(jobs));
            let (tx, mut rx) = futures::channel::mpsc::unbounded();
            let workers = std::thread::available_parallelism().map_or(2, |n| n.get() / 2).clamp(1, 6);
            for _ in 0..workers {
                let (jobs, tx) = (jobs.clone(), tx.clone());
                cx.background_executor()
                    .spawn(async move {
                        loop {
                            let Some((i, path, modified, video)) = jobs.lock().unwrap_or_else(|e| e.into_inner()).pop_front() else { break };
                            let thumb = thumbnail(path.clone(), modified, video).await;
                            // The window is gone.
                            if tx.unbounded_send((i, path, thumb)).is_err() {
                                break;
                            }
                        }
                    })
                    .detach();
            }
            drop(tx);
            cx.spawn(async move |this, cx| {
                use futures::StreamExt;
                while let Some(first) = rx.next().await {
                    cx.background_executor().timer(std::time::Duration::from_millis(40)).await;
                    let mut batch = vec![first];
                    while let Ok(more) = rx.try_recv() {
                        batch.push(more);
                    }
                    let alive = this.update(cx, |history: &mut History, cx| {
                        for (i, path, thumb) in batch {
                            if let Some(item) = history.items.get_mut(i).filter(|it| it.path == path) {
                                item.thumb = thumb;
                            }
                        }
                        cx.notify();
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            })
            .detach();
            // The search's caret blinks like a text field's, only while the
            // window has the keyboard.
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor().timer(CARET_BLINK).await;
                    let alive = this.update_in(cx, |history: &mut History, window, cx| {
                        let caret = history.focus.is_focused(window) && !history.caret;
                        if caret != history.caret {
                            history.caret = caret;
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            })
            .detach();
            History { items, query: String::new(), filter: Filter::All, selected: 0, picked: Default::default(), columns: 4, item_width: MIN_ITEM, scroll: ScrollHandle::new(), grab: Default::default(), focus, caret: true, hovered: None, pitch: None, first_child: 0, filter_fit: Default::default(), hint_fit: Default::default(), filter_open: false, filter_menu: Default::default() }
        })
    });
}

impl History {
    fn visible(&self) -> Vec<usize> {
        self.matching(self.filter)
    }

    /// The items a filter shows for the current search.
    fn matching(&self, filter: Filter) -> Vec<usize> {
        let q = self.query.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| filter.accepts(it.kind) && (q.is_empty() || it.name.to_lowercase().contains(&q) || when(it.modified).to_lowercase().contains(&q)))
            .map(|(i, _)| i)
            .collect()
    }

    fn current(&self) -> Option<usize> {
        self.visible().get(self.selected).copied()
    }

    /// What the actions work on: the picked tiles, or the current one.
    fn targets(&self) -> Vec<usize> {
        if self.picked.is_empty() { self.current().into_iter().collect() } else { self.picked.iter().copied().collect() }
    }

    /// A click: Ctrl adds or takes away a tile, Shift picks the range from
    /// the current one; a plain click picks just that one.
    fn click(&mut self, position: usize, ctrl: bool, shift: bool) {
        let visible = self.visible();
        if shift {
            let (a, b) = (self.selected.min(position), self.selected.max(position));
            self.picked.extend(visible[a..=b.min(visible.len().saturating_sub(1))].iter().copied());
        } else if ctrl {
            // The current tile counts as picked once the picking starts.
            if self.picked.is_empty()
                && let Some(i) = self.current()
            {
                self.picked.insert(i);
            }
            let i = visible[position];
            if !self.picked.remove(&i) {
                self.picked.insert(i);
            }
            self.selected = position;
        } else {
            self.picked.clear();
            self.selected = position;
        }
    }

    fn select(&mut self, index: usize) {
        let count = self.visible().len();
        if count > 0 {
            self.selected = index.min(count - 1);
            // The tile may not be laid out, so scroll by its row.
            let pitch = self.row_pitch();
            let top = PAD_Y + (self.selected / self.columns.max(1)) as f32 * pitch;
            let bottom = top + pitch - GAP + 2. * PAD_Y;
            let (scrolled, seen) = (-f32::from(self.scroll.offset().y), f32::from(self.scroll.bounds().size.height));
            let to = if top - PAD_Y < scrolled { Some(top - PAD_Y) } else if bottom > scrolled + seen { Some(bottom - seen) } else { None };
            if let Some(to) = to {
                self.scroll.set_offset(gpui::point(px(0.), px(-to.max(0.))));
            }
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
            // Recordings open in the studio, like from their card.
            (Kind::Video, _) => {
                window.remove_window();
                crate::studio_editor::open(path, cx);
            }
            (Kind::Gif, _) => util::open_file(&path),
        }
    }

    /// One picture goes on the clipboard as a picture; anything else (or
    /// several) as files.
    fn copy(&self, indices: &[usize]) {
        match indices {
            [i] if self.items.get(*i).is_some_and(|it| it.kind == Kind::Shot) => util::copy_image(&self.items[*i].path),
            _ => util::copy_files(&indices.iter().filter_map(|&i| self.items.get(i)).map(|it| it.path.as_path()).collect::<Vec<_>>()),
        }
    }

    fn delete(&mut self, indices: &[usize], cx: &mut Context<Self>) {
        let mut indices: Vec<usize> = indices.iter().copied().filter(|&i| i < self.items.len()).collect();
        indices.sort_unstable();
        // From the end, so the indices left stay right.
        for &i in indices.iter().rev() {
            util::trash(&self.items[i].path);
            self.items.remove(i);
        }
        self.picked.clear();
        let count = self.visible().len();
        self.selected = self.selected.min(count.saturating_sub(1));
        cx.notify();
    }

    fn pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shots: Vec<PathBuf> = self.targets().into_iter().filter(|&i| self.items[i].kind == Kind::Shot).map(|i| self.items[i].path.clone()).collect();
        if shots.is_empty() {
            return;
        }
        window.remove_window();
        for path in shots {
            pin::open(path, cx);
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Typing keeps the caret on, like a text field's.
        self.caret = true;
        let k = &e.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        let current = self.current();
        let cols = self.columns.max(1);
        match (k.key.as_str(), ctrl) {
            ("escape", _) if !self.query.is_empty() => {
                self.query.clear();
                self.selected = 0;
                self.picked.clear();
            }
            ("escape", _) if !self.picked.is_empty() => self.picked.clear(),
            ("a", true) => self.picked = self.visible().into_iter().collect(),
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
                let targets = self.targets();
                if !targets.is_empty() {
                    self.copy(&targets);
                    return window.remove_window();
                }
            }
            ("tab", _) => {
                let i = Filter::ALL.iter().position(|(f, _)| *f == self.filter).unwrap_or(0);
                self.filter = Filter::ALL[(i + 1) % Filter::ALL.len()].0;
                self.selected = 0;
                self.picked.clear();
            }
            ("delete", _) => {
                let targets = self.targets();
                self.delete(&targets, cx);
            }
            ("backspace", _) => {
                self.query.pop();
                self.selected = 0;
                self.picked.clear();
            }
            // Single-letter shortcuts only when not typing a search.
            ("e", false) if self.query.is_empty() => {
                if let Some(i) = current {
                    return self.open_item(i, true, window, cx);
                }
            }
            ("p", false) if self.query.is_empty() => return self.pin(window, cx),
            (_, false) => {
                if let Some(ch) = k.key_char.as_deref().filter(|c| !c.chars().any(char::is_control)) {
                    self.query.push_str(ch);
                    self.selected = 0;
                    self.picked.clear();
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn row_pitch(&self) -> f32 {
        // Until measured: the ring, the line under it and the gap.
        self.pitch.unwrap_or(self.item_width / THUMB_ASPECT + 10. + 24. + GAP)
    }

    fn render_item(&self, index: usize, position: usize, hover: bool, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.items[index];
        let selected = if self.picked.is_empty() { position == self.selected } else { self.picked.contains(&index) };
        let colors = cx.theme().colors();
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
            pill(div().id((id, index))).size(px(BUTTON)).child(Icon::new(icon).size(IconSize::XSmall).color(Color::Custom(ink))).tip(Tooltip::text(tip))
        };
        let wide = |id: &'static str, label: &'static str| pill(div().id((id, index))).h(px(28.)).px_3().child(Label::new(label).size(LabelSize::Small).color(Color::Custom(ink)));
        let overlay = hover.then(|| div()
            .absolute()
            .inset_0()
            .children(item.thumb.as_ref().map(|t| img(t.blurred.clone()).absolute().inset_0().size_full().object_fit(ObjectFit::Fill).rounded(px(RADIUS))))
            .child(div().absolute().inset_0().rounded(px(RADIUS)).bg(rgba(0x00000052)))
            .child(
                v_flex()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .gap_1p5()
                    .child(wide("copy", t("common.copy")).tip(Tooltip::text(format!("{} ({})", t("common.copy"), crate::ui::shortcut(kadr_platform::keymap::Command::Copy)))).on_click(cx.listener(move |this, _: &ClickEvent, window, _| {
                        this.copy(&[index]);
                        window.remove_window();
                    })))
                    .when(item.kind != Kind::Gif, |col| {
                        col.child(
                            wide(if item.kind == Kind::Shot { "edit" } else { "studio" }, if item.kind == Kind::Shot { t("card.edit") } else { t("common.studio") })
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_item(index, true, window, cx))),
                        )
                    }),
            )
            .child(div().absolute().top(px(INSET)).right(px(INSET)).child(corner("folder", IconName::Folder, t("common.show_in_folder")).on_click({
                let path = path.clone();
                move |_, _, _| util::open_folder(&path)
            })))
            .child(div().absolute().bottom(px(INSET)).right(px(INSET)).child(corner("trash", IconName::Trash, t("history.delete_del")).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.delete(&[index], cx)))))
            .when(item.kind == Kind::Shot, |d| {
                d.child(div().absolute().bottom(px(INSET)).left(px(INSET)).child(corner("pin", IconName::Pin, t("history.pin_p")).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    let path = this.items[index].path.clone();
                    window.remove_window();
                    pin::open(path, cx);
                }))))
            }));

        // A recording's length (or "GIF") on a frosted pill.
        let badge = match item.kind {
            Kind::Shot => None,
            Kind::Video => item.thumb.as_ref().and_then(|t| t.seconds).map(util::format_duration),
            Kind::Gif => Some("GIF".to_string()),
        }
        .map(|text| {
            h_flex()
                .absolute()
                .top(px(INSET))
                .left(px(INSET))
                .h(px(BUTTON))
                .gap_1()
                .px_2()
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
            .rounded(px(RADIUS))
            .overflow_hidden()
            .bg(colors.element_background)
            .children(item.thumb.as_ref().map(|t| img(t.sharp.clone()).size_full().object_fit(ObjectFit::Cover).rounded(px(RADIUS))))
            // A faint shade and a hairline keep pictures from melting into
            // the window behind them.
            .child(div().absolute().inset_0().rounded(px(RADIUS)).bg(rgba(0x0000001f)))
            .children(badge)
            .children(overlay)
            // The hairline last, so the hover doesn't cover it.
            .child(div().absolute().inset_0().rounded(px(RADIUS)).border_1().border_color(colors.border));
        // The selected thumbnail gets an accent ring, like Finder's.
        // Concentric with the thumbnail: its corner plus the gap and the line.
        let ring = div().p(px(3.)).rounded(px(RADIUS + 5.)).border_2().border_color(if selected { colors.text_accent } else { gpui::transparent_black() }).child(thumb);
        let kind = match item.kind {
            Kind::Shot => "PNG",
            Kind::Video => "MP4",
            Kind::Gif => "GIF",
        };
        v_flex()
            .id(("item", index))
            .w(px(width + 10.))
            .gap_0p5()
            .cursor_pointer()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    this.hovered = Some(index);
                } else if this.hovered == Some(index) {
                    this.hovered = None;
                }
                cx.notify();
            }))
            .child(ring)
            .child({
                let (day, time) = day_and_time(item.modified);
                h_flex()
                    .px_1p5()
                    .gap_1()
                    .items_baseline()
                    .child(Label::new(day).size(LabelSize::Small))
                    .children(time.map(|time| Label::new(time).size(LabelSize::Small).color(Color::Muted)))
                    .child(div().flex_1())
                    .child(Label::new(kind).size(LabelSize::Small).color(Color::Muted))
            })
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                let m = e.modifiers;
                this.click(position, m.control || m.platform, m.shift);
                if e.click_count == 2 && !(m.control || m.platform || m.shift) {
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
        // A pixel of slack: a rounding error would wrap the last tile and
        // leave every row a column short.
        self.item_width = ((room - 1. - GAP * (self.columns as f32 - 1.)) / self.columns as f32 - 10.).floor();

        let visible = self.visible();
        let rem = kit::REM;
        let viewport_w = f32::from(viewport.width);
        // The filters as buttons while the search keeps a fair width beside
        // them, else as a dropdown.
        let dropdown = self.filter_fit.pick(viewport_w - 1.5 * rem - rem - 2. * rem - 14. * rem, 2) == 1;
        // Each filter with how many it shows, unless none.
        let filters: Vec<_> = Filter::ALL.iter().map(|&(filter, label)| {
            let count = self.matching(filter).len();
            ButtonLike::new(SharedString::from(format!("filter-{label}")))
                .style(if self.filter == filter { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                .size(ButtonSize::Large)
                .toggle_state(self.filter == filter)
                .child(
                    h_flex()
                        .px_2()
                        .gap_1p5()
                        .child(Label::new(t(label)))
                        .when(count > 0, |row| row.child(Label::new(count.to_string()).size(LabelSize::Small).color(Color::Muted))),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.filter = filter;
                    this.selected = 0;
                    cx.notify();
                }))
        }).collect();
        let filters = if dropdown {
            let label = Filter::ALL.iter().find(|(f, _)| *f == self.filter).map(|&(_, l)| t(l)).unwrap_or_default();
            let hover = cx.theme().colors().element_hover;
            let menu = self.filter_open.then(|| {
                gpui::deferred(
                    v_flex()
                        .id("filter-menu")
                        .occlude()
                        .absolute()
                        .top(px(38.))
                        .right_0()
                        .min_w(px(170.))
                        .p_1()
                        .elevation_3(cx)
                        .children(Filter::ALL.iter().map(|&(filter, label)| {
                            let count = self.matching(filter).len();
                            h_flex()
                                .id(SharedString::from(format!("filter-row-{label}")))
                                .gap_1p5()
                                .px_2()
                                .h(px(28.))
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(move |d| d.bg(hover))
                                .child(div().w(px(14.)).children((self.filter == filter).then(|| Icon::new(IconName::Check).size(IconSize::XSmall))))
                                .child(Label::new(t(label)).size(LabelSize::Small))
                                .child(div().flex_1())
                                .when(count > 0, |row| row.child(Label::new(count.to_string()).size(LabelSize::Small).color(Color::Muted)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.filter = filter;
                                    this.selected = 0;
                                    this.filter_open = false;
                                    cx.notify();
                                }))
                        }))
                        .child(crate::ui::track_bounds(&self.filter_menu)),
                )
            });
            h_flex()
                .relative()
                .on_mouse_down_out(cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    if this.filter_open && !this.filter_menu.get().contains(&e.position) {
                        this.filter_open = false;
                        cx.notify();
                    }
                }))
                .child(
                    ButtonLike::new("filter-dropdown")
                        .style(if self.filter_open { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                        .size(ButtonSize::Large)
                        .toggle_state(self.filter_open)
                        .child(h_flex().px_2().gap_1().child(Label::new(label)).child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted)))
                        .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                            let items = Filter::ALL.iter().map(|&(f, label)| kadr_platform::menu::Item::new(t(label), f == this.filter)).collect();
                            if crate::ui::native_menu(window, cx, e.position(), items, |this: &mut History, i, _, _| {
                                this.filter = Filter::ALL[i].0;
                                this.selected = 0;
                            }) {
                                return;
                            }
                            this.filter_open = !this.filter_open;
                            cx.notify();
                        })),
                )
                .children(menu)
                .child(self.filter_fit.track(1, 0))
                .into_any_element()
        } else {
            h_flex().relative().gap_0p5().children(filters).child(self.filter_fit.track(0, 0)).into_any_element()
        };
        // Big and bold, like Raycast's search.
        let search = h_flex()
            .flex_1()
            .gap_3()
            .text_size(crate::ui::rems_from_px(19.))
            .child(Icon::new(IconName::MagnifyingGlass).size(IconSize::Large).color(Color::Muted))
            // A text field: an I-beam over it, the caret after what's typed
            // (before the placeholder when nothing is).
            .child({
                let caret = div().flex_none().w(px(1.5)).h(crate::ui::rems_from_px(22.)).when(self.caret, |d| d.bg(cx.theme().colors().text_accent));
                let row = h_flex().id("search").flex_1().cursor(gpui::CursorStyle::IBeam);
                if self.query.is_empty() {
                    row.child(caret).child(div().text_color(cx.theme().colors().text_placeholder).child(t("history.search_by_name_date")))
                } else {
                    row.child(div().child(self.query.clone())).child(caret)
                }
            });

        // Only the rows in view, and one either side, are laid out; spacers
        // stand in for the rest so the scrollbar stays true.
        if let (Some(a), Some(b)) = (self.scroll.bounds_for_item(self.first_child), self.scroll.bounds_for_item(self.first_child + self.columns))
            && b.top() > a.top()
        {
            self.pitch = Some(f32::from(b.top() - a.top()));
        }
        let pitch = self.row_pitch();
        let columns = self.columns.max(1);
        let total = visible.len().div_ceil(columns);
        let scrolled = (-f32::from(self.scroll.offset().y) - PAD_Y).max(0.);
        let seen = f32::from(self.scroll.bounds().size.height).max(f32::from(viewport.height));
        let rows = ((scrolled / pitch).floor() as usize).saturating_sub(1).min(total)..(((scrolled + seen) / pitch).ceil() as usize + 1).min(total);
        let spacer = |rows: usize| (rows > 0).then(|| div().w_full().h(px(rows as f32 * pitch - GAP)).into_any_element());
        let hovered = self.hovered.filter(|_| window.is_window_hovered());
        let mut grid: Vec<AnyElement> = spacer(rows.start).into_iter().collect();
        self.first_child = grid.len();
        grid.extend(visible[rows.start * columns..(rows.end * columns).min(visible.len())].iter().enumerate().map(|(k, &i)| self.render_item(i, rows.start * columns + k, hovered == Some(i), cx)));
        grid.extend(spacer(total - rows.end));
        let empty = grid.is_empty();
        let body = div()
            .id("grid")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px_4()
            .py(px(PAD_Y))
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

        // What it does, then the key, like everywhere else in kadr.
        let hint = |keys: &str, label: &'static str| h_flex().gap_1p5().child(Label::new(label).size(LabelSize::Small)).child(kit::keys(keys));

        // Short of room the key hints go from the last; Open and Close stay.
        let hints = [("e", t("history.editor_studio")), ("secondary-c", t("common.copy")), ("delete", t("history.delete_selected")), ("p", t("history.pin")), ("tab", t("history.filter"))];
        let hint_layout = self.hint_fit.pick(viewport_w - 2. * rem - 2.5 * rem, hints.len() + 1);
        let shown = hints.len() - hint_layout;

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
                            .child(filters)
                            .child(
                                IconButton::new("settings", IconName::Settings)
                                    .shape(IconButtonShape::Square)
                                    .size(ButtonSize::Large)
                                    .tip(Tooltip::text(t("common.settings")))
                                    .on_click(|_, window, cx| {
                                        window.remove_window();
                                        crate::settings_view::open(cx);
                                    }),
                            ),
                    )
                    .child(div().relative().flex_1().min_h_0().child(body).child(crate::ui::scrollbar(&self.scroll, &self.grab, cx)))
                    .child(
                        h_flex()
                            .gap_5()
                            .px_4()
                            .py_2()
                            .border_t_1()
                            .border_color(cx.theme().colors().border_variant)
                            .child(
                                h_flex()
                                    .relative()
                                    .gap_5()
                                    .child(hint("enter", t("common.open")))
                                    .children(hints.into_iter().take(shown).map(|(keys, label)| hint(keys, label)))
                                    .child(self.hint_fit.track(hint_layout, 0)),
                            )
                            .child(div().flex_1())
                            .child(
                                h_flex()
                                    .relative()
                                    .gap_5()
                                    .when(self.picked.len() > 1, |bar| bar.child(Label::new(tr!("history.n_selected", self.picked.len())).size(LabelSize::Small).color(Color::Accent)))
                                    .child(hint("escape", t("common.close")))
                                    .child(self.hint_fit.track(hint_layout, 1)),
                            ),
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
/// The capture's day ("Today", "12 Oct", "3 Mar 2025") and its time, if
/// the day is this year's.
fn day_and_time(t: SystemTime) -> (String, Option<String>) {
    use chrono::{Datelike, Local};
    let t: chrono::DateTime<Local> = t.into();
    let today = Local::now().date_naive();
    let date = t.date_naive();
    let time = t.format("%H:%M").to_string();
    let month = |m: u32| crate::i18n::t(["history.jan", "history.feb", "history.mar", "history.apr", "history.may", "history.jun", "history.jul", "history.aug", "history.sep", "history.oct", "history.nov", "history.dec"][m as usize]);
    let capital = |s: String| {
        let mut c = s.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    };
    if date == today {
        (capital(tr!("history.today", "").trim().to_string()), Some(time))
    } else if Some(date) == today.pred_opt() {
        (capital(tr!("history.yesterday", "").trim().to_string()), Some(time))
    } else if date.year() == today.year() {
        (format!("{} {}", date.day(), month(date.month0())), Some(time))
    } else {
        (format!("{} {} {}", date.day(), month(date.month0()), date.year()), None)
    }
}

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
