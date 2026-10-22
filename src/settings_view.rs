//! Settings, laid out like System Settings: sections in a list on the left,
//! the chosen one's rows on the right. Every change is saved on the spot.

use std::path::PathBuf;

use gpui::{
    App, Context, FocusHandle, KeyDownEvent, PathPromptOptions, Window, div,
    linear_color_stop, linear_gradient, px,
};
use crate::ui::{ButtonLike, Switch, ToggleState, Tooltip, prelude::*};

use crate::kit;
use crate::settings::{CARD_SECONDS, FPS, MAX_CARDS, QUALITIES, Settings, Side, ThemeMode};
use crate::studio::{PADDINGS, ZOOMS};
use crate::{hotkeys, ocr, palette, storage, tray, util};
use crate::i18n::t;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
            Tab::General => t("settings.general"),
            Tab::Screenshots => t("common.screenshots"),
            Tab::Recording => t("common.recording"),
            Tab::Studio => t("common.studio"),
            Tab::Text => t("common.text"),
            Tab::Shortcuts => t("settings.shortcuts"),
            Tab::Storage => t("settings.storage"),
            Tab::About => t("settings.about_kadr"),
        }
    }

    fn icon(self) -> Icon {
        match self {
            Tab::General => Icon::new(IconName::Settings),
            Tab::Screenshots => Icon::new(IconName::Image),
            Tab::Recording => Icon::from_path("icons/kadr/record.svg"),
            Tab::Studio => Icon::from_path("icons/kadr/studio.svg"),
            Tab::Text => Icon::new(IconName::CaseSensitive),
            Tab::Shortcuts => Icon::new(IconName::Keyboard),
            Tab::Storage => Icon::from_path("icons/kadr/storage.svg"),
            Tab::About => Icon::from_path("icons/kadr/info.svg"),
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
    /// The language dropdown is open.
    languages_open: bool,
    languages_menu: crate::ui::MenuBounds,
    /// Space kadr's captures take, measured when the window opens.
    usage: storage::Usage,
    /// Type-to-search over every section, like System Settings.
    query: String,
    /// Typing goes into the file name template instead of the search.
    naming: bool,
    scroll: gpui::ScrollHandle,
    grab: std::rc::Rc<std::cell::Cell<Option<f32>>>,
    focus: FocusHandle,
}

pub fn open(cx: &mut App) {
    // Already open: bring it forward instead of a second one.
    if let Some(window) = cx.windows().into_iter().find_map(|w| w.downcast::<SettingsView>()) {
        let _ = window.update(cx, |_, window, _| window.activate());
        return;
    }
    let mon = util::focused_monitor();
    kit::open_window(cx, "kadr-settings", mon.as_ref(), (820. * kit::SCALE, 560. * kit::SCALE), (640., 420.), |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            SettingsView { settings: Settings::load(), tab: Tab::General, languages: ocr::languages(), downloading: false, keys: hotkeys::load(), recording: None, languages_open: false, languages_menu: Default::default(), usage: storage::usage(&Settings::load()), query: String::new(), naming: false, scroll: gpui::ScrollHandle::new(), grab: Default::default(), focus }
        })
    });
}

impl SettingsView {
    /// The language as a dropdown: there will be more than fit in a row.
    fn render_languages(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.settings.language;
        let menu = self.languages_open.then(|| {
            gpui::deferred(
                v_flex()
                    .id("languages")
                    .occlude()
                    .absolute()
                    .top(px(30.))
                    .right_0()
                    .min_w(px(160.))
                    .p_1()
                    .gap_0p5()
                    .elevation_3(cx)
                    .children(crate::i18n::Lang::ALL.into_iter().map(|lang| {
                        h_flex()
                            .id(SharedString::from(format!("lang-{}", lang.code())))
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(crate::ui::rems_from_px(6.))
                            .cursor_pointer()
                            .hover(|d| d.bg(cx.theme().colors().element_hover))
                            .child(Icon::new(IconName::Check).size(IconSize::Small).color(if lang == current { Color::Accent } else { Color::Custom(gpui::transparent_black()) }))
                            .child(Label::new(lang.name()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.languages_open = false;
                                this.settings.language = lang;
                                this.settings.save();
                                crate::i18n::set(lang);
                                cx.refresh_windows();
                                cx.notify();
                            }))
                    }))
                    .child(crate::ui::track_bounds(&self.languages_menu)),
            )
        });
        div()
            .relative()
            .on_mouse_down_out(cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                if this.languages_open && !this.languages_menu.get().contains(&e.position) {
                    this.languages_open = false;
                    cx.notify();
                }
            }))
            .child(
                ButtonLike::new("language")
                    .style(ButtonStyle::Filled)
                    .toggle_state(self.languages_open)
                    .child(h_flex().gap_1p5().px_2().child(Label::new(current.name()).size(LabelSize::Small)).child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.languages_open = !this.languages_open;
                        cx.notify();
                    })),
            )
            .children(menu)
            .into_any_element()
    }

    fn update(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Settings)) {
        change(&mut self.settings);
        self.settings.save();
        cx.notify();
    }

    fn pick_folder(&mut self, screenshots: bool, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(t("settings.choose").into()) });
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
            // Corners inside are the window's less this inset: concentric.
            .p(px(kit::WINDOW_RADIUS - 7.))
            .gap_0p5()
            // The same dark as the page, set apart by a hairline, like the
            // editors' side panels.
            .border_r_1()
            .border_color(colors.border_variant)
            // Under macOS's traffic lights.
            .when(matches!(crate::platform::window_controls(), crate::platform::WindowControls::Left(_)), |d| d.pt(px(30.)))
            .child(
                h_flex()
                    .mb_2()
                    .h(px(30.))
                    .px_2()
                    .gap_1p5()
                    .rounded(px(7.))
                    .rounded_smoothing(crate::ui::SMOOTHING)
                    .bg(colors.element_background)
                    .child(Icon::new(IconName::MagnifyingGlass).size(IconSize::Small).color(Color::Muted))
                    .child(if self.query.is_empty() {
                        div().text_color(colors.text_placeholder).child(t("settings.search")).into_any_element()
                    } else {
                        div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(format!("{}▏", self.query)).into_any_element()
                    }),
            )
            .children(Tab::ALL.into_iter().map(|tab| {
                let on = tab == self.tab && self.query.is_empty();
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
                        this.query.clear();
                        this.scroll.set_offset(gpui::point(px(0.), px(0.)));
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(
                v_flex()
                    .px_2()
                    .pb_1()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(Icon::new(IconName::Check).size(IconSize::XSmall).color(Color::Muted))
                            .child(Label::new(t("settings.changes_saved_right_away")).size(LabelSize::Small).color(Color::Muted)),
                    )
            )
    }

    /// A section's rows, each with its text for the search.
    fn tab_rows(&self, tab: Tab, cx: &mut Context<Self>) -> Vec<(String, AnyElement)> {
        let s = self.settings.clone();
        match tab {
            Tab::General => vec![
                row(t("settings.language"),
                    "",
                    self.render_languages(cx),
                ),
                row(t("common.appearance"),
                    t("settings.like_system_always_dark_light"),
                    {
                        let modes = [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light];
                        h_flex()
                            .gap_0p5()
                            .children(modes.into_iter().zip([t("settings.system"), t("settings.dark"), t("settings.light")]).map(|(mode, label)| {
                                let on = s.theme == mode;
                                ButtonLike::new(SharedString::from(format!("theme-{label}")))
                                    .style(if on { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                                    .toggle_state(on)
                                    .child(div().px_1p5().child(Label::new(label).size(LabelSize::Small)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.settings.theme = mode;
                                        this.settings.save();
                                        palette::refresh(cx);
                                        cx.notify();
                                    }))
                            }))
                            .into_any_element()
                    },
                ),
                row(t("common.tray_icon"),
                    t("settings.click_opens_settings_right_click"),
                    switch("tray", s.tray, cx, |s, on| {
                        s.tray = on;
                        // Saved first: the new tray process reads the setting.
                        s.save();
                        // Started over either way: where it also holds the
                        // hotkeys, it keeps running without the icon.
                        tray::restart();
                    }),
                ),
                heading(t("settings.thumbnails")),
                row(t("settings.thumbnail_goes_away_after"),
                    t("settings.stays_while_pointer_over"),
                    chips(cx, "secs", CARD_SECONDS.iter().map(|&n| if n == 0 { t("settings.never").into() } else { tr!("common.seconds", n) }).collect(), CARD_SECONDS.iter().position(|&n| n == s.card_seconds), |s, i| s.card_seconds = CARD_SECONDS[i]),
                ),
                row(t("settings.where_show"), t("settings.screen_corner_for_thumbnails"), chips(cx, "side", vec![t("settings.left").into(), t("settings.right").into()], Some((s.card_side == Side::Right) as usize), |s, i| s.card_side = if i == 1 { Side::Right } else { Side::Left })),
                row(t("settings.thumbnails_at_once"),
                    t("settings.old_ones_leave_as_new"),
                    chips(cx, "max", MAX_CARDS.iter().map(|n| n.to_string()).collect(), MAX_CARDS.iter().position(|&n| n == s.max_cards), |s, i| s.max_cards = MAX_CARDS[i]),
                ),
            ],
            Tab::Screenshots => {
                let mut rows = vec![
                    heading(t("settings.after_capture")),
                    row(t("settings.show_thumbnail"), t("settings.thumbnail_in_screen_corner_after"), switch("shot-card", s.after_shot.card, cx, |s, on| s.after_shot.card = on)),
                    row(t("settings.copy_to_clipboard"), t("settings.put_screenshot_on_clipboard_right"), switch("shot-copy", s.after_shot.copy, cx, |s, on| s.after_shot.copy = on)),
                    row(t("settings.save_to_folder"), t("settings.off_goes_to_temporary_folder"), switch("shot-save", s.after_shot.save, cx, |s, on| s.after_shot.save = on)),
                    row(t("settings.open_editor"), t("settings.annotate_right_after_capture"), switch("shot-edit", s.after_shot.edit, cx, |s, on| s.after_shot.edit = on)),
                    row(t("card.pin_on_top"), t("settings.screenshot_floats_above_windows"), switch("shot-pin", s.after_shot.pin, cx, |s, on| s.after_shot.pin = on)),
                    heading(t("settings.files")),
                    row(t("settings.screenshots_folder"), t("settings.by_default_same_as_omarchys"), folder(cx, s.screenshot_dir(), s.screenshot_dir.is_some(), true)),
                    self.render_name(crate::storage::Capture::Screenshot, "png", cx),
                    row(t("settings.window_shadow"), t("settings.window_captures_keep_shadow"), switch("shadow", s.window_shadow, cx, |s, on| s.window_shadow = on)),
                    row(
                        t("settings.screenshot_size"),
                        t("settings.screenshot_size_tip"),
                        chips(cx, "shot-size", vec!["1×".into(), t("settings.screen_pixels").into()], Some((!s.logical_size) as usize), |s, i| s.logical_size = i == 0),
                    ),
                ];
                // A choice only where windows tile; elsewhere it's a window.
                if cfg!(target_os = "linux") {
                    rows.push(row(
                        t("settings.editor"),
                        t("settings.window_leaves_other_screens_free"),
                        chips(cx, "editor", vec![t("common.window").into(), t("settings.over_everything").into()], Some((!s.editor_window) as usize), |s, i| s.editor_window = i == 0),
                    ));
                }
                rows
            }
            Tab::Recording => vec![
                heading(t("settings.after_recording")),
                row(t("settings.show_thumbnail"), t("settings.thumbnail_in_screen_corner_after"), switch("video-card", s.after_video.card, cx, |s, on| s.after_video.card = on)),
                row(t("settings.copy_to_clipboard"), t("settings.copy_file_paste_into_chat"), switch("video-copy", s.after_video.copy, cx, |s, on| s.after_video.copy = on)),
                row(t("settings.save_to_folder"), t("settings.off_goes_to_temporary_folder"), switch("video-save", s.after_video.save, cx, |s, on| s.after_video.save = on)),
                row(t("settings.open_studio"), t("settings.studio_mode_opens_it_anyway"), switch("video-edit", s.after_video.edit, cx, |s, on| s.after_video.edit = on)),
                heading(t("settings.video")),
                row(t("settings.recordings_folder"), t("settings.videos_by_default"), folder(cx, s.video_dir(), s.video_dir.is_some(), false)),
                self.render_name(crate::storage::Capture::Recording, "mp4", cx),
                row(t("settings.frame_rate"), t("settings.frame_rate_hint"), chips(cx, "fps", FPS.iter().map(|&f| if f == 0 { t("settings.fps_source").to_string() } else { tr!("common.fps", f) }).collect(), FPS.iter().position(|&f| f == s.fps), |s, i| s.fps = FPS[i])),
                row(t("settings.quality"), t("settings.higher_keeps_small_text_sharp"), chips(cx, "quality", QUALITIES.iter().map(|q| t(q.1).to_string()).collect(), Some(s.quality), |s, i| s.quality = i)),
                row(t("settings.frame_around_area"), t("settings.outline_recorded_area_not_recorded"), switch("frame", s.region_frame, cx, |s, on| s.region_frame = on)),
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
                    row(t("settings.default_zoom"), t("settings.for_zooms_placed_on_clicks"), chips(cx, "zoom", ZOOMS.iter().map(|z| format!("×{z}")).collect(), ZOOMS.iter().position(|&z| (z - s.studio_zoom).abs() < 0.01), |s, i| s.studio_zoom = ZOOMS[i])),
                    row(t("settings.default_background"), t("settings.backdrop_around_video"), swatches.into_any_element()),
                    row(t("settings.default_padding"), t("settings.how_much_background_shows_around"), chips(cx, "pad", vec![t("settings.lang_none").into(), "S".into(), "M".into(), "L".into()], Some(s.studio_padding.min(PADDINGS.len() - 1)), |s, i| s.studio_padding = i)),
                ]
            }
            Tab::Text => {
                let has_rus = self.languages.split('+').any(|l| l == "rus");
                let action = if has_rus {
                    Label::new(t("settings.installed")).color(Color::Muted).into_any_element()
                } else {
                    Button::new("rus", if self.downloading { t("settings.downloading") } else { t("settings.download_15_mb") })
                        .style(ButtonStyle::Accent)
                        .disabled(self.downloading)
                        .on_click(cx.listener(|this, _, _, cx| this.download_russian(cx)))
                        .into_any_element()
                };
                vec![
                    row(t("settings.recognition_languages"), t("settings.tesseract_reads_them_all_at"), Label::new(self.languages.replace('+', ", ")).into_any_element()),
                    row(t("settings.russian"), t("settings.official_tesseract_model_installed_into"), action),
                ]
            }
            Tab::Shortcuts => hotkeys::ACTIONS.iter().enumerate().map(|(i, action)| row(t(action.title), "", self.render_keys(i, cx))).collect(),
            Tab::Storage => vec![
                (format!("{} {} {}", t("settings.used"), t("common.screenshots"), t("common.recordings")), self.render_usage(cx)),
                row(t("settings.delete_old_captures"),
                    t("settings.for_good_once_day_only"),
                    chips(cx, "cleanup", storage::CLEANUP_DAYS.iter().map(|&d| if d == 0 { t("settings.never").into() } else { tr!("settings.days", d) }).collect(), storage::CLEANUP_DAYS.iter().position(|&d| d == s.cleanup_days), |s, i| s.cleanup_days = storage::CLEANUP_DAYS[i]),
                ),
                row(t("settings.clear_now"),
                    if s.cleanup_days == 0 { t("settings.choose_period_above") } else { t("settings.delete_whats_older_than_chosen") },
                    Button::new("clean-now", t("settings.clear"))
                        .style(ButtonStyle::Subtle)
                        .disabled(s.cleanup_days == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let freed = storage::clean(&this.settings, this.settings.cleanup_days);
                            this.usage = storage::usage(&this.settings);
                            util::notify(t("settings.storage_cleared"), &tr!("settings.freed_files", crate::studio::human_size(freed.bytes as f64), freed.files));
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
            ],
            Tab::About => vec![
                row("kadr", t("settings.screenshots_screen_recording"), Label::new(tr!("settings.version", env!("CARGO_PKG_VERSION"))).color(Color::Muted).into_any_element()),
                row(t("settings.interface"), t("settings.interface_details"), Label::new("GPUI CE").color(Color::Muted).into_any_element()),
                row(t("settings.source_code"),
                    "",
                    div()
                        .id("repo")
                        .text_color(cx.theme().colors().text_accent)
                        .cursor_pointer()
                        .hover(|d| d.underline())
                        .child("github.com/vlasmn/kadr")
                        .on_click(|_, _, _| util::open_file(std::path::Path::new("https://github.com/vlasmn/kadr")))
                        .into_any_element(),
                ),
                row(t("settings.settings_folder"),
                    "~/.config/kadr",
                    Button::new("config", t("common.open")).style(ButtonStyle::Subtle).on_click(|_, _, _| util::open_file(&util::config_dir())).into_any_element(),
                ),
            ],
        }
    }

    /// The chosen section; while searching, the matching rows of every
    /// section under their names.
    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.query.to_lowercase();
        let searching = !query.is_empty();
        let tabs: Vec<Tab> = if searching { Tab::ALL.to_vec() } else { vec![self.tab] };
        let (line_color, group_bg) = (cx.theme().colors().border_variant, cx.theme().colors().element_background);
        let mut body = v_flex().px_6().pt_5().pb_4();
        let mut found = false;
        for tab in tabs {
            let whole = tab.label().to_lowercase().contains(&query);
            // Rows in groups, each under its heading; one group while searching.
            let mut groups: Vec<(Option<AnyElement>, Vec<AnyElement>)> = vec![(None, Vec::new())];
            for (text, el) in self.tab_rows(tab, cx) {
                if text.starts_with(HEADING) {
                    if !searching {
                        groups.push((Some(el), Vec::new()));
                    }
                } else if !searching || whole || text.to_lowercase().contains(&query) {
                    groups.last_mut().expect("a group").1.push(el);
                }
            }
            groups.retain(|g| !g.1.is_empty());
            if groups.is_empty() {
                continue;
            }
            // The heading scrolls with its rows.
            body = if searching {
                body.child(div().px_1().pt(px(if found { 18. } else { 0. })).pb_2().child(Label::new(tab.label()).size(LabelSize::Small).color(Color::Muted)))
            } else {
                body.child(div().pb_3().text_size(crate::ui::rems_from_px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child(tab.label()))
            };
            found = true;
            for (title, rows) in groups {
                if let Some(title) = title {
                    body = body.child(div().px_1().pt_5().pb_2().child(title));
                }
                // An inset group, like System Settings: rows on a soft rounded
                // card, hairlines between them.
                let count = rows.len();
                let group = rows.into_iter().enumerate().flat_map(|(i, row)| {
                    let line = (i + 1 < count).then(|| div().h(px(1.)).bg(line_color).into_any_element());
                    std::iter::once(row).chain(line)
                });
                body = body.child(v_flex().px_4().rounded(crate::ui::rems_from_px(12.)).rounded_smoothing(crate::ui::SMOOTHING).bg(group_bg).children(group.collect::<Vec<_>>()));
            }
        }
        if !found {
            body = body.child(div().pt_10().flex().justify_center().child(Label::new(t("settings.nothing_found")).color(Color::Muted)));
        }
        body.into_any_element()
    }
}

/// Marks a heading among a section's rows: the rows after it make a group
/// of their own.
const HEADING: char = '\u{1}';

fn heading(title: &str) -> (String, AnyElement) {
    (format!("{HEADING}{title}"), Label::new(title.to_owned()).size(LabelSize::Small).color(Color::Muted).into_any_element())
}

/// Title and description on the left, the control on the right.
fn row(title: &str, description: &str, control: AnyElement) -> (String, AnyElement) {
    let el = h_flex()
        .w_full()
        .justify_between()
        .gap_6()
        .py_3()
        .child(
            v_flex()
                .min_w_0()
                .child(div().whitespace_nowrap().child(Label::new(title.to_owned())))
                .when(!description.is_empty(), |c| c.child(Label::new(description.to_owned()).size(LabelSize::Small).color(Color::Muted))),
        )
        .child(div().flex_none().child(control))
        .into_any_element();
    (format!("{title} {description}"), el)
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
        .child(Button::new(if screenshots { "pick-shots" } else { "pick-videos" }, t("settings.choose_folder")).style(ButtonStyle::Subtle).on_click(cx.listener(move |this, _, _, cx| this.pick_folder(screenshots, cx))))
        .when(custom, |row| {
            row.child(
                IconButton::new(if screenshots { "reset-shots" } else { "reset-videos" }, IconName::Close)
                    .tip(Tooltip::text(t("settings.default")))
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
            (t("common.screenshots"), u.screenshots, colors.text_accent),
            (t("common.recordings"), u.recordings, cx.theme().status().warning),
            (t("settings.studio_cache"), u.cache, colors.text_muted),
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
                    .child(Label::new(t("settings.used")))
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
                .tip(Tooltip::text(t("settings.remove_shortcut")))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.keys[i].remove(k);
                    this.save_keys();
                    cx.notify();
                }))
                .into_any_element()
        });
        let add = if self.recording == Some(i) {
            Label::new(t("settings.press_shortcut_esc_cancel")).size(LabelSize::Small).color(Color::Accent).into_any_element()
        } else {
            IconButton::new(SharedString::from(format!("add-key-{i}")), IconName::Plus)
                .tip(Tooltip::text(t("settings.add_shortcut")))
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

    /// The file name: a field to type the template into, an example of
    /// the name it makes, and the tokens to put in with a click.
    fn render_name(&self, kind: crate::storage::Capture, ext: &str, cx: &mut Context<Self>) -> (String, AnyElement) {
        let colors = cx.theme().colors();
        let template = &self.settings.name_template;
        let example = format!("{}.{ext}", crate::storage::render_name(template, kind, chrono::Local::now(), 1));
        let id = if ext == "png" { "name-shot" } else { "name-video" };
        let field = h_flex()
            .id(id)
            .w(px(230. * kit::SCALE))
            .h(px(28.))
            .px_2()
            .rounded(px(7.))
            .rounded_smoothing(crate::ui::SMOOTHING)
            .bg(colors.element_background)
            .border_1()
            .border_color(if self.naming { colors.text_accent } else { gpui::transparent_black() })
            .cursor_text()
            .overflow_hidden()
            .child(div().whitespace_nowrap().text_sm().child(if self.naming { format!("{template}▏") } else { template.clone() }))
            .on_click(cx.listener(|this, _, _, cx| {
                this.naming = true;
                cx.notify();
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.naming {
                    this.naming = false;
                    cx.notify();
                }
            }));
        let tokens = h_flex().gap_0p5().children(["{date}", "{time}", "{type}", "{n}"].into_iter().map(|token| {
            ButtonLike::new(SharedString::from(format!("{id}-{token}")))
                .style(ButtonStyle::Subtle)
                .child(div().px_1().font_family(kit::MONO_FONT).text_xs().child(token))
                .tip(Tooltip::text(t("settings.add_to_name")))
                .on_click(cx.listener(move |this, _, _, cx| this.update(cx, |s| s.name_template.push_str(token))))
        }));
        let default = template == crate::storage::DEFAULT_NAME;
        let control = v_flex()
            .items_end()
            .gap_1()
            .child(
                h_flex().gap_1().child(field).when(!default, |row| {
                    row.child(
                        IconButton::new(SharedString::from(format!("{id}-reset")), IconName::Close)
                            .tip(Tooltip::text(t("settings.default")))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.naming = false;
                                this.update(cx, |s| s.name_template = crate::storage::DEFAULT_NAME.into())
                            })),
                    )
                }),
            )
            .child(tokens)
            .into_any_element();
        row(t("settings.file_name"), &tr!("settings.for_example", example), control)
    }

    fn name_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        match k.key.as_str() {
            "escape" | "enter" | "tab" => self.naming = false,
            "backspace" => {
                self.settings.name_template.pop();
                self.settings.save();
            }
            _ if !(k.modifiers.control || k.modifiers.alt || k.modifiers.platform) => {
                if let Some(ch) = k.key_char.as_deref().filter(|c| !c.chars().any(char::is_control)) {
                    self.settings.name_template.push_str(ch);
                    self.settings.save();
                }
            }
            _ => {}
        }
        cx.notify();
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
            util::notify(t("settings.couldnt_save_shortcuts"), &e.to_string());
        }
    }
}

impl SettingsView {
    /// For `kadr dev state`.
    pub(crate) fn dev_state(&self) -> serde_json::Value {
        serde_json::json!({
            "tab": format!("{:?}", self.tab).to_lowercase(),
            "query": self.query,
            "recording_shortcut": self.recording,
            "keys": self.keys,
        })
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
            .bg(cx.theme().colors().editor_background)
            .flex()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if this.recording.is_some() {
                    return this.record_key(e, cx);
                }
                if this.naming {
                    return this.name_key(e, cx);
                }
                let i = Tab::ALL.iter().position(|t| *t == this.tab).unwrap_or(0);
                let k = &e.keystroke;
                match k.key.as_str() {
                    "escape" if !this.query.is_empty() => this.query.clear(),
                    "escape" => window.remove_window(),
                    "tab" | "down" => this.tab = Tab::ALL[(i + 1) % Tab::ALL.len()],
                    "up" => this.tab = Tab::ALL[(i + Tab::ALL.len() - 1) % Tab::ALL.len()],
                    "backspace" => {
                        this.query.pop();
                    }
                    // Typing anywhere searches.
                    _ if !(k.modifiers.control || k.modifiers.alt || k.modifiers.platform) => {
                        if let Some(ch) = k.key_char.as_deref().filter(|c| !c.chars().any(char::is_control)) {
                            this.query.push_str(ch);
                            this.scroll.set_offset(gpui::point(px(0.), px(0.)));
                        }
                    }
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
                                    .relative()
                                    .child(div().id("body").size_full().overflow_y_scroll().track_scroll(&self.scroll).child(self.render_body(cx)))
                                    .child(crate::ui::scrollbar(&self.scroll, &self.grab, cx))
                                    // The window's close button in the top right corner.
                                    .children(kit::window_controls(true, cx).map(|c| h_flex().absolute().top_0().right_0().h(px(36.)).px_1().child(c))),
                            ),
                    ),
            )
    }
}
