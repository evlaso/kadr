//! Window plumbing for kadr's overlays, plus the setup kadr's components
//! expect: theme, UI font and icon assets.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AnyElement, App, AssetSource, Bounds, BoxShadow, Div, Global, Pixels, Render, SharedString, Window,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, canvas, div,
    hsla, layer_shell::*, point, prelude::*, px, size,
};
use crate::ui::{ActiveTheme, KeyBinding, LabelSize};

use crate::util::Mon;

/// Base size of kadr's components (rems). Zed uses 16px; kadr's overlays are
/// read at a glance over other windows, so everything is a notch larger.
pub const REM: f32 = 19.;
/// Scale for kadr's own fixed pixel sizes, to match `REM`.
pub const SCALE: f32 = REM / 16.;

/// Zed's UI font, bundled so kadr looks the same without Zed installed.
pub(crate) const UI_FONT: &str = "IBM Plex Sans";
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
    "icons/copy.svg" => "../data/zed/icons/copy.svg",
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
    "icons/chevron_left.svg" => "../data/zed/icons/chevron_left.svg",
    "icons/chevron_right.svg" => "../data/zed/icons/chevron_right.svg",
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
    // The user's Omarchy theme colors.
    cx.set_global(crate::palette::theme());
    cx.set_global(Busy::default());
}

/// Corner radius of thumbnails and pinned pictures.
pub const RADIUS: f32 = 16.;

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
    let (t, r, b, l) = spec.margin;
    let options = WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(spec.size.0), px(spec.size.1)),
        })),
        app_id: Some("kadr".into()),
        linux_window_background: gpui::LinuxWindowBackground::Transparent,
        display_id: spec.monitor.as_ref().and_then(|m| m.display(cx)),
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: spec.namespace.into(),
            layer: Layer::Overlay,
            anchor: spec.anchor,
            margin: Some((px(t), px(r), px(b), px(l))),
            exclusive_zone: spec.ignore_zones.then(|| px(-1.)),
            keyboard_interactivity: spec.keyboard,
            ..Default::default()
        }),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        window.set_rem_size(px(REM));
        build(window, cx)
    }) {
        Ok(handle) => Some(handle),
        Err(e) => {
            crate::util::notify("kadr: не удалось открыть окно", &e.to_string());
            None
        }
    }
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
