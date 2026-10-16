mod capture;
mod cards;
mod editor;
mod history;
mod palette;
mod panel;
mod pin;
mod record;
mod studio;
mod studio_editor;
mod track;
mod trim;
mod job;
mod kit;
mod ocr;
mod util;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, QuitMode};
use gpui_platform::application;

const USAGE: &str = "kadr — скриншоты с карточкой быстрого доступа

Использование:
  kadr shot [smart|region|windows|fullscreen]   снять скриншот (по умолчанию smart)
  kadr open <файл>                               показать карточку для готовой картинки
  kadr pin <файл>                                закрепить картинку поверх окон

  kadr record                                    панель записи экрана (или стоп, если идёт запись)
  kadr record region|fullscreen [--mic] [--audio] [--webcam] [--keys]
                                                 начать запись сразу (повторный вызов — стоп)
  kadr record area \"X,Y WxH\"                   панель записи для заданной области
  kadr record stop                               остановить запись
  kadr studio <видео> [--render]                 студия: зумы, фон, клики и клавиши (--render — сразу экспорт)
  kadr edit <картинка>                           редактор: стрелки, текст, маркер, размытие…
  kadr trim <видео> [--from С --to С]            обрезать запись (без флагов — окно обрезки)
  kadr gif <видео>                               сохранить запись как GIF (720px, 15 к/с)
  kadr history                                   история скриншотов и записей
  kadr text [картинка]                           распознать текст (в области или на картинке) и скопировать
";

const MODES: [&str; 4] = ["smart", "region", "windows", "fullscreen"];

enum Command {
    Shot(String),
    Open(PathBuf),
    Pin(PathBuf),
    Record(Option<String>, Option<record::Opts>),
    RecordStop,
    RecordArea(String),
    Studio(PathBuf, bool),
    Edit(PathBuf),
    Text(Option<PathBuf>),
    Trim(PathBuf, Option<(f64, f64)>),
    Gif(PathBuf),
    History,
}

impl Command {
    fn from_args(args: &[String]) -> Result<Self, String> {
        let file = |i: usize| -> Result<PathBuf, String> {
            let arg = args.get(i).ok_or("не указан файл")?;
            std::fs::canonicalize(arg).map_err(|e| format!("{arg}: {e}"))
        };
        match args.first().map(String::as_str) {
            None => Ok(Command::Shot("smart".into())),
            Some("shot") => {
                let mode = args.get(1).map(String::as_str).unwrap_or("smart");
                if MODES.contains(&mode) {
                    Ok(Command::Shot(mode.into()))
                } else {
                    Err(format!("неизвестный режим: {mode}"))
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
                        Some(i) => args.get(i + 1).and_then(|v| v.parse().ok()).map(Some).ok_or(format!("{name}: нужны секунды")),
                        None => Ok(None),
                    }
                };
                let range = match (flag("--from")?, flag("--to")?) {
                    (None, None) => None,
                    (from, Some(to)) => Some((from.unwrap_or(0.), to)),
                    (Some(_), None) => return Err("укажи и --to".into()),
                };
                Ok(Command::Trim(path, range))
            }
            Some("gif") => file(1).map(Command::Gif),
            Some("history") => Ok(Command::History),
            Some("record") => match args.get(1).map(String::as_str) {
                None => Ok(Command::Record(None, None)),
                Some("stop") => Ok(Command::RecordStop),
                Some("area") => args.get(2).cloned().map(Command::RecordArea).ok_or_else(|| "укажи область: \"X,Y WxH\"".to_string()),
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
                            other => return Err(format!("неизвестный флаг: {other}")),
                        }
                    }
                    Ok(Command::Record(Some(mode.into()), Some(opts)))
                }
                Some(other) => Err(format!("неизвестный режим записи: {other}")),
            },
            Some(other) => Err(format!("неизвестная команда: {other}")),
        }
    }

    fn run(self, cx: &mut App) {
        match self {
            Command::Shot(mode) => capture::shot(mode, cx),
            Command::Open(path) => cards::show(path, cx),
            Command::Pin(path) => pin::open(path, cx),
            Command::Record(mode, opts) => record::command(mode, opts, cx),
            Command::RecordStop => record::command_stop(cx),
            Command::RecordArea(area) => record::command_area(&area, cx),
            Command::Studio(path, false) => studio_editor::open(path, cx),
            Command::Studio(path, true) => studio::render(path, cx),
            Command::Edit(path) => editor::open(path, cx),
            Command::Text(None) => ocr::region(cx),
            Command::Text(Some(path)) => ocr::image(path, cx),
            Command::Trim(path, None) => trim::open(path, cx),
            Command::Trim(path, Some((from, to))) => trim::save_trim(path, from, to, cx),
            Command::Gif(path) => trim::quick_gif(path, cx),
            Command::History => history::open(cx),
        }
    }
}

fn socket_path() -> PathBuf {
    util::runtime_dir().join("kadr.sock")
}

/// Hand the (already validated, path-resolved) arguments to a running kadr.
fn send_to_running(args: &[String]) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path()) else { return false };
    let line = serde_json::to_string(args).unwrap_or_default();
    stream.write_all(format!("{line}\n").as_bytes()).is_ok()
}

/// Accept commands from later invocations on a helper thread.
fn listen(listener: UnixListener, tx: mpsc::UnboundedSender<Vec<String>>) {
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "--help" | "help")) {
        print!("{USAGE}");
        return;
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

    // Become the running instance. A stale socket from a crash is replaced.
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(_) if send_to_running(&resolved) => return,
        Err(e) => {
            eprintln!("kadr: {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    let (tx, mut rx) = mpsc::unbounded();
    listen(listener, tx);

    // kadr quits on its own once idle (below), not when the last overlay closes.
    application().with_assets(kit::Assets).with_quit_mode(QuitMode::Explicit).run(move |cx: &mut App| {
        kit::init(cx);
        record::init(cx);
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
                    let _ = std::fs::remove_file(socket_path());
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
