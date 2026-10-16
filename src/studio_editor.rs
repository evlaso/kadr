//! The studio's video editor, like Screen Studio's: a live preview of the
//! framed, zoomed picture at the playhead, a timeline with the zoom segments
//! (placed on clicks to start with) to move, stretch, add and delete, per
//! segment zoom and focus, the backdrop and overlays, then a quick preview or
//! the export. Edits are saved to the recording's project as they happen.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, BorderStyle, Bounds, Context, CursorStyle, FocusHandle, Image, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, ObjectFit, Window, canvas, div, fill, img, layer_shell::*,
    linear_color_stop, linear_gradient, point, px, rgba, size,
};
use ui::{ButtonLike, Divider, IconButtonShape, Tooltip, prelude::*};

use crate::kit::{self, LayerSpec};
use crate::studio::{self, Camera, PADDINGS, Probe, Project, Seg, ZOOMS};
use crate::track::Track;
use crate::{palette, trim, util};

const FILMSTRIP: f32 = 44.;
const LANE: f32 = 30.;
/// Edge grab width on segments, in pixels.
const EDGE: f32 = 7.;
const MIN_SEG: f64 = 0.3;
const NEW_SEG: f64 = 2.0;

enum Drag {
    Playhead,
    Move { index: usize, grab: f64 },
    Edge { index: usize, start: bool },
}

pub struct StudioEditor {
    video: PathBuf,
    probe: Probe,
    track: Track,
    project: Project,
    camera: Camera,
    thumbs: Vec<Arc<Image>>,
    frame: Option<Arc<Image>>,
    playhead: f64,
    selected: Option<usize>,
    drag: Option<Drag>,
    /// Waiting for a click on the preview to set the selected segment's focus.
    placing: bool,
    /// Preview stage size (16:9, like the output).
    stage: (f32, f32),
    /// Timeline x origin and width, and the segment lane's top, in window
    /// coordinates.
    timeline: Rc<Cell<(f32, f32, f32)>>,
    /// The video box inside the stage, in window coordinates.
    video_box: Rc<Cell<(f32, f32, f32, f32)>>,
    request: Rc<Cell<u64>>,
    focus: FocusHandle,
}

pub fn open(video: PathBuf, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        let Some(probe) = studio::probe(&video).await else {
            return util::notify("Не удалось открыть видео", &video.display().to_string());
        };
        let Some(mon) = util::focused_monitor() else { return };
        let track = Track::load(&video).unwrap_or_else(|| Track { rect: (0.0, 0.0, probe.w, probe.h), ..Default::default() });
        let project = Project::load(&video, &track, &probe);
        let stage_w = (mon.w as f32 * 0.62).min((mon.h as f32 * 0.52) * 16. / 9.);
        let stage = (stage_w, stage_w * 9. / 16.);
        let thumbs = trim::filmstrip(&video, probe.duration).await;
        let frame = trim::frame(&video, 0., (stage.0 * 2.) as u32, 0).await;
        let _ = cx.update(|cx| {
            let spec = LayerSpec {
                namespace: "kadr-studio",
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
                    let camera = Camera::new(&track, &probe, &project.segments);
                    StudioEditor {
                        video,
                        probe,
                        track,
                        project,
                        camera,
                        thumbs,
                        frame,
                        playhead: 0.,
                        selected: None,
                        drag: None,
                        placing: false,
                        stage,
                        timeline: Rc::new(Cell::new((0., 1., 0.))),
                        video_box: Rc::new(Cell::new((0., 0., 1., 1.))),
                        request: Rc::new(Cell::new(0)),
                        focus,
                    }
                })
            });
        });
    })
    .detach();
}

impl StudioEditor {
    /// Apply an edit: rebuild the camera and save the project.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.camera = Camera::new(&self.track, &self.probe, &self.project.segments);
        self.project.save(&self.video);
        cx.notify();
    }

    fn time_at(&self, x: f32) -> f64 {
        let (ox, w, _) = self.timeline.get();
        (((x - ox) / w.max(1.)) as f64).clamp(0., 1.) * self.probe.duration
    }

    fn x_of(&self, t: f64) -> f32 {
        let (_, w, _) = self.timeline.get();
        w * (t / self.probe.duration.max(0.001)) as f32
    }

    fn seek(&mut self, t: f64, cx: &mut Context<Self>) {
        self.playhead = t.clamp(0., self.probe.duration);
        let id = self.request.get() + 1;
        self.request.set(id);
        let (video, width, request, at) = (self.video.clone(), (self.stage.0 * 2.) as u32, self.request.clone(), self.playhead);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(40)).await;
            if request.get() != id {
                return;
            }
            let frame = trim::frame(&video, at, width, id).await;
            if request.get() == id && frame.is_some() {
                let _ = this.update(cx, |editor, cx| {
                    editor.frame = frame;
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    fn timeline_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        let (ox, _, lane_top) = self.timeline.get();
        let (x, y) = (f32::from(e.position.x), f32::from(e.position.y));
        let t = self.time_at(x);
        self.placing = false;
        if y < lane_top {
            self.drag = Some(Drag::Playhead);
            return self.seek(t, cx);
        }
        // The segment lane: edges resize, bodies move, empty space adds.
        let local = x - ox;
        let hit = self.project.segments.iter().enumerate().rev().find_map(|(i, s)| {
            let (a, b) = (self.x_of(s.start), self.x_of(s.end));
            if (local - a).abs() <= EDGE {
                Some(Drag::Edge { index: i, start: true })
            } else if (local - b).abs() <= EDGE {
                Some(Drag::Edge { index: i, start: false })
            } else if local > a && local < b {
                Some(Drag::Move { index: i, grab: t - s.start })
            } else {
                None
            }
        });
        match hit {
            Some(drag) => {
                self.selected = Some(match drag {
                    Drag::Edge { index, .. } | Drag::Move { index, .. } => index,
                    Drag::Playhead => unreachable!(),
                });
                self.drag = Some(drag);
            }
            None => {
                let start = t.min(self.probe.duration - MIN_SEG).max(0.);
                let end = (start + NEW_SEG).min(self.probe.duration);
                self.project.segments.push(Seg { start, end, zoom: studio::DEFAULT_ZOOM, focus: None });
                self.selected = Some(self.project.segments.len() - 1);
                self.drag = Some(Drag::Edge { index: self.project.segments.len() - 1, start: false });
                self.seek(start + 0.01, cx);
                self.changed(cx);
            }
        }
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        if e.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let t = self.time_at(f32::from(e.position.x));
        let d = self.probe.duration;
        match self.drag {
            Some(Drag::Playhead) => self.seek(t, cx),
            Some(Drag::Move { index, grab }) => {
                let seg = &mut self.project.segments[index];
                let len = seg.end - seg.start;
                seg.start = (t - grab).clamp(0., d - len);
                seg.end = seg.start + len;
                self.changed(cx);
            }
            Some(Drag::Edge { index, start }) => {
                let seg = &mut self.project.segments[index];
                if start {
                    seg.start = t.clamp(0., seg.end - MIN_SEG);
                } else {
                    seg.end = t.clamp(seg.start + MIN_SEG, d);
                }
                let at = if start { seg.start } else { seg.end };
                self.changed(cx);
                self.seek(at, cx);
            }
            None => {}
        }
    }

    /// Clicking the preview while placing sets the segment's focus point.
    fn preview_down(&mut self, e: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(index) = self.selected.filter(|_| self.placing) else { return };
        let (bx, by, bw, bh) = self.video_box.get();
        let (z, cxv, cyv) = self.camera.at(self.playhead);
        // Undo the preview's zoom to find the video pixel under the pointer.
        let (vw, vh) = (self.probe.w, self.probe.h);
        let (cw, ch) = (vw / z, vh / z);
        let (left, top) = ((cxv - cw / 2.).clamp(0., vw - cw), (cyv - ch / 2.).clamp(0., vh - ch));
        let u = ((f32::from(e.position.x) - bx) / bw).clamp(0., 1.) as f64;
        let v = ((f32::from(e.position.y) - by) / bh).clamp(0., 1.) as f64;
        self.project.segments[index].focus = Some((left + u * cw, top + v * ch));
        self.placing = false;
        self.changed(cx);
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(i) = self.selected.take() {
            self.project.segments.remove(i);
            self.changed(cx);
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        match (k.key.as_str(), ctrl) {
            ("escape", _) if self.placing => self.placing = false,
            ("escape", _) if self.selected.is_some() => self.selected = None,
            ("escape", _) => return window.remove_window(),
            ("delete" | "backspace", false) => self.delete_selected(cx),
            ("space", false) => studio::export(self.video.clone(), self.project.clone(), true, cx),
            ("s", true) => {
                window.remove_window();
                return studio::export(self.video.clone(), self.project.clone(), false, cx);
            }
            ("left", _) => self.seek(self.playhead - 0.5, cx),
            ("right", _) => self.seek(self.playhead + 0.5, cx),
            _ => {}
        }
        cx.notify();
    }

    /// The output frame as it will look at the playhead.
    fn render_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (sw, sh) = self.stage;
        let (vw, vh) = (self.probe.w as f32, self.probe.h as f32);
        let pad = PADDINGS[self.project.padding.min(PADDINGS.len() - 1)] as f32 * sh;
        let k = ((sw - 2. * pad) / vw).min((sh - 2. * pad) / vh);
        let (bw, bh) = (vw * k, vh * k);
        let (bx, by) = ((sw - bw) / 2., (sh - bh) / 2.);
        let radius = if pad > 0. { 22. * sh / 1440. } else { 0. };

        let (z, cxv, cyv) = self.camera.at(self.playhead);
        let z = z as f32;
        let (cw, ch) = (vw / z, vh / z);
        let left = (cxv as f32 - cw / 2.).clamp(0., vw - cw);
        let top = (cyv as f32 - ch / 2.).clamp(0., vh - ch);
        let scale = bw / vw * z;

        let video_box = self.video_box.clone();
        let tracker = canvas(
            move |bounds, _, _| {
                video_box.set((f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height)));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .size_full();

        let (c0, c1) = self.project.colors();
        let picture = div()
            .id("video")
            .absolute()
            .left(px(bx))
            .top(px(by))
            .w(px(bw))
            .h(px(bh))
            .rounded(px(radius))
            .overflow_hidden()
            .when(pad > 0., |d| d.shadow(kit::shadow()))
            .bg(gpui::black())
            .when(self.placing, |d| d.cursor(CursorStyle::Crosshair))
            .children(self.frame.clone().map(|f| {
                img(f)
                    .absolute()
                    .left(px(-left * scale))
                    .top(px(-top * scale))
                    .w(px(vw * scale))
                    .h(px(vh * scale))
                    .object_fit(ObjectFit::Fill)
            }))
            .child(tracker)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.preview_down(e, cx)));

        div()
            .relative()
            .w(px(sw))
            .h(px(sh))
            .rounded_lg()
            .overflow_hidden()
            .bg(linear_gradient(135., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.)))
            .child(picture)
            .child(
                div()
                    .absolute()
                    .top_2()
                    .left_2()
                    .px_1p5()
                    .py_0p5()
                    .rounded_md()
                    .bg(rgba(0x000000b3))
                    .font_family(kit::MONO_FONT)
                    .text_xs()
                    .text_color(gpui::white())
                    .child(format!("{}  ×{:.2}", clock(self.playhead), z)),
            )
            .when(self.placing, |d| {
                d.child(
                    div()
                        .absolute()
                        .bottom_2()
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(div().px_2().py_1().rounded_md().bg(rgba(0x000000cc)).text_xs().text_color(gpui::white()).child("Кликни по видео, куда приблизить")),
                )
            })
    }

    fn render_timeline(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let (accent, lane_bg, text) = (colors.text_accent, colors.element_background, colors.text);
        let thumbs = self.thumbs.iter().map(|t| img(t.clone()).flex_1().h_full().object_fit(ObjectFit::Cover));
        let d = self.probe.duration.max(0.001);
        let segments: Vec<(f64, f64, f64, bool, bool)> =
            self.project.segments.iter().enumerate().map(|(i, s)| (s.start, s.end, s.zoom, s.focus.is_some(), self.selected == Some(i))).collect();
        let playhead = self.playhead;
        let timeline = self.timeline.clone();

        // Segments and the playhead are painted; text labels go on top.
        let painter = canvas(
            move |bounds, _, _| bounds,
            move |_, bounds, window, _| {
                let (x0, w) = (bounds.origin.x, bounds.size.width);
                let at = |t: f64| x0 + w * (t / d) as f32;
                let lane_top = bounds.origin.y + px(FILMSTRIP + 6.);
                timeline.set((f32::from(x0), f32::from(w), f32::from(lane_top)));
                for &(s, e, _, _, selected) in &segments {
                    let block = Bounds::new(point(at(s), lane_top + px(2.)), size(at(e) - at(s), px(LANE - 4.)));
                    let mut fill_color = accent;
                    fill_color.a = if selected { 0.55 } else { 0.3 };
                    window.paint_quad(gpui::quad(block, px(6.), fill_color, px(if selected { 2. } else { 1. }), accent, BorderStyle::Solid));
                    // Ease-in/out ramps either side, faint.
                    let mut ramp = accent;
                    ramp.a = 0.12;
                    for (a, b) in [(s - studio::RAMP, s), (e, e + studio::RAMP)] {
                        let (a, b) = (a.max(0.), b.min(d));
                        if b > a {
                            window.paint_quad(fill(Bounds::new(point(at(a), lane_top + px(8.)), size(at(b) - at(a), px(LANE - 16.))), ramp));
                        }
                    }
                }
                let x = at(playhead);
                window.paint_quad(fill(Bounds::new(point(x - px(1.), bounds.origin.y), size(px(2.), bounds.size.height)), text));
                window.paint_quad(gpui::quad(Bounds::new(point(x - px(5.), bounds.origin.y - px(4.)), size(px(10.), px(8.))), px(2.), text, px(0.), text, BorderStyle::Solid));
            },
        )
        .absolute()
        .inset_0()
        .size_full();

        let labels = self.project.segments.iter().map(|s| {
            let left = self.x_of(s.start);
            let width = (self.x_of(s.end) - left).max(0.);
            div()
                .absolute()
                .left(px(left))
                .top(px(FILMSTRIP + 6.))
                .w(px(width))
                .h(px(LANE))
                .flex()
                .items_center()
                .justify_center()
                .gap_1()
                .overflow_hidden()
                .when(width > 44., |d| {
                    d.child(Label::new(format!("×{:.2}", s.zoom).trim_end_matches('0').trim_end_matches('.').to_string()).size(LabelSize::XSmall))
                        .when(s.focus.is_some(), |d| d.child(Icon::new(IconName::Crosshair).size(IconSize::XSmall).color(Color::Muted)))
                })
        });

        v_flex()
            .w(px(self.stage.0))
            .gap_1()
            .p_1p5()
            .elevation_3(cx)
            .child(
                div()
                    .id("timeline")
                    .relative()
                    .w_full()
                    .h(px(FILMSTRIP + 6. + LANE))
                    .cursor(CursorStyle::PointingHand)
                    .child(div().h(px(FILMSTRIP)).w_full().rounded_md().overflow_hidden().child(h_flex().size_full().children(thumbs)))
                    .child(div().absolute().left_0().right_0().top(px(FILMSTRIP + 6.)).h(px(LANE)).rounded_md().bg(lane_bg))
                    .children(labels)
                    .child(painter)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| this.timeline_down(e, cx))),
            )
            .child(
                h_flex()
                    .justify_between()
                    .px_0p5()
                    .child(Label::new(clock(self.playhead)).size(LabelSize::XSmall).color(Color::Muted))
                    .child(Label::new("Клик по нижней дорожке — новый зум, края — длина, перетащи — сдвинуть").size(LabelSize::XSmall).color(Color::Muted))
                    .child(Label::new(clock(self.probe.duration)).size(LabelSize::XSmall).color(Color::Muted)),
            )
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chip = |id: SharedString, label: String, on: bool| {
            ButtonLike::new(id).style(ButtonStyle::Subtle).toggle_state(on).child(div().px_1p5().child(Label::new(label).size(LabelSize::Small)))
        };
        let segment = self.selected.and_then(|i| self.project.segments.get(i).map(|s| (i, s.clone())));
        let inspector = match segment {
            Some((index, seg)) => h_flex()
                .gap_1()
                .child(Label::new("Зум").size(LabelSize::Small).color(Color::Muted))
                .children(ZOOMS.iter().map(|&z| {
                    chip(SharedString::from(format!("zoom-{z}")), format!("×{z}"), (seg.zoom - z).abs() < 0.01).on_click(cx.listener(move |this, _, _, cx| {
                        this.project.segments[index].zoom = z;
                        this.changed(cx);
                    }))
                }))
                .child(Divider::vertical().h_5().mx_1())
                .child(Label::new("Фокус").size(LabelSize::Small).color(Color::Muted))
                .child(chip("focus-cursor".into(), "Курсор".into(), seg.focus.is_none() && !self.placing).on_click(cx.listener(move |this, _, _, cx| {
                    this.placing = false;
                    this.project.segments[index].focus = None;
                    this.changed(cx);
                })))
                .child(chip("focus-point".into(), "Точка".into(), seg.focus.is_some() || self.placing).on_click(cx.listener(|this, _, _, cx| {
                    this.placing = true;
                    cx.notify();
                })))
                .child(Divider::vertical().h_5().mx_1())
                .child(
                    IconButton::new("delete", IconName::Trash)
                        .shape(IconButtonShape::Square)
                        .tooltip(Tooltip::text("Удалить зум (Del)"))
                        .on_click(cx.listener(|this, _, _, cx| this.delete_selected(cx))),
                )
                .into_any_element(),
            None => Label::new(format!("Зумов: {}. Выбери отрезок на дорожке, чтобы настроить", self.project.segments.len()))
                .size(LabelSize::Small)
                .color(Color::Muted)
                .into_any_element(),
        };

        let ring = cx.theme().colors().text;
        let backdrops = palette::backdrops().into_iter().enumerate().map(|(i, (c0, c1))| {
            div()
                .id(("backdrop", i))
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .border_2()
                .border_color(if i == self.project.backdrop { ring } else { gpui::transparent_black() })
                .cursor_pointer()
                .child(div().size(px(14.)).rounded(px(4.)).bg(linear_gradient(135., linear_color_stop(c0, 0.), linear_color_stop(c1, 1.))))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.project.backdrop = i;
                    this.changed(cx);
                }))
        });
        let paddings = ["Нет", "S", "M", "L"].into_iter().enumerate().map(|(i, label)| {
            chip(SharedString::from(format!("pad-{i}")), label.into(), self.project.padding == i).on_click(cx.listener(move |this, _, _, cx| {
                this.project.padding = i;
                this.changed(cx);
            }))
        });
        let has_clicks = !self.track.clicks.is_empty();
        let has_keys = !self.track.keystrokes.is_empty();

        let global = h_flex()
            .w_full()
            .justify_between()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .child(Label::new("Фон").size(LabelSize::Small).color(Color::Muted))
                    .children(backdrops)
                    .child(Divider::vertical().h_5().mx_1())
                    .child(Label::new("Отступ").size(LabelSize::Small).color(Color::Muted))
                    .children(paddings)
                    .child(Divider::vertical().h_5().mx_1())
                    .child(
                        chip("clicks".into(), "Клики".into(), self.project.clicks && has_clicks)
                            .disabled(!has_clicks)
                            .tooltip(Tooltip::text("Кольца на месте кликов"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.project.clicks = !this.project.clicks;
                                this.changed(cx);
                            })),
                    )
                    .child(
                        chip("keys".into(), "Клавиши".into(), self.project.keys && has_keys)
                            .disabled(!has_keys)
                            .tooltip(Tooltip::text(if has_keys { "Плашка с сочетаниями клавиш" } else { "Запиши с включённым «Клавиши», чтобы показать сочетания" }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.project.keys = !this.project.keys;
                                this.changed(cx);
                            })),
                    )
                    .child(
                        Button::new("auto", "Авто")
                            .style(ButtonStyle::Subtle)
                            .color(Color::Muted)
                            .tooltip(Tooltip::text("Заново расставить зумы по кликам"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let auto = Project::auto(&this.track, &this.probe);
                                this.project.segments = auto.segments;
                                this.selected = None;
                                this.changed(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(
                        Button::new("preview", "Предпросмотр")
                            .style(ButtonStyle::Subtle)
                            .key_binding(kit::keys("space"))
                            .on_click(cx.listener(|this, _, _, cx| studio::export(this.video.clone(), this.project.clone(), true, cx))),
                    )
                    .child(
                        Button::new("cancel", "Закрыть")
                            .style(ButtonStyle::Subtle)
                            .color(Color::Muted)
                            .key_binding(kit::keys("escape"))
                            .on_click(|_, window, _| window.remove_window()),
                    )
                    .child(
                        Button::new("export", "Экспорт")
                            .style(ButtonStyle::Filled)
                            .start_icon(Icon::new(IconName::Sparkle).size(IconSize::Small))
                            .key_binding(kit::keys("ctrl-s"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                window.remove_window();
                                studio::export(this.video.clone(), this.project.clone(), false, cx);
                            })),
                    ),
            );

        v_flex()
            .w(px(self.stage.0))
            .gap_1()
            .px_2()
            .py_1()
            .elevation_3(cx)
            .child(h_flex().h(px(30.)).child(inspector))
            .child(Divider::horizontal())
            .child(global)
    }
}

impl Render for StudioEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        kit::root(cx)
            .bg(rgba(0x000000a6))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| this.key_down(e, window, cx)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| this.mouse_move(e, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.drag = None))
            .child(self.render_preview(cx))
            .child(self.render_timeline(cx))
            .child(self.render_controls(cx))
    }
}

/// "1:05.3"
fn clock(t: f64) -> String {
    let t = t.max(0.);
    format!("{}:{:04.1}", (t / 60.) as u64, t % 60.)
}
