use std::path::PathBuf;
use std::time::Duration;

use gpui::App;

use crate::settings::Settings;
use crate::{cards, kit, util};

/// Take a screenshot (`platform::screenshot`), then copy it and show its card.
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
        let result = take(&mode, &settings).await;
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

async fn take(mode: &str, settings: &Settings) -> Option<PathBuf> {
    let _ = std::fs::create_dir_all(settings.screenshot_dir());
    crate::platform::screenshot(mode, &settings.screenshot_dir()).await
}
