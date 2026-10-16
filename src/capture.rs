use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AsyncApp};

use crate::settings::Settings;
use crate::{cards, kit, picker, util};

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

/// What happens to a fresh screenshot: copied and shown in a card, as set.
pub fn deliver(path: PathBuf, settings: &Settings, cx: &mut App) {
    if settings.copy_screenshot {
        util::copy_image(&path);
    }
    if settings.show_card {
        cards::show(path, cx);
    }
}

/// Freeze the screen, pick (or take the whole monitor), save the picked part.
async fn take(mode: &str, settings: &Settings, cx: &mut AsyncApp) -> Option<PathBuf> {
    let picked = match mode {
        "fullscreen" => picker::whole_monitor().await?,
        // Free selection; the others also snap to windows.
        "region" => picker::pick(false, cx).await?,
        _ => picker::pick(true, cx).await?,
    };
    let dir = settings.screenshot_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("screenshot-{}.png", util::timestamp()));
    let target = path.clone();
    // Encoding a big PNG takes a moment; keep the UI free.
    let saved = cx.background_executor().spawn(async move { picked.crop().save(&target) }).await;
    match saved {
        Ok(()) => Some(path),
        Err(e) => {
            util::notify("Не удалось сохранить", &e.to_string());
            None
        }
    }
}
