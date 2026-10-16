mod capture;
mod cards;
mod panel;
mod pin;
mod record;
mod studio;
mod track;
mod util;

use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk::{gio, glib, prelude::*};

const APP_ID: &str = "dev.foxqa.Kadr";

const USAGE: &str = "kadr — скриншоты с карточкой быстрого доступа

Использование:
  kadr shot [smart|region|windows|fullscreen]   снять скриншот (по умолчанию smart)
  kadr open <файл>                               показать карточку для готовой картинки
  kadr pin <файл>                                закрепить картинку поверх окон

  kadr record                                    панель записи экрана (или стоп, если идёт запись)
  kadr record region|fullscreen [--mic] [--audio] [--webcam]
                                                 начать запись сразу (повторный вызов — стоп)
  kadr record stop                               остановить запись
  kadr studio <видео>                            автозум к кликам, фон, тень и скругления
";

const MODES: [&str; 4] = ["smart", "region", "windows", "fullscreen"];

enum Command {
    Shot(String),
    Open(PathBuf),
    Pin(PathBuf),
    Record(Option<String>, Option<record::Opts>),
    RecordStop,
    Studio(PathBuf),
}

impl Command {
    fn to_args(&self) -> Vec<String> {
        match self {
            Command::Shot(mode) => vec!["shot".into(), mode.clone()],
            Command::Open(p) => vec!["open".into(), p.display().to_string()],
            Command::Pin(p) => vec!["pin".into(), p.display().to_string()],
            Command::Record(None, _) => vec!["record".into()],
            Command::Record(Some(mode), opts) => {
                let mut v = vec!["record".into(), mode.clone()];
                v.extend(opts.clone().unwrap_or_default().to_flags());
                v
            }
            Command::RecordStop => vec!["record".into(), "stop".into()],
            Command::Studio(p) => vec!["studio".into(), p.display().to_string()],
        }
    }

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
            Some("studio") => file(1).map(Command::Studio),
            Some("record") => match args.get(1).map(String::as_str) {
                None => Ok(Command::Record(None, None)),
                Some("stop") => Ok(Command::RecordStop),
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
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "--help" | "help")) {
        print!("{USAGE}");
        return glib::ExitCode::SUCCESS;
    }
    // Validate and resolve paths here, before handing off to the running instance.
    let command = match Command::from_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kadr: {e}\n\n{USAGE}");
            return glib::ExitCode::FAILURE;
        }
    };

    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|_| util::load_css());

    let cards = Rc::new(cards::Cards::default());
    let recorder = record::Recorder::new(&app, cards.clone());
    app.connect_command_line(move |app, cmdline| {
        let args: Vec<String> = cmdline
            .arguments()
            .iter()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        match Command::from_args(&args) {
            Ok(Command::Shot(mode)) => capture::shot(app, &cards, mode),
            Ok(Command::Open(path)) => cards.show(app, path),
            Ok(Command::Pin(path)) => pin::open(app, &path),
            Ok(Command::Record(mode, opts)) => recorder.command(mode, opts),
            Ok(Command::RecordStop) => recorder.command_stop(),
            Ok(Command::Studio(path)) => studio::render(app, &cards, path),
            Err(e) => {
                util::notify("kadr", &e);
                return glib::ExitCode::FAILURE;
            }
        }
        glib::ExitCode::SUCCESS
    });

    let mut argv = vec!["kadr".to_string()];
    argv.extend(command.to_args());
    app.run_with_args(&argv)
}
