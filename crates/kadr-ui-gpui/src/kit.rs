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

/// Base size of kadr's components (rems). Over the usual 16px: kadr's
/// overlays are read at a glance over other windows, so a notch larger.
pub const REM: f32 = 19.;
/// Scale for kadr's own fixed pixel sizes, to match `REM`.
pub const SCALE: f32 = REM / 16.;

/// The system's own UI font: San Francisco, Segoe UI, or the desktop's sans
/// on Linux.
pub(crate) const UI_FONT: &str = ".SystemUIFont";
/// Text drawn onto screenshots: bundled, so it looks the same everywhere
/// and matches the editor's own rasteriser.
pub(crate) const ANNOTATION_FONT: &str = "IBM Plex Sans";
/// The bundled monospace font, for timers and numbers.
pub const MONO_FONT: &str = "Lilex";

macro_rules! assets {
    ($($path:literal => $file:literal),* $(,)?) => {
        const ASSETS: &[(&str, &[u8])] = &[$(($path, include_bytes!($file))),*];
    };
}
assets![
    // General icons (Lucide), under the paths `ui::IconName` resolves to.
    "icons/close.svg" => "../../../data/icons/ui/close.svg",
    "icons/image.svg" => "../../../data/icons/ui/image.svg",
    "icons/pencil.svg" => "../../../data/icons/ui/pencil.svg",
    "icons/folder.svg" => "../../../data/icons/ui/folder.svg",
    "icons/trash.svg" => "../../../data/icons/ui/trash.svg",
    "icons/pin.svg" => "../../../data/icons/ui/pin.svg",
    "icons/mic.svg" => "../../../data/icons/ui/mic.svg",
    "icons/audio_on.svg" => "../../../data/icons/ui/audio_on.svg",
    "icons/screen.svg" => "../../../data/icons/ui/screen.svg",
    "icons/crosshair.svg" => "../../../data/icons/ui/crosshair.svg",
    "icons/stop.svg" => "../../../data/icons/ui/stop.svg",
    "icons/debug_pause.svg" => "../../../data/icons/ui/debug_pause.svg",
    "icons/play_filled.svg" => "../../../data/icons/ui/play_filled.svg",
    "icons/check.svg" => "../../../data/icons/ui/check.svg",
    "icons/chevron_down.svg" => "../../../data/icons/ui/chevron_down.svg",
    "icons/dash.svg" => "../../../data/icons/ui/dash.svg",
    "icons/plus.svg" => "../../../data/icons/ui/plus.svg",
    "icons/sparkle.svg" => "../../../data/icons/ui/sparkle.svg",
    "icons/arrow_up_right.svg" => "../../../data/icons/ui/arrow_up_right.svg",
    "icons/slash.svg" => "../../../data/icons/ui/slash.svg",
    "icons/case_sensitive.svg" => "../../../data/icons/ui/case_sensitive.svg",
    "icons/undo.svg" => "../../../data/icons/ui/undo.svg",
    "icons/download.svg" => "../../../data/icons/ui/download.svg",
    "icons/scissors.svg" => "../../../data/icons/ui/scissors.svg",
    "icons/magnifying_glass.svg" => "../../../data/icons/ui/magnifying_glass.svg",
    "icons/keyboard.svg" => "../../../data/icons/ui/keyboard.svg",
    "icons/circle.svg" => "../../../data/icons/ui/circle.svg",
    "icons/settings.svg" => "../../../data/icons/ui/settings.svg",
    "icons/copy.svg" => "../../../data/icons/ui/copy.svg",
    "icons/image_plus.svg" => "../../../data/icons/ui/image_plus.svg",
    "icons/window_max.svg" => "../../../data/icons/ui/window_max.svg",
    "icons/zoom_in.svg" => "../../../data/icons/ui/zoom_in.svg",
    "icons/pointer.svg" => "../../../data/icons/ui/pointer.svg",
    "icons/video.svg" => "../../../data/icons/ui/video.svg",
    "icons/expand.svg" => "../../../data/icons/ui/expand.svg",
    // Editor tools, Lucide too (counter and pixelate are drawn on its grid
    // and stroke, Lucide has neither).
    "icons/kadr/rect.svg" => "../../../data/icons/editor/rect.svg",
    "icons/kadr/ellipse.svg" => "../../../data/icons/editor/ellipse.svg",
    "icons/kadr/marker.svg" => "../../../data/icons/editor/marker.svg",
    "icons/kadr/counter.svg" => "../../../data/icons/editor/counter.svg",
    "icons/kadr/cursor.svg" => "../../../data/icons/editor/cursor.svg",
    "icons/kadr/pixelate.svg" => "../../../data/icons/editor/pixelate.svg",
    "icons/kadr/blur.svg" => "../../../data/icons/editor/blur.svg",
    "icons/kadr/scan_text.svg" => "../../../data/icons/editor/scan_text.svg",
    "icons/kadr/project.svg" => "../../../data/icons/editor/project.svg",
    "icons/kadr/redo.svg" => "../../../data/icons/editor/redo.svg",
    "icons/kadr/pointer.svg" => "../../../data/icons/editor/pointer.svg",
    "icons/kadr/crop.svg" => "../../../data/icons/editor/crop.svg",
    "icons/kadr/rotate.svg" => "../../../data/icons/editor/rotate.svg",
    "icons/kadr/flip.svg" => "../../../data/icons/editor/flip.svg",
    "icons/kadr/swap.svg" => "../../../data/icons/editor/swap.svg",
    "icons/kadr/frame.svg" => "../../../data/icons/editor/frame.svg",
    "icons/kadr/spotlight.svg" => "../../../data/icons/editor/spotlight.svg",
    "icons/kadr/fill.svg" => "../../../data/icons/editor/fill.svg",
    "icons/kadr/timer.svg" => "../../../data/icons/editor/timer.svg",
    "icons/kadr/grip.svg" => "../../../data/icons/editor/grip.svg",
    "icons/kadr/area.svg" => "../../../data/icons/editor/area.svg",
    "icons/kadr/previous_area.svg" => "../../../data/icons/editor/previous_area.svg",
    "icons/kadr/restart.svg" => "../../../data/icons/editor/restart.svg",
    "icons/kadr/window.svg" => "../../../data/icons/editor/window.svg",
    "icons/kadr/scroll.svg" => "../../../data/icons/editor/scroll.svg",
    "icons/kadr/skip_back.svg" => "../../../data/icons/editor/skip_back.svg",
    "icons/kadr/more.svg" => "../../../data/icons/editor/more.svg",
    "icons/kadr/skip_forward.svg" => "../../../data/icons/editor/skip_forward.svg",
    "icons/webcam.svg" => "../../../data/icons/ui/webcam.svg",
    // Settings sections, the pin's lock and links (Lucide).
    "icons/kadr/record.svg" => "../../../data/icons/settings/record.svg",
    "icons/kadr/studio.svg" => "../../../data/icons/settings/studio.svg",
    "icons/kadr/storage.svg" => "../../../data/icons/settings/storage.svg",
    "icons/kadr/info.svg" => "../../../data/icons/settings/info.svg",
    "icons/kadr/lock.svg" => "../../../data/icons/settings/lock.svg",
    "icons/kadr/unlock.svg" => "../../../data/icons/settings/unlock.svg",
    "icons/kadr/link.svg" => "../../../data/icons/settings/link.svg",
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
        Cow::Borrowed(include_bytes!("../../../data/fonts/IBMPlexSans-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../../../data/fonts/IBMPlexSans-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../../../data/fonts/Lilex-Regular.ttf").as_slice()),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("kadr: fonts: {e}");
    }
    let theme = crate::palette::theme(cx);
    cx.set_global(theme);
    cx.set_global(Busy::default());
}

/// Corner radius of thumbnails and pinned pictures.
pub const RADIUS: f32 = 20.;
/// Corners of kadr's windows: the system's (kadr's own Hyprland rule on
/// Linux, 13, where a large button of radius 9 inset by 4 sits concentric).
pub const WINDOW_RADIUS: f32 = kadr_platform::style::current().window_radius;

/// Soft two-layer drop shadow for floating pictures: a wide ambient blur
/// plus a tighter contact shadow.
pub fn shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow { color: hsla(0., 0., 0., 0.38).into(), offset: point(px(0.), px(8.)), blur_radius: px(22.), spread_radius: px(0.), inset: false },
        BoxShadow { color: hsla(0., 0., 0., 0.22).into(), offset: point(px(0.), px(2.)), blur_radius: px(6.), spread_radius: px(0.), inset: false },
    ]
}

/// A key hint, written the system's way, e.g. `keys("enter")`.
pub fn keys(source: &str) -> KeyBinding {
    KeyBinding::new(source)
}

/// Root of every overlay: the UI font and text color.
pub fn root(cx: &App) -> Div {
    div().size_full().font_family(UI_FONT).text_size(LabelSize::Default.rems()).text_color(cx.theme().colors().text)
}

/// One end of a window's top bar: room for the system's controls at the
/// left (macOS), or kadr's own at the right (Windows; a close button on
/// floating Linux desktops). Nothing on tiling window managers.
pub fn window_controls(right: bool, cx: &App) -> Option<AnyElement> {
    use crate::platform::WindowControls as W;
    use crate::ui::{ButtonSize, Icon, IconButton, IconName, Tooltip};
    use gpui::WindowControlArea;
    match (crate::platform::window_controls(), right) {
        (W::Left(w), false) => Some(div().flex_none().w(px(w)).into_any_element()),
        (W::Own, true) => Some(
            IconButton::new("window-close", IconName::Close)
                .size(ButtonSize::Large)
                .tip(Tooltip::text(t("common.close")))
                .on_click(|_, window, _| window.remove_window())
                .into_any_element(),
        ),
        (W::Right(w), true) => {
            let colors = cx.theme().colors();
            let (hover, danger) = (colors.ghost_element_hover, cx.theme().status().error);
            // Over the system's hit areas, so snapping and the rest still work.
            let button = |id: &'static str, icon: Icon, area: WindowControlArea, close: bool| {
                div()
                    .id(id)
                    .w(px(w / 3.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .window_control_area(area)
                    .when(close, |d| d.hover(move |s| s.bg(danger)))
                    .when(!close, |d| d.hover(move |s| s.bg(hover)))
                    .child(icon)
                    .on_click(move |_, window, _| match area {
                        WindowControlArea::Min => window.minimize_window(),
                        WindowControlArea::Max => window.zoom_window(),
                        _ => window.remove_window(),
                    })
            };
            Some(
                div()
                    .flex()
                    .flex_none()
                    .self_stretch()
                    .child(button("window-min", Icon::new(IconName::Dash).size(crate::ui::IconSize::Small), WindowControlArea::Min, false))
                    .child(button("window-max", Icon::from_path("icons/window_max.svg").size(crate::ui::IconSize::XSmall), WindowControlArea::Max, false))
                    .child(button("window-close", Icon::new(IconName::Close).size(crate::ui::IconSize::Small), WindowControlArea::Close, true))
                    .into_any_element(),
            )
        }
        _ => None,
    }
}

/// Empty stretches of a top bar move the window, where it has no title bar
/// of its own; tiling managers place windows themselves.
pub fn drags_window(el: Div) -> Div {
    if crate::platform::window_controls() == crate::platform::WindowControls::None {
        return el;
    }
    el.on_mouse_down(gpui::MouseButton::Left, |e, window, _| {
        if e.click_count == 2 {
            window.zoom_window();
        } else {
            window.start_window_move();
        }
    })
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
        display_id: spec.monitor.as_ref().and_then(|m| crate::util::display_of(m, cx)),
        kind,
        // Overlays rarely have the keyboard, and GPUI holds windows without
        // it to ~30 fps, which made the cards slide in choppily on a 240 Hz
        // screen.
        inactive_frame_interval: None,
        ..Default::default()
    };
    crate::platform::transparent(&mut options);
    match cx.open_window(options, |window, cx| {
        window.set_rem_size(px(REM));
        if let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) {
            crate::platform::bare_window(handle.as_raw());
        }
        // Windows made it an ordinary window first, with a title bar's
        // minimum size: a thin overlay came out tens of pixels wide.
        if cfg!(windows) {
            window.resize(bounds.size);
        }
        build(window, cx)
    }) {
        Ok(handle) => {
            crate::dev::name_window(handle.into(), spec.namespace);
            Some(handle)
        }
        Err(e) => {
            eprintln!("kadr: couldn't open a window: {e:#}");
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
    let display = mon.and_then(|m| crate::util::display_of(m, cx));
    // Never bigger than the screen it opens on, less a margin, nor its
    // minimum on a screen smaller than that: nothing hangs off a small one.
    let (size, min) = match mon {
        Some(m) => {
            let (mw, mh) = (m.w as f32 - 80., m.h as f32 - 80.);
            ((size.0.min(mw), size.1.min(mh)), (min.0.min(mw), min.1.min(mh)))
        }
        None => (size, min),
    };
    let options = WindowOptions {
        titlebar: crate::platform::titlebar(),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(display, gpui::size(px(size.0), px(size.1)), cx))),
        app_id: Some(app_id.into()),
        display_id: display,
        window_min_size: Some(gpui::size(px(min.0), px(min.1))),
        ..Default::default()
    };
    crate::dev::mark("opening");
    match cx.open_window(options, |window, cx| {
        window.set_rem_size(px(REM));
        // Named in the system's window lists (and so kept in the overlay's
        // windows to snap to, while kadr's untitled overlays and cards aren't).
        window.set_window_title("kadr");
        crate::dev::mark("window");
        window.on_next_frame(|_, _| crate::dev::mark("first_frame"));
        let view = build(window, cx);
        crate::dev::mark("built");
        view
    }) {
        Ok(handle) => {
            crate::dev::name_window(handle.into(), app_id);
            Some(handle)
        }
        Err(e) => {
            eprintln!("kadr: couldn't open a window: {e:#}");
            crate::util::notify(t("app.kadr_couldnt_open_window"), &e.to_string());
            None
        }
    }
}

/// Where a layer with this anchor, margin and size would sit, as window
/// bounds on its display, for systems without layer-shell.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn overlay_bounds(spec: &LayerSpec, cx: &App) -> Bounds<Pixels> {
    let display = spec.monitor.as_ref().and_then(|m| crate::util::display_of(m, cx)).and_then(|id| cx.find_display(id)).or_else(|| cx.primary_display());
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
