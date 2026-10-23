//! kadr's command line, without a UI or the OS: what each command takes,
//! checked and with its files' paths resolved, so the program can hand it
//! to a kadr that's already running before any UI starts.

use std::path::PathBuf;

use crate::i18n::t;

pub const USAGE: &str = "kadr — screenshots and screen recording with a quick-access thumbnail

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
  kadr dev <command>                             test hooks of a kadr started with KADR_DEV=1
                                                 (windows, dump, find, click, drag, key, type, state, shot, mem)
  kadr text [image]                              recognize text (in an area or an image) and copy it
  kadr install-desktop                           let .kadr project files open in kadr (file type, icon, menu entry)
";

/// What `kadr shot` takes.
pub const MODES: [&str; 6] = ["smart", "region", "windows", "fullscreen", "timer", "previous"];

/// `kadr record region|fullscreen`'s flags.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct RecordFlags {
    pub mic: bool,
    /// PipeWire source name; empty means the system default input.
    pub mic_device: String,
    pub audio: bool,
    pub webcam: bool,
    pub keys: bool,
    pub studio: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Command {
    Shot(String),
    /// Scrolling screenshot, optionally of a given "X,Y WxH" area.
    Scroll(Option<String>),
    Open(PathBuf),
    Pin(PathBuf),
    Record(Option<String>, Option<RecordFlags>),
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

impl Command {
    pub fn parse(args: &[String]) -> Result<Self, String> {
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
                    Err(crate::tr!("cli.unknown_mode", mode))
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
                        Some(i) => args.get(i + 1).and_then(|v| v.parse().ok()).map(Some).ok_or(crate::tr!("cli.needs_seconds", name)),
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
                    let mut opts = RecordFlags::default();
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
                            other => return Err(crate::tr!("cli.unknown_flag", other)),
                        }
                    }
                    Ok(Command::Record(Some(mode.into()), Some(opts)))
                }
                Some(other) => Err(crate::tr!("cli.unknown_recording_mode", other)),
            },
            Some(other) => Err(crate::tr!("cli.unknown_command", other)),
        }
    }


    /// The command's arguments again, with its files' paths resolved: what's
    /// handed to a running kadr.
    pub fn args(&self, given: &[String]) -> Vec<String> {
        match self {
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
    _ => given.to_vec(),
}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse_and_go_back_to_their_arguments() {
        let a = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(Command::parse(&[]), Ok(Command::Shot("smart".into())));
        assert_eq!(Command::parse(&a("shot region")), Ok(Command::Shot("region".into())));
        assert!(Command::parse(&a("shot nowhere")).is_err());
        let rec = Command::parse(&a("record fullscreen --mic=usb --studio")).unwrap();
        assert_eq!(rec, Command::Record(Some("fullscreen".into()), Some(RecordFlags { mic: true, mic_device: "usb".into(), studio: true, ..Default::default() })));
        assert_eq!(rec.args(&a("record fullscreen --mic=usb --studio")), a("record fullscreen --mic=usb --studio"));
        let here = std::env::current_dir().unwrap();
        let edit = Command::parse(&a("edit .")).unwrap();
        assert_eq!(edit.args(&a("edit .")), vec!["edit".to_owned(), here.canonicalize().unwrap().display().to_string()]);
    }
}
