//! The warm overlay's own picture of the screen on macOS, through
//! ScreenCaptureKit: the displays, a content filter and a configuration for
//! each are made once and a first picture taken (`Grabber::new`), so a pick
//! costs one screenshot, with no cold start of its own.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2_core_graphics::{CGDataProvider, CGImage};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration};

/// How long a picture may take before the pick goes kadr's own way.
const WAIT: Duration = Duration::from_secs(2);

pub struct Grabber {
    displays: Vec<Display>,
}

struct Display {
    /// Its frame on the desktop, in points.
    frame: (f64, f64, f64, f64),
    filter: Retained<SCContentFilter>,
    /// Its picture's size the last one was taken at, and how.
    size: (usize, usize),
    config: Retained<SCStreamConfiguration>,
}

impl Grabber {
    /// The displays as they are now, and a first picture to warm it up;
    /// None where ScreenCaptureKit can't take one (no permission).
    pub fn new() -> Option<Grabber> {
        let from = Instant::now();
        let mut g = Grabber { displays: displays()? };
        let d = g.displays.first()?;
        let (frame, size) = (d.frame, d.size);
        g.grab((frame.0, frame.1), (frame.2 as u32, frame.3 as u32), size.0 as f64 / frame.2.max(1.))?;
        eprintln!("kadr-overlay timing: sck_warm={}ms", from.elapsed().as_millis());
        Some(g)
    }

    /// The monitor at `origin` (points), `logical` in size, in physical
    /// pixels; the displays are looked up again when none is there.
    pub fn grab(&mut self, origin: (f64, f64), logical: (u32, u32), scale: f64) -> Option<image::RgbaImage> {
        let at = |ds: &[Display]| ds.iter().position(|d| (d.frame.0 - origin.0).abs() < 1. && (d.frame.1 - origin.1).abs() < 1.);
        let i = match at(&self.displays) {
            Some(i) => i,
            None => {
                self.displays = displays()?;
                at(&self.displays)?
            }
        };
        let d = &mut self.displays[i];
        let size = ((logical.0 as f64 * scale).round() as usize, (logical.1 as f64 * scale).round() as usize);
        if d.size != size {
            d.config = config(size);
            d.size = size;
        }
        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |image: *mut CGImage, _: *mut NSError| {
            // SAFETY: the picture lives through the handler.
            let _ = tx.send(unsafe { image.as_ref() }.and_then(to_rgba));
        });
        // SAFETY: the filter and configuration are alive and the handler
        // is retained by the call.
        unsafe { SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&d.filter, &d.config, Some(&handler)) };
        rx.recv_timeout(WAIT).ok().flatten()
    }
}

/// Every display with its filter (the whole of it, no window left out).
fn displays() -> Option<Vec<Display>> {
    let (tx, rx) = mpsc::channel();
    let handler = RcBlock::new(move |content: *mut SCShareableContent, _: *mut NSError| {
        // SAFETY: retained past the handler.
        let _ = tx.send(unsafe { Retained::retain(content) });
    });
    // SAFETY: the handler is retained by the call.
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    let content = rx.recv_timeout(WAIT).ok().flatten()?;
    let none = NSArray::new();
    // SAFETY: plain getters and initialisers on live objects.
    let found: Vec<Display> = unsafe { content.displays() }
        .iter()
        .map(|d| {
            let f = unsafe { d.frame() };
            let filter = unsafe { SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &d, &none) };
            // Its pixels per point: the size the picture is first asked at.
            let points = (f.size.width.max(1.), f.size.height.max(1.));
            let scale = mode_scale(unsafe { d.displayID() }).unwrap_or(1.);
            let size = ((points.0 * scale).round() as usize, (points.1 * scale).round() as usize);
            Display { frame: (f.origin.x, f.origin.y, points.0, points.1), filter, size, config: config(size) }
        })
        .collect();
    (!found.is_empty()).then_some(found)
}

/// The display mode's pixels per point.
fn mode_scale(id: u32) -> Option<f64> {
    let mode = objc2_core_graphics::CGDisplayCopyDisplayMode(id)?;
    let (px, pt) = (objc2_core_graphics::CGDisplayMode::pixel_width(Some(&mode)), objc2_core_graphics::CGDisplayMode::width(Some(&mode)));
    (pt > 0).then(|| px as f64 / pt as f64)
}

/// A picture `size` big, without the pointer.
fn config(size: (usize, usize)) -> Retained<SCStreamConfiguration> {
    // SAFETY: a new configuration, set before it's used.
    unsafe {
        let c = SCStreamConfiguration::new();
        c.setWidth(size.0);
        c.setHeight(size.1);
        c.setShowsCursor(false);
        c
    }
}

/// ScreenCaptureKit's BGRA picture as RGBA, its rows unpadded.
fn to_rgba(image: &CGImage) -> Option<image::RgbaImage> {
    let (w, h) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
    let row = CGImage::bytes_per_row(Some(image));
    let provider = CGImage::data_provider(Some(image))?;
    let data = CGDataProvider::data(Some(&provider))?;
    // SAFETY: the data isn't changed while it's read.
    let bytes = unsafe { data.as_bytes_unchecked() };
    let mut out = Vec::with_capacity(w * h * 4);
    for line in bytes.chunks(row).take(h) {
        for px in line.get(..w * 4)?.chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    image::RgbaImage::from_raw(w as u32, h as u32, out)
}
