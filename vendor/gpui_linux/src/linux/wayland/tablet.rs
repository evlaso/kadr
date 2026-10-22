//! Drawing tablets (`zwp_tablet_v2`): a pen tool drives the window the way a
//! mouse does. Hovering moves the pointer, the tip is the left button and the
//! pen's own buttons are the right and middle ones. Compositors don't turn
//! tablet input into pointer input for clients that bind the tablet protocol,
//! and some (mutter, recent KWin, Hyprland) don't do it at all, so without
//! this a pen does nothing in GPUI windows.

use std::time::Instant;

use gpui::{
    MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
    Point, point, px,
};
use wayland_backend::protocol::WEnum;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, event_created_child,
};
use wayland_protocols::wp::{
    cursor_shape::v1::client::wp_cursor_shape_device_v1,
    tablet::zv2::client::{
        zwp_tablet_manager_v2, zwp_tablet_pad_group_v2, zwp_tablet_pad_ring_v2,
        zwp_tablet_pad_strip_v2, zwp_tablet_pad_v2, zwp_tablet_seat_v2, zwp_tablet_tool_v2,
        zwp_tablet_v2,
    },
};

use super::{
    DOUBLE_CLICK_INTERVAL, SerialKind, WaylandClientStatePtr, WaylandWindowStatePtr, get_window,
    is_within_click_distance, to_shape,
};

/// The pen over one of our windows.
#[derive(Default)]
pub(crate) struct TabletState {
    window: Option<WaylandWindowStatePtr>,
    position: Option<Point<gpui::Pixels>>,
    /// The tip touches the tablet.
    down: bool,
    /// A pen button held, as a mouse button.
    button: Option<MouseButton>,
    /// Cursor shape devices, one per tool.
    cursors: Vec<(zwp_tablet_tool_v2::ZwpTabletToolV2, wp_cursor_shape_device_v1::WpCursorShapeDeviceV1)>,
}

impl TabletState {
    fn pressed(&self) -> Option<MouseButton> {
        if self.down { Some(MouseButton::Left) } else { self.button }
    }
}

// From <linux/input-event-codes.h>.
const BTN_STYLUS: u32 = 0x14b;
const BTN_STYLUS2: u32 = 0x14c;
const BTN_STYLUS3: u32 = 0x149;

delegate_noop!(WaylandClientStatePtr: ignore zwp_tablet_manager_v2::ZwpTabletManagerV2);
delegate_noop!(WaylandClientStatePtr: ignore zwp_tablet_v2::ZwpTabletV2);
delegate_noop!(WaylandClientStatePtr: ignore zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2);
delegate_noop!(WaylandClientStatePtr: ignore zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2);

impl Dispatch<zwp_tablet_seat_v2::ZwpTabletSeatV2, ()> for WaylandClientStatePtr {
    fn event(
        _: &mut Self,
        _: &zwp_tablet_seat_v2::ZwpTabletSeatV2,
        _: zwp_tablet_seat_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The tablets, tools and pads it announces get their own events.
    }

    event_created_child!(WaylandClientStatePtr, zwp_tablet_seat_v2::ZwpTabletSeatV2, [
        zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (zwp_tablet_v2::ZwpTabletV2, ()),
        zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (zwp_tablet_tool_v2::ZwpTabletToolV2, ()),
        zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (zwp_tablet_pad_v2::ZwpTabletPadV2, ()),
    ]);
}

impl Dispatch<zwp_tablet_pad_v2::ZwpTabletPadV2, ()> for WaylandClientStatePtr {
    fn event(
        _: &mut Self,
        pad: &zwp_tablet_pad_v2::ZwpTabletPadV2,
        event: zwp_tablet_pad_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_tablet_pad_v2::Event::Removed = event {
            pad.destroy();
        }
    }

    event_created_child!(WaylandClientStatePtr, zwp_tablet_pad_v2::ZwpTabletPadV2, [
        zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, ()),
    ]);
}

impl Dispatch<zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, ()> for WaylandClientStatePtr {
    fn event(
        _: &mut Self,
        _: &zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2,
        _: zwp_tablet_pad_group_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(WaylandClientStatePtr, zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, [
        zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2, ()),
        zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2, ()),
    ]);
}

impl Dispatch<zwp_tablet_tool_v2::ZwpTabletToolV2, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        tool: &zwp_tablet_tool_v2::ZwpTabletToolV2,
        event: zwp_tablet_tool_v2::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        match event {
            zwp_tablet_tool_v2::Event::ProximityIn { serial, surface, .. } => {
                let Some(window) = get_window(&mut state, &surface.id()) else { return };
                state.serial_tracker.update(SerialKind::MouseEnter, serial);
                // The pen gets the window's cursor, as the pointer does.
                if !state.tablet.cursors.iter().any(|(t, _)| t == tool)
                    && let Some(manager) = &state.globals.cursor_shape_manager
                {
                    let device = manager.get_tablet_tool_v2(tool, qh, ());
                    state.tablet.cursors.push((tool.clone(), device));
                }
                if let Some(style) = state.cursor_style
                    && let Some((_, device)) = state.tablet.cursors.iter().find(|(t, _)| t == tool)
                {
                    device.set_shape(serial, to_shape(style));
                }
                state.tablet.window = Some(window.clone());
                state.tablet.down = false;
                state.tablet.button = None;
                drop(state);
                window.set_hovered(true);
            }
            zwp_tablet_tool_v2::Event::ProximityOut => {
                let Some(window) = state.tablet.window.take() else { return };
                let position = state.tablet.position.take().unwrap_or_default();
                let modifiers = state.modifiers;
                // Lifted away while touching: the press ends here.
                let released = state.tablet.pressed();
                state.tablet.down = false;
                state.tablet.button = None;
                drop(state);
                if let Some(button) = released {
                    window.handle_input(PlatformInput::MouseUp(MouseUpEvent { button, position, modifiers, click_count: 1 }));
                }
                window.handle_input(PlatformInput::MouseExited(MouseExitEvent { position, pressed_button: None, modifiers }));
                window.set_hovered(false);
            }
            zwp_tablet_tool_v2::Event::Motion { x, y } => {
                let Some(window) = state.tablet.window.clone() else { return };
                let position = point(px(x as f32), px(y as f32));
                state.tablet.position = Some(position);
                let input = PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: state.tablet.pressed(), modifiers: state.modifiers });
                drop(state);
                window.handle_input(input);
            }
            zwp_tablet_tool_v2::Event::Down { serial } => {
                state.serial_tracker.update(SerialKind::MousePress, serial);
                state.tablet.down = true;
                drop(state);
                press(this, MouseButton::Left);
            }
            zwp_tablet_tool_v2::Event::Up => {
                state.tablet.down = false;
                drop(state);
                release(this, MouseButton::Left);
            }
            zwp_tablet_tool_v2::Event::Button { serial, button, state: WEnum::Value(button_state) } => {
                let button = match button {
                    BTN_STYLUS => MouseButton::Right,
                    BTN_STYLUS2 | BTN_STYLUS3 => MouseButton::Middle,
                    _ => return,
                };
                if button_state == zwp_tablet_tool_v2::ButtonState::Pressed {
                    state.serial_tracker.update(SerialKind::MousePress, serial);
                    state.tablet.button = Some(button);
                    drop(state);
                    press(this, button);
                } else {
                    state.tablet.button = None;
                    drop(state);
                    release(this, button);
                }
            }
            zwp_tablet_tool_v2::Event::Removed => {
                if let Some(at) = state.tablet.cursors.iter().position(|(t, _)| t == tool) {
                    state.tablet.cursors.remove(at).1.destroy();
                }
                tool.destroy();
            }
            _ => {}
        }
    }
}

/// A press at the pen, counted into double clicks like the pointer's.
fn press(this: &mut WaylandClientStatePtr, button: MouseButton) {
    let client = this.get_client();
    let mut state = client.borrow_mut();
    let (Some(window), Some(position)) = (state.tablet.window.clone(), state.tablet.position) else { return };
    let click = &mut state.click;
    if click.last_click.elapsed() < DOUBLE_CLICK_INTERVAL
        && click.last_mouse_button == Some(button)
        && is_within_click_distance(click.last_location, position)
    {
        click.current_count += 1;
    } else {
        click.current_count = 1;
    }
    click.last_click = Instant::now();
    click.last_mouse_button = Some(button);
    click.last_location = position;
    let input = PlatformInput::MouseDown(MouseDownEvent {
        button,
        position,
        modifiers: state.modifiers,
        click_count: state.click.current_count,
        first_mouse: false,
    });
    drop(state);
    window.handle_input(input);
}

fn release(this: &mut WaylandClientStatePtr, button: MouseButton) {
    let client = this.get_client();
    let state = client.borrow_mut();
    let (Some(window), Some(position)) = (state.tablet.window.clone(), state.tablet.position) else { return };
    let input = PlatformInput::MouseUp(MouseUpEvent { button, position, modifiers: state.modifiers, click_count: state.click.current_count });
    drop(state);
    window.handle_input(input);
}
