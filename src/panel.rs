//! The recording bar, laid out like CleanShot's: you pick the area first,
//! then the bar sits right under it (above, or inside, when there's no room)
//! with the audio, camera and keys toggles and Record. The rest of the
//! monitor is dimmed so the area stays obvious.

use gpui::{
    AnyView, App, Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, WeakEntity, Window,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, div,
    layer_shell::*, point, popup::*, px, rgba, size,
};
use crate::ui::{ButtonLike, Divider, Tooltip, prelude::*};

use crate::kit::{self, LayerSpec};
use crate::record::{self, Opts, PanelChoice, Target};
use crate::util::{self, Mon};

/// Height of the bar, for placing it around the area.
const BAR_HEIGHT: f32 = 56. * kit::SCALE;
const BAR_ROOM: f32 = 760. * kit::SCALE;

pub struct Panel {
    opts: Opts,
    /// (source name, description); the empty name is the system default.
    mics: Vec<(String, String)>,
    target: Target,
    mon: Mon,
    /// The area in this monitor's coordinates.
    rect: (f32, f32, f32, f32),
    focus: FocusHandle,
}

/// Show the bar for an area that's already been picked.
pub fn open(target: Target, cx: &mut App) -> Option<WindowHandle<Panel>> {
    let mons = util::monitors();
    let mon = match &target {
        Target::Monitor(name) => mons.iter().find(|m| &m.name == name),
        Target::Region { x, y, w, h } => mons.iter().find(|m| m.contains(x + w / 2, y + h / 2)),
    }?
    .clone();
    let rect = match target {
        Target::Monitor(_) => (0., 0., mon.w as f32, mon.h as f32),
        Target::Region { x, y, w, h } => ((x - mon.x) as f32, (y - mon.y) as f32, w as f32, h as f32),
    };
    let spec = LayerSpec {
        namespace: "kadr-panel",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mon.w as f32, mon.h as f32),
        keyboard: KeyboardInteractivity::Exclusive,
        ignore_zones: true,
        monitor: Some(mon.clone()),
    };
    kit::open_layer(cx, spec, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            Panel::new(target, mon, rect, focus)
        })
    })
}

impl Panel {
    fn new(target: Target, mon: Mon, rect: (f32, f32, f32, f32), focus: FocusHandle) -> Self {
        let mut opts = Opts::load();
        let found = record::microphones();
        // A remembered device that is gone falls back to the system default.
        if !found.iter().any(|(name, _)| *name == opts.mic_device) {
            opts.mic_device.clear();
        }
        let default_name = std::process::Command::new("pactl")
            .arg("get-default-source")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        let default_desc = found
            .iter()
            .find(|(name, _)| *name == default_name)
            .map(|(_, d)| d.clone())
            .unwrap_or_else(|| "системный".into());
        let mut mics = vec![(String::new(), format!("По умолчанию — {default_desc}"))];
        mics.extend(found);
        Panel { opts, mics, target, mon, rect, focus }
    }

    fn choose(&mut self, choice: PanelChoice, window: &mut Window, cx: &mut Context<Self>) {
        let opts = self.opts.clone();
        window.remove_window();
        record::panel_done(choice, opts, cx);
    }

    fn fullscreen(&self) -> PanelChoice {
        PanelChoice::Record(Target::Monitor(self.mon.name.clone()))
    }

    fn open_mic_menu(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let panel = cx.weak_entity();
        let entries = self.mics.clone();
        let selected = self.opts.mic_device.clone();
        let height = (12. + 34. * entries.len() as f32) * kit::SCALE;
        let options = WindowOptions {
            titlebar: None,
            window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(0.), px(0.)), size: size(px(380. * kit::SCALE), px(height)) })),
            linux_window_background: gpui::LinuxWindowBackground::Transparent,
            kind: WindowKind::AnchoredPopup(PopupOptions {
                parent: window.window_handle(),
                anchor_rect: Bounds::new(event.position, size(px(1.), px(1.))),
                anchor: PopupAnchor::BottomLeft,
                gravity: PopupGravity::BottomRight,
                constraint_adjustment: PopupConstraintAdjustment::FLIP_Y | PopupConstraintAdjustment::SLIDE_X,
                offset: point(px(-12.), px(10.)),
                // Must be requested from the mouse-down that opens it.
                grab: true,
            }),
            ..Default::default()
        };
        if let Err(e) = cx.open_window(options, |window, cx| {
            window.set_rem_size(px(kit::REM));
            cx.new(|_| MicMenu { panel, entries, selected })
        }) {
            util::notify("kadr: меню не открылось", &e.to_string());
        }
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // CleanShot-style: icons only, names and shortcuts in tooltips.
        let tip = |title: &'static str, meta: Option<SharedString>, keys: Option<&'static str>| {
            move |_: &mut Window, cx: &mut App| -> AnyView {
                cx.new(|_| {
                    let tip = Tooltip::new(title).key_binding(keys.map(kit::keys));
                    match meta.clone() {
                        Some(meta) => tip.meta(meta),
                        None => tip,
                    }
                })
                .into()
            }
        };
        let toggle = |id: &'static str, icon: Icon, on: bool| {
            ButtonLike::new(id)
                .style(if on { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                .size(ButtonSize::Large)
                .toggle_state(on)
                .child(div().px_1().child(icon.size(IconSize::Small).color(if on { Color::Default } else { Color::Muted })))
        };
        let action = |id: &'static str, icon: Icon| {
            ButtonLike::new(id).style(ButtonStyle::Subtle).size(ButtonSize::Large).child(div().px_1().child(icon.size(IconSize::Small).color(Color::Muted)))
        };
        let mic_desc = self.mics.iter().find(|(n, _)| *n == self.opts.mic_device).map(|(_, d)| SharedString::from(d.clone()));
        let is_region = matches!(self.target, Target::Region { .. });
        let (x, y, w, h) = self.rect;
        let (mw, mh) = (self.mon.w as f32, self.mon.h as f32);
        let on_off = |on: bool| Some(SharedString::from(if on { "Включено" } else { "Выключено" }));

        let mic = h_flex()
            .child(
                toggle("mic", Icon::new(IconName::Mic), self.opts.mic)
                    .tooltip(tip("Микрофон", if self.opts.mic { mic_desc } else { on_off(false) }, None))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.opts.mic = !this.opts.mic;
                        cx.notify();
                    })),
            )
            .child(
                // Opened on mouse-down: a grabbing popup must be requested
                // while the button is still held.
                div()
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_mic_menu(e, window, cx);
                    }))
                    .child(
                        IconButton::new("mic-menu", IconName::ChevronDown)
                            .size(ButtonSize::Compact)
                            .icon_size(IconSize::XSmall)
                            .icon_color(Color::Muted)
                            .tooltip(Tooltip::text("Выбрать микрофон")),
                    ),
            );

        let bar = h_flex()
            .gap_0p5()
            .p_1()
            .elevation_3(cx)
            .child(
                action("cancel", Icon::new(IconName::Close))
                    .tooltip(tip("Отмена", None, Some("escape")))
                    .on_click(cx.listener(|this, _, window, cx| this.choose(PanelChoice::Cancel, window, cx))),
            )
            .child(
                div()
                    .px_2()
                    .font_family(kit::MONO_FONT)
                    .text_sm()
                    .text_color(cx.theme().colors().text_muted)
                    .child(format!("{}×{}", w.round(), h.round())),
            )
            .child(Divider::vertical().h_5().mx_1())
            .child(mic)
            .child(toggle("audio", Icon::new(IconName::AudioOn), self.opts.audio).tooltip(tip("Звук системы", on_off(self.opts.audio), None)).on_click(cx.listener(|this, _, _, cx| {
                this.opts.audio = !this.opts.audio;
                cx.notify();
            })))
            .child(toggle("webcam", Icon::from_path("icons/kadr/camera.svg"), self.opts.webcam).tooltip(tip("Камера", on_off(self.opts.webcam), None)).on_click(cx.listener(|this, _, _, cx| {
                this.opts.webcam = !this.opts.webcam;
                cx.notify();
            })))
            .child(
                toggle("keys", Icon::new(IconName::Keyboard), self.opts.keys)
                    .tooltip(tip("Сочетания клавиш", Some("Покажутся в студии. Обычный набор текста не записывается".into()), None))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.opts.keys = !this.opts.keys;
                        cx.notify();
                    })),
            )
            .child(
                toggle("studio", Icon::new(IconName::Sparkle), self.opts.studio)
                    .tooltip(tip("Режим студии", Some("Курсор нарисует kadr: крупный и плавный, с настройкой в студии. После записи сразу откроется студия".into()), None))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.opts.studio = !this.opts.studio;
                        cx.notify();
                    })),
            )
            .child(Divider::vertical().h_5().mx_1())
            .child(
                action("repick", Icon::new(IconName::Crosshair))
                    .tooltip(tip("Другая область", None, Some("r")))
                    .on_click(cx.listener(|this, _, window, cx| this.choose(PanelChoice::Repick, window, cx))),
            )
            .when(is_region, |bar| {
                bar.child(
                    action("fullscreen", Icon::new(IconName::Screen))
                        .tooltip(tip("Весь экран", None, Some("f")))
                        .on_click(cx.listener(|this, _, window, cx| {
                            let choice = this.fullscreen();
                            this.choose(choice, window, cx)
                        })),
                )
            })
            .child(div().w_1())
            .child(
                Button::new("record", "Записать")
                    .style(ButtonStyle::Filled)
                    .size(ButtonSize::Large)
                    .start_icon(Icon::new(IconName::Circle).size(IconSize::Small).color(Color::Error))
                    .key_binding(kit::keys("enter"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let target = this.target.clone();
                        this.choose(PanelChoice::Record(target), window, cx)
                    })),
            );

        // Under the area; above it, or inside its bottom edge, when there's no room.
        let gap = 14.;
        let bar_y = if y + h + gap + BAR_HEIGHT <= mh - 8. {
            y + h + gap
        } else if y - gap - BAR_HEIGHT >= 8. {
            y - gap - BAR_HEIGHT
        } else {
            y + h - BAR_HEIGHT - 24.
        };
        let room = BAR_ROOM.min(mw);
        let bar_x = (x + w / 2. - room / 2.).clamp(0., (mw - room).max(0.));

        let shade = rgba(0x00000073);
        let dim = || div().absolute().bg(shade);
        kit::root(cx)
            .relative()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
                "escape" => this.choose(PanelChoice::Cancel, window, cx),
                "enter" => {
                    let target = this.target.clone();
                    this.choose(PanelChoice::Record(target), window, cx)
                }
                "r" => this.choose(PanelChoice::Repick, window, cx),
                "f" => {
                    let choice = this.fullscreen();
                    this.choose(choice, window, cx)
                }
                _ => {}
            }))
            .when(is_region, |root| {
                root.child(dim().top_0().left_0().right_0().h(px(y)))
                    .child(dim().top(px(y + h)).left_0().right_0().bottom_0())
                    .child(dim().top(px(y)).left_0().w(px(x)).h(px(h)))
                    .child(dim().top(px(y)).left(px(x + w)).right_0().h(px(h)))
                    .child(div().absolute().left(px(x - 2.)).top(px(y - 2.)).w(px(w + 4.)).h(px(h + 4.)).border_2().border_color(cx.theme().status().error))
            })
            .child(div().absolute().left(px(bar_x)).top(px(bar_y)).w(px(room)).flex().justify_center().child(bar))
    }
}

/// Native popup listing input devices; picking one also turns the mic on.
struct MicMenu {
    panel: WeakEntity<Panel>,
    entries: Vec<(String, String)>,
    selected: String,
}

impl Render for MicMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let (hover, selected) = (colors.element_hover, colors.element_selected);
        let rows = self.entries.iter().enumerate().map(|(i, (name, desc))| {
            let panel = self.panel.clone();
            let name = name.clone();
            let checked = name == self.selected;
            h_flex()
                .id(i)
                .h(px(32.))
                .px_2()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .when(checked, |row| row.bg(selected))
                .hover(move |s| s.bg(hover))
                .child(Icon::new(IconName::Check).size(IconSize::Small).color(if checked { Color::Accent } else { Color::Custom(gpui::transparent_black()) }))
                .child(Label::new(desc.clone()))
                .on_click(move |_, window, cx| {
                    let _ = panel.update(cx, |panel, cx| {
                        panel.opts.mic_device = name.clone();
                        panel.opts.mic = true;
                        cx.notify();
                    });
                    window.remove_window();
                })
        });
        kit::root(cx).child(v_flex().size_full().p_1().gap_0p5().elevation_2(cx).children(rows))
    }
}
