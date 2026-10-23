//! `kadr-shot`: what's on screen, for the e2e tests on macOS and Windows
//! (Linux has grim and hyprctl):
//!
//!   kadr-shot windows              the windows: pid, title, x, y, w, h (JSON)
//!   kadr-shot screen OUT.png       the first monitor
//!   kadr-shot window PID OUT.png   a process's first window
//!   kadr-shot pixel X Y            the first monitor's colour at a point (#rrggbb)
//!   kadr-shot monitor              the first monitor: x, y, w, h (JSON)

#[cfg(not(target_os = "linux"))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("windows") => {
            let list: Vec<serde_json::Value> = xcap::Window::all()
                .unwrap_or_default()
                .iter()
                .filter_map(|w| {
                    Some(serde_json::json!({
                        "pid": w.pid().ok()?,
                        "title": w.title().unwrap_or_default(),
                        "app": w.app_name().unwrap_or_default(),
                        "x": w.x().ok()?,
                        "y": w.y().ok()?,
                        "w": w.width().ok()?,
                        "h": w.height().ok()?,
                    }))
                })
                .collect();
            println!("{}", serde_json::Value::Array(list));
        }
        Some("screen") => {
            let out = args.get(1).unwrap_or_else(|| usage());
            let monitor = xcap::Monitor::all().ok().and_then(|m| m.into_iter().next()).unwrap_or_else(|| fail("no monitor"));
            save(monitor.capture_image(), out);
        }
        Some("window") => {
            let pid: u32 = args.get(1).and_then(|p| p.parse().ok()).unwrap_or_else(|| usage());
            let out = args.get(2).unwrap_or_else(|| usage());
            let window = xcap::Window::all().unwrap_or_default().into_iter().find(|w| w.pid().ok() == Some(pid)).unwrap_or_else(|| fail("no such window"));
            save(window.capture_image(), out);
        }
        Some("monitor") => {
            let m = xcap::Monitor::all().ok().and_then(|m| m.into_iter().next()).unwrap_or_else(|| fail("no monitor"));
            let n = |v: Result<i64, _>| v.unwrap_or(0);
            println!("{}", serde_json::json!({ "x": n(m.x().map(i64::from)), "y": n(m.y().map(i64::from)), "w": n(m.width().map(i64::from)), "h": n(m.height().map(i64::from)) }));
        }
        Some("pixel") => {
            let at = |i: usize| args.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or_else(|| usage());
            let (x, y) = (at(1), at(2));
            let monitor = xcap::Monitor::all().ok().and_then(|m| m.into_iter().next()).unwrap_or_else(|| fail("no monitor"));
            let image = monitor.capture_image().unwrap_or_else(|e| fail(&format!("capture: {e}")));
            // The picture is in pixels, the point in the monitor's own units.
            let scale = image.width() as f64 / monitor.width().unwrap_or(image.width()).max(1) as f64;
            let (px, py) = (((x - monitor.x().unwrap_or(0) as f64) * scale) as u32, ((y - monitor.y().unwrap_or(0) as f64) * scale) as u32);
            let p = image.get_pixel_checked(px, py).unwrap_or_else(|| fail("off the monitor"));
            println!("#{:02x}{:02x}{:02x}", p[0], p[1], p[2]);
        }
        _ => usage(),
    }
}

#[cfg(not(target_os = "linux"))]
fn save(image: Result<image::RgbaImage, xcap::XCapError>, out: &str) {
    let image = image.unwrap_or_else(|e| fail(&format!("capture: {e}")));
    image.save(out).unwrap_or_else(|e| fail(&format!("{out}: {e}")));
}

#[cfg(target_os = "linux")]
fn main() {
    fail("on Linux the e2e tests use grim and hyprctl");
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
fn usage() -> ! {
    fail("usage: kadr-shot windows | screen OUT.png | window PID OUT.png | pixel X Y | monitor")
}

fn fail(why: &str) -> ! {
    eprintln!("kadr-shot: {why}");
    std::process::exit(2);
}
