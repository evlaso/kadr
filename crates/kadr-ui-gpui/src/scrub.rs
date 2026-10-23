//! Quick frames for scrubbing: a small JPEG of every frame (up to 60 a
//! second), made in the background when a video opens, so dragging over a
//! timeline shows the frame under the pointer at once. Seeking with ffmpeg
//! takes a tenth of a second to half a second a frame; the exact frame is
//! still fetched that way once the pointer rests.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gpui::RenderImage;

use crate::util;

/// Width of the quick frames: sharp enough to recognise, small to decode.
const WIDTH: u32 = 1280;

pub struct Scrub {
    video: PathBuf,
    dir: PathBuf,
    fps: f64,
    /// ffmpeg while it's still writing frames, and whether it decodes on
    /// the graphics card (falls back to the CPU if that fails).
    child: Mutex<Option<(Child, bool)>>,
    /// The newest request, so a fast drag doesn't queue up stale decodes.
    latest: AtomicU64,
}

impl Scrub {
    /// Start making the frames of `video`, whose frame rate is `fps`.
    pub fn start(video: &Path, fps: f64) -> Option<Arc<Scrub>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = util::cache_dir().join("scrub");
        // kadr runs once, so other processes' folders are left from a crash.
        let own = format!("{}-", std::process::id());
        for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            if !entry.file_name().to_string_lossy().starts_with(&own) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        let dir = root.join(format!("{own}{}", NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).ok()?;
        let fps = if fps.is_finite() { fps.clamp(1., 60.) } else { 60. };
        let gpu = crate::platform::cuda();
        let child = spawn(video, &dir, fps, gpu).map(|c| (c, gpu));
        Some(Arc::new(Scrub { video: video.to_owned(), dir, fps, child: Mutex::new(child), latest: AtomicU64::new(0) }))
    }

    fn path(&self, n: usize) -> PathBuf {
        self.dir.join(format!("{n:06}.jpg"))
    }

    pub fn finished(&self) -> bool {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        match child.as_mut().map(|(c, gpu)| (c.try_wait(), *gpu)) {
            Some((Ok(None), _)) => false,
            // The card couldn't decode it: again on the CPU.
            Some((Ok(Some(status)), true)) if !status.success() => {
                let _ = std::fs::remove_dir_all(&self.dir);
                let _ = std::fs::create_dir_all(&self.dir);
                *child = spawn(&self.video, &self.dir, self.fps, false).map(|c| (c, false));
                child.is_none()
            }
            _ => {
                *child = None;
                true
            }
        }
    }

    /// `n` frames evenly spread over the video, `height` pixels tall, as
    /// PNGs, once all are made: exact frames for the filmstrip.
    pub fn strip(&self, n: usize, height: u32) -> Option<Vec<Vec<u8>>> {
        if !self.finished() {
            return None;
        }
        let made = std::fs::read_dir(&self.dir).ok()?.count();
        (made > 0).then_some(())?;
        (0..n)
            .map(|i| {
                let k = ((i as f64 + 0.5) * made as f64 / n as f64) as usize;
                let small = image::open(self.path(k.min(made - 1))).ok()?.resize(u32::MAX, height, image::imageops::FilterType::Triangle);
                let mut png = Vec::new();
                small.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
                Some(png)
            })
            .collect()
    }

    /// Note request `id` as the newest; call before `frame`.
    pub fn want(&self, id: u64) {
        self.latest.fetch_max(id, Ordering::Relaxed);
    }

    /// The frame nearest `t` seconds for request `id`, as GPUI's BGRA, once
    /// it has been made and unless a newer request came in. Decodes, so call
    /// it off the main thread.
    pub fn frame(&self, t: f64, id: u64) -> Option<Arc<RenderImage>> {
        if self.latest.load(Ordering::Relaxed) > id {
            return None;
        }
        let mut n = (t.max(0.) * self.fps).round() as usize;
        if !self.path(n).exists() {
            // Past the last frame (the end of the video), or not made yet.
            if !self.finished() {
                return None;
            }
            let made = std::fs::read_dir(&self.dir).ok()?.count();
            n = n.min(made.checked_sub(1)?);
        } else if !self.path(n + 1).exists() && !self.finished() {
            // Still being written.
            return None;
        }
        let mut image = image::open(self.path(n)).ok()?.to_rgba8();
        for px in image.pixels_mut() {
            px.0.swap(0, 2);
        }
        Some(Arc::new(RenderImage::new([image::Frame::new(image)])))
    }
}

/// ffmpeg writing the frames, at low priority so it never slows the rest.
/// On an NVIDIA card it decodes and scales there, a quarter of the CPU.
fn spawn(video: &Path, dir: &Path, fps: f64, gpu: bool) -> Option<Child> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-loglevel", "error"]);
    if gpu {
        cmd.args(["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
    }
    cmd.arg("-i").arg(video);
    let filter = if gpu {
        format!("fps={fps},scale_cuda=w='min({WIDTH},iw)':h=-2,hwdownload,format=nv12")
    } else {
        format!("fps={fps},scale='min({WIDTH},iw)':-2")
    };
    cmd.args(["-an", "-vf", &filter, "-q:v", "6", "-start_number", "0"])
        .arg(dir.join("%06d.jpg"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::platform::low_priority(&mut cmd);
    cmd.spawn().ok()
}

impl Drop for Scrub {
    fn drop(&mut self) {
        if let Some((mut child, _)) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::Scrub;

    #[test]
    fn frames_come_quickly_once_made() {
        let dir = std::env::temp_dir().join(format!("kadr-scrub-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("clip.mp4");
        let made = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=120:duration=3", "-pix_fmt", "yuv420p"])
            .arg(&video)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return; // No ffmpeg here.
        }
        let scrub = Scrub::start(&video, 120.).unwrap();
        while !scrub.finished() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let began = std::time::Instant::now();
        let frame = scrub.frame(1.5, 0).expect("a frame in the middle");
        let took = began.elapsed();
        assert_eq!(frame.size(0).width.0, 1280);
        assert!(took.as_millis() < 60, "decoding took {took:?}");
        // Past the end gives the last frame.
        assert!(scrub.frame(10., 0).is_some());
        // A newer request makes older ones give up.
        scrub.want(2);
        assert!(scrub.frame(1., 1).is_none());
        drop(scrub);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
