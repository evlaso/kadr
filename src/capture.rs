use std::path::PathBuf;
use std::time::Duration;

use gpui::App;

use crate::{cards, kit, util};

/// Selection, screen freeze and saving are delegated to Omarchy's own
/// screenshot command so the picker behaves exactly like the stock one.
pub fn shot(mode: String, cx: &mut App) {
    let hold = kit::hold(cx);
    cx.spawn(async move |cx| {
        let _hold = hold;
        // Keep existing cards out of the frozen frame; give the compositor
        // a moment to present the emptied overlay.
        if cx.update(|cx| cards::set_hidden(true, cx)) {
            cx.background_executor().timer(Duration::from_millis(120)).await;
        }
        let result = take(&mode).await;
        let _ = cx.update(|cx| {
            cards::set_hidden(false, cx);
            match result {
                Some(path) => {
                    util::copy_image(&path);
                    cards::show(path, cx);
                }
                // A cancelled pick prints nothing.
                None => {}
            }
        });
    })
    .detach();
}

async fn take(mode: &str) -> Option<PathBuf> {
    let (_, stdout) = util::output(&["omarchy-capture-screenshot", mode, "save"]).await?;
    let path = stdout.lines().map(str::trim).rfind(|l| !l.is_empty()).map(PathBuf::from)?;
    path.is_file().then_some(path)
}
