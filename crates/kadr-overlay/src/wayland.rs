//! The overlay's own window on Wayland: a layer-shell surface over one
//! whole monitor, drawn on the CPU into shared memory (no GPU, nothing to
//! set up but a buffer), on its own connection to the compositor, so it
//! runs on any thread beside the app's UI. Fractional scales go through
//! wp_viewporter: the buffer has the monitor's physical pixels.

use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers};
use smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager;
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1};
use wayland_protocols::wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter};

use crate::draw::{self, Frozen, Words};
use crate::model::{BOTTOM, Cursor, LEFT, Model, Outcome, RIGHT, TOP};
use crate::{Options, Screen};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
/// Two presses this close in time and place are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Shows the overlay on `screen`'s monitor and runs it to its end: Err when
/// it couldn't be shown (no compositor, no layer shell).
pub fn pick(screen: &Screen, options: &Options) -> Result<Option<Outcome>, ()> {
    let conn = Connection::connect_to_env().map_err(drop)?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(drop)?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(drop)?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(drop)?;
    let shm = Shm::bind(&globals, &qh).map_err(drop)?;
    let viewporter: Option<WpViewporter> = globals.bind(&qh, 1..=1, ()).ok();
    let cursor_shape = CursorShapeManager::bind(&globals, &qh).ok();

    let frozen = Frozen::new(&screen.frozen);
    let mut model = Model::new((screen.logical.0 as f32, screen.logical.1 as f32), screen.scale as f32);
    model.windows = screen.windows.clone();
    model.previous = screen.previous;
    model.snap = options.snap;
    model.color = options.color;

    let pool = SlotPool::new((frozen.width * frozen.height * 4) as usize, &shm).map_err(drop)?;
    let mut state = State {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer: None,
        viewport: None,
        frozen,
        model,
        accent: options.accent,
        free: options.free.clone(),
        t: options.t,
        configured: false,
        dirty: true,
        frame_pending: false,
        keyboard: None,
        pointer: None,
        cursor_shape,
        shape_device: None,
        enter_serial: 0,
        shown_cursor: None,
        mods: Modifiers::default(),
        last_press: None,
        outcome: None,
    };
    // The outputs' names come with the first roundtrip.
    queue.roundtrip(&mut state).map_err(drop)?;
    let output = state.output_state.outputs().find(|o| state.output_state.info(o).and_then(|i| i.name).as_deref() == Some(screen.output.as_str()));
    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("kadr-picker"), output.as_ref());
    layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.set_size(0, 0);
    state.viewport = viewporter.as_ref().map(|v| v.get_viewport(layer.wl_surface(), &qh, ()));
    layer.commit();
    state.layer = Some(layer);

    while state.outcome.is_none() {
        if queue.dispatch_pending(&mut state).is_err() || queue.flush().is_err() {
            return Ok(None);
        }
        // Wait for the compositor; while dragging, no longer than a frame,
        // to keep the pointer on the monitor.
        let dragging = state.model.drag.is_some() && options.pointer.is_some();
        if let Some(guard) = queue.prepare_read() {
            use std::os::fd::AsRawFd;
            let mut fd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one pollfd for the connection's open descriptor.
            let ready = unsafe { libc::poll(&mut fd, 1, if dragging { 8 } else { -1 }) };
            if ready > 0 && guard.read().is_err() {
                return Ok(None);
            }
        }
        if dragging {
            state.keep_pointer(screen.origin, options.pointer, &qh);
        }
    }
    // Gone from the screen before the app goes on (it may take a shot).
    if let Some(layer) = state.layer.take() {
        drop(layer);
        let _ = queue.roundtrip(&mut state);
    }
    Ok(state.outcome.flatten())
}

struct State {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: Option<LayerSurface>,
    viewport: Option<WpViewport>,
    frozen: Frozen,
    model: Model,
    accent: kadr_core::color::Rgba,
    free: String,
    t: fn(&'static str) -> String,
    configured: bool,
    dirty: bool,
    frame_pending: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    cursor_shape: Option<CursorShapeManager>,
    shape_device: Option<WpCursorShapeDeviceV1>,
    enter_serial: u32,
    shown_cursor: Option<Cursor>,
    mods: Modifiers,
    last_press: Option<(Instant, (f32, f32))>,
    /// Set once it's over: what it ended in.
    outcome: Option<Option<Outcome>>,
}

impl State {
    /// Past the monitor's edge while dragging: the pointer back on its edge,
    /// the selection out to it.
    fn keep_pointer(&mut self, (ox, oy): (f64, f64), pointer: Option<(fn() -> Option<(f64, f64)>, fn(f64, f64))>, qh: &QueueHandle<Self>) {
        let Some((at, put)) = pointer else { return };
        let Some((x, y)) = at() else { return };
        let (sw, sh) = self.model.screen;
        let (lx, ly) = ((x - ox) as f32, (y - oy) as f32);
        if lx < 0. || ly < 0. || lx >= sw || ly >= sh {
            let (cx, cy) = (lx.clamp(0., sw - 1.), ly.clamp(0., sh - 1.));
            put(ox + cx as f64, oy + cy as f64);
            self.model.drag_to((lx.clamp(0., sw), ly.clamp(0., sh)));
            self.model.cursor = Some((cx, cy));
            self.changed(qh);
        }
    }

    fn changed(&mut self, qh: &QueueHandle<Self>) {
        self.dirty = true;
        self.update_cursor();
        if !self.frame_pending {
            self.draw(qh);
        }
    }

    fn end(&mut self, outcome: Option<Outcome>) {
        if let Some(o) = outcome {
            self.outcome = Some(match o {
                Outcome::Done(None) => None,
                o => Some(o),
            });
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let Some(layer) = self.layer.as_ref() else { return };
        if !self.configured || !self.dirty {
            return;
        }
        let (w, h) = (self.frozen.width as i32, self.frozen.height as i32);
        let Ok((buffer, canvas)) = self.pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else { return };
        let t = self.t;
        let words = Words { keys: draw::keys(&self.model, t), free: self.free.clone() };
        draw::frame(canvas, &self.frozen, &self.model, self.accent, &words);
        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, w, h);
        surface.frame(qh, FrameCallbackData(surface.clone()));
        let _ = buffer.attach_to(surface);
        layer.commit();
        self.dirty = false;
        self.frame_pending = true;
    }

    fn update_cursor(&mut self) {
        let shape = self.model.cursor_shape();
        if self.shown_cursor == Some(shape) {
            return;
        }
        if let Some(device) = &self.shape_device {
            device.set_shape(
                self.enter_serial,
                match shape {
                    Cursor::Crosshair => Shape::Crosshair,
                    Cursor::PointingHand => Shape::Pointer,
                    Cursor::OpenHand => Shape::Grab,
                    Cursor::ClosedHand => Shape::Grabbing,
                    Cursor::Resize(e) if e == LEFT || e == RIGHT => Shape::EwResize,
                    Cursor::Resize(e) if e == TOP || e == BOTTOM => Shape::NsResize,
                    Cursor::Resize(e) if e == LEFT | TOP || e == RIGHT | BOTTOM => Shape::NwseResize,
                    Cursor::Resize(_) => Shape::NeswResize,
                },
            );
            self.shown_cursor = Some(shape);
        }
    }
}

/// GPUI's key names, which the model speaks.
fn key_name(e: &KeyEvent) -> Option<String> {
    let named = match e.keysym {
        Keysym::Escape => "escape",
        Keysym::Return | Keysym::KP_Enter => "enter",
        Keysym::space => "space",
        Keysym::Left => "left",
        Keysym::Right => "right",
        Keysym::Up => "up",
        Keysym::Down => "down",
        Keysym::BackSpace => "backspace",
        Keysym::Tab => "tab",
        _ => return e.utf8.as_deref().filter(|s| s.chars().count() == 1).map(str::to_lowercase),
    };
    Some(named.to_owned())
}

impl CompositorHandler for State {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_pending = false;
        self.draw(qh);
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.outcome = Some(None);
    }

    fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &LayerSurface, configure: LayerSurfaceConfigure, _: u32) {
        if let Some(viewport) = &self.viewport {
            let (w, h) = configure.new_size;
            viewport.set_destination(w.max(1) as i32, h.max(1) as i32);
        }
        self.configured = true;
        self.dirty = true;
        self.draw(qh);
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
            if let (Some(manager), Some(pointer)) = (&self.cursor_shape, &self.pointer) {
                self.shape_device = Some(manager.get_shape_device(pointer, qh));
            }
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for State {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}

    fn press_key(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if let Some(name) = key_name(&event) {
            let outcome = self.model.key(&name, self.mods.shift, self.mods.alt, false);
            self.end(outcome);
            self.changed(qh);
        }
    }

    fn repeat_key(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if let Some(name) = key_name(&event) {
            let outcome = self.model.key(&name, self.mods.shift, self.mods.alt, true);
            self.end(outcome);
            self.changed(qh);
        }
    }

    fn release_key(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        if let Some(name) = key_name(&event) {
            self.model.key_up(&name);
            self.changed(qh);
        }
    }

    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, modifiers: Modifiers, _: RawModifiers, _: u32) {
        self.mods = modifiers;
    }
}

impl PointerHandler for State {
    fn pointer_frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for event in events {
            let p = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.enter_serial = serial;
                    self.shown_cursor = None;
                    self.model.motion(p, self.mods.shift);
                }
                PointerEventKind::Motion { .. } => self.model.motion(p, self.mods.shift),
                PointerEventKind::Press { button: BTN_LEFT, .. } => {
                    let now = Instant::now();
                    let double = self.last_press.is_some_and(|(at, q)| now - at < DOUBLE_CLICK && (q.0 - p.0).abs() < 4. && (q.1 - p.1).abs() < 4.);
                    self.last_press = Some((now, p));
                    self.model.press(p, if double { 2 } else { 1 });
                }
                PointerEventKind::Release { button: BTN_LEFT, .. } => {
                    let outcome = self.model.release();
                    self.end(outcome);
                }
                // On release, like the left button: the click doesn't go to
                // the window below.
                PointerEventKind::Release { button: BTN_RIGHT, .. } => self.outcome = Some(None),
                _ => {}
            }
        }
        self.changed(qh);
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(State);
smithay_client_toolkit::delegate_dispatch2!(State);
wayland_client::delegate_noop!(State: ignore WpViewporter);
wayland_client::delegate_noop!(State: ignore WpViewport);
