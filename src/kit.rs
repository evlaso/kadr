//! Window plumbing for kadr's overlays, plus the setup kadr's components
//! expect: theme, UI font and icon assets.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AnyElement, App, AssetSource, Bounds, BoxShadow, Div, Global, Pixels, Render, SharedString, Window,
    WindowBounds, WindowHandle, WindowOptions, canvas, div,
    hsla, point, prelude::*, px, size,
};
use crate::ui::{ActiveTheme, KeyBinding, LabelSize};

use crate::util::Mon;
use crate::i18n::t;

/// Base size of kadr's components (rems). Zed uses 16px; kadr's overlays are
/// read at a glance over other windows, so everything is a notch larger.
pub const REM: f32 = 19.;
/// Scale for kadr's own fixed pixel sizes, to match `REM`.
pub const SCALE: f32 = REM / 16.;

/// Zed's UI font, bundled so kadr looks the same without Zed installed.
/// The system's own UI font: San Francisco, Segoe UI, or the desktop's sans
/// on Linux.
pub(crate) const UI_FONT: &str = ".SystemUIFont";
/// Text drawn onto screenshots: bundled, so it looks the same everywhere
/// and matches the editor's own rasteriser.
pub(crate) const ANNOTATION_FONT: &str = "IBM Plex Sans";
/// Zed's monospace font, for timers and numbers.
pub const MONO_FONT: &str = "Lilex";

macro_rules! assets {
    ($($path:literal => $file:literal),* $(,)?) => {
        const ASSETS: &[(&str, &[u8])] = &[$(($path, include_bytes!($file))),*];
    };
}
assets![
    // Zed's icon set, under the paths `ui::IconName` resolves to.
    "icons/close.svg" => "../data/zed/icons/close.svg",
    "icons/image.svg" => "../data/zed/icons/image.svg",
    "icons/pencil.svg" => "../data/zed/icons/pencil.svg",
    "icons/folder.svg" => "../data/zed/icons/folder.svg",
    "icons/trash.svg" => "../data/zed/icons/trash.svg",
    "icons/pin.svg" => "../data/zed/icons/pin.svg",
    "icons/mic.svg" => "../data/zed/icons/mic.svg",
    "icons/audio_on.svg" => "../data/zed/icons/audio_on.svg",
    "icons/screen.svg" => "../data/zed/icons/screen.svg",
    "icons/crosshair.svg" => "../data/zed/icons/crosshair.svg",
    "icons/stop.svg" => "../data/zed/icons/stop.svg",
    "icons/debug_pause.svg" => "../data/zed/icons/debug_pause.svg",
    "icons/play_filled.svg" => "../data/zed/icons/play_filled.svg",
    "icons/check.svg" => "../data/zed/icons/check.svg",
    "icons/chevron_down.svg" => "../data/zed/icons/chevron_down.svg",
    "icons/dash.svg" => "../data/zed/icons/dash.svg",
    "icons/plus.svg" => "../data/zed/icons/plus.svg",
    "icons/sparkle.svg" => "../data/zed/icons/sparkle.svg",
    "icons/arrow_up_right.svg" => "../data/zed/icons/arrow_up_right.svg",
    "icons/slash.svg" => "../data/zed/icons/slash.svg",
    "icons/case_sensitive.svg" => "../data/zed/icons/case_sensitive.svg",
    "icons/undo.svg" => "../data/zed/icons/undo.svg",
    "icons/download.svg" => "../data/zed/icons/download.svg",
    "icons/scissors.svg" => "../data/zed/icons/scissors.svg",
    "icons/magnifying_glass.svg" => "../data/zed/icons/magnifying_glass.svg",
    "icons/keyboard.svg" => "../data/zed/icons/keyboard.svg",
    "icons/circle.svg" => "../data/zed/icons/circle.svg",
    "icons/settings.svg" => "../data/zed/icons/settings.svg",
    // Editor tools Zed's set lacks, drawn in the same style.
    "icons/kadr/rect.svg" => "../data/icons/editor/rect.svg",
    "icons/kadr/ellipse.svg" => "../data/icons/editor/ellipse.svg",
    "icons/kadr/marker.svg" => "../data/icons/editor/marker.svg",
    "icons/kadr/counter.svg" => "../data/icons/editor/counter.svg",
    "icons/kadr/pixelate.svg" => "../data/icons/editor/pixelate.svg",
    "icons/kadr/redo.svg" => "../data/icons/editor/redo.svg",
    "icons/kadr/pointer.svg" => "../data/icons/editor/pointer.svg",
    "icons/kadr/crop.svg" => "../data/icons/editor/crop.svg",
    "icons/kadr/frame.svg" => "../data/icons/editor/frame.svg",
    "icons/kadr/spotlight.svg" => "../data/icons/editor/spotlight.svg",
    "icons/kadr/fill.svg" => "../data/icons/editor/fill.svg",
    "icons/kadr/timer.svg" => "../data/icons/editor/timer.svg",
    "icons/kadr/grip.svg" => "../data/icons/editor/grip.svg",
    "icons/kadr/area.svg" => "../data/icons/editor/area.svg",
    "icons/kadr/window.svg" => "../data/icons/editor/window.svg",
    "icons/kadr/scroll.svg" => "../data/icons/editor/scroll.svg",
    "icons/kadr/skip_back.svg" => "../data/icons/editor/skip_back.svg",
    "icons/kadr/more.svg" => "../data/icons/editor/more.svg",
    "icons/kadr/skip_forward.svg" => "../data/icons/editor/skip_forward.svg",
    // Zed has no camera icon.
    "icons/kadr/camera.svg" => "../data/icons/kadr-camera-symbolic.svg",
];

/// Icons are compiled into the binary.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ASSETS.iter().find(|(p, _)| *p == path).map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ASSETS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| (*p).into()).collect())
    }
}

pub fn init(cx: &mut App) {
    let fonts = vec![
        Cow::Borrowed(include_bytes!("../data/zed/fonts/IBMPlexSans-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../data/zed/fonts/IBMPlexSans-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../data/zed/fonts/Lilex-Regular.ttf").as_slice()),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("kadr: fonts: {e}");
    }
    let theme = crate::palette::theme(cx);
    cx.set_global(theme);
    cx.set_global(Busy::default());
}

/// Corner radius of thumbnails and pinned pictures.
pub const RADIUS: f32 = 16.;
/// Corners of kadr's windows (also set in the Hyprland rule): a large button
/// (radius 9) inset by 4 in a corner sits concentric with it.
pub const WINDOW_RADIUS: f32 = 13.;

/// Soft two-layer drop shadow for floating pictures: a wide ambient blur
/// plus a tighter contact shadow.
pub fn shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow { color: hsla(0., 0., 0., 0.38).into(), offset: point(px(0.), px(8.)), blur_radius: px(22.), spread_radius: px(0.), inset: false },
        BoxShadow { color: hsla(0., 0., 0., 0.22).into(), offset: point(px(0.), px(2.)), blur_radius: px(6.), spread_radius: px(0.), inset: false },
    ]
}

/// A key hint drawn the way Zed shows shortcuts, e.g. `keys("enter")`.
pub fn keys(source: &str) -> KeyBinding {
    KeyBinding::new(source)
}

/// Root of every overlay: Zed's UI font and text color.
pub fn root(cx: &App) -> Div {
    div().size_full().font_family(UI_FONT).text_size(LabelSize::Default.rems()).text_color(cx.theme().colors().text)
}

/// Element bounds gathered during prepaint become the window's input region,
/// so transparent parts of an overlay let clicks through.
#[derive(Clone, Default)]
pub struct InputRegion(Rc<RefCell<Vec<Bounds<Pixels>>>>);

impl InputRegion {
    /// Call at the top of `render`.
    pub fn reset(&self) {
        self.0.borrow_mut().clear();
    }

    /// Place inside an element (as a child) to make its area clickable.
    pub fn mark(&self) -> AnyElement {
        let rects = self.0.clone();
        canvas(
            move |bounds, window, _| {
                rects.borrow_mut().push(bounds);
                window.set_input_region(Some(&rects.borrow()));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }
}

/// Which screen edges an overlay sticks to (a layer-shell anchor on
/// Wayland, a placement rule elsewhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor(u8);

impl Anchor {
    pub const TOP: Anchor = Anchor(1);
    pub const BOTTOM: Anchor = Anchor(2);
    pub const LEFT: Anchor = Anchor(4);
    pub const RIGHT: Anchor = Anchor(8);

    pub fn contains(self, other: Anchor) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Anchor {
    type Output = Anchor;
    fn bitor(self, other: Anchor) -> Anchor {
        Anchor(self.0 | other.0)
    }
}

/// Whether an overlay takes the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardInteractivity {
    None,
    Exclusive,
    OnDemand,
}

pub struct LayerSpec {
    pub namespace: &'static str,
    pub anchor: Anchor,
    /// Top, right, bottom, left.
    pub margin: (f32, f32, f32, f32),
    pub size: (f32, f32),
    pub keyboard: KeyboardInteractivity,
    /// Ignore other surfaces' exclusive zones (the bar) when positioning.
    pub ignore_zones: bool,
    pub monitor: Option<Mon>,
}

pub fn open_layer<V: Render + 'static>(
    cx: &mut App,
    spec: LayerSpec,
    build: impl FnOnce(&mut Window, &mut App) -> gpui::Entity<V>,
) -> Option<WindowHandle<V>> {
    // A Wayland layer on Linux, a borderless window above the others elsewhere.
    let (kind, bounds) = crate::platform::overlay_kind(&spec, cx);
    let mut options = WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        app_id: Some("kadr".into()),
        display_id: spec.monitor.as_ref().and_then(|m| m.display(cx)),
        kind,
        ..Default::default()
    };
    crate::platform::transparent(&mut options);
    match cx.open_window(options, |window, cx| {
        window.set_rem_size(px(REM));
        build(window, cx)
    }) {
        Ok(handle) => Some(handle),
        Err(e) => {
            crate::util::notify(t("app.kadr_couldnt_open_window"), &e.to_string());
            None
        }
    }
}

/// An ordinary app window (it floats by the Hyprland rule in
/// `data/hypr/kadr.lua`), centred on `mon`. `app_id` names it for rules.
pub fn open_window<V: Render + 'static>(
    cx: &mut App,
    app_id: &str,
    mon: Option<&crate::util::Mon>,
    size: (f32, f32),
    min: (f32, f32),
    build: impl FnOnce(&mut Window, &mut App) -> gpui::Entity<V>,
) -> Option<WindowHandle<V>> {
    let display = mon.and_then(|m| m.display(cx));
    let options = WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(display, gpui::size(px(size.0), px(size.1)), cx))),
        app_id: Some(app_id.into()),
        display_id: display,
        window_min_size: Some(gpui::size(px(min.0), px(min.1))),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        window.set_rem_size(px(REM));
        build(window, cx)
    }) {
        Ok(handle) => Some(handle),
        Err(e) => {
            crate::util::notify(t("app.kadr_couldnt_open_window"), &e.to_string());
            None
        }
    }
}

/// Where a layer with this anchor, margin and size would sit, as window
/// bounds on its display, for systems without layer-shell.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn overlay_bounds(spec: &LayerSpec, cx: &App) -> Bounds<Pixels> {
    let display = spec.monitor.as_ref().and_then(|m| m.display(cx)).and_then(|id| cx.find_display(id)).or_else(|| cx.primary_display());
    let screen = display.map(|d| d.bounds()).unwrap_or(Bounds { origin: point(px(0.), px(0.)), size: size(px(1280.), px(800.)) });
    let (t, r, b, l) = spec.margin;
    let (sw, sh) = (f32::from(screen.size.width), f32::from(screen.size.height));
    let fill_w = spec.anchor.contains(Anchor::LEFT) && spec.anchor.contains(Anchor::RIGHT);
    let fill_h = spec.anchor.contains(Anchor::TOP) && spec.anchor.contains(Anchor::BOTTOM);
    let w = if fill_w { sw - l - r } else { spec.size.0 };
    let h = if fill_h { sh - t - b } else { spec.size.1 };
    let x = if spec.anchor.contains(Anchor::LEFT) { l } else if spec.anchor.contains(Anchor::RIGHT) { sw - w - r } else { (sw - w) / 2. };
    let y = if spec.anchor.contains(Anchor::TOP) { t } else if spec.anchor.contains(Anchor::BOTTOM) { sh - h - b } else { (sh - h) / 2. };
    Bounds { origin: point(screen.origin.x + px(x), screen.origin.y + px(y)), size: size(px(w), px(h)) }
}

/// While any `Hold` is alive kadr keeps running even with no windows open
/// (a capture, a recording, an ffmpeg job…). Otherwise it exits when idle.
#[derive(Default)]
pub struct Busy(Rc<Cell<usize>>);

impl Global for Busy {}

pub struct Hold(Rc<Cell<usize>>);

impl Drop for Hold {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

pub fn hold(cx: &App) -> Hold {
    let count = cx.global::<Busy>().0.clone();
    count.set(count.get() + 1);
    Hold(count)
}

pub fn is_busy(cx: &App) -> bool {
    cx.global::<Busy>().0.get() > 0
}
