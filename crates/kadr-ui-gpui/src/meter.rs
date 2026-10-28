//! A microphone's level, read through `parec` while something shows it: the
//! recording panel's microphone button, the recording bar's microphone. The
//! level is drawn as the button's shape filled from the bottom, like in call
//! apps.

use gpui::{App, Context, Div, Styled, div};

use crate::ui::{ActiveTheme, rems_from_px};
use crate::util;

/// Steps the level is told in: a change smaller than one doesn't redraw.
const STEPS: f32 = 16.;

/// A meter reading a microphone; it stops when dropped.
pub struct Meter {
    /// The device it reads ("" for the system's default).
    pub device: String,
    pid: u32,
}

impl Drop for Meter {
    fn drop(&mut self) {
        util::signal(self.pid, util::Signal::Terminate);
    }
}

/// Read `device` ("" for the default) and hand its level, 0..1, to `set` on
/// the entity: up at once, down slowly, like a VU meter, and only when it
/// moved a step, so a quiet room doesn't redraw.
pub fn listen<T: 'static>(device: &str, cx: &mut Context<T>, set: fn(&mut T) -> &mut f32) -> Option<Meter> {
    let source = if device.is_empty() { "@DEFAULT_SOURCE@".to_owned() } else { device.to_owned() };
    let mut child = std::process::Command::new("parec")
        .args([format!("--device={source}"), "--format=s16le".into(), "--rate=16000".into(), "--channels=1".into(), "--latency-msec=40".into()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let pid = child.id();
    let (mut tx, mut rx) = futures::channel::mpsc::channel::<f32>(4);
    std::thread::spawn(move || {
        use std::io::Read;
        let Some(mut out) = child.stdout.take() else { return };
        // 50 ms of samples per reading.
        let mut buf = [0u8; 1600];
        while out.read_exact(&mut buf).is_ok() {
            let sum: f64 = buf.chunks_exact(2).map(|b| (i16::from_le_bytes([b[0], b[1]]) as f64 / 32768.).powi(2)).sum();
            let rms = (sum / 800.).sqrt().max(1e-6);
            let level = ((20. * rms.log10() + 55.) / 50.).clamp(0., 1.) as f32;
            if futures::executor::block_on(futures::SinkExt::send(&mut tx, level)).is_err() {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    });
    cx.spawn(async move |this, cx| {
        let mut shown = 0.;
        while let Some(level) = futures::StreamExt::next(&mut rx).await {
            let alive = this.update(cx, |entity, cx| {
                let now = set(entity);
                *now = level.max(*now * 0.75);
                let step = (*now * STEPS).round() / STEPS;
                if step != shown {
                    shown = step;
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
    Some(Meter { device: device.to_owned(), pid })
}

/// The level over a button: its whole shape, filled up to `level` by a
/// hard-edged gradient (a shorter box would round into a pill of its own,
/// since GPUI can't clip to the corners). A shade of the text color, so
/// darker than the button in light themes and lighter in dark ones. None
/// when it's too quiet to show.
pub fn fill(level: f32, radius: f32, cx: &App) -> Option<Div> {
    let level = (level * STEPS).round() / STEPS;
    if level <= 0.03 {
        return None;
    }
    let mut fill = cx.theme().colors().text;
    fill.alpha = 0.12;
    let mut clear = fill;
    clear.alpha = 0.;
    Some(
        div()
            .absolute()
            .inset_0()
            .rounded(rems_from_px(radius))
            .rounded_smoothing(crate::ui::SMOOTHING)
            .bg(gpui::linear_gradient(0., gpui::linear_color_stop(fill, level), gpui::linear_color_stop(clear, level + 0.01))),
    )
}
