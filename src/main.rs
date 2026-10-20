#[macro_use]
mod i18n;
mod bundle;
mod capture;
mod cards;
mod cursors;
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
mod tray;
mod ui;
mod job;
mod kit;
mod ocr;
mod util;
mod webcam;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, QuitMode};
use gpui_platform::application;
use crate::i18n::t;

const USAGE: &str = "kadr — screenshots and screen recording with a quick-access thumbnail

Usage:
  kadr capture                                   every mode on one bar
  kadr shot [smart|region|windows|fullscreen|timer|previous]
                                                 take a screenshot (smart by default; previous: the last area)
  kadr shot scroll [\"X,Y WxH\"]                  scrolling screenshot (again to finish)
  kadr open <file>                               show the thumbnail for an existing image (a .kadr opens its project)
  kadr pin <file>                                pin an image on top of windows

  kadr record                                    the recording bar (or stop a running recording)
  kadr record region|fullscreen [--mic] [--audio] [--webcam] [--keys] [--studio]
                                                 start recording at once (again to stop)
  kadr record area \"X,Y WxH\"                   the recording bar for a given area
  kadr record previous                           the recording bar for the last area picked
  kadr record stop                               stop recording
  kadr studio <video|project.kadr> [--render]    studio: zooms, background, clicks and keys (--render exports at once)
  kadr edit <image|project.kadr>                 editor: arrows, text, highlighter, blur…
  kadr trim <video> [--from S --to S]            trim a recording (no flags: the trim window)
  kadr gif <video>                               save a recording as GIF (720px, 15 fps)
  kadr history                                   history of screenshots and recordings
  kadr settings                                  settings
  kadr tray                                      tray icon (for autostart)
  kadr text [image]                              recognize text (in an area or an image) and copy it
  kadr install-desktop                           let .kadr project files open in kadr (file type, icon, menu entry)
";

const MODES: [&str; 6] = ["smart", "region", "windows", "fullscreen", "timer", "previous"];

enum Command {
    Shot(String),
    /// Scrolling screenshot, optionally of a given "X,Y WxH" area.
    Scroll(Option<String>),
    Open(PathBuf),
    Pin(PathBuf),
    Record(Option<String>, Option<record::Opts>),
    RecordStop,
    RecordArea(String),
    RecordPrevious,
    Studio(PathBuf, bool),
    Edit(PathBuf),
    Text(Option<PathBuf>),
    Trim(PathBuf, Option<(f64, f64)>),
    Gif(PathBuf),
    History,
    Settings,
    /// Every mode on one bar.
    Capture,
}

/// Run `kadr <args>` inside this instance (the capture bar's modes).
pub(crate) fn run_args(args: &[String], cx: &mut App) {
    match Command::from_args(args) {
        Ok(command) => command.run(cx),
        Err(e) => util::notify("kadr", &e),
    }
}

impl Command {
    fn from_args(args: &[String]) -> Result<Self, String> {
        let file = |i: usize| -> Result<PathBuf, String> {
            let arg = args.get(i).ok_or(t("cli.no_file_given"))?;
            std::fs::canonicalize(arg).map_err(|e| format!("{arg}: {e}"))
        };
        match args.first().map(String::as_str) {
            None => Ok(Command::Shot("smart".into())),
            Some("shot") if args.get(1).map(String::as_str) == Some("scroll") => Ok(Command::Scroll(args.get(2).cloned())),
            Some("shot") => {
                let mode = args.get(1).map(String::as_str).unwrap_or("smart");
                if MODES.contains(&mode) {
                    Ok(Command::Shot(mode.into()))
                } else {
                    Err(tr!("cli.unknown_mode", mode))
                }
            }
            Some("open") => file(1).map(Command::Open),
            Some("pin") => file(1).map(Command::Pin),
            Some("studio") => file(1).map(|p| Command::Studio(p, args.iter().any(|a| a == "--render"))),
            Some("edit") => file(1).map(Command::Edit),
            Some("text") if args.len() > 1 => file(1).map(|p| Command::Text(Some(p))),
            Some("text") => Ok(Command::Text(None)),
            Some("trim") => {
                let path = file(1)?;
                let flag = |name: &str| -> Result<Option<f64>, String> {
                    match args.iter().position(|a| a == name) {
                        Some(i) => args.get(i + 1).and_then(|v| v.parse().ok()).map(Some).ok_or(tr!("cli.needs_seconds", name)),
                        None => Ok(None),
                    }
                };
                let range = match (flag("--from")?, flag("--to")?) {
                    (None, None) => None,
                    (from, Some(to)) => Some((from.unwrap_or(0.), to)),
                    (Some(_), None) => return Err(t("cli.give_as_well").into()),
                };
                Ok(Command::Trim(path, range))
            }
            Some("gif") => file(1).map(Command::Gif),
            Some("history") => Ok(Command::History),
            Some("settings") => Ok(Command::Settings),
            Some("capture") => Ok(Command::Capture),
            Some("record") => match args.get(1).map(String::as_str) {
                None => Ok(Command::Record(None, None)),
                Some("stop") => Ok(Command::RecordStop),
                Some("previous") => Ok(Command::RecordPrevious),
                Some("area") => args.get(2).cloned().map(Command::RecordArea).ok_or_else(|| t("cli.give_area_x_y_wxh").to_string()),
                Some(mode @ ("region" | "fullscreen")) => {
                    let mut opts = record::Opts::default();
                    for flag in &args[2..] {
                        match flag.as_str() {
                            "--mic" => opts.mic = true,
                            f if f.starts_with("--mic=") => {
                                opts.mic = true;
                                opts.mic_device = f["--mic=".len()..].to_owned();
                            }
                            "--audio" => opts.audio = true,
                            "--webcam" => opts.webcam = true,
                            "--keys" => opts.keys = true,
                            "--studio" => opts.studio = true,
                            other => return Err(tr!("cli.unknown_flag", other)),
                        }
                    }
                    Ok(Command::Record(Some(mode.into()), Some(opts)))
                }
                Some(other) => Err(tr!("cli.unknown_recording_mode", other)),
            },
            Some(other) => Err(tr!("cli.unknown_command", other)),
        }
    }

    fn run(self, cx: &mut App) {
        match self {
            Command::Scroll(area) => scroll::start(area, cx),
            Command::Shot(mode) => capture::shot(mode, cx),
            Command::Open(path) | Command::Edit(path) | Command::Studio(path, false) if bundle::is_bundle(&path) => bundle::open(path, cx),
            Command::Open(path) => cards::show(path, cx),
            Command::Pin(path) => pin::open(path, cx),
            Command::Record(mode, opts) => record::command(mode, opts, cx),
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
        }
    }
}

/// Hand the (already validated, path-resolved) arguments to a running kadr.
fn send_to_running(args: &[String]) -> bool {
    let Some(mut stream) = platform::ipc::connect() else { return false };
    let line = serde_json::to_string(args).unwrap_or_default();
    stream.write_all(format!("{line}\n").as_bytes()).is_ok()
}

/// Accept commands from later invocations on a helper thread.
fn listen(listener: platform::ipc::Listener, tx: mpsc::UnboundedSender<Vec<String>>) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_ok()
                && let Ok(args) = serde_json::from_str::<Vec<String>>(&line)
                && tx.unbounded_send(args).is_err()
            {
                break;
            }
        }
    });
}

fn main() {
    i18n::set(settings::Settings::load().language);
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "--help" | "help")) {
        print!("{USAGE}");
        return;
    }
    if args.first().map(String::as_str) == Some("install-desktop") {
        match platform::install_desktop() {
            Ok(()) => println!("{}", t("bundle.desktop_installed")),
            Err(e) => {
                eprintln!("kadr: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    // The tray icon is its own small process, without GPUI.
    if args.first().map(String::as_str) == Some("tray") {
        return tray::run();
    }
    // Validate and resolve paths here, before handing off to the running instance.
    let command = match Command::from_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kadr: {e}\n\n{USAGE}");
            std::process::exit(1);
        }
    };
    let resolved: Vec<String> = match &command {
        Command::Open(p) => vec!["open".into(), p.display().to_string()],
        Command::Pin(p) => vec!["pin".into(), p.display().to_string()],
        Command::Studio(p, render) => {
            let mut v = vec!["studio".into(), p.display().to_string()];
            if *render {
                v.push("--render".into());
            }
            v
        }
        Command::Edit(p) => vec!["edit".into(), p.display().to_string()],
        Command::Text(Some(p)) => vec!["text".into(), p.display().to_string()],
        Command::Trim(p, range) => {
            let mut v = vec!["trim".into(), p.display().to_string()];
            if let Some((from, to)) = range {
                v.extend(["--from".into(), from.to_string(), "--to".into(), to.to_string()]);
            }
            v
        }
        Command::Gif(p) => vec!["gif".into(), p.display().to_string()],
        _ => args.clone(),
    };
    if send_to_running(&resolved) {
        return;
    }

    // Become the running instance.
    let listener = match platform::ipc::bind() {
        Ok(l) => l,
        Err(_) if send_to_running(&resolved) => return,
        Err(e) => {
            eprintln!("kadr: {e}");
            std::process::exit(1);
        }
    };
    let (tx, mut rx) = mpsc::unbounded();
    listen(listener, tx);

    // kadr quits on its own once idle (below), not when the last overlay closes.
    application().with_assets(kit::Assets).with_quit_mode(QuitMode::Explicit).run(move |cx: &mut App| {
        kit::init(cx);
        record::init(cx);
        storage::auto();
        platform::ensure_desktop();
        command.run(cx);

        cx.spawn(async move |cx| {
            while let Some(args) = rx.next().await {
                let _ = cx.update(|cx| match Command::from_args(&args) {
                    Ok(command) => command.run(cx),
                    Err(e) => util::notify("kadr", &e),
                });
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
