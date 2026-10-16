//! Settings, laid out like System Settings: sections in a list on the left,
//! the chosen one's rows on the right. Every change is saved on the spot.

use std::path::PathBuf;

use gpui::{
    App, Context, FocusHandle, KeyDownEvent, PathPromptOptions, Window, div,
    linear_color_stop, linear_gradient, px,
};
use crate::ui::{ButtonLike, Switch, ToggleState, Tooltip, prelude::*};

use crate::kit;
use crate::settings::{CARD_SECONDS, FPS, MAX_CARDS, QUALITIES, Settings, Side};
use crate::studio::{PADDINGS, ZOOMS};
use crate::{hotkeys, ocr, palette, storage, tray, util};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Screenshots,
    Recording,
    Studio,
    Text,
    Shortcuts,
    Storage,
    About,
}

impl Tab {
    const ALL: [Tab; 8] = [Tab::General, Tab::Screenshots, Tab::Recording, Tab::Studio, Tab::Text, Tab::Shortcuts, Tab::Storage, Tab::About];

    fn label(self) -> &'static str {
        match self {
            Tab::General => "Основные",
            Tab::Screenshots => "Скриншоты",
            Tab::Recording => "Запись",
            Tab::Studio => "Студия",
            Tab::Text => "Текст",
            Tab::Shortcuts => "Клавиши",
            Tab::Storage => "Хранилище",
            Tab::About => "О kadr",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Tab::General => Icon::new(IconName::Settings),
            Tab::Screenshots => Icon::new(IconName::Image),
            Tab::Recording => Icon::new(IconName::Circle),
            Tab::Studio => Icon::new(IconName::Sparkle),
            Tab::Text => Icon::new(IconName::CaseSensitive),
            Tab::Shortcuts => Icon::new(IconName::Keyboard),
            Tab::Storage => Icon::new(IconName::Folder),
            Tab::About => Icon::from_path("icons/kadr/camera.svg"),
        }
    }
}

pub struct SettingsView {
    settings: Settings,
    tab: Tab,
    /// Installed OCR languages, e.g. "rus+eng".
    languages: String,
    downloading: bool,
    /// Hyprland key specs for each of `hotkeys::ACTIONS`.
    keys: Vec<Vec<String>>,
    /// The action a new shortcut is being recorded for; binds are paused.
    recording: Option<usize>,
    /// Space kadr's captures take, measured when the window opens.
    usage: storage::Usage,
    focus: FocusHandle,
}

pub fn open(cx: &mut App) {
    // Already open: bring it forward instead of a second one.
    if let Some(window) = cx.windows().into_iter().find_map(|w| w.downcast::<SettingsView>()) {
        let _ = window.update(cx, |_, window, _| window.activate());
        return;
    }
    let mon = util::focused_monitor();
    kit::open_window(cx, "kadr-settings", mon.as_ref(), (820. * kit::SCALE, 560. * kit::SCALE), |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            SettingsView { settings: Settings::load(), tab: Tab::General, languages: ocr::languages(), downloading: false, keys: hotkeys::load(), recording: None, usage: storage::usage(&Settings::load()), focus }
        })
    });
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

    /// The section list on the left, like System Settings: the current one
    /// highlighted in the accent colour.
    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let accent = colors.text_accent;
        let on_accent = crate::ui::on(accent);
        v_flex()
            .w(px(210. * kit::SCALE))
            .h_full()
            .flex_none()
            .p_2()
            .gap_0p5()
            .bg(colors.editor_background)
            // The window clips to a rectangle; the sidebar rounds its own corners.
            .rounded_l(crate::ui::rems_from_px(11.))
            .rounded_smoothing(crate::ui::SMOOTHING)
            .child(div().px_2().pt_1().pb_2().child(Label::new("Настройки").size(LabelSize::Small).color(Color::Muted)))
            .children(Tab::ALL.into_iter().map(|tab| {
                let on = tab == self.tab;
                h_flex()
                    .id(SharedString::from(format!("tab-{}", tab.label())))
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded(crate::ui::rems_from_px(7.))
                    .rounded_smoothing(crate::ui::SMOOTHING)
                    .cursor_pointer()
                    .when(on, |d| d.bg(accent))
                    .when(!on, |d| d.hover(|d| d.bg(colors.ghost_element_hover)))
                    .child(tab.icon().size(IconSize::Small).color(if on { Color::Custom(on_accent) } else { Color::Muted }))
                    .child(Label::new(tab.label()).color(if on { Color::Custom(on_accent) } else { Color::Default }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = tab;
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(
                v_flex()
                    .px_2()
                    .pb_1()
                    .gap_1()
                    .child(Label::new("Изменения сохраняются сразу").size(LabelSize::XSmall).color(Color::Muted))
                    .child(h_flex().gap_1().child(kit::keys("escape")).child(Label::new("Закрыть").size(LabelSize::XSmall).color(Color::Muted))),
            )
    }

    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = self.settings.clone();
        let rows: Vec<AnyElement> = match self.tab {
            Tab::General => vec![
                row("Показывать карточку", "Миниатюра в углу экрана после снимка или записи", switch("show_card", s.show_card, cx, |s, on| s.show_card = on)),
                row("Копировать скриншот", "Сразу класть снимок в буфер обмена", switch("copy", s.copy_screenshot, cx, |s, on| s.copy_screenshot = on)),
                row("Иконка в трее",
                    "Клик открывает настройки, правый клик — снимки и запись",
                    switch("tray", s.tray, cx, |s, on| {
                        s.tray = on;
                        // Saved first: the new tray process reads the setting.
                        s.save();
                        if on { tray::start() } else { tray::stop() }
                    }),
                ),
                row("Карточка исчезает через",
                    "Пока на ней курсор, она не пропадает",
                    chips(cx, "secs", CARD_SECONDS.iter().map(|&n| if n == 0 { "Никогда".into() } else { format!("{n} с") }).collect(), CARD_SECONDS.iter().position(|&n| n == s.card_seconds), |s, i| s.card_seconds = CARD_SECONDS[i]),
                ),
                row("Где показывать", "Угол экрана для карточек", chips(cx, "side", vec!["Слева".into(), "Справа".into()], Some((s.card_side == Side::Right) as usize), |s, i| s.card_side = if i == 1 { Side::Right } else { Side::Left })),
                row("Сколько карточек сразу",
                    "Старые уходят, когда приходят новые",
                    chips(cx, "max", MAX_CARDS.iter().map(|n| n.to_string()).collect(), MAX_CARDS.iter().position(|&n| n == s.max_cards), |s, i| s.max_cards = MAX_CARDS[i]),
                ),
            ],
            Tab::Screenshots => {
                let mut rows = vec![row("Папка для скриншотов", "По умолчанию та же, что у Omarchy", folder(cx, s.screenshot_dir(), s.screenshot_dir.is_some(), true))];
                // A choice only where windows tile; elsewhere it's a window.
                if cfg!(target_os = "linux") {
                    rows.push(row(
                        "Редактор",
                        "Окно не мешает снимать другие экраны, а поверх всего закрывает экран целиком",
                        chips(cx, "editor", vec!["Окно".into(), "Поверх всего".into()], Some((!s.editor_window) as usize), |s, i| s.editor_window = i == 0),
                    ));
                }
                rows
            }
            Tab::Recording => vec![
                row("Папка для записей", "По умолчанию «Видео»", folder(cx, s.video_dir(), s.video_dir.is_some(), false)),
                row("Частота кадров", "60 — плавнее, 30 — файлы меньше", chips(cx, "fps", FPS.iter().map(|f| format!("{f} к/с")).collect(), FPS.iter().position(|&f| f == s.fps), |s, i| s.fps = FPS[i])),
                row("Качество", "Выше — чётче мелкий текст, но файлы больше", chips(cx, "quality", QUALITIES.iter().map(|q| q.1.to_string()).collect(), Some(s.quality), |s, i| s.quality = i)),
                row("Рамка вокруг области", "Подсвечивать записываемую область (в запись не попадает)", switch("frame", s.region_frame, cx, |s, on| s.region_frame = on)),
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
                    row("Зум по умолчанию", "Для зумов, расставленных по кликам", chips(cx, "zoom", ZOOMS.iter().map(|z| format!("×{z}")).collect(), ZOOMS.iter().position(|&z| (z - s.studio_zoom).abs() < 0.01), |s, i| s.studio_zoom = ZOOMS[i])),
                    row("Фон по умолчанию", "Подложка вокруг видео", swatches.into_any_element()),
                    row("Отступ по умолчанию", "Сколько фона видно вокруг видео", chips(cx, "pad", vec!["Нет".into(), "S".into(), "M".into(), "L".into()], Some(s.studio_padding.min(PADDINGS.len() - 1)), |s, i| s.studio_padding = i)),
                ]
            }
            Tab::Text => {
                let has_rus = self.languages.split('+').any(|l| l == "rus");
                let action = if has_rus {
                    Label::new("Установлен").color(Color::Muted).into_any_element()
                } else {
                    Button::new("rus", if self.downloading { "Скачиваю…" } else { "Скачать (15 МБ)" })
                        .style(ButtonStyle::Accent)
                        .disabled(self.downloading)
                        .on_click(cx.listener(|this, _, _, cx| this.download_russian(cx)))
                        .into_any_element()
                };
                vec![
                    row("Языки распознавания", "Tesseract читает все сразу", Label::new(self.languages.replace('+', ", ")).into_any_element()),
                    row("Русский язык", "Официальная модель Tesseract, ставится в папку kadr без sudo", action),
                ]
            }
            Tab::Shortcuts if cfg!(target_os = "linux") => hotkeys::ACTIONS.iter().enumerate().map(|(i, action)| row(action.title, "", self.render_keys(i, cx))).collect(),
            Tab::Shortcuts => vec![row("Сочетания клавиш", "Пока настраиваются только в Hyprland", div().into_any_element())],
            Tab::Storage => vec![
                self.render_usage(cx),
                row("Удалять старые снимки и записи",
                    "Насовсем, раз в день. Только файлы kadr, остальное в папках не трогается",
                    chips(cx, "cleanup", storage::CLEANUP_DAYS.iter().map(|&d| if d == 0 { "Никогда".into() } else { format!("{d} дн.") }).collect(), storage::CLEANUP_DAYS.iter().position(|&d| d == s.cleanup_days), |s, i| s.cleanup_days = storage::CLEANUP_DAYS[i]),
                ),
                row("Очистить сейчас",
                    if s.cleanup_days == 0 { "Выбери срок выше" } else { "Удалить то, что старше выбранного срока" },
                    Button::new("clean-now", "Очистить")
                        .style(ButtonStyle::Subtle)
                        .disabled(s.cleanup_days == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let freed = storage::clean(&this.settings, this.settings.cleanup_days);
                            this.usage = storage::usage(&this.settings);
                            util::notify("Хранилище очищено", &format!("Освобождено {} ({} файлов)", crate::studio::human_size(freed.bytes as f64), freed.files));
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
            ],
            Tab::About => vec![
                row("kadr", "Скриншоты и запись экрана", Label::new(format!("версия {}", env!("CARGO_PKG_VERSION"))).color(Color::Muted).into_any_element()),
                row("Интерфейс", "GPUI CE и свои компоненты, цвета из темы Omarchy", Label::new("GPUI CE").color(Color::Muted).into_any_element()),
                row("Исходный код",
                    "github.com/vlasmn/kadr",
                    Button::new("repo", "Открыть").style(ButtonStyle::Subtle).on_click(|_, _, _| util::open_file(std::path::Path::new("https://github.com/vlasmn/kadr"))).into_any_element(),
                ),
                row("Папка настроек",
                    "~/.config/kadr",
                    Button::new("config", "Открыть").style(ButtonStyle::Subtle).on_click(|_, _, _| util::open_file(&util::config_dir())).into_any_element(),
                ),
            ],
        };
        // An inset group, like System Settings: rows on a soft rounded card,
        // hairlines between them.
        let colors = cx.theme().colors();
        let count = rows.len();
        let group = rows.into_iter().enumerate().flat_map(|(i, row)| {
            let line = (i + 1 < count).then(|| div().h(px(1.)).bg(colors.border_variant).into_any_element());
            std::iter::once(row).chain(line)
        });
        v_flex()
            .px_6()
            .py_4()
            .child(v_flex().px_4().rounded(crate::ui::rems_from_px(12.)).rounded_smoothing(crate::ui::SMOOTHING).bg(colors.element_background).children(group.collect::<Vec<_>>()))
            .into_any_element()
    }
}

/// Title and description on the left, the control on the right.
fn row(title: &str, description: &str, control: AnyElement) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .gap_6()
        .py_3()
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(div().whitespace_nowrap().child(Label::new(title.to_owned())))
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

impl SettingsView {
    /// Total size and a bar split into screenshots, recordings and cache.
    fn render_usage(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let u = self.usage;
        let total = u.total().max(1) as f32;
        let parts = [
            ("Скриншоты", u.screenshots, colors.text_accent),
            ("Записи", u.recordings, cx.theme().status().warning),
            ("Кэш студии", u.cache, colors.text_muted),
        ];
        let size = |b: u64| crate::studio::human_size(b as f64);
        let bar = h_flex()
            .w_full()
            .h(px(8.))
            .rounded_full()
            .overflow_hidden()
            .bg(colors.element_background)
            .children(parts.iter().filter(|p| p.1.bytes > 0).map(|&(_, part, color)| div().h_full().w(gpui::relative(part.bytes as f32 / total)).bg(color)));
        let legend = h_flex().gap_4().children(parts.iter().map(|&(name, part, color)| {
            h_flex()
                .gap_1p5()
                .child(div().size(px(8.)).rounded_full().bg(color))
                .child(Label::new(format!("{name} {}", size(part.bytes))).size(LabelSize::Small))
                .child(Label::new(format!("· {}", part.files)).size(LabelSize::Small).color(Color::Muted))
        }));
        v_flex()
            .w_full()
            .gap_2()
            .py_3()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Занято"))
                    .child(div().text_size(crate::ui::rems_from_px(18.)).font_weight(gpui::FontWeight::SEMIBOLD).child(size(u.total()))),
            )
            .child(bar)
            .child(legend)
            .into_any_element()
    }

    /// An action's shortcuts: click one to remove it, "+" records another.
    fn render_keys(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let chips = self.keys[i].iter().enumerate().map(|(k, spec)| {
            ButtonLike::new(SharedString::from(format!("key-{i}-{k}")))
                .style(ButtonStyle::Filled)
                .child(div().px_1().font_family(kit::MONO_FONT).text_sm().child(hotkeys::label(spec)))
                .tooltip(Tooltip::text("Убрать сочетание"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.keys[i].remove(k);
                    this.save_keys();
                    cx.notify();
                }))
                .into_any_element()
        });
        let add = if self.recording == Some(i) {
            Label::new("Нажми сочетание… (Esc — отмена)").size(LabelSize::Small).color(Color::Accent).into_any_element()
        } else {
            IconButton::new(SharedString::from(format!("add-key-{i}")), IconName::Plus)
                .tooltip(Tooltip::text("Добавить сочетание"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.recording.is_none() {
                        hotkeys::quiet(true);
                    }
                    this.recording = Some(i);
                    cx.notify();
                }))
                .into_any_element()
        };
        h_flex().gap_1().children(chips).child(add).into_any_element()
    }

    fn record_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(i) = self.recording else { return };
        let k = &e.keystroke;
        let m = &k.modifiers;
        if k.key == "escape" && !(m.control || m.alt || m.shift || m.platform) {
            self.stop_recording();
            return cx.notify();
        }
        // Modifiers alone: wait for the key.
        let Some(spec) = hotkeys::spec(&k.key, m.control, m.alt, m.shift, m.platform) else { return };
        // A shortcut belongs to one action.
        for keys in &mut self.keys {
            keys.retain(|s| *s != spec);
        }
        self.keys[i].push(spec);
        self.stop_recording();
        self.save_keys();
        cx.notify();
    }

    fn stop_recording(&mut self) {
        if self.recording.take().is_some() {
            hotkeys::quiet(false);
        }
    }

    fn save_keys(&self) {
        if let Err(e) = hotkeys::save(&self.keys) {
            util::notify("Не удалось сохранить сочетания", &e.to_string());
        }
    }
}

/// Closing mid-recording must not leave every bind paused.
impl Drop for SettingsView {
    fn drop(&mut self) {
        self.stop_recording();
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        kit::root(cx)
            .bg(cx.theme().colors().elevated_surface_background)
            .flex()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if this.recording.is_some() {
                    return this.record_key(e, cx);
                }
                let i = Tab::ALL.iter().position(|t| *t == this.tab).unwrap_or(0);
                match e.keystroke.key.as_str() {
                    "escape" => window.remove_window(),
                    "tab" | "down" => this.tab = Tab::ALL[(i + 1) % Tab::ALL.len()],
                    "up" => this.tab = Tab::ALL[(i + Tab::ALL.len() - 1) % Tab::ALL.len()],
                    _ => {}
                }
                cx.notify();
            }))
            .child(
                v_flex()
                    .id("settings")
                    .size_full()
                    .child(
                        h_flex()
                            .size_full()
                            .child(self.render_sidebar(cx))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .child(div().px_6().pt_5().pb_1().text_size(crate::ui::rems_from_px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child(self.tab.label()))
                                    .child(div().id("body").flex_1().min_h_0().overflow_y_scroll().child(self.render_body(cx))),
                            ),
                    ),
            )
    }
}
