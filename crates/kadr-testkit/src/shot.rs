//! `kadr-shot`: what's on screen, for the e2e tests on macOS and Windows
//! (Linux has grim and hyprctl):
//!
//!   kadr-shot windows              the windows: pid, title, x, y, w, h (JSON)
//!   kadr-shot screen OUT.png       the first monitor
//!   kadr-shot window PID OUT.png   a process's first window

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
    fail("usage: kadr-shot windows | screen OUT.png | window PID OUT.png")
}

fn fail(why: &str) -> ! {
    eprintln!("kadr-shot: {why}");
    std::process::exit(2);
}
