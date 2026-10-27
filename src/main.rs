// A window app on Windows: no console opens with it (one it's started from
// gets its output, `platform::attach_console`).
#![cfg_attr(windows, windows_subsystem = "windows")]

//! The `kadr` program: its command line. Checks what it's asked, hands it
//! to a kadr that's already running (one instance takes every command),
//! else becomes that kadr and starts the UI (`kadr-ui-gpui`).

use std::io::{BufRead, BufReader, Write};

use kadr_core::cli::{Command, USAGE};
use kadr_core::i18n::t;

/// Hand the (already validated, path-resolved) arguments to a running kadr.
fn send_to_running(args: &[String]) -> bool {
    let Some(mut stream) = kadr_platform::ipc::connect() else { return false };
    let line = serde_json::to_string(args).unwrap_or_default();
    stream.write_all(format!("{line}\n").as_bytes()).is_ok()
}

/// `kadr dev …`: ask the running kadr and print its answer.
fn dev_client(args: &[String]) -> ! {
    let Some(mut stream) = kadr_platform::ipc::connect() else {
        eprintln!("kadr: no kadr is running (start one with KADR_DEV=1)");
        std::process::exit(2);
    };
    let line = serde_json::to_string(args).unwrap_or_default();
    let mut answer = String::new();
    if stream.write_all(format!("{line}\n").as_bytes()).is_err() || BufReader::new(stream).read_line(&mut answer).is_err() || answer.is_empty() {
        eprintln!("kadr: no answer");
        std::process::exit(2);
    }
    print!("{answer}");
    let ok = serde_json::from_str::<serde_json::Value>(&answer).is_ok_and(|v| v["ok"] == true);
    std::process::exit(if ok { 0 } else { 1 });
}

fn main() {
    kadr_platform::attach_console();
    // The area overlay as kadr itself, where it's one file (portable on
    // Windows) with no kadr-overlay beside it.
    #[cfg(not(target_os = "linux"))]
    if std::env::args_os().nth(1).is_some_and(|a| a == "overlay") {
        kadr_overlay::program(std::env::args_os().skip(2).collect());
    }
    kadr_ui_gpui::prepare();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h" | "--help" | "help") => return print!("{USAGE}"),
        Some("-V" | "--version") => return println!("kadr {}", kadr_core::update::CURRENT),
        Some("install-desktop") => {
            match kadr_platform::install_desktop() {
                Ok(()) => println!("{}", t("bundle.desktop_installed")),
                Err(e) => {
                    eprintln!("kadr: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        // The tray icon is its own small process, without the UI.
        Some("tray") => return kadr_tray::run(),
        Some("dev") => dev_client(&args),
        _ => {}
    }
    // Checked and with paths resolved here, before handing off.
    let command = match Command::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("kadr: {e}\n\n{USAGE}");
            std::process::exit(1);
        }
    };
    let resolved = command.args(&args);
    if send_to_running(&resolved) {
        return;
    }
    // Become the running instance.
    let listener = match kadr_platform::ipc::bind() {
        Ok(l) => l,
        Err(_) if send_to_running(&resolved) => return,
        Err(e) => {
            eprintln!("kadr: {e}");
            std::process::exit(1);
        }
    };
    // A version just put in that keeps failing to start: the old one is
    // back, and takes this command instead.
    if let Some(old) = kadr_platform::update::started() {
        drop(listener);
        kadr_platform::ipc::close();
        // Its tray too: the old one is back beside it.
        kadr_tray::restart();
        let mut argv = vec![old.into_os_string()];
        argv.extend(resolved.iter().map(Into::into));
        kadr_platform::spawn(&argv);
        return;
    }
    kadr_ui_gpui::run(command, listener);
}
