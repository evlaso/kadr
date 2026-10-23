//! Recording on Linux: gpu-screen-recorder, which captures through the
//! compositor and encodes on the GPU. SIGINT makes it write out its file,
//! SIGUSR2 toggles a pause.

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::Spec;
use crate::{Signal, signal};

pub struct Session {
    pid: u32,
    paused: AtomicBool,
    ended: Arc<AtomicBool>,
}

impl Session {
    pub fn start(spec: &Spec, ended: Box<dyn FnOnce() + Send>) -> Result<Session, String> {
        let log = std::fs::File::create(&spec.log).map_err(|e| e.to_string())?;
        let mut child = Command::new("gpu-screen-recorder").args(args(spec)).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log).spawn().map_err(|e| e.to_string())?;
        // The file appears once capture has actually started.
        let running = |c: &mut Child| matches!(c.try_wait(), Ok(None));
        for _ in 0..100 {
            if spec.out.exists() || !running(&mut child) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !spec.out.exists() || !running(&mut child) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&spec.out);
            return Err(super::log_tail(&spec.log).unwrap_or_default());
        }
        let pid = child.id();
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            flag.store(true, Ordering::SeqCst);
            ended();
        });
        Ok(Session { pid, paused: AtomicBool::new(false), ended: done })
    }

    pub fn set_paused(&self, paused: bool) {
        // One signal toggles it: only when it changes.
        if self.paused.swap(paused, Ordering::SeqCst) != paused {
            signal(self.pid, Signal::Pause);
        }
    }

    pub fn stop(&self) {
        signal(self.pid, Signal::Interrupt);
        let (pid, ended) = (self.pid, self.ended.clone());
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            if !ended.load(Ordering::SeqCst) {
                signal(pid, Signal::Kill);
            }
        });
    }
}

/// gpu-screen-recorder's arguments: it takes areas in logical coordinates
/// and scales them itself.
fn args(spec: &Spec) -> Vec<String> {
    let mut args: Vec<String> = match spec.area {
        Some((x, y, w, h)) => vec!["-w".into(), format!("{w}x{h}+{x}+{y}")],
        None => {
            // Anything above 4K is capped, like Omarchy does.
            let (pw, ph) = spec.monitor.phys;
            let size = if pw > 3840 || ph > 2160 { "3840x2160" } else { "0x0" };
            vec!["-w".into(), spec.monitor.name.clone(), "-s".into(), size.into()]
        }
    };
    if !spec.cursor {
        args.extend(["-cursor".into(), "no".into()]);
    }
    // NVENC's H.264 drops to a frame or two a second at 4K 240, while HEVC
    // keeps up.
    let codec = if spec.fps > 144 { "hevc" } else { "auto" };
    let quality = kadr_core::settings::QUALITIES.get(spec.quality).map_or("very_high", |q| q.0);
    for a in ["-k", codec, "-f", &spec.fps.to_string(), "-q", quality, "-fm", "cfr", "-fallback-cpu-encoding", "yes", "-o"] {
        args.push(a.into());
    }
    args.push(spec.out.display().to_string());
    // Separate tracks, the system's first.
    let mic = spec.mic.as_deref().map(|m| if m.is_empty() { "default_input" } else { m });
    let audio: Vec<&str> = [spec.system_audio.then_some("default_output"), mic].into_iter().flatten().collect();
    for dev in &audio {
        args.extend(["-a".into(), (*dev).into()]);
    }
    if !audio.is_empty() {
        args.extend(["-ac".into(), "aac".into()]);
    }
    args
}

/// Microphones as gpu-screen-recorder names them: (source name, description).
pub fn microphones() -> Vec<(String, String)> {
    let out = Command::new("gpu-screen-recorder").arg("--list-audio-devices").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    out.lines()
        .filter_map(|l| l.split_once('|'))
        .filter(|(name, _)| !name.starts_with("default_") && !name.ends_with(".monitor"))
        .map(|(n, d)| (n.to_owned(), d.to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_area_and_a_monitor_become_gpu_screen_recorders_arguments() {
        let monitor = crate::Mon { name: "DP-1".into(), x: 0, y: 0, w: 2560, h: 1440, phys: (5120, 2880), scale: 2., hz: 60., focused: true };
        let spec = Spec { monitor, area: Some((10, 20, 640, 360)), fps: 60, quality: 2, cursor: false, system_audio: true, mic: Some(String::new()), out: "/tmp/a.mp4".into(), log: "/tmp/a.log".into() };
        let a = args(&spec).join(" ");
        assert_eq!(a, "-w 640x360+10+20 -cursor no -k auto -f 60 -q very_high -fm cfr -fallback-cpu-encoding yes -o /tmp/a.mp4 -a default_output -a default_input -ac aac");
        let whole = Spec { area: None, fps: 240, cursor: true, system_audio: false, mic: None, ..spec };
        assert_eq!(args(&whole).join(" "), "-w DP-1 -s 3840x2160 -k hevc -f 240 -q very_high -fm cfr -fallback-cpu-encoding yes -o /tmp/a.mp4");
    }
}
