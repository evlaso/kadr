//! `kadr-pointer`: a pointer for the e2e tests, on any compositor with
//! wlr-virtual-pointer (Hyprland does), so a test can click and drag in any
//! UI the way a person would, whatever it's built with.
//!
//!   kadr-pointer move X Y
//!   kadr-pointer click X Y
//!   kadr-pointer drag X1 Y1 X2 Y2 [steps]
//!
//! Coordinates are in the first output's pixels (the nested session has one).

use std::time::{Duration, Instant};

use wayland_client::protocol::{wl_output, wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{zwlr_virtual_pointer_manager_v1 as manager, zwlr_virtual_pointer_v1 as vpointer};

#[derive(Default)]
struct State {
    manager: Option<manager::ZwlrVirtualPointerManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    output: Option<wl_output::WlOutput>,
    size: Option<(u32, u32)>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(state: &mut Self, registry: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "zwlr_virtual_pointer_manager_v1" => state.manager = Some(registry.bind(name, version.min(2), qh, ())),
                "wl_seat" if state.seat.is_none() => state.seat = Some(registry.bind(name, 1, qh, ())),
                "wl_output" if state.output.is_none() => state.output = Some(registry.bind(name, version.min(2), qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(state: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Mode { width, height, .. } = event {
            state.size = Some((width as u32, height as u32));
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<manager::ZwlrVirtualPointerManagerV1, ()> for State {
    fn event(_: &mut Self, _: &manager::ZwlrVirtualPointerManagerV1, _: manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<vpointer::ZwlrVirtualPointerV1, ()> for State {
    fn event(_: &mut Self, _: &vpointer::ZwlrVirtualPointerV1, _: vpointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

/// evdev's left button.
const BTN_LEFT: u32 = 0x110;

struct Pointer {
    conn: Connection,
    pointer: vpointer::ZwlrVirtualPointerV1,
    size: (u32, u32),
    start: Instant,
}

impl Pointer {
    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn to(&self, x: f64, y: f64) {
        let (w, h) = self.size;
        self.pointer.motion_absolute(self.time(), x.clamp(0., w as f64 - 1.) as u32, y.clamp(0., h as f64 - 1.) as u32, w, h);
        self.pointer.frame();
        self.flush();
    }

    fn button(&self, down: bool) {
        let state = if down { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
        self.pointer.button(self.time(), BTN_LEFT, state);
        self.pointer.frame();
        self.flush();
    }

    fn flush(&self) {
        let _ = self.conn.flush();
        // Room for the compositor to deliver each event on its own.
        std::thread::sleep(Duration::from_millis(8));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize| -> f64 { args.get(i).and_then(|v| v.parse().ok()).unwrap_or_else(|| usage()) };
    let conn = Connection::connect_to_env().unwrap_or_else(|e| fail(&format!("no Wayland session: {e}")));
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).unwrap_or_else(|e| fail(&e.to_string()));
    queue.roundtrip(&mut state).unwrap_or_else(|e| fail(&e.to_string()));
    let manager = state.manager.clone().unwrap_or_else(|| fail("the compositor has no wlr-virtual-pointer"));
    let size = state.size.unwrap_or_else(|| fail("no output"));
    let pointer = manager.create_virtual_pointer(state.seat.as_ref(), &qh, ());
    let p = Pointer { conn, pointer, size, start: Instant::now() };
    match args.first().map(String::as_str) {
        Some("move") => p.to(num(1), num(2)),
        Some("click") => {
            p.to(num(1), num(2));
            p.button(true);
            p.button(false);
        }
        Some("drag") => {
            let (x1, y1, x2, y2) = (num(1), num(2), num(3), num(4));
            let steps = args.get(5).and_then(|v| v.parse::<u32>().ok()).unwrap_or(16).max(1);
            p.to(x1, y1);
            p.button(true);
            for i in 1..=steps {
                let k = i as f64 / steps as f64;
                p.to(x1 + (x2 - x1) * k, y1 + (y2 - y1) * k);
            }
            p.button(false);
        }
        _ => usage(),
    }
    let _ = queue.roundtrip(&mut state);
}

fn usage() -> ! {
    fail("usage: kadr-pointer move X Y | click X Y | drag X1 Y1 X2 Y2 [steps]")
}

fn fail(why: &str) -> ! {
    eprintln!("kadr-pointer: {why}");
    std::process::exit(2);
}
