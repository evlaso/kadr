//! `kadr-overlay SPEC.json`: the overlay as a program of its own, for macOS
//! and Windows (winit wants the main thread, which kadr's UI has). Reads
//! what kadr wrote (`kadr_overlay::spec`), shows the overlay, prints one
//! JSON line with what it ended in; exits 2 when it couldn't show itself.
//!
//! `kadr-overlay --serve SOCKET` (macOS): stays up, warm, and takes picks
//! over the socket, a spec's path in, the outcome's line out; the tray
//! keeps it running (`keep_warm`).

fn main() {
    kadr_overlay::started();
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: kadr-overlay SPEC.json | --serve SOCKET");
        std::process::exit(2);
    };
    if path == "--serve" {
        #[cfg(all(unix, not(target_os = "linux")))]
        if let Some(socket) = std::env::args_os().nth(2) {
            let _ = kadr_overlay::serve(std::path::Path::new(&socket));
        }
        eprintln!("kadr-overlay: --serve needs a socket, and macOS");
        std::process::exit(2);
    }
    let Some((screen, options, lang)) = kadr_overlay::spec::read(std::path::Path::new(&path)) else {
        eprintln!("kadr-overlay: can't read {}", std::path::Path::new(&path).display());
        std::process::exit(2);
    };
    let read_ms = kadr_overlay::since_start();
    kadr_core::i18n::set(kadr_core::i18n::Lang::from_code(&lang));
    eprintln!("kadr-overlay timing: read={read_ms}ms");
    // The words were read before the language was set.
    let options = kadr_overlay::Options { free: kadr_core::i18n::t("picker.free").to_owned(), ..options };
    match kadr_overlay::pick(&screen, &options) {
        Ok(outcome) => println!("{}", kadr_overlay::spec::outcome_line(outcome)),
        Err(()) => {
            eprintln!("kadr-overlay: couldn't show the overlay here");
            std::process::exit(2);
        }
    }
}
