//! The system's own pop-up menus (SPEC §3.10), through muda: an NSMenu on
//! macOS, a Win32 pop-up menu on Windows. Linux has none outside a GTK
//! window (a Wayland app's menus are its own surfaces), so there kadr draws
//! the same list itself; `native()` says which.
//!
//! `popup` runs the menu to its end and blocks meanwhile, as the system's
//! menus do. Call it outside the UI framework's update (the app spawns a
//! task for it): the menu's own event loop may deliver the framework's
//! events while it's open.

use raw_window_handle::RawWindowHandle;

/// A choice in a menu.
pub struct Item {
    pub label: String,
    /// Shown with a check mark: the current choice.
    pub checked: bool,
    pub enabled: bool,
    /// A line above it, between groups.
    pub separated: bool,
}

impl Item {
    pub fn new(label: impl Into<String>, checked: bool) -> Item {
        Item { label: label.into(), checked, enabled: true, separated: false }
    }
}

/// Whether this system's menus are its own (else the app draws them).
pub const fn native() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// Shows `items` as the system's menu at `at` (in the window's logical
/// pixels, from its top left) over `window`, and gives back the chosen
/// one's index: None when it was closed without a choice, or when this
/// system has no menus of its own.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn popup(window: RawWindowHandle, at: (f32, f32), items: &[Item]) -> Option<usize> {
    use muda::{CheckMenuItem, ContextMenu, Menu, MenuEvent, MenuItem, PredefinedMenuItem, dpi::LogicalPosition};
    use std::sync::atomic::{AtomicU64, Ordering};

    // Ids unique to this menu, so an old menu's late event can't pick here.
    static SHOWN: AtomicU64 = AtomicU64::new(0);
    let menu_id = SHOWN.fetch_add(1, Ordering::Relaxed);
    let id = |i: usize| format!("kadr-menu-{menu_id}-{i}");

    let menu = Menu::new();
    for (i, item) in items.iter().enumerate() {
        if item.separated {
            menu.append(&PredefinedMenuItem::separator()).ok()?;
        }
        // The current choice carries a check; the rest are plain, as in the
        // system's own pop-up buttons.
        let added = if item.checked {
            menu.append(&CheckMenuItem::with_id(id(i), &item.label, item.enabled, true, None))
        } else {
            menu.append(&MenuItem::with_id(id(i), &item.label, item.enabled, None))
        };
        added.ok()?;
    }
    while MenuEvent::receiver().try_recv().is_ok() {}
    let at = Some(LogicalPosition::new(at.0 as f64, at.1 as f64).into());
    let picked = match window {
        #[cfg(target_os = "macos")]
        // SAFETY: the handle is the live window's NSView, taken from it on
        // this (the main) thread just before.
        RawWindowHandle::AppKit(h) => unsafe { menu.show_context_menu_for_nsview(h.ns_view.as_ptr(), at) },
        #[cfg(target_os = "windows")]
        // SAFETY: as above, the live window's HWND.
        RawWindowHandle::Win32(h) => unsafe { menu.show_context_menu_for_hwnd(h.hwnd.get(), at) },
        _ => false,
    };
    if !picked {
        return None;
    }
    std::iter::from_fn(|| MenuEvent::receiver().try_recv().ok()).find_map(|e| (0..items.len()).find(|&i| e.id.0 == id(i)))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn popup(_: RawWindowHandle, _: (f32, f32), _: &[Item]) -> Option<usize> {
    None
}
