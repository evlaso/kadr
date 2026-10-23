//! The OS layer is `kadr_platform`; here are only the window options GPUI
//! needs on each system.

pub use kadr_platform::*;

#[cfg(target_os = "linux")]
mod window {
    use gpui::{App, WindowOptions};

    /// kadr draws its own window chrome.
    pub fn titlebar() -> Option<gpui::TitlebarOptions> {
        None
    }

    pub fn transparent(options: &mut WindowOptions) {
        options.linux_window_background = gpui::LinuxWindowBackground::Transparent;
    }

    /// Overlays are Wayland layer-shell surfaces above every window.
    pub fn overlay_kind(spec: &crate::kit::LayerSpec, _cx: &App) -> (gpui::WindowKind, gpui::Bounds<gpui::Pixels>) {
        use crate::kit::{Anchor as A, KeyboardInteractivity as K};
        use gpui::{layer_shell::*, point, px, size};
        let (t, r, b, l) = spec.margin;
        let mut anchor = Anchor::empty();
        for (ours, theirs) in [(A::TOP, Anchor::TOP), (A::BOTTOM, Anchor::BOTTOM), (A::LEFT, Anchor::LEFT), (A::RIGHT, Anchor::RIGHT)] {
            if spec.anchor.contains(ours) {
                anchor |= theirs;
            }
        }
        let keyboard_interactivity = match spec.keyboard {
            K::None => KeyboardInteractivity::None,
            K::Exclusive => KeyboardInteractivity::Exclusive,
            K::OnDemand => KeyboardInteractivity::OnDemand,
        };
        let kind = gpui::WindowKind::LayerShell(LayerShellOptions {
            namespace: spec.namespace.into(),
            layer: Layer::Overlay,
            anchor,
            margin: Some((px(t), px(r), px(b), px(l))),
            exclusive_zone: spec.ignore_zones.then(|| px(-1.)),
            keyboard_interactivity,
            ..Default::default()
        });
        (kind, gpui::Bounds { origin: point(px(0.), px(0.)), size: size(px(spec.size.0), px(spec.size.1)) })
    }
}

#[cfg(target_os = "macos")]
mod window {
    use gpui::{App, WindowOptions};

    /// A transparent title bar over kadr's own top bar, or none at all.
    pub fn titlebar() -> Option<gpui::TitlebarOptions> {
        Some(gpui::TitlebarOptions { title: None, appears_transparent: true, traffic_light_position: Some(gpui::point(gpui::px(14.), gpui::px(14.))) })
    }

    pub fn transparent(options: &mut WindowOptions) {
        options.macos_window_background = gpui::MacosWindowBackground::Transparent;
    }

    /// Overlays are borderless windows above the others, placed like the
    /// Wayland layer would be.
    pub fn overlay_kind(spec: &crate::kit::LayerSpec, cx: &App) -> (gpui::WindowKind, gpui::Bounds<gpui::Pixels>) {
        (gpui::WindowKind::PopUp, crate::kit::overlay_bounds(spec, cx))
    }
}

#[cfg(target_os = "windows")]
mod window {
    use gpui::{App, WindowOptions};

    /// A transparent title bar over kadr's own top bar, or none at all.
    pub fn titlebar() -> Option<gpui::TitlebarOptions> {
        None
    }

    pub fn transparent(options: &mut WindowOptions) {
        options.windows_window_background = gpui::WindowsWindowBackground::Transparent;
    }

    pub fn overlay_kind(spec: &crate::kit::LayerSpec, cx: &App) -> (gpui::WindowKind, gpui::Bounds<gpui::Pixels>) {
        (gpui::WindowKind::PopUp, crate::kit::overlay_bounds(spec, cx))
    }
}

pub use window::*;
