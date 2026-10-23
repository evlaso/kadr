//! Recording the screen, the same for the app on every system: what to
//! record (`Spec`), a running recording (`Recording`: pause, stop) and the
//! microphones there are. Each system has its own backend: Linux runs
//! gpu-screen-recorder (`record/gsr.rs`), macOS ScreenCaptureKit with
//! AVAssetWriter, Windows Windows.Graphics.Capture with Media Foundation.
//!
//! The file is an MP4: H.264 (HEVC above 144 fps on Linux), and the audio as
//! AAC tracks, the system's first, then the microphone's, for the app to mix
//! afterwards.

use std::path::PathBuf;

use crate::Mon;

#[cfg(target_os = "linux")]
#[path = "record/gsr.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "record/sck.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "record/none.rs"]
mod imp;

/// What to record and how.
#[derive(Clone, Debug)]
pub struct Spec {
    /// The monitor recorded, or the one the area is on.
    pub monitor: Mon,
    /// The area, in the desktop's logical layout (as `Mon`'s x and y), whole
    /// pixels on the monitor and even in size (the app fits it); None takes
    /// the whole monitor.
    pub area: Option<(i32, i32, i32, i32)>,
    /// Frames a second.
    pub fps: u32,
    /// Index into `kadr_core::settings::QUALITIES`.
    pub quality: usize,
    /// The pointer drawn into the video.
    pub cursor: bool,
    /// What the computer plays.
    pub system_audio: bool,
    /// A microphone: an id from `microphones`, or "" for the system's default.
    pub mic: Option<String>,
    pub out: PathBuf,
    /// Where the backend says what went wrong (`log_tail`).
    pub log: PathBuf,
}

/// A recording under way. Dropping it doesn't stop it: `stop` does.
pub struct Recording(imp::Session);

impl Recording {
    /// Starts recording and returns once frames are coming in, or why it
    /// couldn't. `ended` runs once, on another thread, when the recording is
    /// over (stopped and its file written, or failed on its own).
    pub fn start(spec: &Spec, ended: impl FnOnce() + Send + 'static) -> Result<Recording, String> {
        imp::Session::start(spec, Box::new(ended)).map(Recording)
    }

    /// Pauses or goes on; the paused time is left out of the file.
    pub fn set_paused(&self, paused: bool) {
        self.0.set_paused(paused)
    }

    /// Asks it to finish and write its file; `ended` runs once it has (or
    /// after a few seconds, with whatever was written).
    pub fn stop(&self) {
        self.0.stop()
    }
}

/// The microphones there are: (id, what to show).
pub fn microphones() -> Vec<(String, String)> {
    imp::microphones()
}

/// The last lines of a recording's log, to show why it failed.
pub fn log_tail(log: &std::path::Path) -> Option<String> {
    let log = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(3)..].join("\n");
    (!tail.is_empty()).then_some(tail)
}
