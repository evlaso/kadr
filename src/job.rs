//! Long ffmpeg jobs (studio render, trimming, GIF export) with a Zed-style
//! progress toast at the bottom of the screen that can cancel them.

use std::rc::Rc;
use std::time::Duration;

use crate::kit::{Anchor, KeyboardInteractivity};
use gpui::{App, AsyncApp, Context, SharedString, Window, WindowHandle};
use crate::ui::{IconButtonShape, ProgressBar, Tooltip, prelude::*};

use crate::kit::{self, Hold, InputRegion, LayerSpec};
use crate::util;
use crate::i18n::t;
pub use kadr_media::job::{Job, ffmpeg};

/// A running job: keeps kadr alive and its progress on screen until
/// `finish`: a blurred card in the thumbnail stack for a job on a video,
/// a toast otherwise.
pub struct Running {
    pub job: Rc<Job>,
    window: Option<WindowHandle<ProgressView>>,
    card: Option<usize>,
    _hold: Hold,
}

/// Start a job. With `source` (the video it works on) and cards turned on,
/// progress shows as a card that becomes the result's card when done.
pub fn start(title: &'static str, icon: IconName, source: Option<&std::path::Path>, cx: &mut App) -> Running {
    let job = Rc::new(Job::default());
    if let Some(source) = source.filter(|_| crate::settings::Settings::load().after_video.card) {
        let card = crate::cards::show_job(job.clone(), title, source.to_path_buf(), cx);
        return Running { job, window: None, card: Some(card), _hold: kit::hold(cx) };
    }
    let spec = LayerSpec {
        namespace: "kadr-progress",
        anchor: Anchor::BOTTOM,
        margin: (0., 0., 24., 0.),
        size: (340. * kit::SCALE, 90. * kit::SCALE),
        keyboard: KeyboardInteractivity::None,
        ignore_zones: false,
        monitor: util::focused_monitor(),
    };
    let state = job.clone();
    let window = kit::open_layer(cx, spec, |window, cx| cx.new(|cx| ProgressView::new(title.into(), icon, state, window, cx)));
    Running { job, window, card: None, _hold: kit::hold(cx) }
}

impl Running {
    /// The job is over: its card turns into the result's (or goes away),
    /// or the toast closes and the result gets a card of its own.
    pub fn finish(self, result: Option<std::path::PathBuf>, cx: &mut AsyncApp) {
        // A finished export, trim or GIF is copied like a fresh recording.
        if let Some(out) = &result
            && crate::settings::Settings::load().after_video.copy
        {
            util::copy_file(out);
        }
        if let Some(w) = self.window {
            let _ = w.update(cx, |_, window, _| window.remove_window());
        }
        match self.card {
            Some(card) => {
                let _ = cx.update(|cx| crate::cards::complete(card, result, cx));
            }
            None => {
                if let Some(out) = result {
                    let _ = cx.update(|cx| crate::cards::show(out, cx));
                }
            }
        }
    }
}


/// Small notification at the bottom of the screen while ffmpeg works.
pub struct ProgressView {
    title: SharedString,
    icon: IconName,
    state: Rc<Job>,
    region: InputRegion,
}

impl ProgressView {
    fn new(title: SharedString, icon: IconName, state: Rc<Job>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        ProgressView { title, icon, state, region: InputRegion::default() }
    }
}

impl Render for ProgressView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.region.reset();
        let fraction = self.state.fraction();
        let state = self.state.clone();
        kit::root(cx).flex().items_center().justify_center().child(
            v_flex()
                .relative()
                .w(px(300.))
                .gap_1p5()
                .pl_2p5()
                .pr_1()
                .pt_1()
                .pb_2p5()
                .elevation_3(cx)
                .child(
                    h_flex()
                        .gap_1p5()
                        .child(Icon::new(self.icon).size(IconSize::Small).color(Color::Accent))
                        .child(div().flex_1().child(Label::new(self.title.clone()).size(LabelSize::Small)))
                        .child(Label::new(format!("{}%", (fraction * 100.) as u32)).size(LabelSize::Small).color(Color::Muted))
                        .child(
                            IconButton::new("cancel", IconName::Close)
                                .shape(IconButtonShape::Square)
                                .icon_size(IconSize::XSmall)
                                .tip(Tooltip::text(t("job.cancel")))
                                .on_click(move |_, _, _| state.cancel()),
                        ),
                )
                .child(div().pr_1p5().child(ProgressBar::new("progress", fraction, 1., cx)))
                .child(self.region.mark()),
        )
    }
}
