//! Settings, laid out like CleanShot's preferences (tabs along the top, one
//! topic each) in Zed's components. Every change is saved on the spot.

use std::path::PathBuf;

use gpui::{
    App, Context, FocusHandle, Hsla, KeyDownEvent, MouseButton, PathPromptOptions, Window, div,
    layer_shell::*, linear_color_stop, linear_gradient, px, rgba,
};
use ui::{ButtonLike, Divider, Switch, ToggleState, Tooltip, prelude::*};

use crate::kit::{self, LayerSpec};
use crate::settings::{CARD_SECONDS, FPS, MAX_CARDS, QUALITIES, Settings, Side};
use crate::studio::{PADDINGS, ZOOMS};
use crate::{ocr, palette, util};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Screenshots,
    Recording,
    Studio,
    Text,
    Shortcuts,
    About,
}

impl Tab {
    const ALL: [Tab; 7] = [Tab::General, Tab::Screenshots, Tab::Recording, Tab::Studio, Tab::Text, Tab::Shortcuts, Tab::About];

    fn label(self) -> &'static str {
        match self {
            Tab::General => "Основные",
            Tab::Screenshots => "Скриншоты",
            Tab::Recording => "Запись",
            Tab::Studio => "Студия",
            Tab::Text => "Текст",
            Tab::Shortcuts => "Клавиши",
            Tab::About => "О kadr",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Tab::General => Icon::new(IconName::Copy),
            Tab::Screenshots => Icon::new(IconName::Image),
            Tab::Recording => Icon::new(IconName::Circle),
            Tab::Studio => Icon::new(IconName::Sparkle),
            Tab::Text => Icon::new(IconName::CaseSensitive),
            Tab::Shortcuts => Icon::new(IconName::Keyboard),
            Tab::About => Icon::new(IconName::Screen),
        }
    }
}

pub struct SettingsView {
    settings: Settings,
    tab: Tab,
    /// Installed OCR languages, e.g. "rus+eng".
    languages: String,
    downloading: bool,
    /// (description, keys) of kadr's Hyprland bindings.
    bindings: Vec<(String, String)>,
    focus: FocusHandle,
}

pub fn open(cx: &mut App) {
    let Some(mon) = util::focused_monitor() else { return };
    let spec = LayerSpec {
        namespace: "kadr-settings",
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
            SettingsView { settings: Settings::load(), tab: Tab::General, languages: ocr::languages(), downloading: false, bindings: kadr_bindings(), focus }
        })
    });
}

/// kadr's lines from Omarchy's keybinding list.
fn kadr_bindings() -> Vec<(String, String)> {
    let out = std::process::Command::new("omarchy")
        .args(["menu", "keybindings", "--print"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    out.lines()
        .filter(|l| l.contains("(kadr)"))
        .filter_map(|l| l.split_once('→'))
        .map(|(keys, desc)| (desc.trim().trim_end_matches("(kadr)").trim().to_owned(), keys.split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect()
}

impl SettingsView {
    fn update(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Settings)) {
        change(&mut self.settings);
        self.settings.save();
        cx.notify();
    }

    fn pick_folder(&mut self, screenshots: bool, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Выбрать".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = answer.await
                && let Some(dir) = paths.pop()
            {
                let _ = this.update(cx, |view, cx| {
                    view.update(cx, |s| {
                        if screenshots {
                            s.screenshot_dir = Some(dir);
                        } else {
                            s.video_dir = Some(dir);
                        }
                    })
                });
            }
        })
        .detach();
    }

    fn download_russian(&mut self, cx: &mut Context<Self>) {
        self.downloading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = ocr::download_russian().await;
            let _ = this.update(cx, |view, cx| {
                view.downloading = false;
                view.languages = ocr::languages();
                if let Err(e) = result {
                    util::notify("kadr", &e);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        h_flex().justify_center().gap_1().px_2().pt_2().pb_1().children(Tab::ALL.into_iter().map(|tab| {
            let on = tab == self.tab;
            v_flex()
                .id(SharedString::from(format!("tab-{}", tab.label())))
                .w(px(96. * kit::SCALE))
                .py_1p5()
                .gap_1()
                .items_center()
                .rounded_lg()
                .cursor_pointer()
                .when(on, |d| d.bg(colors.element_selected))
                .hover(|d| d.bg(colors.ghost_element_hover))
                .child(tab.icon().size(IconSize::Medium).color(if on { Color::Accent } else { Color::Muted }))
                .child(Label::new(tab.label()).size(LabelSize::Small).color(if on { Color::Default } else { Color::Muted }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = tab;
                    cx.notify();
                }))
        }))
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = self.settings.clone();
        let line = cx.theme().colors().border_variant;
        let rows: Vec<AnyElement> = match self.tab {
            Tab::General => vec![
                row(line, "Показывать карточку", "Миниатюра в углу экрана после снимка или записи", switch("show_card", s.show_card, cx, |s, on| s.show_card = on)),
                row(line, "Копировать скриншот", "Сразу класть снимок в буфер обмена", switch("copy", s.copy_screenshot, cx, |s, on| s.copy_screenshot = on)),
                row(
                    line,
                    "Карточка исчезает через",
                    "Пока на ней курсор, она не пропадает",
                    chips(cx, "secs", CARD_SECONDS.iter().map(|&n| if n == 0 { "Никогда".into() } else { format!("{n} с") }).collect(), CARD_SECONDS.iter().position(|&n| n == s.card_seconds), |s, i| s.card_seconds = CARD_SECONDS[i]),
                ),
                row(line, "Где показывать", "Угол экрана для карточек", chips(cx, "side", vec!["Слева".into(), "Справа".into()], Some((s.card_side == Side::Right) as usize), |s, i| s.card_side = if i == 1 { Side::Right } else { Side::Left })),
                row(
                    line,
                    "Сколько карточек сразу",
                    "Старые уходят, когда приходят новые",
                    chips(cx, "max", MAX_CARDS.iter().map(|n| n.to_string()).collect(), MAX_CARDS.iter().position(|&n| n == s.max_cards), |s, i| s.max_cards = MAX_CARDS[i]),
                ),
            ],
            Tab::Screenshots => vec![row(line, "Папка для скриншотов", "По умолчанию та же, что у Omarchy", folder(cx, s.screenshot_dir(), s.screenshot_dir.is_some(), true))],
            Tab::Recording => vec![
                row(line, "Папка для записей", "По умолчанию «Видео»", folder(cx, s.video_dir(), s.video_dir.is_some(), false)),
                row(line, "Частота кадров", "60 — плавнее, 30 — файлы меньше", chips(cx, "fps", FPS.iter().map(|f| format!("{f} к/с")).collect(), FPS.iter().position(|&f| f == s.fps), |s, i| s.fps = FPS[i])),
                row(line, "Качество", "Выше — чётче мелкий текст, но файлы больше", chips(cx, "quality", QUALITIES.iter().map(|q| q.1.to_string()).collect(), Some(s.quality), |s, i| s.quality = i)),
                row(line, "Рамка вокруг области", "Подсвечивать записываемую область (в запись не попадает)", switch("frame", s.region_frame, cx, |s, on| s.region_frame = on)),
            ],
            Tab::Studio => {
                let ring = cx.theme().colors().text;
                let swatches = h_flex().gap_1().children(palette::backdrops().into_iter().enumerate().map(|(i, (c0, c1))| {
                    div()
                        .id(("bd", i))
                        .size(px(26.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(7.))
                        .border_2()
                        .border_color(if i == s.studio_backdrop { ring } else { gpui::transparent_black() })
                        .cursor_pointer()
                        .child(div().size(px(18.)).rounded(px(5.)).bg(linear_gradient(135., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.))))
                        .on_click(cx.listener(move |this, _, _, cx| this.update(cx, |s| s.studio_backdrop = i)))
                }));
                vec![
                    row(line, "Зум по умолчанию", "Для зумов, расставленных по кликам", chips(cx, "zoom", ZOOMS.iter().map(|z| format!("×{z}")).collect(), ZOOMS.iter().position(|&z| (z - s.studio_zoom).abs() < 0.01), |s, i| s.studio_zoom = ZOOMS[i])),
                    row(line, "Фон по умолчанию", "Подложка вокруг видео", swatches.into_any_element()),
                    row(line, "Отступ по умолчанию", "Сколько фона видно вокруг видео", chips(cx, "pad", vec!["Нет".into(), "S".into(), "M".into(), "L".into()], Some(s.studio_padding.min(PADDINGS.len() - 1)), |s, i| s.studio_padding = i)),
                ]
            }
            Tab::Text => {
                let has_rus = self.languages.split('+').any(|l| l == "rus");
                let action = if has_rus {
                    Label::new("Установлен").color(Color::Muted).into_any_element()
                } else {
                    Button::new("rus", if self.downloading { "Скачиваю…" } else { "Скачать (15 МБ)" })
                        .style(ButtonStyle::Filled)
                        .disabled(self.downloading)
                        .on_click(cx.listener(|this, _, _, cx| this.download_russian(cx)))
                        .into_any_element()
                };
                vec![
                    row(line, "Языки распознавания", "Tesseract читает все сразу", Label::new(self.languages.replace('+', ", ")).into_any_element()),
                    row(line, "Русский язык", "Официальная модель Tesseract, ставится в папку kadr без sudo", action),
                ]
            }
            Tab::Shortcuts => {
                let mut rows: Vec<AnyElement> = self
                    .bindings
                    .iter()
                    .map(|(desc, keys)| row(line, desc, "", div().font_family(kit::MONO_FONT).text_sm().child(keys.clone()).into_any_element()))
                    .collect();
                rows.push(row(
                    line,
                    "Изменить сочетания",
                    "Сочетания задаются в конфиге Hyprland",
                    Button::new("bindings", "Открыть bindings.lua")
                        .style(ButtonStyle::Subtle)
                        .on_click(|_, _, _| {
                            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
                            util::open_file(&home.join(".config/hypr/bindings.lua"));
                        })
                        .into_any_element(),
                ));
                rows
            }
            Tab::About => vec![
                row(line, "kadr", "Скриншоты и запись экрана для Omarchy", Label::new(format!("версия {}", env!("CARGO_PKG_VERSION"))).color(Color::Muted).into_any_element()),
                row(line, "Интерфейс", "GPUI и компоненты Zed v1.24.2, цвета из темы Omarchy", Label::new("Zed UI").color(Color::Muted).into_any_element()),
                row(
                    line,
                    "Исходный код",
                    "github.com/vlasmn/kadr-linux",
                    Button::new("repo", "Открыть").style(ButtonStyle::Subtle).on_click(|_, _, _| util::open_file(std::path::Path::new("https://github.com/vlasmn/kadr-linux"))).into_any_element(),
                ),
                row(
                    line,
                    "Папка настроек",
                    "~/.config/kadr",
                    Button::new("config", "Открыть").style(ButtonStyle::Subtle).on_click(|_, _, _| util::open_file(&util::config_dir())).into_any_element(),
                ),
            ],
        };
        v_flex().px_6().py_2().children(rows).into_any_element()
    }
}

/// Title and description on the left, the control on the right.
fn row(line: Hsla, title: &str, description: &str, control: AnyElement) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .gap_6()
        .py_3()
        .border_b_1()
        .border_color(line)
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(Label::new(title.to_owned()))
                .when(!description.is_empty(), |c| c.child(Label::new(description.to_owned()).size(LabelSize::Small).color(Color::Muted))),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

fn switch(id: &'static str, on: bool, cx: &mut Context<SettingsView>, set: fn(&mut Settings, bool)) -> AnyElement {
    Switch::new(id, ToggleState::from(on))
        .on_click(cx.listener(move |this, state: &ToggleState, _, cx| {
            let on = *state == ToggleState::Selected;
            this.update(cx, |s| set(s, on))
        }))
        .into_any_element()
}

fn chips(cx: &mut Context<SettingsView>, prefix: &'static str, labels: Vec<String>, selected: Option<usize>, pick: fn(&mut Settings, usize)) -> AnyElement {
    h_flex()
        .gap_0p5()
        .children(labels.into_iter().enumerate().map(|(i, label)| {
            ButtonLike::new(SharedString::from(format!("{prefix}-{i}")))
                .style(if selected == Some(i) { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                .toggle_state(selected == Some(i))
                .child(div().px_2().child(Label::new(label).size(LabelSize::Small)))
                .on_click(cx.listener(move |this, _, _, cx| this.update(cx, |s| pick(s, i))))
        }))
        .into_any_element()
}

fn folder(cx: &mut Context<SettingsView>, dir: PathBuf, custom: bool, screenshots: bool) -> AnyElement {
    let home = std::env::var("HOME").unwrap_or_default();
    let shown = dir.display().to_string().replacen(&home, "~", 1);
    h_flex()
        .gap_1()
        .child(div().max_w(px(260. * kit::SCALE)).child(Label::new(shown).size(LabelSize::Small).color(Color::Muted).truncate()))
        .child(Button::new(if screenshots { "pick-shots" } else { "pick-videos" }, "Выбрать…").style(ButtonStyle::Subtle).on_click(cx.listener(move |this, _, _, cx| this.pick_folder(screenshots, cx))))
        .when(custom, |row| {
            row.child(
                IconButton::new(if screenshots { "reset-shots" } else { "reset-videos" }, IconName::Close)
                    .tooltip(Tooltip::text("По умолчанию"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update(cx, |s| {
                            if screenshots {
                                s.screenshot_dir = None;
                            } else {
                                s.video_dir = None;
                            }
                        })
                    })),
            )
        })
        .into_any_element()
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let divider = cx.theme().colors().border_variant;
        kit::root(cx)
            .bg(rgba(0x00000099))
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let i = Tab::ALL.iter().position(|t| *t == this.tab).unwrap_or(0);
                match e.keystroke.key.as_str() {
                    "escape" => window.remove_window(),
                    "tab" | "right" => this.tab = Tab::ALL[(i + 1) % Tab::ALL.len()],
                    "left" => this.tab = Tab::ALL[(i + Tab::ALL.len() - 1) % Tab::ALL.len()],
                    _ => {}
                }
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.remove_window())
            .child(
                v_flex()
                    .id("settings")
                    .w(px(820. * kit::SCALE))
                    .h(px(560. * kit::SCALE))
                    .elevation_3(cx)
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(self.render_tabs(cx))
                    .child(div().h(px(1.)).bg(divider))
                    .child(div().id("body").flex_1().min_h_0().overflow_y_scroll().child(self.render_body(cx)))
                    .child(Divider::horizontal())
                    .child(
                        h_flex()
                            .justify_between()
                            .px_3()
                            .py_1p5()
                            .child(Label::new("Изменения сохраняются сразу").size(LabelSize::XSmall).color(Color::Muted))
                            .child(h_flex().gap_1().child(kit::keys("escape")).child(Label::new("Закрыть").size(LabelSize::XSmall).color(Color::Muted))),
                    ),
            )
    }
}
