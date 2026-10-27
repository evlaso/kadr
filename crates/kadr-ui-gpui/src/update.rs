//! Updating (SPEC 12.3): Check now in the settings, which says how it goes
//! in its own row, and `kadr update` and the tray's find, which say it on a
//! card. The release of the chosen channel is downloaded, checked (sha256
//! and the release key's signature) and put in by `kadr_platform::update`,
//! the tray started again on it. kadr itself runs the new version from its
//! next start (it quits when idle); on Windows the installer takes over once
//! kadr closes.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{App, Global};
use kadr_core::update::{CURRENT, Channel, LATEST_URL, Latest, Release};
use kadr_platform::update::{Applied, Install};

use crate::cards::{self, Action};
use crate::i18n::t;
use crate::settings::Settings;

/// Where an update says how it goes.
#[derive(Clone, Copy)]
pub enum Out {
    /// On this card.
    Card(usize),
    /// In the settings' About row (`Status`).
    Settings,
}

/// How updating looks in the settings' About row.
#[derive(Clone, Default)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Found { version: String, page: String },
    Downloading { version: String, progress: f32 },
    Installed(String),
    Ready { version: String, setup: PathBuf },
    Failed(String),
}

/// The status, and the channel it was found on.
struct Here(Status, Channel);

impl Global for Here {}

pub fn status(cx: &App) -> Status {
    match cx.try_global::<Here>() {
        // What was found on another channel says nothing about this one.
        Some(Here(Status::UpToDate | Status::Found { .. } | Status::Failed(_), on)) if *on != Channel::chosen(Settings::load().update_channel) => Status::Idle,
        Some(Here(status, _)) => status.clone(),
        None => Status::Idle,
    }
}

fn set(status: Status, cx: &mut App) {
    cx.set_global(Here(status, Channel::chosen(Settings::load().update_channel)));
    cx.refresh_windows();
}

/// The settings open afresh: an answer from before is forgotten, work
/// going on isn't.
pub fn forget(cx: &mut App) {
    if matches!(status(cx), Status::UpToDate | Status::Found { .. } | Status::Failed(_)) {
        cx.set_global(Here(Status::Idle, Channel::chosen(Settings::load().update_channel)));
    }
}

/// The newest release on the chosen channel, fetched now.
fn latest() -> Result<Release, String> {
    let url = std::env::var("KADR_UPDATE_URL").ok().filter(|_| std::env::var_os("KADR_DEV").is_some()).unwrap_or(LATEST_URL.into());
    let body = if let Some(path) = url.strip_prefix("file://") { std::fs::read(path).ok() } else { kadr_platform::fetch(&url) };
    let latest = body.and_then(|b| Latest::parse(&b)).ok_or_else(|| t("update.no_connection").to_owned())?;
    latest.on(Channel::chosen(Settings::load().update_channel)).cloned().ok_or_else(|| t("update.no_connection").to_owned())
}

/// The chosen channel's name.
pub fn channel_name() -> &'static str {
    match Channel::chosen(Settings::load().update_channel) {
        Channel::Stable => t("settings.channel_stable"),
        Channel::Beta => t("settings.channel_beta"),
    }
}

/// Looks for a newer release now, saying what it found on a card
/// (`kadr update`): its card, or one saying this is the newest.
pub fn check_now(cx: &mut App) {
    cx.spawn(async move |cx| {
        let found = cx.background_executor().spawn(async { latest() }).await;
        let _ = cx.update(|cx| match found {
            Ok(r) if kadr_core::update::newer(&r.version, CURRENT) => cards::show_update(r.version, r.notes_url, cx),
            Ok(_) => {
                cards::show_notice(tr!("update.up_to_date", CURRENT), tr!("update.on_channel", channel_name()), None, None, cx);
            }
            Err(e) => {
                cards::show_notice(t("update.failed").into(), e, None, None, cx);
            }
        });
    })
    .detach();
}

/// The same from the settings' Check now: the answer in its row.
pub fn check_here(cx: &mut App) {
    if matches!(status(cx), Status::Checking | Status::Downloading { .. }) {
        return;
    }
    set(Status::Checking, cx);
    cx.spawn(async move |cx| {
        let found = cx.background_executor().spawn(async { latest() }).await;
        let _ = cx.update(|cx| {
            let status = match found {
                Ok(r) if kadr_core::update::newer(&r.version, CURRENT) => Status::Found { version: r.version, page: r.notes_url },
                Ok(_) => Status::UpToDate,
                Err(e) => Status::Failed(e),
            };
            set(status, cx);
        });
    })
    .detach();
}

/// A card's button.
pub fn act(id: usize, action: Action, cx: &mut App) {
    match action {
        Action::Update(version) => download(Out::Card(id), version, cx),
        Action::Install(setup) => install(&setup, cx),
    }
}

pub fn install(setup: &std::path::Path, cx: &mut App) {
    if kadr_platform::update::run_installer(setup) {
        cx.quit();
    }
}

/// How the download goes.
enum Step {
    Downloading(f32),
    Done(String, Applied),
    Failed(String),
}

fn tell(out: Out, version: &str, step: Step, cx: &mut App) {
    match out {
        Out::Card(id) => match step {
            Step::Downloading(p) => cards::update_notice(id, tr!("update.downloading", version), String::new(), None, Some(p), cx),
            Step::Done(version, Applied::Done { .. }) => cards::update_notice(id, tr!("update.installed", version), t("update.installed_body").into(), None, None, cx),
            Step::Done(version, Applied::Installer(setup)) => {
                // Nothing open but the cards: nothing to lose by closing now.
                if cx.windows().len() <= 1 {
                    install(&setup, cx);
                } else {
                    cards::update_notice(id, tr!("update.ready", version), t("update.ready_body").into(), Some(Action::Install(setup)), None, cx);
                }
            }
            Step::Failed(e) => cards::update_notice(id, t("update.failed").into(), e, None, None, cx),
        },
        Out::Settings => set(
            match step {
                Step::Downloading(progress) => Status::Downloading { version: version.into(), progress },
                Step::Done(version, Applied::Done { .. }) => Status::Installed(version),
                Step::Done(version, Applied::Installer(setup)) => Status::Ready { version, setup },
                Step::Failed(e) => Status::Failed(e),
            },
            cx,
        ),
    }
}

/// Downloads `version`, puts it in, and says how it went at `out`.
pub fn download(out: Out, version: String, cx: &mut App) {
    let Install::Own(layout) = kadr_platform::update::install() else { return };
    tell(out, &version, Step::Downloading(0.), cx);
    let got = Arc::new(AtomicU64::new(0));
    let size = Arc::new(AtomicU64::new(0));
    let (got2, size2) = (got.clone(), size.clone());
    let wanted = version.clone();
    let work = cx.background_executor().spawn(async move {
        let release = latest()?;
        if kadr_core::update::compare(&release.version, &wanted) == Some(std::cmp::Ordering::Less) {
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
        let mut work = std::pin::pin!(work);
        let result = loop {
            let tick = cx.background_executor().timer(Duration::from_millis(200));
            match futures::future::select(work.as_mut(), tick).await {
                futures::future::Either::Left((done, _)) => break done,
                futures::future::Either::Right(_) => {
                    let total = size.load(Ordering::Relaxed);
                    let fraction = if total > 0 { got.load(Ordering::Relaxed) as f32 / total as f32 } else { 0. };
                    let _ = cx.update(|cx| tell(out, &version, Step::Downloading(fraction.min(1.)), cx));
                }
            }
        };
        let step = match result {
            Ok((version, applied)) => Step::Done(version, applied),
            Err(e) => Step::Failed(e),
        };
        let _ = cx.update(|cx| tell(out, &version, step, cx));
    })
    .detach();
}
