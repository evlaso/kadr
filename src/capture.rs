use std::ffi::OsStr;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk::{gio, glib, prelude::*};

use crate::cards::Cards;
use crate::util;

/// Selection, screen freeze and saving are delegated to Omarchy's own
/// screenshot command so the picker behaves exactly like the stock one.
pub fn shot(app: &gtk::Application, cards: &Rc<Cards>, mode: String) {
    let hold = app.hold();
    let app = app.clone();
    let cards = cards.clone();
    glib::spawn_future_local(async move {
        let _hold = hold;
        // Keep existing cards out of the frozen frame; give the compositor
        // time to finish the layer's close animation.
        if cards.set_hidden(true) {
            glib::timeout_future(Duration::from_millis(250)).await;
        }
        let result = take(&mode).await;
        cards.set_hidden(false);
        match result {
            Ok(Some(path)) => {
                util::copy_image(&path);
                cards.show(&app, path);
            }
            Ok(None) => {}
            Err(e) => util::notify("Не удалось снять скриншот", &e),
        }
    });
}

async fn take(mode: &str) -> Result<Option<PathBuf>, String> {
    let argv: [&OsStr; 3] = ["omarchy-capture-screenshot".as_ref(), mode.as_ref(), "save".as_ref()];
    let proc = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE)
        .map_err(|e| e.to_string())?;
    let (stdout, _) = proc
        .communicate_utf8_future(None)
        .await
        .map_err(|e| e.to_string())?;
    let path = stdout
        .as_deref()
        .and_then(|s| s.lines().map(str::trim).rfind(|l| !l.is_empty()))
        .map(PathBuf::from);
    // A cancelled pick prints nothing.
    Ok(path.filter(|p| p.is_file()))
}
