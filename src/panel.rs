//! The recording bar, laid out like CleanShot's: you pick the area first,
//! then the bar sits right under it (above, or inside, when there's no room)
//! with the audio, camera and keys toggles and Record. The rest of the
//! monitor is dimmed so the area stays obvious.

use crate::kit::{Anchor, KeyboardInteractivity};
use gpui::{
    AnyView, App, Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, WeakEntity, Window,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, div,
    point, popup::*, px, rgba, size,
};
use crate::ui::{ButtonLike, Divider, Tooltip, prelude::*};

use crate::kit::{self, InputRegion, LayerSpec};
use crate::record::{self, Opts, PanelChoice, Target};
use crate::util::{self, Mon};
use crate::i18n::t;

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
    /// The microphone's level 0..1 while it's on, and the `parec` reading it
    /// (with the device it reads).
    level: f32,
    meter: Option<(String, u32)>,
    /// Only the bar takes the pointer: the camera bubble and the windows
    /// around stay reachable while the panel is up.
    region: InputRegion,
    /// Re-render when the camera bubble's state changes (it's drawn here).
    watching_camera: Option<gpui::EntityId>,
    focus: FocusHandle,
}

impl Drop for Panel {
    fn drop(&mut self) {
        if let Some((_, pid)) = self.meter.take() {
            util::signal(pid, util::Signal::Terminate);
        }
    }
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
    let shown_mon = mon.clone();
    let handle = kit::open_layer(cx, spec, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            Panel::new(target, mon, rect, focus)
        })
    });
    // The camera shows up right away, to set it up before recording.
    if handle.is_some() && Opts::load().webcam {
        crate::webcam::show(&shown_mon, rect, true, cx);
    }
    handle
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
            .unwrap_or_else(|| t("record_bar.system").into());
        let mut mics = vec![(String::new(), tr!("record_bar.default_mic", default_desc))];
        mics.extend(found);
        Panel { opts, mics, target, mon, rect, level: 0., meter: None, region: InputRegion::default(), watching_camera: None, focus }
    }

    /// Keep a level meter on the chosen microphone while it's on.
    fn sync_meter(&mut self, cx: &mut Context<Self>) {
        let want = self.opts.mic.then(|| self.opts.mic_device.clone());
        if self.meter.as_ref().map(|(d, _)| d) == want.as_ref() {
            return;
        }
        if let Some((_, pid)) = self.meter.take() {
            util::signal(pid, util::Signal::Terminate);
        }
        self.level = 0.;
        let Some(device) = want else { return };
        let source = if device.is_empty() { "@DEFAULT_SOURCE@".to_owned() } else { device.clone() };
        let Ok(mut child) = std::process::Command::new("parec")
            .args([format!("--device={source}"), "--format=s16le".into(), "--rate=16000".into(), "--channels=1".into(), "--latency-msec=40".into()])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return;
        };
        self.meter = Some((device, child.id()));
        let (mut tx, mut rx) = futures::channel::mpsc::channel::<f32>(4);
        std::thread::spawn(move || {
            use std::io::Read;
            let Some(mut out) = child.stdout.take() else { return };
            // 50 ms of samples per reading.
            let mut buf = [0u8; 1600];
            while out.read_exact(&mut buf).is_ok() {
                let sum: f64 = buf.chunks_exact(2).map(|b| (i16::from_le_bytes([b[0], b[1]]) as f64 / 32768.).powi(2)).sum();
                let rms = (sum / 800.).sqrt().max(1e-6);
                let level = ((20. * rms.log10() + 55.) / 50.).clamp(0., 1.) as f32;
                if futures::executor::block_on(futures::SinkExt::send(&mut tx, level)).is_err() {
                    break;
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        });
        cx.spawn(async move |this, cx| {
            while let Some(level) = futures::StreamExt::next(&mut rx).await {
                let alive = this.update(cx, |panel, cx| {
                    // Up at once, down slowly, like a VU meter.
                    panel.level = level.max(panel.level * 0.75);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
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
        // Exactly the rows, their gaps, the padding and the border: the popup
        // can't size itself to its content.
        let n = entries.len().max(1) as f32;
        let height = n * MENU_ROW + (n - 1.) * MENU_GAP + 2. * MENU_PAD + 2.;
        let mut options = WindowOptions {
            titlebar: None,
            window_bounds: Some(WindowBounds::Windowed(Bounds { origin: point(px(0.), px(0.)), size: size(px(380. * kit::SCALE), px(height)) })),
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
        crate::platform::transparent(&mut options);
        if let Err(e) = cx.open_window(options, |window, cx| {
            window.set_rem_size(px(kit::REM));
            cx.new(|_| MicMenu { panel, entries, selected })
        }) {
            util::notify(t("record_bar.kadr_menu_didnt_open"), &e.to_string());
        }
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // While the panel is up it draws the camera bubble itself, so the
        // bubble surely gets the pointer.
        let camera = crate::webcam::state(cx).filter(|_| self.opts.webcam);
        if let Some(state) = &camera
            && self.watching_camera != Some(state.entity_id())
        {
            self.watching_camera = Some(state.entity_id());
            cx.observe(state, |_, _, cx| cx.notify()).detach();
        }
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
                .child(div().px_1().child(icon.size(IconSize::Medium)))
        };
        let action = |id: &'static str, icon: Icon| {
            ButtonLike::new(id).style(ButtonStyle::Subtle).size(ButtonSize::Large).child(div().px_1().child(icon.size(IconSize::Medium)))
        };
        let mic_desc = self.mics.iter().find(|(n, _)| *n == self.opts.mic_device).map(|(_, d)| SharedString::from(d.clone()));
        let is_region = matches!(self.target, Target::Region { .. });
        let (x, y, w, h) = self.rect;
        let (mw, mh) = (self.mon.w as f32, self.mon.h as f32);
        let on_off = |on: bool| Some(SharedString::from(if on { t("record_bar.on") } else { t("record_bar.off") }));

        self.sync_meter(cx);
        self.region.reset();
        // While the microphone is on, its button fills from the bottom with
        // how loud it is, like in call apps.
        let level = self.level;
        let mut fill = cx.theme().colors().text_accent;
        fill.alpha = 0.45;
        let height = 32. * (level.clamp(0., 1.));
        let mic = h_flex()
            .child(
                div()
                    .relative()
                    .when(self.opts.mic && height > 1., |d| {
                        d.child(div().absolute().left_0().right_0().bottom_0().h(px(height)).rounded_b(px(9.)).when(height > 23., |d| d.rounded_t(px(9. - (32. - height)))).bg(fill))
                    })
                    .child(
                        ButtonLike::new("mic")
                            .style(if self.opts.mic { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                            .size(ButtonSize::Large)
                            .toggle_state(self.opts.mic)
                            .child(div().px_1().child(Icon::new(IconName::Mic).size(IconSize::Medium)))
                            .tooltip(tip(t("record_bar.microphone"), if self.opts.mic { mic_desc } else { on_off(false) }, None))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.opts.mic = !this.opts.mic;
                                cx.notify();
                            })),
                    ),
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
                            .icon_size(IconSize::XSmall)
                            .tooltip(Tooltip::text(t("record_bar.choose_microphone"))),
                    ),
            );

        let bar = h_flex()
            .relative()
            .gap_0p5()
            .p_1()
            .elevation_3(cx)
            .child(self.region.mark())
            .child(
                action("cancel", Icon::new(IconName::Close))
                    .tooltip(tip(t("common.cancel"), None, Some("escape")))
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
            .child(toggle("audio", Icon::new(IconName::AudioOn), self.opts.audio).tooltip(tip(t("record_bar.system_audio"), on_off(self.opts.audio), None)).on_click(cx.listener(|this, _, _, cx| {
                this.opts.audio = !this.opts.audio;
                cx.notify();
            })))
            .child(toggle("webcam", Icon::from_path("icons/kadr/camera.svg"), self.opts.webcam).tooltip(tip(t("common.camera"), on_off(self.opts.webcam), None)).on_click(cx.listener(|this, _, _, cx| {
                this.opts.webcam = !this.opts.webcam;
                if this.opts.webcam {
                    crate::webcam::show(&this.mon, this.rect, true, cx);
                } else {
                    crate::webcam::hide(cx);
                }
                cx.notify();
            })))
            .child(
                toggle("keys", Icon::new(IconName::Keyboard), self.opts.keys)
                    .tooltip(tip(t("common.keyboard_shortcuts"), Some(t("record_bar.shown_in_studio_plain_typing").into()), None))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.opts.keys = !this.opts.keys;
                        cx.notify();
                    })),
            )
            .child(
                toggle("studio", Icon::new(IconName::Sparkle), self.opts.studio)
                    .tooltip(tip(t("record_bar.studio_mode"), Some(t("record_bar.kadr_draws_cursor_large_smooth").into()), None))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.opts.studio = !this.opts.studio;
                        cx.notify();
                    })),
            )
            .child({
                // Off → 3 s → 5 s: counts down over the area before recording.
                let n = self.opts.countdown;
                ButtonLike::new("countdown")
                    .style(if n > 0 { ButtonStyle::Filled } else { ButtonStyle::Subtle })
                    .size(ButtonSize::Large)
                    .toggle_state(n > 0)
                    .child(
                        h_flex()
                            .px_1()
                            .gap_1()
                            .child(Icon::from_path("icons/kadr/timer.svg").size(IconSize::Medium))
                            .when(n > 0, |row| row.child(Label::new(format!("{n}")).size(LabelSize::Small))),
                    )
                    .tooltip(tip(t("record_bar.countdown_before_recording"), Some(if n > 0 { tr!("record_bar.s_esc_cancels", n).into() } else { t("record_bar.countdown_off").into() }), None))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let i = record::COUNTDOWNS.iter().position(|&c| c == this.opts.countdown).unwrap_or(0);
                        this.opts.countdown = record::COUNTDOWNS[(i + 1) % record::COUNTDOWNS.len()];
                        cx.notify();
                    }))
            })
            .child(Divider::vertical().h_5().mx_1())
            .child(
                action("repick", Icon::new(IconName::Crosshair))
                    .tooltip(tip(t("record_bar.another_area"), None, Some("r")))
                    .on_click(cx.listener(|this, _, window, cx| this.choose(PanelChoice::Repick, window, cx))),
            )
            .when(is_region, |bar| {
                bar.child(
                    action("fullscreen", Icon::new(IconName::Screen))
                        .tooltip(tip(t("common.fullscreen"), None, Some("f")))
                        .on_click(cx.listener(|this, _, window, cx| {
                            let choice = this.fullscreen();
                            this.choose(choice, window, cx)
                        })),
                )
            })
            .child(div().w_1())
            .child(
                Button::new("record", t("record_bar.record"))
                    .style(ButtonStyle::Accent)
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
                    .child(div().absolute().left(px(x - 2.)).top(px(y - 2.)).w(px(w + 4.)).h(px(h + 4.)).border_2().border_color(gpui::white()))
            })
            .child(div().absolute().left(px(bar_x)).top(px(bar_y)).w(px(room)).flex().justify_center().child(bar))
            .children(camera.as_ref().map(|state| crate::webcam::bubble(state, true, Some(self.region.mark()), window, cx)))
            .when_some(camera.clone(), |root, state| {
                let released = state.clone();
                root.on_mouse_move(move |e: &MouseMoveEvent, _, cx| crate::webcam::pointer_moved(&state, e.position, cx))
                    .on_mouse_up(MouseButton::Left, move |_, _, cx| crate::webcam::pointer_released(&released, cx))
            })
    }
}

/// Native popup listing input devices; picking one also turns the mic on.
struct MicMenu {
    panel: WeakEntity<Panel>,
    entries: Vec<(String, String)>,
    selected: String,
}

/// Microphone menu metrics, shared by the popup's size and its layout.
const MENU_ROW: f32 = 32. * kit::SCALE;
const MENU_GAP: f32 = 2. * kit::SCALE;
const MENU_PAD: f32 = 5. * kit::SCALE;

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
                .h(px(MENU_ROW))
                .flex_none()
                .px_2()
                .gap_2()
                .rounded(crate::ui::rems_from_px(7.))
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
        kit::root(cx).child(v_flex().size_full().p(px(MENU_PAD)).gap(px(MENU_GAP)).elevation_2(cx).children(rows))
    }
}
