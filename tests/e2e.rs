//! End to end: the real kadr binary in a nested Hyprland of its own
//! (`scripts/nested.sh`), driven through its dev hooks and checked by what
//! it writes and shows. `just e2e`; each test starts a session of its own.
//!
//! They're ignored in plain `cargo test` (they need a running Hyprland to
//! nest in, and take seconds each).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use image::RgbaImage;
use serde_json::Value;

const KADR: &str = env!("CARGO_BIN_EXE_kadr");

/// The virtual pointer (`crates/kadr-testkit`), built by `just e2e`.
fn pointer() -> PathBuf {
    root().join("target/release/kadr-pointer")
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A nested Hyprland with kadr's folders, stopped when dropped.
struct Session {
    dir: PathBuf,
    env: Vec<(String, String)>,
}

impl Session {
    fn start() -> Session {
        let out = Command::new(root().join("scripts/nested.sh")).arg("start").stderr(Stdio::inherit()).output().expect("scripts/nested.sh");
        assert!(out.status.success(), "the nested session didn't start");
        let env: Vec<(String, String)> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix("export ")?.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
            .collect();
        let dir = PathBuf::from(&env.iter().find(|(k, _)| k == "KADR_E2E_DIR").expect("KADR_E2E_DIR").1);
        Session { dir, env }
    }

    fn var(&self, key: &str) -> PathBuf {
        PathBuf::from(&self.env.iter().find(|(k, _)| k == key).unwrap_or_else(|| panic!("{key}")).1)
    }

    /// A command run inside the session.
    fn cmd(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        c.envs(self.env.iter().map(|(k, v)| (k, v)));
        c.stdin(Stdio::null());
        c
    }

    /// `kadr <args>` in the background (it stays to serve its windows).
    fn spawn(&self, args: &[&str]) {
        let log = std::fs::File::create(self.dir.join("kadr.log")).ok();
        let mut c = self.cmd(KADR);
        c.args(args).stdout(Stdio::null());
        if let Some(log) = log {
            c.stderr(log);
        }
        c.spawn().expect("kadr");
    }

    /// `kadr dev <args>`: the answer's result, or a panic with its error.
    fn dev(&self, args: &[&str]) -> Value {
        let out = self.cmd(KADR).arg("dev").args(args).output().expect("kadr dev");
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("kadr dev {args:?}: {}", String::from_utf8_lossy(&out.stderr)));
        assert_eq!(v["ok"], true, "kadr dev {args:?}: {v}");
        v["result"].clone()
    }

    /// The same, None while kadr isn't there yet or says no.
    fn try_dev(&self, args: &[&str]) -> Option<Value> {
        let out = self.cmd(KADR).arg("dev").args(args).output().ok()?;
        let v: Value = serde_json::from_slice(&out.stdout).ok()?;
        (v["ok"] == true).then(|| v["result"].clone())
    }

    /// The window of this kind, once it's up.
    fn window(&self, kind: &str) -> Value {
        wait(Duration::from_secs(10), || self.try_dev(&["windows"])?.as_array()?.iter().find(|w| w["kind"] == kind).cloned()).unwrap_or_else(|| panic!("no {kind} window"))
    }

    /// `kadr-pointer <args>`: a real pointer in the session.
    fn pointer(&self, args: &[&str]) {
        let ok = self.cmd(pointer().to_str().unwrap()).args(args).status().expect("kadr-pointer (just e2e builds it)");
        assert!(ok.success(), "kadr-pointer {args:?}");
    }

    /// The middle of an element, by its id, in screen pixels: its place in
    /// the window plus where Hyprland put the (focused, floating) window,
    /// which Wayland doesn't tell the window itself.
    fn middle(&self, id: &str) -> (String, String) {
        let b = self.dev(&["find", id])["bounds"].clone();
        let out = self.cmd("hyprctl").args(["-j", "activewindow"]).output().expect("hyprctl");
        let at: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        let origin = |k: usize| at["at"][k].as_f64().unwrap_or(0.);
        let mid = |i: usize, j: usize| (origin(i) + b[i].as_f64().unwrap_or(0.) + b[j].as_f64().unwrap_or(0.) / 2.).round().to_string();
        (mid(0, 2), mid(1, 3))
    }

    /// The screen, or an area of it, as the compositor has it.
    fn grab(&self, area: Option<(u32, u32, u32, u32)>) -> RgbaImage {
        let path = self.dir.join("grab.png");
        let mut c = self.cmd("grim");
        if let Some((x, y, w, h)) = area {
            c.args(["-g", &format!("{x},{y} {w}x{h}")]);
        }
        assert!(c.arg(&path).status().expect("grim").success(), "grim");
        image::open(&path).expect("grab").to_rgba8()
    }

    /// kadr's main process (not the tray), the session's only one.
    fn kadr_pids(&self) -> Vec<u32> {
        let mut pids = Vec::new();
        for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
            let Ok(env) = std::fs::read(entry.path().join("environ")) else { continue };
            let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else { continue };
            let ours = env.split(|&b| b == 0).any(|kv| kv == format!("XDG_RUNTIME_DIR={}", self.dir.display()).as_bytes());
            let argv: Vec<&[u8]> = cmdline.split(|&b| b == 0).collect();
            let kadr = argv.first().is_some_and(|a| a.ends_with(b"kadr")) && argv.get(1).is_none_or(|a| *a != b"tray" && *a != b"dev");
            if ours && kadr {
                pids.push(pid);
            }
        }
        pids
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = Command::new(root().join("scripts/nested.sh")).arg("stop").arg(&self.dir).status();
    }
}

fn wait<T>(timeout: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let end = Instant::now() + timeout;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if Instant::now() > end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// How alike two pictures are: the share of pixels within 8 of each other in
/// every channel.
fn alike(a: &RgbaImage, b: &RgbaImage) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions(), "sizes differ");
    let same = a.pixels().zip(b.pixels()).filter(|(p, q)| (0..4).all(|i| (p.0[i] as i32 - q.0[i] as i32).abs() <= 8)).count();
    same as f64 / (a.width() * a.height()) as f64
}

/// Mean brightness, 0..1.
fn brightness(img: &RgbaImage) -> f64 {
    img.pixels().map(|p| (p.0[0] as f64 + p.0[1] as f64 + p.0[2] as f64) / 765.).sum::<f64>() / (img.width() * img.height()) as f64
}

fn fixture(session: &Session, name: &str) -> PathBuf {
    let path = session.dir.join(name);
    std::fs::copy(root().join("tests/golden/cases/arrow/base.png"), &path).expect("fixture");
    path
}

/// The newest file in a folder.
fn newest(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok()).map(|e| e.path())
}

/// Scenario 1: the hotkey's command brings the picker up in time, a drag
/// picks an area, and the saved screenshot is that area of the screen.
#[test]
#[ignore = "e2e: just e2e"]
fn an_area_screenshot_is_what_was_on_screen() {
    let s = Session::start();
    // Something to take a picture of: a pinned picture in the middle.
    let picture = fixture(&s, "pinned.png");
    s.spawn(&["pin", picture.to_str().unwrap()]);
    s.window("kadr-pin");
    std::thread::sleep(Duration::from_millis(300));
    let area = (760, 420, 400, 240);
    let before = s.grab(Some(area));

    let t0 = Instant::now();
    s.spawn(&["shot", "region"]);
    // The overlay is a layer of its own (kadr-overlay), not a GPUI window:
    // the compositor says when it's up.
    let layer = || {
        let out = s.cmd("hyprctl").args(["-j", "layers"]).output().ok()?;
        String::from_utf8_lossy(&out.stdout).contains("\"kadr-picker\"").then_some(())
    };
    wait(Duration::from_secs(3), layer).expect("no picker on screen");
    let shown = t0.elapsed();
    assert!(shown < Duration::from_millis(300), "the picker took {shown:?}");
    std::thread::sleep(Duration::from_millis(100));

    // Dragged with a real pointer, corner to corner.
    let n = |v: i32| v.to_string();
    s.pointer(&["drag", &n(area.0 as i32), &n(area.1 as i32), &n((area.0 + area.2) as i32), &n((area.1 + area.3) as i32)]);
    let shots = s.var("OMARCHY_SCREENSHOT_DIR");
    let saved = wait(Duration::from_secs(5), || newest(&shots).filter(|p| p.extension().is_some_and(|e| e == "png"))).expect("no screenshot saved");
    std::thread::sleep(Duration::from_millis(200));
    let shot = image::open(&saved).expect("the screenshot").to_rgba8();
    let share = alike(&shot, &before);
    assert!(share > 0.99, "the screenshot differs from the screen: {:.1}% alike", share * 100.);

    // And its card shows in the corner.
    let card = s.window("kadr");
    let state = s.dev(&["state", &card["id"].to_string()]);
    let paths: Vec<&str> = state["view"]["cards"].as_array().into_iter().flatten().filter_map(|c| c["path"].as_str()).collect();
    assert_eq!(paths, [saved.to_str().unwrap()], "the card is the screenshot's");
}

/// A scrolling screenshot: a terminal's numbered lines, scrolled by the
/// wheel under the bar, come out as one picture longer than the area, ending
/// with what the area shows at the end. Needs foot to have something to
/// scroll; skipped without it.
#[test]
#[ignore = "e2e: just e2e"]
fn a_scrolled_area_is_stitched() {
    if Command::new("foot").arg("--version").output().is_err() {
        eprintln!("no foot: skipped");
        return;
    }
    let s = Session::start();
    let lines = "for i in $(seq 1 300); do printf '%04d %s\\n' $i \"$(printf '%.0s#' $(seq 1 $((i % 50 + 1))))\"; done; sleep 600";
    s.cmd("foot").args(["sh", "-c", lines]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("foot");
    let up = || {
        let out = s.cmd("hyprctl").args(["-j", "clients"]).output().ok()?;
        String::from_utf8_lossy(&out.stdout).contains("\"foot\"").then_some(())
    };
    wait(Duration::from_secs(5), up).expect("no terminal");
    std::thread::sleep(Duration::from_millis(500));
    let area = (200, 150, 1400, 700);
    s.pointer(&["move", "900", "500"]);
    s.pointer(&["scroll", "0", "-200"]);
    std::thread::sleep(Duration::from_millis(300));

    s.spawn(&["shot", "scroll", "200,150 1400x700"]);
    s.window("kadr-scroll");
    std::thread::sleep(Duration::from_millis(500));
    for _ in 0..8 {
        s.pointer(&["scroll", "0", "3"]);
        std::thread::sleep(Duration::from_millis(150));
    }
    std::thread::sleep(Duration::from_millis(500));
    let end = s.grab(Some(area));
    // The same hotkey again finishes.
    assert!(s.cmd(KADR).args(["shot", "scroll"]).status().expect("kadr").success());
    let shots = s.var("OMARCHY_SCREENSHOT_DIR");
    let saved = wait(Duration::from_secs(5), || newest(&shots).filter(|p| p.extension().is_some_and(|e| e == "png"))).expect("no screenshot saved");
    std::thread::sleep(Duration::from_millis(200));
    let shot = image::open(&saved).expect("the screenshot").to_rgba8();
    assert_eq!(shot.width(), area.2, "as wide as the area");
    assert!(shot.height() > area.3 * 2, "stitched: {} rows from a {} high area", shot.height(), area.3);
    let bottom = image::imageops::crop_imm(&shot, 0, shot.height() - area.3, area.2, area.3).to_image();
    let share = alike(&bottom, &end);
    assert!(share > 0.99, "it ends with what the area showed: {:.1}% alike", share * 100.);
}

/// The editor's window on screen, its place and size, under a magenta
/// picture pinned over it; then `kadr shot windows` clicked on it. The shot
/// and the window's size.
fn shoot_covered_editor(s: &Session) -> (RgbaImage, (i64, i64), PathBuf) {
    let picture = fixture(s, "window.png");
    s.spawn(&["edit", picture.to_str().unwrap()]);
    s.window("kadr-editor");
    std::thread::sleep(Duration::from_millis(500));
    let out = s.cmd("hyprctl").args(["-j", "clients"]).output().expect("hyprctl");
    let clients: Value = serde_json::from_slice(&out.stdout).expect("clients");
    let editor = clients.as_array().into_iter().flatten().find(|c| c["class"] == "kadr-editor").expect("the editor's window");
    let n = |v: &Value| v.as_i64().unwrap_or(0);
    let (x, y, w, h) = (n(&editor["at"][0]), n(&editor["at"][1]), n(&editor["size"][0]), n(&editor["size"][1]));
    // Magenta over the middle of the screen, where the editor is too.
    let cover = s.dir.join("cover.png");
    RgbaImage::from_pixel(300, 200, image::Rgba([255, 0, 255, 255])).save(&cover).expect("cover");
    s.spawn(&["pin", cover.to_str().unwrap()]);
    s.window("kadr-pin");
    std::thread::sleep(Duration::from_millis(300));
    let on_screen = magenta(&s.grab(Some((x as u32, y as u32, w as u32, h as u32))));
    assert!(on_screen > 10_000, "the cover isn't over the window ({on_screen} magenta pixels there)");

    s.spawn(&["shot", "windows"]);
    let layer = || {
        let out = s.cmd("hyprctl").args(["-j", "layers"]).output().ok()?;
        String::from_utf8_lossy(&out.stdout).contains("\"kadr-picker\"").then_some(())
    };
    wait(Duration::from_secs(3), layer).expect("no picker on screen");
    std::thread::sleep(Duration::from_millis(200));
    let (cx, cy) = ((x + w / 2).to_string(), (y + h / 2).to_string());
    s.pointer(&["move", &cx, &cy]);
    std::thread::sleep(Duration::from_millis(100));
    s.pointer(&["click", &cx, &cy]);
    let shots = s.var("OMARCHY_SCREENSHOT_DIR");
    let saved = wait(Duration::from_secs(5), || newest(&shots).filter(|p| p.extension().is_some_and(|e| e == "png"))).expect("no screenshot saved");
    std::thread::sleep(Duration::from_millis(200));
    (image::open(&saved).expect("the screenshot").to_rgba8(), (w, h), saved)
}

fn magenta(img: &RgbaImage) -> usize {
    img.pixels().filter(|p| p[0] > 240 && p[1] < 20 && p[2] > 240).count()
}

/// A clicked window is taken by itself: a picture pinned over it (a layer
/// above every window) isn't in the shot, and kadr's shadow is around it.
#[test]
#[ignore = "e2e: just e2e"]
fn a_window_is_taken_by_itself() {
    let s = Session::start();
    let (shot, (w, h), _) = shoot_covered_editor(&s);
    // The window and kadr's shadow margin, 68 pixels each side at 1x.
    let (sw, sh) = shot.dimensions();
    assert!((sw as i64 - (w + 136)).abs() <= 4 && (sh as i64 - (h + 136)).abs() <= 4, "{sw}x{sh} for a {w}x{h} window");
    let left = magenta(&shot);
    assert!(left < 100, "what covers the window is in the shot ({left} magenta pixels)");
}

/// With a look saved in the editor chosen for window shots, the window goes
/// on its background (opaque all round, the margin the look's) and the
/// editor opens the shot with that background still to change.
#[test]
#[ignore = "e2e: just e2e"]
fn a_window_goes_on_the_saved_background() {
    let s = Session::start();
    let config = s.var("XDG_CONFIG_HOME").join("kadr");
    std::fs::create_dir_all(&config).expect("config");
    let look = serde_json::json!({ "backdrop": { "gradient": 0 }, "padding": 0.06, "radius": 0.012, "shadow": 0.5, "aspect": 0 });
    std::fs::write(config.join("editor-presets.json"), serde_json::json!([{ "name": "Ocean", "look": look }]).to_string()).expect("presets");
    let settings = config.join("settings.toml");
    let before = std::fs::read_to_string(&settings).ok();
    // First in the file, ahead of its tables, as a person would add it.
    let rest: String = before.as_deref().unwrap_or("").lines().filter(|l| !l.trim_start().trim_start_matches("# ").starts_with("window_backdrop")).map(|l| format!("{l}\n")).collect();
    std::fs::write(&settings, format!("window_backdrop = \"Ocean\"\n{rest}")).expect("settings");
    // The other tests' window shots stay as they were.
    struct Restore(PathBuf, Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            match &self.1 {
                Some(t) => drop(std::fs::write(&self.0, t)),
                None => drop(std::fs::remove_file(&self.0)),
            }
        }
    }
    let _restore = Restore(settings, before);

    let (shot, (w, h), saved) = shoot_covered_editor(&s);
    let (sw, sh) = shot.dimensions();
    // A margin of 6% of the longer side each side.
    let pad = (w.max(h) as f64 * 0.06).round() as i64;
    assert!((sw as i64 - (w + 2 * pad)).abs() <= 6 && (sh as i64 - (h + 2 * pad)).abs() <= 6, "{sw}x{sh} for a {w}x{h} window with a {pad} margin");
    let corners = [(0, 0), (sw - 1, 0), (0, sh - 1), (sw - 1, sh - 1)];
    assert!(corners.iter().all(|&(x, y)| shot.get_pixel(x, y)[3] == 255), "the background isn't opaque in the corners");
    assert!(magenta(&shot) < 100, "what covers the window is in the shot");
    // The editor opens it on that background, the window as its picture.
    s.spawn(&["edit", saved.to_str().unwrap()]);
    let framed = wait(Duration::from_secs(5), || {
        let windows = s.try_dev(&["windows"])?;
        windows.as_array()?.iter().filter(|w| w["kind"] == "kadr-editor").find_map(|w| {
            let state = s.try_dev(&["state", &w["id"].to_string()])?;
            (state["view"]["frame"]["backdrop"]["gradient"] == 0).then_some(state)
        })
    });
    let state = framed.expect("the editor didn't open the shot on its background");
    let image = &state["view"]["image"];
    let near = |v: &Value, n: i64| v.as_i64().is_some_and(|v| (v - n).abs() <= 4);
    assert!(near(&image[0], w) && near(&image[1], h), "the editor's picture is {image}, not the {w}x{h} window");
}

/// A screen grab that never answers (`KADR_DEV_STALL=grab`) ends in a card
/// saying the capture didn't finish, within the grab's limit, and kadr keeps
/// answering meanwhile: nothing waits for the system on the UI thread.
#[test]
#[ignore = "e2e: just e2e"]
fn a_stuck_capture_gives_a_card_not_a_hang() {
    let s = Session::start();
    let t0 = Instant::now();
    s.cmd(KADR).args(["shot", "fullscreen"]).env("KADR_DEV", "1").env("KADR_DEV_STALL", "grab").stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("kadr");
    // Answering while the grab hangs.
    let answered = wait(Duration::from_secs(3), || s.try_dev(&["windows"]));
    assert!(answered.is_some(), "kadr didn't answer while the grab hung");
    let card = wait(Duration::from_secs(10), || {
        let card = s.try_dev(&["windows"])?.as_array()?.iter().find(|w| w["kind"] == "kadr").cloned()?;
        let state = s.try_dev(&["state", &card["id"].to_string()])?;
        state["view"]["cards"].as_array()?.iter().find_map(|c| c["notice"].as_str().map(str::to_owned))
    })
    .expect("no card about the capture");
    assert_eq!(card, "The capture didn't finish");
    let took = t0.elapsed();
    assert!(took < Duration::from_secs(9), "the card took {took:?}");
}

/// Scenario 2: draw an arrow in the editor and save; the file is what the
/// rasteriser makes of the drawing kept with it (the golden tests' layer).
#[test]
#[ignore = "e2e: just e2e"]
fn the_editor_saves_what_the_rasteriser_draws() {
    let s = Session::start();
    let path = fixture(&s, "shot.png");
    s.spawn(&["edit", path.to_str().unwrap()]);
    s.window("kadr-editor");
    wait(Duration::from_secs(5), || s.try_dev(&["state"]).filter(|v| !v["view"]["image"].is_null())).expect("the picture never loaded");
    s.dev(&["click", "tool-a"]);
    s.dev(&["drag", "900,480", "160,90"]);
    let state = s.dev(&["state"]);
    assert_eq!(state["view"]["shapes"].as_array().map(Vec::len), Some(1), "one arrow drawn: {state}");
    assert_eq!(state["view"]["shapes"][0]["kind"], "arrow");
    s.dev(&["key", "ctrl-s"]);

    // Saved, with its drawing kept alongside for the next edit.
    kadr_core::dirs::set(kadr_core::dirs::Dirs {
        config: s.var("XDG_CONFIG_HOME").join("kadr"),
        cache: s.var("XDG_CACHE_HOME").join("kadr"),
        data: s.var("XDG_DATA_HOME").join("kadr"),
        pictures: s.var("OMARCHY_SCREENSHOT_DIR"),
        videos: s.dir.join("home/videos"),
        runtime: s.dir.clone(),
    });
    let restored = wait(Duration::from_secs(5), || {
        let bytes = std::fs::read(&path).ok()?;
        kadr_core::project::load(&path, &bytes)
    })
    .expect("no drawing kept with the saved file");
    let base = image::load_from_memory(&restored.base).expect("base").to_rgba8();
    let expected = kadr_raster::render(&base, &restored.doc, &restored.frame).expect("render");
    let saved = image::open(&path).expect("saved").to_rgba8();
    let share = alike(&saved, &expected);
    assert!(share > 0.999, "the saved file isn't the rasteriser's: {:.2}% alike", share * 100.);
}

/// A JPEG opened in the editor is saved back as a JPEG, not as PNG bytes
/// under a .jpg name.
#[test]
#[ignore = "e2e: just e2e"]
fn the_editor_saves_a_jpeg_as_a_jpeg() {
    let s = Session::start();
    let path = s.dir.join("shot.jpg");
    image::open(root().join("tests/golden/cases/arrow/base.png")).expect("fixture").to_rgb8().save(&path).expect("fixture");
    s.spawn(&["edit", path.to_str().unwrap()]);
    s.window("kadr-editor");
    wait(Duration::from_secs(5), || s.try_dev(&["state"]).filter(|v| !v["view"]["image"].is_null())).expect("the picture never loaded");
    s.dev(&["click", "tool-a"]);
    s.dev(&["drag", "900,480", "160,90"]);
    let before = std::fs::read(&path).expect("fixture");
    s.dev(&["key", "ctrl-s"]);
    let saved = wait(Duration::from_secs(5), || std::fs::read(&path).ok().filter(|b| *b != before)).expect("never saved");
    assert_eq!(image::guess_format(&saved).ok(), Some(image::ImageFormat::Jpeg), "the .jpg isn't a JPEG");
}

/// The pipette takes a colour off the picture: it becomes the current one,
/// the first of the recent ones and stays in the settings; + keeps it.
#[test]
#[ignore = "e2e: just e2e"]
fn the_pipette_takes_a_colour_from_the_picture() {
    let s = Session::start();
    let path = s.dir.join("teal.png");
    RgbaImage::from_pixel(1600, 1000, image::Rgba([0x2a, 0x9d, 0x8f, 255])).save(&path).expect("fixture");
    s.spawn(&["edit", path.to_str().unwrap()]);
    s.window("kadr-editor");
    wait(Duration::from_secs(5), || s.try_dev(&["state"]).filter(|v| !v["view"]["image"].is_null())).expect("the picture never loaded");
    s.dev(&["key", "i"]);
    assert_eq!(s.dev(&["state"])["view"]["picking"], true);
    // The picture fills most of the window.
    s.dev(&["drag", "600,400", "0,0"]);
    let state = s.dev(&["state"]);
    assert_eq!(state["view"]["color"], "#2a9d8f", "{state}");
    assert_eq!(state["view"]["picking"], false);
    assert_eq!(state["view"]["recent_colors"][0], "#2a9d8f");
    s.dev(&["click", "colors"]);
    s.dev(&["click", "color-keep"]);
    assert_eq!(s.dev(&["state"])["view"]["saved_colors"][0], "#2a9d8f");
    let settings = std::fs::read_to_string(s.var("XDG_CONFIG_HOME").join("kadr/settings.toml")).expect("settings");
    assert!(settings.contains(r##"recent_colors = ["#2a9d8f"]"##) && settings.contains(r##"saved_colors = ["#2a9d8f"]"##), "{settings}");
}

/// Typing a text with the system's keys (SPEC 3.10): Unicode in, a word
/// out with Ctrl+Backspace, Shift+Left selecting and typing over it, all
/// through the input handler the input method uses too.
#[test]
#[ignore = "e2e: just e2e"]
fn the_editor_types_with_the_systems_keys() {
    let s = Session::start();
    let path = fixture(&s, "shot.png");
    s.spawn(&["edit", path.to_str().unwrap()]);
    s.window("kadr-editor");
    wait(Duration::from_secs(5), || s.try_dev(&["state"]).filter(|v| !v["view"]["image"].is_null())).expect("the picture never loaded");
    let (x, y) = s.middle("tool-t");
    s.pointer(&["click", &x, &y]);
    // On the picture, left of the middle (Hyprland knows where the window is).
    let out = s.cmd("hyprctl").args(["-j", "activewindow"]).output().expect("hyprctl");
    let w: Value = serde_json::from_slice(&out.stdout).expect("activewindow");
    let at = |i: usize, d: f64| (w["at"][i].as_f64().unwrap() + w["size"][i].as_f64().unwrap() / 2. + d).round().to_string();
    s.pointer(&["click", &at(0, -150.), &at(1, 60.)]);
    let typed = || s.try_dev(&["state"]).and_then(|v| v["view"]["typing"]["text"].as_str().map(str::to_owned));
    let expect = |want: &str| wait(Duration::from_secs(3), || typed().filter(|t| t == want)).unwrap_or_else(|| panic!("typed {:?}, not {want:?}", typed()));
    let wtype = |args: &[&str]| assert!(s.cmd("wtype").args(args).status().expect("wtype").success());
    wtype(&["Hello world"]);
    expect("Hello world");
    wtype(&["-M", "ctrl", "-k", "BackSpace", "-m", "ctrl"]);
    expect("Hello ");
    wtype(&["мир ✓"]);
    expect("Hello мир ✓");
    wtype(&["-M", "shift", "-k", "Left", "-k", "Left", "-m", "shift"]);
    wtype(&["!"]);
    expect("Hello мир!");
}

/// Scenario 2 again, with a real pointer: the tool's button clicked where
/// it's drawn, the box dragged out on the picture.
#[test]
#[ignore = "e2e: just e2e"]
fn the_editor_draws_with_a_real_pointer() {
    let s = Session::start();
    let path = fixture(&s, "shot.png");
    s.spawn(&["edit", path.to_str().unwrap()]);
    s.window("kadr-editor");
    wait(Duration::from_secs(5), || s.try_dev(&["state"]).filter(|v| !v["view"]["image"].is_null())).expect("the picture never loaded");
    let (x, y) = s.middle("tool-r");
    s.pointer(&["click", &x, &y]);
    s.pointer(&["drag", "900", "450", "1100", "600"]);
    let state = wait(Duration::from_secs(2), || s.try_dev(&["state"]).filter(|v| v["view"]["shapes"].as_array().is_some_and(|a| !a.is_empty()))).expect("nothing drawn");
    assert_eq!(state["view"]["tool"], "rect");
    let shape = &state["view"]["shapes"][0];
    assert_eq!(shape["kind"], "rect");
    // 200×150 screen pixels at the picture's scale.
    let (a, b) = (&shape["a"], &shape["b"]);
    let (w, h) = ((b[0].as_f64().unwrap() - a[0].as_f64().unwrap()).abs(), (b[1].as_f64().unwrap() - a[1].as_f64().unwrap()).abs());
    assert!((w / h - 200. / 150.).abs() < 0.05, "a {w}×{h} box");
}

/// Scenario 3, its studio half: a recording exported through the studio has
/// the layout's size and the recording's length. (Recording itself isn't
/// here: gpu-screen-recorder captures the real screen, not a nested one.)
#[test]
#[ignore = "e2e: just e2e"]
fn the_studio_exports_a_recording() {
    let s = Session::start();
    let video = s.dir.join("home/videos/rec.mp4");
    let made = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", "-t", "3", "-pix_fmt", "yuv420p"])
        .arg(&video)
        .status()
        .expect("ffmpeg");
    assert!(made.success());
    s.spawn(&["studio", video.to_str().unwrap(), "--render"]);
    let out = video.with_file_name("rec-studio.mp4");
    let probe = wait(Duration::from_secs(60), || {
        let o = Command::new("ffprobe").args(["-v", "error", "-show_entries", "stream=width,height:format=duration", "-of", "json"]).arg(&out).output().ok()?;
        let v: Value = serde_json::from_slice(&o.stdout).ok()?;
        // Only once it's whole: a duration means the file is finished.
        v["format"]["duration"].as_str()?.parse::<f64>().ok().map(|d| (d, v))
    })
    .expect("no export");
    let (duration, v) = probe;
    assert!((duration - 3.).abs() < 0.2, "lasts {duration} s");
    let (w, h) = (v["streams"][0]["width"].as_u64().unwrap_or(0), v["streams"][0]["height"].as_u64().unwrap_or(0));
    assert!(w >= 1280 && h >= 720 && w % 2 == 0 && h % 2 == 0, "{w}×{h}");

    // The export's card became the result's.
    let card = s.window("kadr");
    let done = wait(Duration::from_secs(5), || {
        let state = s.try_dev(&["state", &card["id"].to_string()])?;
        let c = state["view"]["cards"].as_array()?.first()?.clone();
        (c["working"] == false).then_some(c)
    })
    .expect("the job's card never finished");
    assert_eq!(done["path"], out.to_str().unwrap());
}

/// Scenario 4: the theme switch in the settings changes the window from dark
/// to light (by brightness: the system font's glyphs differ by machine).
#[test]
#[ignore = "e2e: just e2e"]
fn the_settings_switch_theme() {
    let s = Session::start();
    s.spawn(&["settings"]);
    let w = s.window("kadr-settings");
    let id = w["id"].to_string();
    let shot = |name: &str| {
        std::thread::sleep(Duration::from_millis(300));
        let path = s.dir.join(name);
        s.dev(&["shot", path.to_str().unwrap(), &id]);
        image::open(&path).expect("shot").to_rgba8()
    };
    s.dev(&["click", "theme-Dark", &id]);
    let dark = brightness(&shot("dark.png"));
    s.dev(&["click", "theme-Light", &id]);
    let light = brightness(&shot("light.png"));
    assert!(dark < 0.3 && light > 0.7, "dark {dark:.2}, light {light:.2}");

    // Typing searches every section: the tray's switch stays, the language
    // dropdown goes; Escape brings the section back.
    s.dev(&["type", "tray", "--window", &id]);
    assert_eq!(s.dev(&["state", &id])["view"]["query"], "tray");
    let shown = |id_: &str| s.dev(&["dump", &id]).as_array().is_some_and(|n| n.iter().any(|n| n["id"] == id_));
    assert!(shown("tray") && !shown("language"), "the search shows the tray switch alone");
    s.dev(&["key", "escape", "--window", &id]);
    assert_eq!(s.dev(&["state", &id])["view"]["query"], "");
    assert!(shown("language"));
}

/// The tray runs once: a second `kadr-tray` (or `kadr tray`) leaves at once.
#[test]
#[ignore = "e2e: just e2e"]
fn the_tray_runs_once() {
    let s = Session::start();
    let tray = root().join("target/release/kadr-tray");
    let mut first = s.cmd(tray.to_str().unwrap()).spawn().expect("kadr-tray (just e2e builds it)");
    std::thread::sleep(Duration::from_millis(500));
    assert!(first.try_wait().expect("the tray").is_none(), "the tray quit on its own");
    for second in [s.cmd(tray.to_str().unwrap()).status(), s.cmd(KADR).arg("tray").status()] {
        assert!(second.expect("a second tray").success(), "a second tray leaves quietly");
    }
    assert!(first.try_wait().expect("the tray").is_none(), "the first one stays");
    let _ = first.kill();
}

/// Scenario 5: a second `kadr edit` opens in the running kadr.
#[test]
#[ignore = "e2e: just e2e"]
fn one_kadr_serves_every_call() {
    let s = Session::start();
    let (a, b) = (fixture(&s, "a.png"), fixture(&s, "b.png"));
    s.spawn(&["edit", a.to_str().unwrap()]);
    s.window("kadr-editor");
    s.spawn(&["edit", b.to_str().unwrap()]);
    let two = wait(Duration::from_secs(5), || {
        let n = s.try_dev(&["windows"])?.as_array()?.iter().filter(|w| w["kind"] == "kadr-editor").count();
        (n == 2).then_some(())
    });
    assert!(two.is_some(), "the second editor didn't open");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(s.kadr_pids().len(), 1, "more than one kadr: {:?}", s.kadr_pids());
}
