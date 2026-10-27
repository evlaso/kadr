//! Updating from the card (SPEC 12.3): Check now in the settings and
//! `kadr update`, and what the card's Update button does: the release of
//! the chosen channel downloaded, checked (sha256 and the release key's
//! signature) and put in by `kadr_platform::update`, the tray started
//! again on it. kadr itself runs the new version from its next start (it
//! quits when idle); on Windows the installer takes over once kadr closes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::App;
use kadr_core::update::{CURRENT, Channel, LATEST_URL, Latest, Release};
use kadr_platform::update::{Applied, Install};

use crate::cards::{self, Action};
use crate::i18n::t;
use crate::settings::Settings;

/// The newest release on the chosen channel, fetched now.
fn latest() -> Result<Release, String> {
    let url = std::env::var("KADR_UPDATE_URL").ok().filter(|_| std::env::var_os("KADR_DEV").is_some()).unwrap_or(LATEST_URL.into());
    let body = if let Some(path) = url.strip_prefix("file://") { std::fs::read(path).ok() } else { kadr_platform::fetch(&url) };
    let latest = body.and_then(|b| Latest::parse(&b)).ok_or_else(|| t("update.no_connection").to_owned())?;
    latest.on(Channel::chosen(Settings::load().update_channel)).cloned().ok_or_else(|| t("update.no_connection").to_owned())
}

/// Looks for a newer release now: its card, or one saying this is the newest.
pub fn check_now(cx: &mut App) {
    cx.spawn(async move |cx| {
        let found = cx.background_executor().spawn(async { latest() }).await;
        let _ = cx.update(|cx| match found {
            Ok(r) if kadr_core::update::newer(&r.version, CURRENT) => cards::show_update(r.version, r.notes_url, cx),
            Ok(_) => {
                let channel = match Channel::chosen(Settings::load().update_channel) {
                    Channel::Stable => t("settings.channel_stable"),
                    Channel::Beta => t("settings.channel_beta"),
                };
                cards::show_notice(tr!("update.up_to_date", CURRENT), tr!("update.on_channel", channel), None, None, cx);
            }
            Err(e) => {
                cards::show_notice(t("update.failed").into(), e, None, None, cx);
            }
        });
    })
    .detach();
}

/// The card's button.
pub fn act(id: usize, action: Action, cx: &mut App) {
    match action {
        Action::Update(version) => download(id, version, cx),
        Action::Install(setup) => install(&setup, cx),
    }
}

fn install(setup: &std::path::Path, cx: &mut App) {
    if kadr_platform::update::run_installer(setup) {
        cx.quit();
    }
}

/// Downloads `version`, puts it in, and says how it went on card `id`.
fn download(id: usize, version: String, cx: &mut App) {
    let Install::Own(layout) = kadr_platform::update::install() else { return };
    let title = tr!("update.downloading", version);
    cards::update_notice(id, title.clone(), String::new(), None, Some(0.), cx);
    let got = Arc::new(AtomicU64::new(0));
    let size = Arc::new(AtomicU64::new(0));
    let (got2, size2) = (got.clone(), size.clone());
    let work = cx.background_executor().spawn(async move {
        let release = latest()?;
        if kadr_core::update::compare(&release.version, &version) == Some(std::cmp::Ordering::Less) {
            return Err(t("update.no_connection").to_owned());
        }
        let asset = release.platforms.get(layout.asset()).filter(|a| !a.sig.is_empty()).ok_or_else(|| t("update.no_file").to_owned())?;
        size2.store(asset.size.unwrap_or(0), Ordering::Relaxed);
        let name = asset.url.rsplit('/').next().unwrap_or("kadr-update");
        let file = kadr_core::dirs::cache().join("updates").join(name);
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
        let fetched = match asset.url.strip_prefix("file://") {
            Some(path) if std::env::var_os("KADR_DEV").is_some() => std::fs::copy(path, &file).map(|_| ()).map_err(|e| e.to_string()),
            _ => kadr_platform::update::download(&asset.url, &file, &|n| got2.store(n, Ordering::Relaxed)),
        };
        fetched.map_err(|e| format!("{}\n{e}", t("update.no_connection")))?;
        let data = std::fs::read(&file).map_err(|e| e.to_string())?;
        if !asset.holds(&data) {
            let _ = std::fs::remove_file(&file);
            return Err(t("update.not_genuine").to_owned());
        }
        drop(data);
        let applied = kadr_platform::update::apply(&layout, &file, &release.version)?;
        if !matches!(applied, Applied::Installer(_)) {
            let _ = std::fs::remove_file(&file);
        }
        if let Applied::Done { tray } = &applied {
            kadr_tray::restart_as(tray);
        }
        Ok((release.version, applied))
    });
    cx.spawn(async move |cx| {
        let work = work;
        let mut work = std::pin::pin!(work);
        let result = loop {
            let tick = cx.background_executor().timer(Duration::from_millis(200));
            match futures::future::select(work.as_mut(), tick).await {
                futures::future::Either::Left((done, _)) => break done,
                futures::future::Either::Right(_) => {
                    let total = size.load(Ordering::Relaxed);
                    let fraction = if total > 0 { got.load(Ordering::Relaxed) as f32 / total as f32 } else { 0. };
                    let title = title.clone();
                    let _ = cx.update(|cx| cards::update_notice(id, title, String::new(), None, Some(fraction.min(1.)), cx));
                }
            }
        };
        let _ = cx.update(|cx| match result {
            Ok((version, Applied::Done { .. })) => cards::update_notice(id, tr!("update.installed", version), t("update.installed_body").into(), None, None, cx),
            Ok((version, Applied::Installer(setup))) => {
                // Nothing open but the cards: nothing to lose by closing now.
                if cx.windows().len() <= 1 {
                    install(&setup, cx);
                } else {
                    cards::update_notice(id, tr!("update.ready", version), t("update.ready_body").into(), Some(Action::Install(setup)), None, cx);
                }
            }
            Err(e) => cards::update_notice(id, t("update.failed").into(), e, None, None, cx),
        });
    })
    .detach();
}
