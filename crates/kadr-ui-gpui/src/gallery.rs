//! A sheet of kadr's components drawn with the real ones: the theme's
//! colours, type, icons, every button and control in each variant and
//! state it has, the patterns that live inside screens, and the sizes and
//! radii they share. A tool for whoever works on kadr's look (`kadr
//! gallery`, left out of the usage): look at a component here before and
//! after changing it. Captions are developer text, in English and not
//! through i18n; the specimens' own words are the UI's.
//!
//! One section is built at a time, and nothing animates: an untouched
//! gallery draws nothing.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, AssetSource, ClickEvent, Context, FocusHandle, Hsla, KeyDownEvent, ScrollHandle, Window, canvas, div, linear_color_stop, linear_gradient, px, rems};
use serde_json::{Value, json};

use crate::i18n::{Lang, t};
use crate::kit;
use crate::palette;
use crate::presets::{HasPresets, Presets};
use crate::settings::Settings;
use crate::ui::{ButtonLike, Divider, KeyBinding, ProgressBar, SMOOTHING, Switch, Theme, ToggleState, Tooltip, prelude::*, rems_from_px};
use kadr_platform::keymap::Command as Keys;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Section {
    Colours,
    Type,
    Icons,
    Buttons,
    Controls,
    Patterns,
    Sizes,
}

impl Section {
    const ALL: [Section; 7] = [Section::Colours, Section::Type, Section::Icons, Section::Buttons, Section::Controls, Section::Patterns, Section::Sizes];

    fn name(self) -> &'static str {
        match self {
            Section::Colours => "Colours",
            Section::Type => "Type",
            Section::Icons => "Icons",
            Section::Buttons => "Buttons",
            Section::Controls => "Controls",
            Section::Patterns => "Patterns",
            Section::Sizes => "Sizes",
        }
    }

    /// Its heading, over the section.
    fn title(self) -> &'static str {
        match self {
            Section::Sizes => "Sizes and radii",
            other => other.name(),
        }
    }

    /// The list's element id, for `kadr dev click section-buttons`.
    fn id(self) -> &'static str {
        match self {
            Section::Colours => "section-colours",
            Section::Type => "section-type",
            Section::Icons => "section-icons",
            Section::Buttons => "section-buttons",
            Section::Controls => "section-controls",
            Section::Patterns => "section-patterns",
            Section::Sizes => "section-sizes",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Section::Colours => Icon::from_path("icons/kadr/fill.svg"),
            Section::Type => Icon::new(IconName::CaseSensitive),
            Section::Icons => Icon::new(IconName::Image),
            Section::Buttons => Icon::from_path("icons/pointer.svg"),
            Section::Controls => Icon::new(IconName::Settings),
            Section::Patterns => Icon::from_path("icons/kadr/frame.svg"),
            Section::Sizes => Icon::from_path("icons/kadr/crop.svg"),
        }
    }
}

/// The theme the gallery shows: the app's own setting, or dark or light
/// for this run only (settings.toml isn't touched).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    App,
    Dark,
    Light,
}

pub struct Gallery {
    section: Section,
    look: Look,
    scroll: ScrollHandle,
    grab: Rc<Cell<Option<f32>>>,
    /// The scrollbar's specimen: a small box of its own.
    demo_scroll: ScrollHandle,
    demo_grab: Rc<Cell<Option<f32>>>,
    focus: FocusHandle,
    /// The saved-looks dropdown's specimen, over a few made-up looks kept
    /// only in memory.
    presets: Presets,
    preset_look: Value,
    /// The switch that can be clicked.
    switch_on: bool,
}

pub fn open(cx: &mut App) {
    // Already open: bring it forward instead of a second one.
    if let Some(window) = cx.windows().into_iter().find_map(|w| w.downcast::<Gallery>()) {
        let _ = window.update(cx, |_, window, _| window.activate());
        return;
    }
    let mon = crate::util::focused_monitor();
    kit::open_window(cx, "kadr-gallery", mon.as_ref(), (980. * kit::SCALE, 700. * kit::SCALE), (640., 420.), |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            // The theme and language picked here were for the gallery only:
            // the settings' own come back with it closed.
            cx.on_release(|_: &mut Gallery, cx: &mut App| {
                crate::i18n::set(Settings::load().language);
                palette::refresh(cx);
            })
            .detach();
            let looks = vec![("Soft".to_owned(), json!({ "padding": 0.1, "radius": 0.5 })), ("Wide margins".to_owned(), json!({ "padding": 0.3, "radius": 0.2 }))];
            Gallery {
                section: Section::Colours,
                look: Look::App,
                scroll: ScrollHandle::new(),
                grab: Default::default(),
                demo_scroll: ScrollHandle::new(),
                demo_grab: Default::default(),
                focus,
                presets: Presets::new(looks, |_| {}, cx),
                preset_look: json!({ "padding": 0.1, "radius": 0.5 }),
                switch_on: true,
            }
        })
    });
}

impl HasPresets for Gallery {
    fn presets(&mut self) -> &mut Presets {
        &mut self.presets
    }

    fn current_look(&self) -> Value {
        self.preset_look.clone()
    }

    fn apply_look(&mut self, look: &Value, _: &mut Context<Self>) {
        self.preset_look = look.clone();
    }
}

// ---------------------------------------------------------------- layout

/// A quiet caption under a specimen.
fn caption(text: impl Into<SharedString>) -> Label {
    Label::new(text).size(LabelSize::XSmall).color(Color::Muted)
}

/// A specimen over its caption; the specimens of a row line up on a
/// common height so their captions do too.
fn spec(text: impl Into<SharedString>, thing: impl IntoElement) -> AnyElement {
    v_flex().gap_1p5().items_start().child(h_flex().min_h(rems_from_px(32.)).child(thing)).child(caption(text)).into_any_element()
}

/// A specimen as wide as `max`, or its card where that's narrower.
fn wide_spec(text: impl Into<SharedString>, max: f32, thing: impl IntoElement) -> AnyElement {
    v_flex().w_full().max_w(px(max)).gap_1p5().child(thing).child(caption(text)).into_any_element()
}

/// A group's title and maybe a line on it, over what it shows.
fn titled(title: &str, note: Option<&str>, body: impl IntoElement) -> AnyElement {
    v_flex()
        .w_full()
        .gap_2()
        .child(div().px_1().font_weight(gpui::FontWeight::SEMIBOLD).child(title.to_owned()))
        .children(note.map(|n| div().px_1().child(Label::new(n.to_owned()).size(LabelSize::Small).color(Color::Muted))))
        .child(body)
        .into_any_element()
}

/// A group: its specimens on a card a tone apart from the page.
fn group(title: &str, note: Option<&str>, items: Vec<AnyElement>, cx: &App) -> AnyElement {
    titled(
        title,
        note,
        div()
                .w_full()
                .flex()
                .flex_wrap()
                .items_start()
                .gap_x_6()
                .gap_y_4()
                .p_4()
                .rounded(rems_from_px(12.))
                .rounded_smoothing(SMOOTHING)
                .bg(cx.theme().colors().panel_background)
                .children(items),
    )
}

/// One row of a table-like group: its name at the left, the specimens after.
fn line(name: &str, items: Vec<AnyElement>) -> AnyElement {
    h_flex()
        .w_full()
        .items_start()
        .gap_4()
        .child(div().w(px(84.)).flex_none().pt(px(6.)).child(Label::new(name.to_owned()).size(LabelSize::Small)))
        .child(div().flex_1().min_w_0().flex().flex_wrap().items_start().gap_x_5().gap_y_3().children(items))
        .into_any_element()
}

/// The settings' chips (`settings_view::chips`, not shared yet), for the
/// gallery's own theme and language switches.
fn chip(id: impl Into<SharedString>, label: impl Into<SharedString>, on: bool) -> ButtonLike {
    ButtonLike::new(gpui::ElementId::Name(id.into())).style(if on { ButtonStyle::Filled } else { ButtonStyle::Subtle }).toggle_state(on).child(div().px_1p5().child(Label::new(label).size(LabelSize::Small)))
}

/// A colour as `#rrggbb`, with its opacity when it has one.
fn hex(c: Hsla) -> String {
    let rgb = crate::ui::to_rgba(c).to_hex();
    if c.alpha < 0.999 { format!("{rgb} · {}%", (c.alpha * 100.).round()) } else { rgb }
}

/// `px` of a size given at a 16px base, as drawn at kadr's `REM`.
fn at_rem(v: f32) -> String {
    let drawn = v * kit::REM / 16.;
    if (drawn - drawn.round()).abs() < 0.05 { format!("{drawn:.0}") } else { format!("{drawn:.1}") }
}

impl Gallery {
    fn set_look(&mut self, look: Look, cx: &mut Context<Self>) {
        self.look = look;
        match look {
            Look::App => palette::refresh(cx),
            Look::Dark | Look::Light => {
                cx.set_global(palette::built_in(look == Look::Dark));
                cx.refresh_windows();
            }
        }
        cx.notify();
    }

    /// The sections, down the left.
    fn render_list(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(px(if width < 820. { 150. } else { 180. } * kit::SCALE))
            .h_full()
            .flex_none()
            .p_2()
            .pt_4()
            .gap_0p5()
            .bg(cx.theme().colors().panel_background)
            .child(div().px_2().pb_2().child(Label::new("Components").size(LabelSize::Small).color(Color::Muted)))
            .children(Section::ALL.into_iter().map(|section| {
                let on = section == self.section;
                ButtonLike::new(section.id())
                    .size(ButtonSize::Large)
                    .toggle_state(on)
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .px_1()
                            .child(section.icon().size(IconSize::Small).color(if on { Color::Default } else { Color::Muted }))
                            .child(Label::new(section.name()).truncate()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.section = section;
                        this.scroll.set_offset(gpui::point(px(0.), px(0.)));
                        cx.notify();
                    }))
            }))
    }

    /// The section's name, and the theme and language the gallery is shown in.
    fn render_top(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let looks = [(Look::App, "App setting", "look-app"), (Look::Dark, "Dark", "look-dark"), (Look::Light, "Light", "look-light")];
        let lang = crate::i18n::current();
        h_flex()
            .w_full()
            .flex_none()
            .items_start()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x_4()
                    .gap_y_2()
                    .px_6()
                    .pt_4()
                    .pb_3()
                    .child(div().flex_1().min_w(px(140.)).text_size(rems_from_px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child(self.section.title()))
                    .child(h_flex().gap_0p5().children(looks.into_iter().map(|(look, name, id)| {
                        chip(id, name, self.look == look).on_click(cx.listener(move |this, _, _, cx| this.set_look(look, cx)))
                    })))
                    .child(h_flex().gap_0p5().children(Lang::ALL.into_iter().map(|l| {
                        chip(format!("lang-{}", l.code()), l.name(), l == lang).on_click(cx.listener(move |_, _, _, cx| {
                            // For this run: the settings keep theirs.
                            crate::i18n::set(l);
                            cx.refresh_windows();
                        }))
                    }))),
            )
            .children(kit::window_controls(true, cx).map(|c| h_flex().flex_none().h(px(40.)).px_1().child(c)))
    }

    // ------------------------------------------------------------ colours

    fn colours(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let column = |name: &str, theme: &Theme| {
            let (text, muted) = (theme.colors.text, theme.colors.text_muted);
            let rows = colour_groups(theme).into_iter().map(|(heading, list)| {
                v_flex()
                    .gap_2()
                    .child(Label::new(heading).size(LabelSize::XSmall).color(Color::Custom(muted)))
                    .children(list.into_iter().map(|(field, colour, used)| {
                        h_flex()
                            .gap_3()
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(44.))
                                    .h(px(28.))
                                    .rounded(px(7.))
                                    .border_1()
                                    .border_color(theme.colors.border)
                                    .bg(colour),
                            )
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .child(Label::new(field).size(LabelSize::Small).color(Color::Custom(text)))
                                    .child(Label::new(format!("{} · {used}", hex(colour))).size(LabelSize::XSmall).color(Color::Custom(muted))),
                            )
                    }))
            });
            v_flex()
                .flex_1()
                .min_w(px(260.))
                .gap_4()
                .p_4()
                .rounded(rems_from_px(12.))
                .rounded_smoothing(SMOOTHING)
                .bg(theme.colors.panel_background)
                .border_1()
                .border_color(theme.colors.border)
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).text_color(text).child(name.to_owned()))
                .children(rows)
                .into_any_element()
        };
        let (dark, light) = (palette::built_in(true), palette::built_in(false));
        let themes = div().w_full().flex().flex_wrap().items_start().gap_4().child(column("Dark (palette::built_in(true))", &dark)).child(column("Light (palette::built_in(false))", &light));

        let theme = cx.theme().clone();
        let fixed = vec![
            spec("ui::on(accent): text on the accent", div().px_3().py_1().rounded(px(7.)).bg(theme.colors.text_accent).text_color(crate::ui::on(theme.colors.text_accent)).child(t("common.save"))),
            spec("tip_plate: black, white hairline 14%", crate::ui::tip_plate().h(px(22.)).child("#000000")),
            spec("ui::TIP_DIM: white 50%", crate::ui::tip_plate().h(px(22.)).child(div().text_color(crate::ui::TIP_DIM).child("Ctrl+S"))),
            spec("Switch knob, slider knob: white", div().size(px(22.)).rounded_full().bg(gpui::white()).border_1().border_color(theme.colors.border)),
        ];
        let annotation = palette::annotation_colors()
            .into_iter()
            .map(|c| spec(hex(c), div().size(px(26.)).rounded_full().bg(c).border_1().border_color(theme.colors.border)))
            .collect();
        let backdrops = palette::backdrops()
            .into_iter()
            .enumerate()
            .map(|(i, (a, b))| spec(format!("{i}"), div().size(px(34.)).rounded(px(8.)).bg(linear_gradient(135., linear_color_stop(a, 0.), linear_color_stop(b, 1.)))))
            .collect();
        vec![
            // Each set on its own background, so no card around them.
            titled(
                "Theme",
                Some("Every ui::Colors and ui::Status field, both sets side by side; translucent ones show over the panel, as the UI draws them (the text colour at a low opacity)."),
                themes,
            ),
            group("Fixed colours", Some("The same in both themes."), fixed, cx),
            group("Annotation colours", Some("palette::annotation_colors: the editor's pen colours."), annotation, cx),
            group("Backdrops", Some("palette::backdrops: the editor's and the studio's gradients and plain colours."), backdrops, cx),
        ]
    }

    // ------------------------------------------------------------ type

    fn type_(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        const EN: &str = "Screenshots and screen recording";
        const RU: &str = "Снимки и запись экрана, предпросмотр";
        let sizes = [(LabelSize::Default, "Default", 14.), (LabelSize::Small, "Small", 12.), (LabelSize::XSmall, "XSmall", 10.)]
            .into_iter()
            .map(|(size, name, v)| {
                spec(
                    format!("Label · LabelSize::{name} · {v} ({} px)", at_rem(v)),
                    v_flex().child(Label::new(EN).size(size)).child(Label::new(RU).size(size)),
                )
            })
            .collect();
        let colours = [
            (Color::Default, "Default"),
            (Color::Muted, "Muted"),
            (Color::Accent, "Accent"),
            (Color::Error, "Error"),
            (Color::Disabled, "Disabled"),
            (Color::Placeholder, "Placeholder"),
        ]
        .into_iter()
        .map(|(c, name)| spec(format!("Color::{name}"), Label::new(t("settings.changes_saved_right_away")).color(c)))
        .collect();
        let weight = |w: gpui::FontWeight, size: f32, text: &'static str| div().text_size(rems_from_px(size)).font_weight(w).child(text);
        let weights = vec![
            spec("normal · 14", weight(gpui::FontWeight::NORMAL, 14., EN)),
            spec("MEDIUM · 14 (one use: the studio's zoom chips, text_xs)", weight(gpui::FontWeight::MEDIUM, 14., EN)),
            spec("SEMIBOLD · 14: the editor's text-style samples", weight(gpui::FontWeight::SEMIBOLD, 14., EN)),
            spec("SEMIBOLD · 18: the storage total", weight(gpui::FontWeight::SEMIBOLD, 18., "2.4 GB")),
            spec("SEMIBOLD · 20: a settings section's title", weight(gpui::FontWeight::SEMIBOLD, 20., "Общие")),
        ];
        let font = |family: &'static str, w: gpui::FontWeight| v_flex().font_family(family).font_weight(w).child(EN).child(RU).child("0123456789 · 00:42.17 · 1920×1080");
        let fonts = vec![
            spec(format!("kit::UI_FONT ({}): the system's", kit::UI_FONT), font(kit::UI_FONT, gpui::FontWeight::NORMAL)),
            spec(format!("kit::ANNOTATION_FONT ({}) Regular: text on screenshots", kit::ANNOTATION_FONT), font(kit::ANNOTATION_FONT, gpui::FontWeight::NORMAL)),
            spec(format!("kit::ANNOTATION_FONT ({}) SemiBold", kit::ANNOTATION_FONT), font(kit::ANNOTATION_FONT, gpui::FontWeight::SEMIBOLD)),
            spec(format!("kit::MONO_FONT ({}): timers, sizes, shortcuts", kit::MONO_FONT), font(kit::MONO_FONT, gpui::FontWeight::NORMAL)),
        ];
        let truncate = vec![spec("Label::truncate in 160 px", div().w(px(160.)).child(Label::new(t("settings.thumbnail_in_screen_corner_after")).truncate()))];
        vec![
            group("Label sizes", Some("Each in English and Russian (longer words)."), sizes, cx),
            group("Label colours", None, colours, cx),
            group("Weights in use", None, weights, cx),
            group("Fonts", None, fonts, cx),
            group("Truncation", None, truncate, cx),
        ]
    }

    // ------------------------------------------------------------ icons

    fn icons(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        const SIZES: [IconSize; 5] = [IconSize::XSmall, IconSize::Small, IconSize::Medium, IconSize::Tool, IconSize::Large];
        let cell = |icon: &dyn Fn() -> Icon, name: String| {
            v_flex()
                .w(px(160.))
                .gap_1p5()
                .child(h_flex().h(px(28.)).gap_2().children(SIZES.map(|s| icon().size(s))))
                .child(caption(name))
                .into_any_element()
        };
        let named = ICONS.iter().map(|&name| cell(&move || Icon::new(name), icon_name(name).to_owned())).collect();
        // Everything else bundled, used by its path (`Icon::from_path`).
        let enum_paths: Vec<&str> = ICONS.iter().map(|i| i.path()).collect();
        let mut paths: Vec<SharedString> = kit::Assets.list("icons/").unwrap_or_default().into_iter().filter(|p| !enum_paths.contains(&p.as_ref())).collect();
        paths.sort();
        let by_path = paths
            .into_iter()
            .map(|p| {
                let shown = p.trim_start_matches("icons/").trim_end_matches(".svg").to_owned();
                cell(&move || Icon::from_path(p.clone()), shown)
            })
            .collect();
        let colours = [(Color::Default, "Default"), (Color::Muted, "Muted"), (Color::Accent, "Accent"), (Color::Error, "Error"), (Color::Disabled, "Disabled")]
            .into_iter()
            .map(|(c, name)| spec(format!("Color::{name}"), Icon::new(IconName::Folder).size(IconSize::Medium).color(c)))
            .collect();
        vec![
            group("IconName", Some("Each at XSmall 12, Small 14, Medium 16, Tool 18, Large 22."), named, cx),
            group("Bundled, by path", Some("Icon::from_path: the editor's tools, the settings' sections and the rest kit::Assets holds."), by_path, cx),
            group("Icon colours", None, colours, cx),
        ]
    }

    // ------------------------------------------------------------ buttons

    fn buttons(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let styles = [(ButtonStyle::Accent, "Accent"), (ButtonStyle::Filled, "Filled"), (ButtonStyle::Subtle, "Subtle"), (ButtonStyle::Outlined, "Outlined")];
        let button_rows = styles
            .into_iter()
            .map(|(style, name)| {
                let id = |what: &str| SharedString::from(format!("button-{name}-{what}"));
                line(
                    name,
                    vec![
                        spec("rest", Button::new(id("rest"), t("common.save")).style(style)),
                        spec("disabled", Button::new(id("disabled"), t("common.save")).style(style).disabled(true)),
                        spec("start_icon", Button::new(id("icon"), t("common.copy")).style(style).start_icon(Icon::from_path("icons/copy.svg"))),
                        spec("key_binding", Button::new(id("keys"), t("common.save")).style(style).key_binding(kit::keys("secondary-s"))),
                        spec("Large", Button::new(id("large"), t("common.save")).style(style).size(ButtonSize::Large)),
                        spec("Large, icon, keys", Button::new(id("large-all"), t("common.save")).style(style).size(ButtonSize::Large).start_icon(Icon::new(IconName::Check)).key_binding(kit::keys("secondary-s"))),
                        spec("Large disabled", Button::new(id("large-disabled"), t("common.save")).style(style).size(ButtonSize::Large).disabled(true)),
                        spec(
                            "ButtonLike toggle_state",
                            ButtonLike::new(id("toggled")).style(style).toggle_state(true).child(div().px_1().child(Label::new(t("common.window")).size(LabelSize::Small))),
                        ),
                    ],
                )
            })
            .collect();
        let options = vec![
            spec("label_size(Small)", Button::new("button-small-label", t("common.cancel")).label_size(LabelSize::Small)),
            spec("color(Muted)", Button::new("button-muted", t("common.cancel")).color(Color::Muted).key_binding(kit::keys("escape"))),
            spec("color(Error)", Button::new("button-error", t("common.delete_preset")).color(Color::Error)),
            spec("full_width in 240 px", h_flex().w(px(240.)).child(Button::new("button-full", t("common.save")).style(ButtonStyle::Filled).full_width())),
            spec("tooltip: point at it", Button::new("button-tip", t("common.copy")).style(ButtonStyle::Filled).tip(Tooltip::text(t("common.show_in_folder")))),
        ];
        let icon_rows = styles
            .into_iter()
            .map(|(style, name)| {
                let id = |what: &str| SharedString::from(format!("icon-button-{name}-{what}"));
                line(
                    name,
                    vec![
                        spec("rest", IconButton::new(id("rest"), IconName::Pencil).style(style)),
                        spec("disabled", IconButton::new(id("disabled"), IconName::Pencil).style(style).disabled(true)),
                        spec("Large", IconButton::new(id("large"), IconName::Pencil).style(style).size(ButtonSize::Large)),
                        spec("Large, Medium icon", IconButton::new(id("large-medium"), IconName::Pencil).style(style).size(ButtonSize::Large).icon_size(IconSize::Medium)),
                        spec("XSmall icon, Muted", IconButton::new(id("muted"), IconName::Trash).style(style).icon_size(IconSize::XSmall).icon_color(Color::Muted)),
                        spec("Large disabled", IconButton::new(id("large-disabled"), IconName::Undo).style(style).size(ButtonSize::Large).disabled(true)),
                    ],
                )
            })
            .collect();
        let accent = cx.theme().colors().text_accent;
        let like = vec![
            spec(
                "square, icon by path (editor tool)",
                ButtonLike::new("like-tool").size(ButtonSize::Large).square().child(Icon::from_path("icons/kadr/rect.svg").size(IconSize::Tool)),
            ),
            spec(
                "square, Accent (chosen tool)",
                ButtonLike::new("like-tool-on")
                    .style(ButtonStyle::Accent)
                    .size(ButtonSize::Large)
                    .square()
                    .child(Icon::from_path("icons/kadr/rect.svg").size(IconSize::Tool).color(Color::Custom(crate::ui::on(accent)))),
            ),
            spec("square, toggle_state (a mode on)", ButtonLike::new("like-mode").size(ButtonSize::Large).square().toggle_state(true).child(Icon::from_path("icons/kadr/frame.svg").size(IconSize::Tool))),
            spec(
                "Filled + Label + chevron: a dropdown's button",
                ButtonLike::new("like-dropdown")
                    .style(ButtonStyle::Filled)
                    .child(h_flex().gap_1p5().px_2().child(Label::new(t("settings.system")).size(LabelSize::Small)).child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted))),
            ),
            spec("Filled, mono text: a shortcut chip", ButtonLike::new("like-keys").style(ButtonStyle::Filled).child(div().px_1().font_family(kit::MONO_FONT).text_sm().child("Super+Shift+S"))),
        ];
        vec![
            group(
                "Button",
                Some("Every ButtonStyle at both ButtonSizes. Hover and pressed exist only under the pointer (GPUI's hover and active styles): point at a specimen, or `kadr dev scroll <x,y> 0` to move the test pointer there. Button has no toggle_state of its own: the last column is a ButtonLike."),
                button_rows,
                cx,
            ),
            group("Button options", None, options, cx),
            group("IconButton", Some("IconButtonShape::Square is its only shape; IconButton takes no toggle_state."), icon_rows, cx),
            group("ButtonLike", Some("The base of both, with any children: how the screens build tools, chips and dropdown buttons."), like, cx),
        ]
    }

    // ------------------------------------------------------------ controls

    fn controls(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let switches = vec![
            spec("on", Switch::new("switch-on", ToggleState::Selected)),
            spec("off", Switch::new("switch-off", ToggleState::Unselected)),
            spec("on, disabled", Switch::new("switch-on-disabled", ToggleState::Selected).disabled(true)),
            spec("off, disabled", Switch::new("switch-off-disabled", ToggleState::Unselected).disabled(true)),
            spec(
                "click it",
                Switch::new("switch-live", ToggleState::from(self.switch_on)).on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                    this.switch_on = *state == ToggleState::Selected;
                    cx.notify();
                })),
            ),
        ];
        let bars = [0., 0.25, 0.6, 1.].into_iter().map(|v| spec(format!("{}%", (v * 100.) as u32), div().w(px(160.)).child(ProgressBar::new("progress", v, 1., cx)))).collect();
        let keys = ["secondary-s", "secondary-shift-z", "escape", "enter", "space", "shift-tab", "g", "secondary-c secondary-v"]
            .into_iter()
            .map(|k| spec(format!("KeyBinding::new(\"{k}\")"), KeyBinding::new(k)))
            .chain(
                [(Keys::Copy, "Copy"), (Keys::Paste, "Paste"), (Keys::Undo, "Undo"), (Keys::Redo, "Redo"), (Keys::SaveAs, "SaveAs"), (Keys::Quit, "Quit")]
                    .into_iter()
                    .map(|(c, name)| spec(format!("ui::shortcut({name})"), Label::new(crate::ui::shortcut(c)).size(LabelSize::Small))),
            )
            .collect();
        let tips = vec![
            spec("tip_plate: a tooltip, drawn still", crate::ui::tip_plate().h(px(22.)).gap_1().child(t("common.save")).child(div().text_color(crate::ui::TIP_DIM).child(crate::ui::shortcut(Keys::Save)))),
            spec("Tooltip::text: point at it", Button::new("tip-text", t("common.copy")).style(ButtonStyle::Filled).tip(Tooltip::text(t("common.show_in_folder")))),
            spec(
                "Tooltip with a key_binding: point at it",
                IconButton::new("tip-keys", IconName::Check).style(ButtonStyle::Filled).tip(|_, cx| cx.new(|_| Tooltip::new(t("common.save")).key_binding(kit::keys("secondary-s"))).into()),
            ),
        ];
        let colors = cx.theme().colors().clone();
        let divider = vec![
            spec("Divider::vertical: air, 1 px", h_flex().h(px(24.)).child(Label::new("A")).child(Divider::vertical().h_5().mx_1()).child(Label::new("B"))),
        ];
        let scroll = vec![spec(
            "ui::scrollbar: drag it, or click the track",
            div()
                .relative()
                .w(px(220.))
                .h(px(120.))
                .rounded(px(8.))
                .bg(colors.element_background)
                .child(div().id("demo-scroll").size_full().overflow_y_scroll().track_scroll(&self.demo_scroll).p_2().children((1..=14).map(|i| Label::new(format!("{} {i}", t("common.recording"))).size(LabelSize::Small))))
                .child(crate::ui::scrollbar(&self.demo_scroll, &self.demo_grab, cx)),
        )];
        let accent = colors.text_accent;
        let knob = vec![spec(
            "ui::paint_knob: the picker's and the editor's handles",
            canvas(|_, _, _| {}, move |bounds, _, window, _| crate::ui::paint_knob(window, bounds.center(), accent)).w(px(40.)).h(px(40.)),
        )];
        vec![
            group("Switch", None, switches, cx),
            group("ProgressBar", Some("No indeterminate variant."), bars, cx),
            group("Key hints", Some("Written the system's way: ⌘ on macOS, Ctrl+ elsewhere."), keys, cx),
            group("Tooltip", None, tips, cx),
            group("Divider", Some("Only a vertical one can be made: Divider has no constructor for its horizontal hairline."), divider, cx),
            group("Scrollbar", None, scroll, cx),
            group("Knob", None, knob, cx),
        ]
    }

    // ------------------------------------------------------------ patterns

    fn patterns(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let shared = vec![
            wide_spec("presets::render: the saved-looks dropdown and +", 300., div().w_full().child(crate::presets::render(&self.presets, &self.preset_look.clone(), cx))),
            wide_spec(
                "settings_view::row with a Switch",
                420.,
                div().w_full().px_4().rounded(rems_from_px(12.)).bg(colors.element_background).child(
                    crate::settings_view::row(
                        t("settings.window_shadow"),
                        t("settings.window_captures_keep_shadow"),
                        Switch::new("row-switch", ToggleState::from(self.switch_on))
                            .on_click(cx.listener(|this, state: &ToggleState, _, cx| {
                                this.switch_on = *state == ToggleState::Selected;
                                cx.notify();
                            }))
                            .into_any_element(),
                    )
                    .1,
                ),
            ),
            wide_spec(
                "settings_view::row with a Button",
                420.,
                div().w_full().px_4().rounded(rems_from_px(12.)).bg(colors.element_background).child(
                    crate::settings_view::row(t("settings.clear_now"), t("settings.delete_whats_older_than_chosen"), Button::new("row-button", t("settings.clear")).style(ButtonStyle::Subtle).into_any_element()).1,
                ),
            ),
            spec(
                "editor::checker_tile(22, 5) in a round dot, as the background panel draws it",
                div().size(px(22.)).rounded_full().overflow_hidden().border_1().border_color(colors.border_variant).child(crate::editor::checker_tile(22., 5.)),
            ),
            spec(
                "ui::native_menu: the system's menu (macOS, Windows); none on Linux",
                Button::new("native-menu", t("common.open")).style(ButtonStyle::Filled).on_click(cx.listener(|_, e: &ClickEvent, window, cx| {
                    let items = vec![kadr_platform::menu::Item::new(t("settings.system"), true), kadr_platform::menu::Item::new(t("settings.dark"), false), kadr_platform::menu::Item::new(t("settings.light"), false)];
                    crate::ui::native_menu(window, cx, e.position(), items, |_: &mut Gallery, _, _, _| {});
                })),
            ),
            spec(format!("kit::window_controls: {}, top right of this window", window_controls_name()), div()),
        ];
        let backlog = PRIVATE
            .iter()
            .map(|(name, lives, copies)| {
                v_flex()
                    .w(px(250.))
                    .gap_0p5()
                    .p_3()
                    .rounded(rems_from_px(9.))
                    .rounded_smoothing(SMOOTHING)
                    .bg(colors.element_background)
                    .child(Label::new(*name).size(LabelSize::Small))
                    .child(caption(format!("lives in {lives}, not yet a shared component")))
                    .child(caption(*copies))
                    .into_any_element()
            })
            .collect();
        vec![
            group("Shared", Some("Patterns that can be reused as they are."), shared, cx),
            group("Not yet shared", Some("Components in practice, private to their screens: copying them here would only be another copy. The backlog for making them shared."), backlog, cx),
        ]
    }

    // ------------------------------------------------------------ sizes

    fn sizes(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let colors = cx.theme().colors().clone();
        let tone = colors.element_active;
        let rem_box = |r: f32| div().w(px(64.)).h(px(44.)).rounded(rems_from_px(r)).rounded_smoothing(SMOOTHING).bg(tone);
        let px_box = |r: f32| div().w(px(64.)).h(px(44.)).rounded(px(r)).rounded_smoothing(SMOOTHING).bg(tone);
        let radii = vec![
            spec(format!("tooltip · 6 ({} px)", at_rem(6.)), rem_box(6.)),
            spec(format!("Button Default · 7 ({} px)", at_rem(7.)), rem_box(7.)),
            spec(format!("Button Large · 9 ({} px)", at_rem(9.)), rem_box(9.)),
            spec(format!("elevated surface · 12 ({} px)", at_rem(12.)), rem_box(12.)),
            spec(format!("kit::WINDOW_RADIUS · {} px", kit::WINDOW_RADIUS), px_box(kit::WINDOW_RADIUS)),
            spec(format!("kit::RADIUS (thumbnails) · {} px", kit::RADIUS), px_box(kit::RADIUS)),
            spec("Switch, ProgressBar · full", div().w(px(64.)).h(px(24.)).rounded_full().bg(tone)),
        ];
        let height = |h: f32, w: f32| div().w(px(w)).h(rems_from_px(h)).rounded(px(3.)).bg(tone);
        let heights = vec![
            spec(format!("ButtonSize::Default · 22 ({} px)", at_rem(22.)), height(22., 48.)),
            spec(format!("ButtonSize::Large · 32 ({} px)", at_rem(32.)), height(32., 48.)),
            spec(format!("Switch · 30 × 18 ({} × {} px)", at_rem(30.), at_rem(18.)), div().w(rems_from_px(30.)).h(rems_from_px(18.)).rounded(px(3.)).bg(tone)),
            spec(format!("ProgressBar · 5 ({} px)", at_rem(5.)), height(5., 48.)),
        ];
        let steps = [("0p5", 0.125), ("1", 0.25), ("1p5", 0.375), ("2", 0.5), ("3", 0.75), ("4", 1.), ("5", 1.25), ("6", 1.5)]
            .into_iter()
            .map(|(name, r)| spec(format!("_{name} · {r}rem · {} px", at_rem(r * 16.)), div().w(rems(r)).h(px(24.)).bg(colors.text_accent)))
            .collect();
        // The rule as kit says it, and the real Large button in the window's
        // corner: its radius is in rems, so at kadr's REM it isn't 9 px.
        let (outer, inset, inner) = (kit::WINDOW_RADIUS, 4., kit::WINDOW_RADIUS - 4.);
        let concentric = vec![
            spec(
                format!("outer {outer} − inset {inset} = inner {inner} (px)"),
                div().w(px(120.)).h(px(46.)).p(px(inset)).rounded(px(outer)).rounded_smoothing(SMOOTHING).bg(colors.element_active).child(div().size_full().rounded(px(inner)).rounded_smoothing(SMOOTHING).bg(colors.text_accent)),
            ),
            spec(
                format!("a real Large button {inset} px inside the window's {outer} px corner: its radius is {} px", at_rem(9.)),
                div().p(px(inset)).rounded(px(outer)).rounded_smoothing(SMOOTHING).bg(colors.element_active).child(Button::new("concentric-button", t("common.save")).style(ButtonStyle::Accent).size(ButtonSize::Large)),
            ),
        ];
        let surface = |el: Div| el.w(px(150.)).h(px(70.)).flex().items_center().justify_center();
        let surfaces = vec![
            spec("StyledExt::elevation_2: popovers, floating bars", surface(div().elevation_2(cx)).child(Label::new(t("common.presets")).size(LabelSize::Small))),
            spec("StyledExt::elevation_3: menus, toasts", surface(div().elevation_3(cx)).child(Label::new(t("common.presets")).size(LabelSize::Small))),
            spec("kit::shadow: floating pictures", surface(div().rounded(px(kit::RADIUS)).bg(colors.elevated_surface_background).shadow(kit::shadow()))),
        ];
        vec![
            group("Radii", Some("Sizes given at a 16 px base grow with the window's rem (kit::REM = 19); the px in brackets is what's drawn."), radii, cx),
            group("Control heights", None, heights, cx),
            group("Spacing steps", Some("GPUI's gap_ / p_ / m_ steps, in rems."), steps, cx),
            group("Concentric corners", Some("An inner corner's radius is the outer one less the inset."), concentric, cx),
            group("Surfaces", None, surfaces, cx),
        ]
    }
}

/// Every `IconName`. `icon_name` matches them all, so a new one stops the
/// build there until it's named, and so remembered here too.
const ICONS: [IconName; 28] = [
    IconName::ArrowUpRight,
    IconName::AudioOn,
    IconName::CaseSensitive,
    IconName::Check,
    IconName::ChevronDown,
    IconName::Circle,
    IconName::Close,
    IconName::Crosshair,
    IconName::Dash,
    IconName::DebugPause,
    IconName::Folder,
    IconName::Image,
    IconName::Keyboard,
    IconName::MagnifyingGlass,
    IconName::Mic,
    IconName::Pencil,
    IconName::Pin,
    IconName::PlayFilled,
    IconName::Plus,
    IconName::Scissors,
    IconName::Screen,
    IconName::Settings,
    IconName::Slash,
    IconName::Sparkle,
    IconName::Stop,
    IconName::Trash,
    IconName::Undo,
    IconName::Restart,
];

fn icon_name(icon: IconName) -> &'static str {
    match icon {
        IconName::ArrowUpRight => "ArrowUpRight",
        IconName::AudioOn => "AudioOn",
        IconName::CaseSensitive => "CaseSensitive",
        IconName::Check => "Check",
        IconName::ChevronDown => "ChevronDown",
        IconName::Circle => "Circle",
        IconName::Close => "Close",
        IconName::Crosshair => "Crosshair",
        IconName::Dash => "Dash",
        IconName::DebugPause => "DebugPause",
        IconName::Folder => "Folder",
        IconName::Image => "Image",
        IconName::Keyboard => "Keyboard",
        IconName::MagnifyingGlass => "MagnifyingGlass",
        IconName::Mic => "Mic",
        IconName::Pencil => "Pencil",
        IconName::Pin => "Pin",
        IconName::PlayFilled => "PlayFilled",
        IconName::Plus => "Plus",
        IconName::Scissors => "Scissors",
        IconName::Screen => "Screen",
        IconName::Settings => "Settings",
        IconName::Slash => "Slash",
        IconName::Sparkle => "Sparkle",
        IconName::Stop => "Stop",
        IconName::Trash => "Trash",
        IconName::Undo => "Undo",
        IconName::Restart => "Restart",
    }
}

/// A theme's colours in groups: the field, the colour and what it's for.
fn colour_groups(theme: &Theme) -> Vec<(&'static str, Vec<(&'static str, Hsla, &'static str)>)> {
    let (c, s) = (&theme.colors, &theme.status);
    vec![
        (
            "Surfaces",
            vec![
                ("panel_background", c.panel_background, "panels, side bars"),
                ("editor_background", c.editor_background, "work area, settings page"),
                ("title_bar_background", c.title_bar_background, "top bars, the studio's rail"),
                ("status_bar_background", c.status_bar_background, "bottom bars"),
                ("elevated_surface_background", c.elevated_surface_background, "menus, popovers"),
            ],
        ),
        ("Lines", vec![("border", c.border, "surfaces' hairline"), ("border_variant", c.border_variant, "separators, Outlined")]),
        (
            "Controls",
            vec![
                ("element_background", c.element_background, "Filled, fields, tracks"),
                ("element_hover", c.element_hover, "Filled hover, menu rows"),
                ("element_active", c.element_active, "Filled pressed, switch off"),
                ("element_selected", c.element_selected, "Filled toggled"),
                ("ghost_element_hover", c.ghost_element_hover, "Subtle hover"),
                ("ghost_element_active", c.ghost_element_active, "Subtle pressed"),
                ("ghost_element_selected", c.ghost_element_selected, "Subtle toggled"),
            ],
        ),
        (
            "Text",
            vec![
                ("text", c.text, "Color::Default"),
                ("text_muted", c.text_muted, "Color::Muted"),
                ("text_placeholder", c.text_placeholder, "Color::Placeholder"),
                ("text_disabled", c.text_disabled, "Color::Disabled"),
                ("text_accent", c.text_accent, "the accent: Accent, switch on"),
            ],
        ),
        ("Status", vec![("status.error", s.error, "Color::Error"), ("status.warning", s.warning, "warnings"), ("status.info", s.info, "ProgressBar fill")]),
    ]
}

/// What `kit::window_controls` draws on this desktop.
fn window_controls_name() -> &'static str {
    use crate::platform::WindowControls as W;
    match crate::platform::window_controls() {
        W::Left(_) => "room for the system's at the left",
        W::Right(_) => "minimise, maximise and close at the right",
        W::Own => "kadr's close button",
        W::None => "none (a tiling manager)",
    }
}

/// Patterns that are components in practice but private to their screens:
/// the name, where they live, and their copies.
const PRIVATE: &[(&str, &str, &str)] = &[
    ("Option chips (segmented choice)", "settings_view.rs `chips`, trim.rs, studio_editor.rs `choices`, editor.rs aspect row", "4 looks: Filled/Subtle chips (settings, trim, the theme row), accent-filled tabs 28 px (studio), a tray of cells (editor)"),
    ("Dropdown button + kadr-drawn menu", "settings_view.rs `dropdown`, editor.rs `crop_dropdown`/`menu_list`, studio_editor.rs `aspect_dropdown`, history.rs filter, panel.rs size and mic menus", "6 copies; rows 28 or 30 px or MENU_ROW, corners 6 or 7, the check Small/Accent or XSmall/Default"),
    ("Slider", "editor.rs `slider`, studio_editor.rs `slider` + `labeled_slider`", "2 near-identical copies (the studio's has ticks)"),
    ("Checkbox", "studio_editor.rs `check`, editor.rs crop footer (snap to edges)", "2 copies"),
    ("Text field", "settings_view.rs search and file name, presets.rs rename, history.rs search, editor.rs crop size and text tool", "6; only the editor's text tool goes through kadr_ui::text::Field and the input handler; the rest keep a String and draw their own caret (a ▏, a 1 px bar, history's blinking one)"),
    ("Colour swatch", "editor.rs colour menu and fill picker, editor.rs + studio_editor.rs backdrop tiles, settings_view.rs studio backdrops", "5; the chosen ring is the text colour in some, the accent in others"),
    ("Colour menu", "editor.rs `render_toolbar`", "1: own colours, pipette, recent, saved and +"),
    ("Backdrop picker", "editor.rs, studio_editor.rs (TILE, TILE_RADIUS, TILE_GAP, DOT, GRADIENT_ROWS)", "2 copies with the same constants; the settings' studio backdrops are a third, not concentric (7 outer, 5 inner, 4 inset)"),
    ("Popover surface (deferred, bounds tracked, closes outside)", "editor.rs colour/text/fill/more menus, studio_editor.rs export, presets.rs", "about a dozen, each with its own offset"),
    ("Settings group (inset card, hairlines) and heading", "settings_view.rs `render_body`, `heading`", "1; studio_editor.rs `group` is a plainer cousin"),
    ("Labelled switch row (compact)", "studio_editor.rs `switch_row`", "1, beside settings_view.rs `row`"),
    ("Section list", "settings_view.rs sidebar, studio_editor.rs `render_rail`", "2 looks: accent-filled rows, accent-filled 36 px icons"),
    ("White pill buttons over a picture", "cards.rs `pill` (twice), history.rs `pill`, pin.rs `pill`", "4 copies, sizes 24, 20 and 26; history's has no pressed state"),
    ("Quick-access card frame", "cards.rs `render_card`", "1, with its job and text variants"),
    ("History grid item", "history.rs `render_item`", "1: thumbnail, hover overlay, frosted duration badge"),
    ("Storage bar (stacked progress)", "settings_view.rs `render_usage`", "1"),
];

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        let body = match self.section {
            Section::Colours => self.colours(cx),
            Section::Type => self.type_(cx),
            Section::Icons => self.icons(cx),
            Section::Buttons => self.buttons(cx),
            Section::Controls => self.controls(cx),
            Section::Patterns => self.patterns(cx),
            Section::Sizes => self.sizes(cx),
        };
        kit::root(cx)
            .bg(cx.theme().colors().editor_background)
            .flex()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, window, _| {
                if e.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .child(self.render_list(width, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.render_top(cx))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .child(div().id("gallery-body").size_full().overflow_y_scroll().track_scroll(&self.scroll).child(v_flex().px_6().pt_1().pb_8().gap_6().children(body)))
                            .child(crate::ui::scrollbar(&self.scroll, &self.grab, cx)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_listed_once() {
        let mut paths: Vec<&str> = ICONS.iter().map(|i| i.path()).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), ICONS.len());
    }

    #[test]
    fn rem_sizes_read_as_drawn() {
        assert_eq!(at_rem(16.), "19");
        assert_eq!(at_rem(22.), "26.1");
    }
}
