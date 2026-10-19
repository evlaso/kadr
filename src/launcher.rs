//! `kadr capture`: every mode on one bar, like CleanShot's all-in-one, so
//! one hotkey is enough. Pick with a click, the arrows and Enter, or 1–8;
//! Esc or a click outside closes it.

use gpui::{App, Context, FocusHandle, Global, KeyDownEvent, MouseButton, Window, WindowHandle, div, px};

use crate::kit::{self, Anchor, KeyboardInteractivity, LayerSpec};
use crate::ui::{ButtonLike, Divider, prelude::*};
use crate::util;
use crate::i18n::t;

/// A mode: its icon, its name and the `kadr` arguments it runs.
struct Mode {
    icon: &'static str,
    label: &'static str,
    args: &'static [&'static str],
}

const MODES: [Mode; 8] = [
    Mode { icon: "icons/kadr/area.svg", label: "capture_bar.area", args: &["shot", "region"] },
    Mode { icon: "icons/kadr/previous_area.svg", label: "capture_bar.previous", args: &["shot", "previous"] },
    Mode { icon: "icons/screen.svg", label: "common.fullscreen", args: &["shot", "fullscreen"] },
    Mode { icon: "icons/kadr/window.svg", label: "common.window", args: &["shot", "windows"] },
    Mode { icon: "icons/kadr/scroll.svg", label: "capture_bar.scrolling", args: &["shot", "scroll"] },
    Mode { icon: "icons/kadr/timer.svg", label: "capture_bar.timer", args: &["shot", "timer"] },
    Mode { icon: "icons/case_sensitive.svg", label: "common.text", args: &["text"] },
    Mode { icon: "icons/video.svg", label: "common.recording", args: &["record"] },
];

/// Groups end after these: screenshots, then text, then recording.
const BREAKS: [usize; 2] = [5, 6];

#[derive(Default)]
struct Open(Option<WindowHandle<Launcher>>);

impl Global for Open {}

/// Show the bar, or close it when it's already up (the same hotkey).
pub fn toggle(cx: &mut App) {
    if let Some(handle) = cx.try_global::<Open>().and_then(|o| o.0) {
        cx.set_global(Open(None));
        if handle.update(cx, |_, window, _| window.remove_window()).is_ok() {
            return;
        }
    }
    let Some(mon) = util::focused_monitor() else { return };
    let spec = LayerSpec {
        namespace: "kadr-capture",
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        margin: (0., 0., 0., 0.),
        size: (mon.w as f32, mon.h as f32),
        keyboard: KeyboardInteractivity::Exclusive,
        ignore_zones: true,
        monitor: Some(mon),
    };
    let handle = kit::open_layer(cx, spec, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            Launcher { selected: 0, focus }
        })
    });
    cx.set_global(Open(handle));
}

struct Launcher {
    selected: usize,
    focus: FocusHandle,
}

impl Launcher {
    /// Close the bar, then run the mode once it's off the screen.
    fn run(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        cx.set_global(Open(None));
        let args: Vec<String> = MODES[i].args.iter().map(|a| a.to_string()).collect();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(150)).await;
            let _ = cx.update(|cx| crate::run_args(&args, cx));
        })
        .detach();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        cx.set_global(Open(None));
    }
}

impl Render for Launcher {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = h_flex().gap_1().p_1p5().elevation_3(cx);
        for (i, mode) in MODES.iter().enumerate() {
            let on = i == self.selected;
            bar = bar.child(
                ButtonLike::new(("capture-mode", i))
                    .style(if on { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .toggle_state(on)
                    .child(
                        v_flex()
                            .w(px(84. * kit::SCALE))
                            .py_2()
                            .gap_1p5()
                            .items_center()
                            .child(Icon::from_path(mode.icon).size(IconSize::Large))
                            .child(Label::new(t(mode.label)).color(if on { Color::Default } else { Color::Muted })),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.run(i, window, cx))),
            );
            if BREAKS.contains(&i) {
                bar = bar.child(Divider::vertical().h(px(44.)).mx_1());
            }
        }
        let hint = Label::new(t("capture_bar.keys_enter_esc_close")).size(LabelSize::Small).color(Color::Muted);
        kit::root(cx)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                let key = e.keystroke.key.as_str();
                match key {
                    "escape" => this.close(window, cx),
                    "left" => this.selected = (this.selected + MODES.len() - 1) % MODES.len(),
                    "right" | "tab" => this.selected = (this.selected + 1) % MODES.len(),
                    "enter" | "space" => this.run(this.selected, window, cx),
                    _ => {
                        if let Some(n) = key.parse::<usize>().ok().filter(|n| (1..=MODES.len()).contains(n)) {
                            this.run(n - 1, window, cx);
                        }
                    }
                }
                cx.notify();
            }))
            // A click anywhere but the bar closes it.
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close(window, cx)))
            .flex()
            .flex_col()
            .items_center()
            .justify_end()
            .pb(px(96. * kit::SCALE))
            .gap_2()
            .child(div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(bar))
            .child(hint)
    }
}
