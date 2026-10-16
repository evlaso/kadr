//! Pre-recording toolbar: audio/camera toggles and the capture mode, like
//! CleanShot's recording bar. Enter picks a region, Esc cancels.

use gpui::{
    App, Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, WeakEntity, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div,
    layer_shell::*, point, popup::*, px, size,
};
use ui::{Divider, Tooltip, prelude::*};

use crate::record::{self, Opts};
use crate::kit::{self, InputRegion, LayerSpec};
use crate::util;

pub struct Panel {
    opts: Opts,
    /// (source name, description); the empty name is the system default.
    mics: Vec<(String, String)>,
    focus: FocusHandle,
    region: InputRegion,
}

pub fn open(cx: &mut App) -> Option<WindowHandle<Panel>> {
    let spec = LayerSpec {
        namespace: "kadr-panel",
        anchor: Anchor::TOP,
        margin: (40., 0., 0., 0.),
        size: (1060. * kit::SCALE, 80. * kit::SCALE),
        keyboard: KeyboardInteractivity::Exclusive,
        ignore_zones: false,
        monitor: util::focused_monitor(),
    };
    kit::open_layer(cx, spec, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            Panel::new(focus)
        })
    })
}

impl Panel {
    fn new(focus: FocusHandle) -> Self {
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
        Panel { opts, mics, focus, region: InputRegion::default() }
    }

    fn choose(&mut self, mode: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let opts = self.opts.clone();
        window.remove_window();
        record::panel_done(mode.map(|m| (m, opts)), cx);
    }

    fn open_mic_menu(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let panel = cx.weak_entity();
        let entries = self.mics.clone();
        let selected = self.opts.mic_device.clone();
        let height = (12. + 34. * entries.len() as f32) * kit::SCALE;
        let options = WindowOptions {
            titlebar: None,
            window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(0.), px(0.)), size: size(px(380. * kit::SCALE), px(height)) })),
            window_background: WindowBackgroundAppearance::Transparent,
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
        self.region.reset();
        let toggle = |id: &'static str, icon: Icon, label: &'static str, on: bool| {
            Button::new(id, label)
                .style(ButtonStyle::Subtle)
                .size(ButtonSize::Large)
                .start_icon(icon.size(IconSize::Small).color(if on { Color::Default } else { Color::Muted }))
                .color(if on { Color::Default } else { Color::Muted })
                .toggle_state(on)
        };
        let mic_desc = self.mics.iter().find(|(n, _)| *n == self.opts.mic_device).map(|(_, d)| d.clone()).unwrap_or_default();

        let mic = h_flex()
            .child(
                toggle("mic", Icon::new(IconName::Mic), "Микрофон", self.opts.mic)
                    .tooltip(Tooltip::text(mic_desc))
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
                            .size(ButtonSize::Large)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Выбрать микрофон")),
                    ),
            );

        let bar = h_flex()
            .relative()
            .gap_1()
            .p_1()
            .elevation_3(cx)
            .child(mic)
            .child(toggle("audio", Icon::new(IconName::AudioOn), "Звук системы", self.opts.audio).on_click(cx.listener(|this, _, _, cx| {
                this.opts.audio = !this.opts.audio;
                cx.notify();
            })))
            .child(toggle("webcam", Icon::from_path("icons/kadr/camera.svg"), "Камера", self.opts.webcam).on_click(cx.listener(|this, _, _, cx| {
                this.opts.webcam = !this.opts.webcam;
                cx.notify();
            })))
            .child(
                toggle("keys", Icon::new(IconName::Keyboard), "Клавиши", self.opts.keys)
                    .tooltip(Tooltip::text("Показать сочетания клавиш в «Улучшить». Обычный набор текста не записывается"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.opts.keys = !this.opts.keys;
                        cx.notify();
                    })),
            )
            .child(Divider::vertical().h_5().mx_1())
            .child(
                Button::new("region", "Записать область")
                    .style(ButtonStyle::Filled)
                    .size(ButtonSize::Large)
                    .start_icon(Icon::new(IconName::Crosshair).size(IconSize::Small).color(Color::Error))
                    .key_binding(kit::keys("enter"))
                    .on_click(cx.listener(|this, _, window, cx| this.choose(Some("region"), window, cx))),
            )
            .child(
                Button::new("fullscreen", "Весь экран")
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Large)
                    .start_icon(Icon::new(IconName::Screen).size(IconSize::Small))
                    .on_click(cx.listener(|this, _, window, cx| this.choose(Some("fullscreen"), window, cx))),
            )
            .child(
                Button::new("cancel", "Отмена")
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Large)
                    .color(Color::Muted)
                    .key_binding(kit::keys("escape"))
                    .on_click(cx.listener(|this, _, window, cx| this.choose(None, window, cx))),
            )
            .child(self.region.mark());

        kit::root(cx)
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
                "escape" => this.choose(None, window, cx),
                "enter" => this.choose(Some("region"), window, cx),
                _ => {}
            }))
            .child(bar)
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
