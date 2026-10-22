//! Long ffmpeg runs with their progress: what the app shows as a card or a
//! toast and can cancel.

use std::cell::Cell;
use std::ffi::OsStr;
use std::process::Stdio;
use std::time::Duration;

use futures::{AsyncBufReadExt, StreamExt, io::BufReader};
use kadr_core::dirs;
use kadr_core::i18n::t;

/// Shared between the running job and its toast.
#[derive(Default)]
pub struct Job {
    fraction: Cell<f64>,
    cancelled: Cell<bool>,
}

impl Job {
    pub fn cancelled(&self) -> bool {
        self.cancelled.get()
    }

    /// How far along, 0..1 (short of 1 until it's done).
    pub fn fraction(&self) -> f32 {
        self.fraction.get().clamp(0.0, 0.99) as f32
    }

    /// Stop it: the running ffmpeg is ended within a moment.
    pub fn cancel(&self) {
        self.cancelled.set(true);
    }
}

/// Run ffmpeg (argv without "-progress", which is added) and report progress
/// against `duration` seconds of output. stderr goes to ~/.cache/kadr/<log>.
pub async fn ffmpeg(job: &Job, argv: &[&OsStr], duration: f64, log: &str) -> Result<(), String> {
    let log = dirs::cache().join(log);
    let _ = std::fs::create_dir_all(dirs::cache());
    let stderr = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let (cmd, rest) = argv.split_first().ok_or(t("job.empty_command"))?;
    let mut child = async_process::Command::new(cmd)
        .args(["-nostats", "-progress", "pipe:1"])
        .args(rest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or(t("job.no_output_from_ffmpeg"))?;
    let mut lines = BufReader::new(stdout).lines();
    loop {
        // A line of progress, or a look at whether it was cancelled.
        let tick = async_io::Timer::after(Duration::from_millis(100));
        match futures::future::select(lines.next(), tick).await {
            futures::future::Either::Left((Some(Ok(line)), _)) => {
                if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                    job.fraction.set(us / 1e6 / duration.max(0.1));
                }
            }
            futures::future::Either::Left(_) => break,
            futures::future::Either::Right(_) => {}
        }
        if job.cancelled() {
            let _ = child.kill();
            break;
        }
    }
    let status = child.status().await.map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        let tail = std::fs::read_to_string(&log).unwrap_or_default();
        Err(tail.lines().rev().take(3).collect::<Vec<_>>().join("\n"))
    }
}
