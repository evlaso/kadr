//! The recording bar, laid out like CleanShot's: you pick the area first,
//! then the bar sits right under it (above, or inside, when there's no room)
//! with the area's size, the audio, camera and keys toggles and Record. The
//! area stays adjustable meanwhile: knobs on its corners and edges, dragged
//! from inside, nudged with the arrows (Alt+arrows resize), or set to a size
//! from the size menu or typed. The rest of the monitor is dimmed so the
//! area stays obvious.

use crate::kit::{Anchor, KeyboardInteractivity};
use gpui::{
    AnyView, App, Bounds, Context, CursorStyle, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, WeakEntity,
    Window, WindowBounds, WindowHandle, WindowKind, WindowOptions, canvas, deferred, div, point, popup::*, px, rgba, size,
};
use crate::ui::{ButtonLike, Divider, Tooltip, prelude::*};

use crate::kit::{self, InputRegion, LayerSpec};
use crate::picker::{BOTTOM, LEFT, RIGHT, Rect, TOP};
use crate::record::{self, Opts, PanelChoice, Target};
use crate::util::{self, Mon};
use crate::i18n::t;

/// Height of the bar, for placing it around the area.
const BAR_HEIGHT: f32 = 56. * kit::SCALE;
const BAR_ROOM: f32 = 760. * kit::SCALE;
/// The area's smallest side, in logical pixels.
const MIN_SIDE: f32 = 16.;
/// The square around a knob that grabs it.
const KNOB_REACH: f32 = 22.;
/// Sizes the size menu offers, in real pixels.
const PRESETS: [(i64, i64); 5] = [(640, 480), (1280, 720), (1920, 1080), (1080, 1080), (1080, 1920)];

pub struct Panel {
    opts: Opts,
    /// (source name, description); the empty name is the system default.
    mics: Vec<(String, String)>,
    mon: Mon,
    /// The area in this monitor's coordinates.
    rect: Rect,
    /// A part of the monitor rather than all of it: dimmed around, with knobs.
    part: bool,
    /// Moving (no edges) or resizing the area: the edges, the area and the
    /// pointer when it started.
    drag: Option<(u8, Rect, (f32, f32))>,
    size_menu: bool,
    /// A size being typed, in real pixels: width, height, on the height.
    typing: Option<(String, String, bool)>,
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
            Panel::new(&target, mon, rect, focus)
        })
    });
    // The camera shows up right away, to set it up before recording.
    if handle.is_some() && Opts::load().webcam {
        crate::webcam::show(&shown_mon, rect, true, cx);
    }
    handle
}

impl Panel {
    fn new(target: &Target, mon: Mon, rect: Rect, focus: FocusHandle) -> Self {
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
        let part = matches!(target, Target::Region { .. });
        Panel {
            opts,
            mics,
            mon,
            rect,
            part,
            drag: None,
            size_menu: false,
            typing: None,
            level: 0.,
            meter: None,
            region: InputRegion::default(),
            watching_camera: None,
            focus,
        }
    }

    /// What gets recorded: the area, or the monitor when it's all of it.
    fn target(&self) -> Target {
        let (x, y, w, h) = self.rect;
        let whole = x < 0.5 && y < 0.5 && w > self.mon.w as f32 - 1. && h > self.mon.h as f32 - 1.;
        if !self.part || whole {
            return Target::Monitor(self.mon.name.clone());
        }
        Target::Region { x: self.mon.x + x.round() as i32, y: self.mon.y + y.round() as i32, w: w.round() as i32, h: h.round() as i32 }
    }

    /// Kept inside the monitor and no smaller than `MIN_SIDE`.
    fn clamp(&self, (x, y, w, h): Rect) -> Rect {
        let (mw, mh) = (self.mon.w as f32, self.mon.h as f32);
        let (w, h) = (w.clamp(MIN_SIDE, mw), h.clamp(MIN_SIDE, mh));
        (x.clamp(0., mw - w), y.clamp(0., mh - h), w, h)
    }

    fn set_rect(&mut self, rect: Rect, cx: &mut Context<Self>) {
        self.rect = self.clamp(rect);
        self.part = true;
        // The camera bubble keeps to the area.
        if self.opts.webcam {
            crate::webcam::show(&self.mon, self.rect, true, cx);
        }
        cx.notify();
    }

    /// `edges` of `from` moved by (dx, dy); no edges moves it whole.
    fn dragged(&self, edges: u8, (x, y, w, h): Rect, (dx, dy): (f32, f32)) -> Rect {
        if edges == 0 {
            return (x + dx, y + dy, w, h);
        }
        let (mw, mh) = (self.mon.w as f32, self.mon.h as f32);
        let (mut l, mut t, mut r, mut b) = (x, y, x + w, y + h);
        if edges & LEFT != 0 {
            l = (l + dx).clamp(0., r - MIN_SIDE);
        }
        if edges & RIGHT != 0 {
            r = (r + dx).clamp(l + MIN_SIDE, mw);
        }
        if edges & TOP != 0 {
            t = (t + dy).clamp(0., b - MIN_SIDE);
        }
        if edges & BOTTOM != 0 {
            b = (b + dy).clamp(t + MIN_SIDE, mh);
        }
        (l, t, r - l, b - t)
    }

    /// The area at a size in real pixels, around its centre.
    fn resize_to(&mut self, (pw, ph): (f32, f32), cx: &mut Context<Self>) {
        let s = self.mon.scale as f32;
        let (x, y, w, h) = self.rect;
        let (nw, nh) = ((pw / s).min(self.mon.w as f32), (ph / s).min(self.mon.h as f32));
        self.set_rect((x + w / 2. - nw / 2., y + h / 2. - nh / 2., nw, nh), cx);
    }

    /// The typed size; a missing side stays as it is.
    fn apply_typed(&mut self, cx: &mut Context<Self>) {
        let Some((w, h, _)) = self.typing.take() else { return };
        let s = self.mon.scale as f32;
        let (_, _, cw, ch) = self.rect;
        let side = |v: &str, now: f32| v.parse::<f32>().ok().filter(|v| *v > 0.).unwrap_or(now * s);
        self.resize_to((side(&w, cw), side(&h, ch)), cx);
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        let (shift, alt) = (e.keystroke.modifiers.shift, e.keystroke.modifiers.alt);
        // A size being typed: digits, then x (or Tab) and the height.
        if let Some((w, h, on_h)) = self.typing.as_mut() {
            let field = if *on_h { h } else { w };
            match key {
                "escape" => self.typing = None,
                "enter" => self.apply_typed(cx),
                "x" | "tab" | "space" => *on_h = true,
                "backspace" if field.is_empty() => *on_h = false,
                "backspace" => {
                    field.pop();
                }
                k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) && field.len() < 5 => field.push_str(k),
                _ => {}
            }
            return cx.notify();
        }
        match key {
            "escape" if self.size_menu => self.size_menu = false,
            "escape" => self.choose(PanelChoice::Cancel, window, cx),
            "enter" => {
                let target = self.target();
                self.choose(PanelChoice::Record(target), window, cx)
            }
            "r" => self.choose(PanelChoice::Repick, window, cx),
            "f" => {
                let choice = self.fullscreen();
                self.choose(choice, window, cx)
            }
            "left" | "right" | "up" | "down" if self.part => {
                let step = if shift { 10. } else { 1. };
                let d = match key {
                    "left" => (-step, 0.),
                    "right" => (step, 0.),
                    "up" => (0., -step),
                    _ => (0., step),
                };
                // Alt: the right and bottom edges grow and shrink.
                let next = self.dragged(if alt { RIGHT | BOTTOM } else { 0 }, self.rect, d);
                self.set_rect(next, cx);
            }
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => {
                self.size_menu = false;
                self.typing = Some((k.to_owned(), String::new(), false));
            }
            _ => {}
        }
        cx.notify();
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

    /// Under the area; above it, or inside its bottom edge, when there's no room.
    fn bar_y(&self) -> f32 {
        let (_, y, _, h) = self.rect;
        let (gap, mh) = (14., self.mon.h as f32);
        if y + h + gap + BAR_HEIGHT <= mh - 8. {
            y + h + gap
        } else if y - gap - BAR_HEIGHT >= 8. {
            y - gap - BAR_HEIGHT
        } else {
            y + h - BAR_HEIGHT - 24.
        }
    }

    /// The area's size in real pixels, a menu of sizes under it (or above,
    /// short of room) and a typed size while it's typed.
    fn size_button(&self, bar_y: f32, cx: &mut Context<Self>) -> AnyElement {
        let s = self.mon.scale as f32;
        let (_, _, w, h) = self.rect;
        let current = ((w * s).round(), (h * s).round());
        let text = match &self.typing {
            Some((tw, th, on_h)) => format!("{}{} × {}{}", tw, if *on_h { "" } else { "|" }, th, if *on_h { "|" } else { "" }),
            None => format!("{} × {}", current.0, current.1),
        };
        let button = ButtonLike::new("size")
            .style(if self.size_menu || self.typing.is_some() { ButtonStyle::Filled } else { ButtonStyle::Subtle })
            .size(ButtonSize::Large)
            .toggle_state(self.size_menu)
            .child(
                h_flex()
                    .px_1()
                    .gap_1()
                    .child(div().font_family(kit::MONO_FONT).text_sm().child(text))
                    .child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall).color(Color::Muted)),
            )
            .tooltip(Tooltip::text(t("record_bar.area_size")))
            .on_click(cx.listener(|this, _, _, cx| {
                this.typing = None;
                this.size_menu = !this.size_menu;
                cx.notify();
            }));

        let menu = self.size_menu.then(|| {
            let colors = cx.theme().colors();
            let (hover, selected) = (colors.element_hover, colors.element_selected);
            let phys = (self.mon.phys.0 as f32, self.mon.phys.1 as f32);
            let mut sizes: Vec<((f32, f32), Option<SharedString>)> = PRESETS.iter().map(|&(pw, ph)| ((pw as f32, ph as f32), None)).collect();
            sizes.push((phys, Some(t("common.fullscreen").into())));
            let mut rows: Vec<AnyElement> = sizes
                .into_iter()
                .enumerate()
                .map(|(i, (size, hint))| {
                    let checked = (size.0.min(phys.0), size.1.min(phys.1)) == current;
                    h_flex()
                        .id(("size", i))
                        .h(px(MENU_ROW))
                        .flex_none()
                        .px_2()
                        .gap_2()
                        .rounded(crate::ui::rems_from_px(7.))
                        .cursor_pointer()
                        .when(checked, |row| row.bg(selected))
                        .hover(move |st| st.bg(hover))
                        .child(Icon::new(IconName::Check).size(IconSize::Small).color(if checked { Color::Accent } else { Color::Custom(gpui::transparent_black()) }))
                        .child(Label::new(format!("{} × {}", size.0, size.1)))
                        .children(hint.map(|hint| Label::new(hint).color(Color::Muted)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.size_menu = false;
                            this.resize_to(size, cx);
                        }))
                        .into_any_element()
                })
                .collect();
            rows.push(
                h_flex()
                    .id("size-custom")
                    .h(px(MENU_ROW))
                    .flex_none()
                    .px_2()
                    .gap_2()
                    .rounded(crate::ui::rems_from_px(7.))
                    .cursor_pointer()
                    .hover(move |st| st.bg(hover))
                    .child(Icon::new(IconName::Check).size(IconSize::Small).color(Color::Custom(gpui::transparent_black())))
                    .child(Label::new(t("record_bar.custom_size")))
                    .child(Label::new(t("record_bar.type_digits")).color(Color::Muted))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.size_menu = false;
                        this.typing = Some((String::new(), String::new(), false));
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            let n = rows.len() as f32;
            let height = n * MENU_ROW + (n - 1.) * MENU_GAP + 2. * MENU_PAD;
            // Down when it fits under the bar, else up.
            let below = bar_y + BAR_HEIGHT + height + 8. < self.mon.h as f32;
            deferred(
                v_flex()
                    .id("size-menu")
                    .occlude()
                    .absolute()
                    .left_0()
                    .when(below, |d| d.top(px(BAR_HEIGHT - 8.)))
                    .when(!below, |d| d.bottom(px(BAR_HEIGHT - 8.)))
                    .w(px(240. * kit::SCALE))
                    .p(px(MENU_PAD))
                    .gap(px(MENU_GAP))
                    .elevation_2(cx)
                    .child(self.region.mark())
                    .children(rows),
            )
        });
        div().relative().child(button).children(menu).into_any_element()
    }

    /// The area's frame and knobs, and what grabs them.
    fn render_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (x, y, w, h) = self.rect;
        let accent = cx.theme().colors().text_accent;
        let knobs = crate::picker::handles(self.rect);
        let paint = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                let at = |px_: f32, py: f32| point(o.x + px(px_), o.y + px(py));
                window.paint_quad(crate::ui::outline(Bounds::new(at(x - 1., y - 1.), size(px(w + 2.), px(h + 2.))), gpui::white(), gpui::BorderStyle::Solid));
                for (_, (kx, ky)) in knobs {
                    crate::ui::paint_knob(window, at(kx, ky), accent);
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();
        let moving = matches!(self.drag, Some((0, ..)));
        let body = div()
            .id("area")
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .h(px(h))
            .cursor(if moving { CursorStyle::ClosedHand } else { CursorStyle::OpenHand })
            .child(self.region.mark())
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
                this.size_menu = false;
                this.drag = Some((0, this.rect, (f32::from(e.position.x), f32::from(e.position.y))));
                cx.notify();
            }));
        let grips = knobs.map(|(edges, (kx, ky))| {
            div()
                .id(("knob", edges as usize))
                .absolute()
                .left(px(kx - KNOB_REACH / 2.))
                .top(px(ky - KNOB_REACH / 2.))
                .size(px(KNOB_REACH))
                .cursor(crate::picker::resize_cursor(edges))
                .child(self.region.mark())
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.size_menu = false;
                    this.drag = Some((edges, this.rect, (f32::from(e.position.x), f32::from(e.position.y))));
                    cx.notify();
                }))
        });
        div().absolute().inset_0().child(body).child(paint).children(grips)
    }

    fn choose(&mut self, choice: PanelChoice, window: &mut Window, cx: &mut Context<Self>) {
        let opts = self.opts.clone();
        // The adjusted area is what "previous area" brings back.
        if let PanelChoice::Record(Target::Region { x, y, w, h }) = &choice {
            crate::picker::remember(&self.mon.name, (*x, *y, *w, *h));
        }
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
        let is_region = self.part;
        let (x, y, w, h) = self.rect;
        let mw = self.mon.w as f32;
        let on_off = |on: bool| Some(SharedString::from(if on { t("record_bar.on") } else { t("record_bar.off") }));

        self.sync_meter(cx);
        self.region.reset();
        let bar_y = self.bar_y();
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
            // Presses here don't reach the area beneath.
            .occlude()
            .gap_0p5()
            .p_1()
            .elevation_3(cx)
            .child(self.region.mark())
            .child(
                action("cancel", Icon::new(IconName::Close))
                    .tooltip(tip(t("common.cancel"), None, Some("escape")))
                    .on_click(cx.listener(|this, _, window, cx| this.choose(PanelChoice::Cancel, window, cx))),
            )
            .child(self.size_button(bar_y, cx))
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
                        let target = this.target();
                        this.choose(PanelChoice::Record(target), window, cx)
                    })),
            );

        let room = BAR_ROOM.min(mw);
        let bar_x = (x + w / 2. - room / 2.).clamp(0., (mw - room).max(0.));

        let shade = rgba(0x00000073);
        let dim = || div().absolute().bg(shade);
        kit::root(cx)
            .relative()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            // A press on the area or a knob holds the pointer, so the drag
            // goes on past the input region.
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                let Some((edges, from, start)) = this.drag else { return };
                if e.pressed_button != Some(MouseButton::Left) {
                    this.drag = None;
                    return cx.notify();
                }
                let d = (f32::from(e.position.x) - start.0, f32::from(e.position.y) - start.1);
                let next = this.dragged(edges, from, d);
                this.set_rect(next, cx);
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
                if this.drag.take().is_some() {
                    cx.notify();
                }
            }))
            .when(is_region, |root| {
                root.child(dim().top_0().left_0().right_0().h(px(y)))
                    .child(dim().top(px(y + h)).left_0().right_0().bottom_0())
                    .child(dim().top(px(y)).left_0().w(px(x)).h(px(h)))
                    .child(dim().top(px(y)).left(px(x + w)).right_0().h(px(h)))
                    .child(self.render_area(cx))
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
