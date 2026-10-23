//! kadr's UI on GPUI CE (SPEC §3.9): every screen, its windows and how they
//! draw and take input, over the view-models in `kadr-ui` and the rest of
//! the workspace. The only crate that knows GPUI (`scripts/check-deps.sh`);
//! the `kadr` program is its command line.

#[macro_use]
extern crate kadr_core;

mod i18n;
mod bundle;
mod capture;
mod cards;
mod dev;
mod editor;
mod history;
mod launcher;
mod hotkeys;
mod palette;
mod picker;
mod presets;
mod platform;
mod panel;
mod pin;
mod record;
mod scroll;
mod scrub;
mod settings;
mod storage;
mod settings_view;
mod studio;
mod studio_editor;
mod track;
mod trim;
mod ui;
mod job;
mod kit;
mod menubar;
mod ocr;
mod util;
mod webcam;

use std::io::{BufRead, BufReader};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, QuitMode};
use gpui_platform::application;
use kadr_core::cli::Command;

/// What every run needs before anything else: kadr's folders, its language,
/// the user's cursor theme, the dev hooks' bookkeeping.
pub fn prepare() {
    dev::track();
    dev::log_to_stderr();
    platform::set_dirs();
    i18n::set(settings::Settings::load().language);
    // The cursor stamps and the studio's pointer: the system's own.
    kadr_raster::cursors::set_theme(platform::theme_cursor);
}

/// Run `kadr <args>` inside this instance (the capture bar's modes, and
/// later invocations handed over).
pub(crate) fn run_args(args: &[String], cx: &mut App) {
    match Command::parse(args) {
        Ok(command) => run_command(command, cx),
        Err(e) => util::notify("kadr", &e),
    }
}

fn run_command(command: Command, cx: &mut App) {
    match command {
        Command::Scroll(area) => scroll::start(area, cx),
        Command::Shot(mode) => capture::shot(mode, cx),
        Command::Open(path) | Command::Edit(path) | Command::Studio(path, false) if bundle::is_bundle(&path) => bundle::open(path, cx),
        Command::Open(path) => cards::show(path, cx),
        Command::Pin(path) => pin::open(path, cx),
        Command::Record(mode, flags) => record::command(mode, flags.map(record::Opts::from), cx),
        Command::RecordStop => record::command_stop(cx),
        Command::RecordArea(area) => record::command_area(&area, cx),
        Command::RecordPrevious => record::command_previous(cx),
        Command::Studio(path, false) => studio_editor::open(path, cx),
        Command::Studio(path, true) => studio::render(path, cx),
        Command::Edit(path) => editor::open(path, cx),
        Command::Text(None) => ocr::region(cx),
        Command::Text(Some(path)) => ocr::image(path, cx),
        Command::Trim(path, None) => trim::open(path, cx),
        Command::Trim(path, Some((from, to))) => trim::save_trim(path, from, to, cx),
        Command::Gif(path) => trim::quick_gif(path, cx),
        Command::History => history::open(cx),
        Command::Settings => settings_view::open(cx),
        Command::Capture => launcher::toggle(cx),
        Command::UpdateAvailable(version, page) => cards::show_update(version, page, cx),
    }
}

/// Accept commands from later invocations on a helper thread. `dev` ones
/// keep their connection for the answer.
fn listen(listener: platform::ipc::Listener, tx: mpsc::UnboundedSender<(Vec<String>, Option<platform::ipc::Stream>)>) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let Ok(reply) = stream.try_clone() else { continue };
            if BufReader::new(stream).read_line(&mut line).is_ok()
                && let Ok(args) = serde_json::from_str::<Vec<String>>(&line)
            {
                let reply = (args.first().map(String::as_str) == Some("dev")).then_some(reply);
                if tx.unbounded_send((args, reply)).is_err() {
                    break;
                }
            }
        }
    });
}

/// The system allocator, noting big blocks for `kadr dev mem` (a build with
/// `--features dev`: the stack traces would cost the binary megabytes).
#[cfg(feature = "dev")]
#[global_allocator]
static ALLOC: dev::Counting = dev::Counting;

/// Become the running kadr: take `command`, then every later one that comes
/// over `listener`, and quit once idle.
pub fn run(command: Command, listener: platform::ipc::Listener) {
    let (tx, mut rx) = mpsc::unbounded();
    listen(listener, tx);

    // kadr quits on its own once idle (below), not when the last overlay closes.
    application().with_assets(kit::Assets).with_quit_mode(QuitMode::Explicit).run(move |cx: &mut App| {
        kit::init(cx);
        // macOS's cursors, taken on the main thread once the application is
        // there (before it, AppKit has none to give).
        platform::snapshot_cursors();
        menubar::init(cx);
        record::init(cx);
        storage::auto();
        platform::ensure_desktop();
        kadr_tray::ensure();
        run_command(command, cx);

        cx.spawn(async move |cx| {
            while let Some((args, reply)) = rx.next().await {
                if let Some(reply) = reply {
                    let _ = cx.update(|cx| dev::handle(args, reply, cx));
                    continue;
                }
                let _ = cx.update(|cx| run_args(&args, cx));
            }
        })
        .detach();

        // Exit once nothing is on screen and no job is running.
        cx.spawn(async move |cx| {
            let mut idle = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let quiet = cx.update(|cx| cx.windows().is_empty() && !kit::is_busy(cx));
                idle = if quiet { idle + 1 } else { 0 };
                // A short grace period lets a command that just arrived open its window.
                if idle >= 3 {
                    // Stop taking new commands first; anything already in flight
                    // gets a moment to land, and then we only quit if still idle.
                    platform::ipc::close();
                    cx.background_executor().timer(Duration::from_millis(300)).await;
                    if cx.update(|cx| cx.windows().is_empty() && !kit::is_busy(cx)) {
                        cx.update(|cx| cx.quit());
                        break;
                    }
                    idle = 0;
                }
            }
        })
        .detach();
    });
}
