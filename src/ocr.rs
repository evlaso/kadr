//! Text recognition, like CleanShot's "Capture Text": pick an area (or use
//! a screenshot you already have), the text goes to the clipboard and a
//! Zed-style toast shows what was found. Tesseract does the reading, with
//! every installed language that matters here.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context, SharedString, Window, WindowHandle, div, layer_shell::*, px};
use ui::{IconButtonShape, Tooltip, prelude::*};

use crate::kit::{self, InputRegion, LayerSpec};
use crate::util;

const TOAST_TIME: Duration = Duration::from_secs(7);

/// `kadr text`: select an area on a frozen screen and read it.
pub fn region(cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        let Some(png) = grab_region().await else { return };
        let text = recognize(&png, Layout::Block).await;
        let _ = std::fs::remove_file(&png);
        let _ = cx.update(|cx| done(text, cx));
    })
    .detach();
}

/// Read a whole picture (a screenshot card, the editor).
pub fn image(path: PathBuf, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        let text = recognize(&path, Layout::Auto).await;
        let _ = cx.update(|cx| done(text, cx));
    })
    .detach();
}

/// Omarchy's freeze + slurp, then grim on the picked area.
async fn grab_region() -> Option<PathBuf> {
    let (_, out) = util::output(&["omarchy-capture-region", "region", "--keep-freeze"]).await?;
    let mut lines = out.lines();
    let freeze = lines.next().and_then(|l| l.trim().parse::<u32>().ok());
    let selection = lines.next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let png = util::runtime_dir().join(format!("kadr-ocr-{}.png", std::process::id()));
    let grabbed = match &selection {
        Some(geometry) => util::output::<&OsStr>(&["grim".as_ref(), "-g".as_ref(), geometry.as_ref(), png.as_os_str()])
            .await
            .is_some_and(|(ok, _)| ok),
        None => false,
    };
    if let Some(pid) = freeze {
        util::signal(pid, libc::SIGTERM);
    }
    grabbed.then_some(png)
}

enum Layout {
    /// A selected block of text.
    Block,
    /// A full screenshot with columns, sidebars and so on.
    Auto,
}

/// Installed Tesseract languages kadr cares about, e.g. "rus+eng".
fn languages() -> String {
    let installed = std::process::Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let wanted: Vec<&str> = ["rus", "eng", "ukr", "deu", "fra", "spa"]
        .into_iter()
        .filter(|l| installed.lines().any(|i| i.trim() == *l))
        .collect();
    if wanted.is_empty() { "eng".into() } else { wanted.join("+") }
}

pub fn has_russian() -> bool {
    languages().split('+').any(|l| l == "rus")
}

async fn recognize(png: &Path, layout: Layout) -> Option<String> {
    let psm = match layout {
        Layout::Block => "6",
        Layout::Auto => "3",
    };
    let langs = languages();
    let (ok, text) = util::output::<&OsStr>(&[
        "tesseract".as_ref(),
        png.as_os_str(),
        "stdout".as_ref(),
        "--oem".as_ref(),
        "1".as_ref(),
        "--psm".as_ref(),
        psm.as_ref(),
        "-l".as_ref(),
        langs.as_ref(),
        "-c".as_ref(),
        "preserve_interword_spaces=1".as_ref(),
    ])
    .await?;
    let text = text.trim().to_owned();
    (ok && !text.is_empty()).then_some(text)
}

fn done(text: Option<String>, cx: &mut App) {
    if let Some(text) = &text {
        copy_text(text);
    }
    show_toast(text, cx);
}

fn copy_text(text: &str) {
    util::spawn(&["wl-copy", "--", text]);
}

struct Toast {
    text: Option<String>,
    region: InputRegion,
}

fn show_toast(text: Option<String>, cx: &mut App) {
    let lines = text.as_deref().map_or(1, |t| t.lines().count().min(4)) as f32;
    let spec = LayerSpec {
        namespace: "kadr-toast",
        anchor: Anchor::BOTTOM,
        margin: (0., 0., 24., 0.),
        size: (440., 120. + lines * 18.),
        keyboard: KeyboardInteractivity::None,
        ignore_zones: false,
        monitor: util::focused_monitor(),
    };
    let handle: Option<WindowHandle<Toast>> = kit::open_layer(cx, spec, |_, cx| cx.new(|_| Toast { text, region: InputRegion::default() }));
    if let Some(handle) = handle {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(TOAST_TIME).await;
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        })
        .detach();
    }
}

impl Render for Toast {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.region.reset();
        let (title, body): (&str, SharedString) = match &self.text {
            Some(text) => {
                let preview: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).take(4).collect();
                ("Текст скопирован", preview.join("\n").into())
            }
            None if !has_russian() => (
                "Текст не найден",
                "Для русского текста поставь пакет tesseract-data-rus".into(),
            ),
            None => ("Текст не найден", "Попробуй выделить область покрупнее".into()),
        };
        let found = self.text.clone().map(Rc::<str>::from);
        let chars = self.text.as_ref().map(|t| t.chars().count());

        kit::root(cx).flex().items_end().justify_center().child(
            v_flex()
                .relative()
                .w(px(400.))
                .gap_1()
                .pl_2p5()
                .pr_1()
                .pt_1()
                .pb_2()
                .elevation_3(cx)
                .child(
                    h_flex()
                        .gap_1p5()
                        .child(Icon::new(IconName::CaseSensitive).size(IconSize::Small).color(if found.is_some() { Color::Accent } else { Color::Muted }))
                        .child(div().flex_1().child(Label::new(title).size(LabelSize::Small)))
                        .when_some(chars, |row, n| row.child(Label::new(format!("{n} симв.")).size(LabelSize::XSmall).color(Color::Muted)))
                        .child(
                            IconButton::new("close", IconName::Close)
                                .shape(IconButtonShape::Square)
                                .icon_size(IconSize::XSmall)
                                .tooltip(Tooltip::text("Закрыть"))
                                .on_click(|_, window, _| window.remove_window()),
                        ),
                )
                .child(
                    div()
                        .pr_1p5()
                        .font_family(kit::MONO_FONT)
                        .text_xs()
                        .text_color(cx.theme().colors().text_muted)
                        .line_clamp(4)
                        .child(body),
                )
                .when_some(found, |col, text| {
                    col.child(
                        h_flex().justify_end().pr_1().child(
                            Button::new("again", "Скопировать ещё раз")
                                .style(ButtonStyle::Subtle)
                                .start_icon(Icon::new(IconName::Copy).size(IconSize::Small))
                                .on_click(move |_, window, _| {
                                    copy_text(&text);
                                    window.remove_window();
                                }),
                        ),
                    )
                })
                .child(self.region.mark()),
        )
    }
}
