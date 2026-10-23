//! The studio's edit and export are `kadr_media::studio`; here they're run
//! with the app's progress card and notifications.

use std::ffi::OsStr;
use std::path::PathBuf;

use gpui::App;
pub use kadr_media::studio::*;

use crate::i18n::t;
use crate::job;
use crate::track::Track;
use crate::ui::IconName;
use crate::util;

/// Export with the saved project (or automatic zooms when there's none).
pub fn render(video: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        let Some(p) = probe(&video).await else {
            return util::notify(t("common.couldnt_open_video"), &video.display().to_string());
        };
        let track = Track::load(&video).unwrap_or_else(|| Track { rect: (0.0, 0.0, p.w, p.h), ..Default::default() });
        let project = Project::load(&video, &track, &p);
        let _ = cx.update(|cx| export(video, project, false, cx));
    })
    .detach();
}

/// Render `project`; the full export lands in a card, the quick preview
/// (720p) opens in the player.
pub fn export(video: PathBuf, project: Project, preview: bool, cx: &mut App) {
    let title = if preview { t("studio.preparing_preview") } else { t("studio.exporting_video") };
    let running = job::start(title, if preview { IconName::PlayFilled } else { IconName::Sparkle }, (!preview).then_some(video.as_path()), cx);
    cx.spawn(async move |cx| {
        let out = if preview { preview_path(&video) } else { output_path(&video, project.format) };
        let result = run(&video, &project, &out, preview, &running.job, crate::platform::cuda()).await;
        running.finish(matches!(result, Ok(true)).then(|| out.clone()).filter(|_| !preview), cx);
        match result {
            Ok(true) if preview => {
                util::spawn(&[OsStr::new("mpv"), "--force-window=yes".as_ref(), "--loop-file=inf".as_ref(), out.as_os_str()]);
            }
            Ok(_) => {}
            Err(e) => util::notify(t("studio.couldnt_build_video"), &e),
        }
    })
    .detach();
}
