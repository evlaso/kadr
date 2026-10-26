// A window app on Windows: no console opens with it.
#![cfg_attr(windows, windows_subsystem = "windows")]

//! `kadr-overlay SPEC.json`: the overlay as a program of its own, for macOS
//! and Windows (winit wants the main thread, which kadr's UI has). Reads
//! what kadr wrote (`kadr_overlay::spec`), shows the overlay, prints one
//! JSON line with what it ended in; exits 2 when it couldn't show itself.
//!
//! `kadr-overlay --serve SOCKET` (macOS): stays up, warm, and takes picks
//! over the socket, a spec's path in, the outcome's line out; the tray
//! keeps it running (`keep_warm`).

fn main() {
    kadr_overlay::program(std::env::args_os().skip(1).collect());
}
