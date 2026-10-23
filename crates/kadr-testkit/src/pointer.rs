//! `kadr-pointer`: a pointer (and on macOS and Windows a keyboard) for the
//! e2e tests, so a test can click, drag and type in any UI the way a person
//! would, whatever it's built with. On Linux through wlr-virtual-pointer (the
//! nested Hyprland; keys go through wtype there), elsewhere through the
//! system's own input (enigo).
//!
//!   kadr-pointer move X Y
//!   kadr-pointer click X Y
//!   kadr-pointer drag X1 Y1 X2 Y2 [steps]
//!   kadr-pointer key ctrl+s        (macOS, Windows)
//!   kadr-pointer type TEXT         (macOS, Windows)
//!   kadr-pointer type-hex HEX      the same, the text as UTF-8 in hex: a
//!                                  Windows shell may pass non-ASCII
//!                                  arguments in its ANSI code page
//!
//! Coordinates are the screen's: the first output's pixels on Linux, the
//! system's own coordinates (points on macOS) elsewhere.

#[cfg(target_os = "linux")]
mod device {
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

    pub struct Pointer {
        conn: Connection,
        pointer: vpointer::ZwlrVirtualPointerV1,
        size: (u32, u32),
        start: Instant,
    }

    impl Pointer {
        fn time(&self) -> u32 {
            self.start.elapsed().as_millis() as u32
        }

        pub fn to(&mut self, x: f64, y: f64) {
            let (w, h) = self.size;
            self.pointer.motion_absolute(self.time(), x.clamp(0., w as f64 - 1.) as u32, y.clamp(0., h as f64 - 1.) as u32, w, h);
            self.pointer.frame();
            self.flush();
        }

        pub fn button(&mut self, down: bool) {
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

    impl Pointer {
        pub fn open() -> Pointer {
            let conn = Connection::connect_to_env().unwrap_or_else(|e| super::fail(&format!("no Wayland session: {e}")));
            let mut queue = conn.new_event_queue();
            let qh = queue.handle();
            conn.display().get_registry(&qh, ());
            let mut state = State::default();
            queue.roundtrip(&mut state).unwrap_or_else(|e| super::fail(&e.to_string()));
            queue.roundtrip(&mut state).unwrap_or_else(|e| super::fail(&e.to_string()));
            let manager = state.manager.clone().unwrap_or_else(|| super::fail("the compositor has no wlr-virtual-pointer"));
            let size = state.size.unwrap_or_else(|| super::fail("no output"));
            let pointer = manager.create_virtual_pointer(state.seat.as_ref(), &qh, ());
            let p = Pointer { conn, pointer, size, start: Instant::now() };
            let _ = queue.roundtrip(&mut state);
            p
        }

        pub fn key(&mut self, _: &str) {
            super::fail("keys go through wtype on Linux")
        }

        pub fn text(&mut self, _: &str) {
            super::fail("text goes through wtype on Linux")
        }
    }
}


#[cfg(not(target_os = "linux"))]
mod device {
    use std::time::Duration;

    use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};

    pub struct Pointer(Enigo);

    impl Pointer {
        pub fn open() -> Pointer {
            // Physical pixels on Windows, as kadr-shot reports windows.
            #[cfg(windows)]
            let _ = enigo::set_dpi_awareness();
            // On CI nobody's there to grant a prompt.
            let settings = Settings { open_prompt_to_get_permissions: false, ..Settings::default() };
            Pointer(Enigo::new(&settings).unwrap_or_else(|e| super::fail(&format!("no input: {e}"))))
        }

        fn pause() {
            std::thread::sleep(Duration::from_millis(15));
        }

        pub fn to(&mut self, x: f64, y: f64) {
            let _ = self.0.move_mouse(x.round() as i32, y.round() as i32, Coordinate::Abs);
            Self::pause();
        }

        pub fn button(&mut self, down: bool) {
            let _ = self.0.button(Button::Left, if down { Direction::Press } else { Direction::Release });
            Self::pause();
        }

        /// "ctrl+s", "escape", "enter": modifiers held around the last key.
        pub fn key(&mut self, combo: &str) {
            let parts: Vec<&str> = combo.split('+').collect();
            let key = |name: &str| match name.to_lowercase().as_str() {
                "ctrl" | "control" => Key::Control,
                "shift" => Key::Shift,
                "alt" | "option" => Key::Alt,
                "cmd" | "super" | "meta" | "win" => Key::Meta,
                "escape" | "esc" => Key::Escape,
                "enter" | "return" => Key::Return,
                "tab" => Key::Tab,
                "backspace" => Key::Backspace,
                "delete" => Key::Delete,
                "left" => Key::LeftArrow,
                "right" => Key::RightArrow,
                "up" => Key::UpArrow,
                "down" => Key::DownArrow,
                "home" => Key::Home,
                "end" => Key::End,
                other => Key::Unicode(other.chars().next().unwrap_or(' ')),
            };
            let (last, mods) = parts.split_last().unwrap_or_else(|| super::usage());
            for m in mods {
                let _ = self.0.key(key(m), Direction::Press);
            }
            let _ = self.0.key(key(last), Direction::Click);
            for m in mods.iter().rev() {
                let _ = self.0.key(key(m), Direction::Release);
            }
            Self::pause();
        }

        pub fn text(&mut self, text: &str) {
            let _ = self.0.text(text);
            Self::pause();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize| -> f64 { args.get(i).and_then(|v| v.parse().ok()).unwrap_or_else(|| usage()) };
    let mut p = device::Pointer::open();
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
        Some("key") => p.key(args.get(1).map(String::as_str).unwrap_or_else(|| usage())),
        Some("type") => p.text(&args[1..].join(" ")),
        Some("type-hex") => {
            let hex = args.get(1).map(String::as_str).unwrap_or("");
            let bytes: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok())).collect();
            match bytes.and_then(|b| String::from_utf8(b).ok()) {
                Some(text) => p.text(&text),
                None => fail("type-hex: not UTF-8 in hex"),
            }
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    fail("usage: kadr-pointer move X Y | click X Y | drag X1 Y1 X2 Y2 [steps] | key COMBO | type TEXT | type-hex HEX")
}

fn fail(why: &str) -> ! {
    eprintln!("kadr-pointer: {why}");
    std::process::exit(2);
}
