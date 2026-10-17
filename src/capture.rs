use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AsyncApp};

use crate::settings::Settings;
use crate::{cards, kit, picker, util};
use crate::i18n::t;

/// Take a screenshot with kadr's own picker, then copy it and show its card.
pub fn shot(mode: String, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        // Keep existing cards out of the frozen frame; give the compositor
        // a moment to present the emptied overlay.
        if cx.update(|cx| cards::set_hidden(true, cx)) {
            cx.background_executor().timer(Duration::from_millis(120)).await;
        }
        let settings = Settings::load();
        let result = take(&mode, &settings, cx).await;
        let _ = cx.update(|cx| {
            cards::set_hidden(false, cx);
            // A cancelled pick gives nothing.
            if let Some(path) = result {
                deliver(path, &settings, cx);
            }
        });
    })
    .detach();
}

/// What happens to a fresh screenshot: as set in the settings.
pub fn deliver(path: PathBuf, _settings: &Settings, cx: &mut App) {
    cards::after_capture(path, crate::storage::Capture::Screenshot, cx);
}

/// Freeze the screen, pick (or take the whole monitor), save the picked part.
async fn take(mode: &str, settings: &Settings, cx: &mut AsyncApp) -> Option<PathBuf> {
    let picked = match mode {
        "fullscreen" => picker::whole_monitor().await?,
        // The self-timer: 3, 2, 1 over the screen, then all of it.
        "timer" => {
            let mons = util::monitors();
            let name = mons.iter().find(|m| m.focused)?.name.clone();
            if !crate::record::countdown(&crate::record::Target::Monitor(name), 3, &mons, cx).await {
                return None;
            }
            picker::whole_monitor().await?
        }
        // Free selection; the others also snap to windows.
        "region" => picker::pick(false, true, cx).await?,
        _ => picker::pick(true, true, cx).await?,
    };
    let dir = settings.screenshot_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = crate::storage::capture_path(&dir, crate::storage::Capture::Screenshot, "png");
    let target = path.clone();
    // Encoding a big PNG takes a moment; keep the UI free.
    let saved = cx.background_executor().spawn(async move { picked.crop().save(&target) }).await;
    match saved {
        Ok(()) => Some(path),
        Err(e) => {
            util::notify(t("common.couldnt_save"), &e.to_string());
            None
        }
    }
}
