//! The overlay's own window on macOS and Windows: a borderless window over
//! the whole monitor, above everything (on macOS at the screen saver's
//! level, over the menu bar and the Dock), drawn on the CPU through
//! softbuffer. It runs in the `kadr-overlay` program, since winit wants the
//! main thread, which the app's UI has: once per pick (`pick`), or kept
//! running with its event loop warm, taking picks over a socket (`serve`,
//! macOS), since starting AppKit takes most of a cold start there. The
//! warm one keeps its window made and hidden between picks, and on macOS
//! freezes the screen itself, ScreenCaptureKit warmed up (`sck`).

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId, WindowLevel};

use crate::draw::{self, Frozen, Words};
use crate::model::{BOTTOM, Cursor, LEFT, Model, Outcome, RIGHT, TOP};
use crate::{Options, Screen};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Where a pick's outcome goes when it ends.
type Reply = Box<dyn FnOnce(Option<Outcome>) + Send>;

/// A pick handed to the running loop.
pub(crate) struct Request {
    pub screen: Screen,
    pub options: Options,
    pub reply: Reply,
}

fn event_loop() -> Result<EventLoop<Request>, ()> {
    let mut builder = EventLoop::<Request>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        // No Dock icon, and the keys come here at once.
        builder.with_activation_policy(ActivationPolicy::Accessory).with_activate_ignoring_other_apps(true);
    }
    builder.build().map_err(drop)
}

/// Shows the overlay once and runs it to its end; Err when it couldn't
/// show itself.
pub fn pick(screen: &Screen, options: &Options) -> Result<Option<Outcome>, ()> {
    let event_loop = event_loop()?;
    let done = std::sync::Arc::new(std::sync::Mutex::new(None));
    let sink = done.clone();
    let first = Request {
        screen: Screen { output: screen.output.clone(), logical: screen.logical, scale: screen.scale, frozen: screen.frozen.clone(), windows: screen.windows.clone(), previous: screen.previous, origin: screen.origin },
        options: Options { free: options.free.clone(), ..*options },
        reply: Box::new(move |o| *sink.lock().unwrap() = Some(o)),
    };
    let mut app = App { first: Some(first), session: None, spare: None, once: true, failed: false };
    event_loop.run_app(&mut app).map_err(drop)?;
    if app.failed {
        return Err(());
    }
    Ok(done.lock().unwrap().take().flatten())
}

/// Keeps the overlay's event loop running and takes picks over `socket`:
/// a line with a spec's path in (`spec::write`, or `spec::write_request`
/// for it to freeze the screen itself), the outcome's line back (with the
/// frozen file it wrote beside the spec, for a request).
#[cfg(unix)]
pub fn serve(socket: &std::path::Path) -> Result<(), ()> {
    use std::io::{BufRead, BufReader, Write};
    let event_loop = event_loop()?;
    let proxy = event_loop.create_proxy();
    let _ = std::fs::remove_file(socket);
    let listener = std::os::unix::net::UnixListener::bind(socket).map_err(drop)?;
    std::thread::spawn(move || {
        // Warmed up before the first pick; picks wait in the socket's queue.
        #[cfg(target_os = "macos")]
        let mut grabber = crate::sck::Grabber::new();
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let Ok(mut reply) = stream.try_clone() else { continue };
            if BufReader::new(stream).read_line(&mut line).is_err() {
                continue;
            }
            let path = std::path::Path::new(line.trim());
            let error = |mut reply: std::os::unix::net::UnixStream, what: &str| {
                let _ = writeln!(reply, "{}", serde_json::json!({ "error": what }));
            };
            let Some((mut screen, options, lang, capture)) = crate::spec::read_request(path) else {
                error(reply, "can't read the spec");
                continue;
            };
            let mut frozen_file = None;
            if capture {
                let from = Instant::now();
                #[cfg(target_os = "macos")]
                let image = grabber.as_mut().and_then(|g| g.grab(screen.origin, screen.logical, screen.scale));
                #[cfg(not(target_os = "macos"))]
                let image = None;
                // No picture: kadr takes its own and asks again with it.
                let Some(image) = image else {
                    error(reply, "can't freeze the screen");
                    continue;
                };
                eprintln!("kadr-overlay timing: grab={}ms", from.elapsed().as_millis());
                screen.frozen = std::sync::Arc::new(image);
                frozen_file = Some(path.with_file_name("frozen.rgba"));
            }
            kadr_core::i18n::set(kadr_core::i18n::Lang::from_code(&lang));
            let options = Options { free: kadr_core::i18n::t("picker.free").to_owned(), ..options };
            let frozen = screen.frozen.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let request = Request { screen, options, reply: Box::new(move |o| {
                let _ = tx.send(o);
            }) };
            if proxy.send_event(request).is_err() {
                break;
            }
            // Written while the overlay is up; the outcome waits for it.
            let written = frozen_file.filter(|f| crate::spec::write_frozen(f, &frozen).is_ok());
            let Ok(outcome) = rx.recv() else { break };
            let _ = writeln!(reply, "{}", crate::spec::outcome_line_with(outcome, written.as_deref()));
        }
    });
    let mut app = App { first: None, session: None, spare: None, once: false, failed: false };
    event_loop.run_app(&mut app).map_err(drop)
}

/// The overlay's window and what it's drawn through.
type Shown = (Rc<Window>, softbuffer::Surface<Rc<Window>, Rc<Window>>);

/// A borderless window above everything, `size` big at `at` (logical).
fn make_window(el: &ActiveEventLoop, at: (f64, f64), size: (f64, f64), visible: bool) -> Option<Shown> {
    #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
    let mut attrs = Window::default_attributes()
        .with_title("kadr-picker")
        .with_decorations(false)
        .with_resizable(false)
        .with_visible(visible)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_position(LogicalPosition::new(at.0, at.1))
        .with_inner_size(LogicalSize::new(size.0, size.1));
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::WindowAttributesExtWindows;
        attrs = attrs.with_skip_taskbar(true);
    }
    let window = Rc::new(el.create_window(attrs).ok()?);
    #[cfg(target_os = "macos")]
    above_everything(&window);
    let surface = softbuffer::Context::new(window.clone()).and_then(|ctx| softbuffer::Surface::new(&ctx, window.clone())).ok()?;
    Some((window, surface))
}

struct App {
    /// The pick to show once the loop is up (a one-time run).
    first: Option<Request>,
    session: Option<Session>,
    /// The warm one's window between picks, hidden.
    spare: Option<Shown>,
    /// Ends the loop after the first pick.
    once: bool,
    failed: bool,
}

/// One pick on screen.
struct Session {
    screen: Screen,
    options: Options,
    frozen: Frozen,
    model: Model,
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    mods: ModifiersState,
    last_press: Option<(Instant, (f32, f32))>,
    shown_cursor: Option<Cursor>,
    presented: bool,
    reply: Option<Reply>,
    /// When it was asked for, for its timing line.
    asked: Instant,
}

impl App {
    fn start(&mut self, el: &ActiveEventLoop, request: Request) {
        if self.session.is_some() {
            // One at a time: a second pick while one is up is cancelled.
            return (request.reply)(None);
        }
        let asked = Instant::now();
        let Request { screen, options, reply } = request;
        let at = screen.origin;
        let size = (screen.logical.0 as f64, screen.logical.1 as f64);
        // The warm one's spare, moved onto this monitor; else a new one.
        let reused = self.spare.take().map(|(window, surface)| {
            window.set_outer_position(LogicalPosition::new(at.0, at.1));
            let _ = window.request_inner_size(LogicalSize::new(size.0, size.1));
            (window, surface)
        });
        let spare = reused.is_some();
        let Some((window, surface)) = reused.or_else(|| make_window(el, at, size, true)) else {
            self.failed = true;
            reply(None);
            return if self.once { el.exit() } else {};
        };
        eprintln!("kadr-overlay timing: window={}ms spare={spare} asked_to_window={}ms", crate::since_start(), asked.elapsed().as_millis());
        let mut model = Model::new((screen.logical.0 as f32, screen.logical.1 as f32), screen.scale as f32);
        model.windows = screen.windows.clone();
        model.previous = screen.previous;
        model.snap = options.snap;
        model.color = options.color;
        let frozen = Frozen::new(&screen.frozen);
        self.session = Some(Session {
            screen,
            options,
            frozen,
            model,
            window: window.clone(),
            surface,
            mods: ModifiersState::empty(),
            last_press: None,
            shown_cursor: None,
            presented: false,
            reply: Some(reply),
            asked,
        });
        if let Some(s) = self.session.as_mut() {
            s.update_cursor();
            if spare {
                // Its frame before it shows, not the last pick's.
                s.draw();
                s.window.set_visible(true);
            }
        }
        window.focus_window();
        window.request_redraw();
    }

    fn end(&mut self, outcome: Option<Outcome>, el: &ActiveEventLoop) {
        let Some(o) = outcome else { return };
        let o = match o {
            Outcome::Done(None) => None,
            o => Some(o),
        };
        // The window goes with the session, or is hidden for the next.
        if let Some(mut s) = self.session.take() {
            if !self.once {
                s.window.set_visible(false);
            }
            if let Some(reply) = s.reply.take() {
                reply(o);
            }
            if !self.once {
                self.spare = Some((s.window, s.surface));
            }
        }
        if self.once {
            el.exit();
        }
    }
}

impl Session {
    fn update_cursor(&mut self) {
        let shape = self.model.cursor_shape();
        if self.shown_cursor == Some(shape) {
            return;
        }
        self.window.set_cursor(match shape {
            Cursor::Crosshair => CursorIcon::Crosshair,
            Cursor::PointingHand => CursorIcon::Pointer,
            Cursor::OpenHand => CursorIcon::Grab,
            Cursor::ClosedHand => CursorIcon::Grabbing,
            Cursor::Resize(e) if e == LEFT || e == RIGHT => CursorIcon::EwResize,
            Cursor::Resize(e) if e == TOP || e == BOTTOM => CursorIcon::NsResize,
            Cursor::Resize(e) if e == LEFT | TOP || e == RIGHT | BOTTOM => CursorIcon::NwseResize,
            Cursor::Resize(_) => CursorIcon::NeswResize,
        });
        self.shown_cursor = Some(shape);
    }

    fn draw(&mut self) {
        let size = self.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        // The window may come out a pixel off the monitor's mode: the frozen
        // screen follows it.
        if (self.frozen.width, self.frozen.height) != (size.width, size.height) {
            let resized = image::imageops::resize(self.screen.frozen.as_ref(), size.width, size.height, image::imageops::FilterType::Triangle);
            self.frozen = Frozen::new(&resized);
            self.model.scale = size.width as f32 / self.model.screen.0;
        }
        if self.surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = self.surface.buffer_mut() else { return };
        // softbuffer's 0RGB words are B G R x in memory, as draw writes.
        let bytes = unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, buffer.len() * 4) };
        let words = Words { keys: draw::keys(&self.model, self.options.t), free: self.options.free.clone() };
        draw::frame(bytes, &self.frozen, &self.model, self.options.accent, &words);
        self.window.pre_present_notify();
        let _ = buffer.present();
        if !self.presented {
            self.presented = true;
            eprintln!("kadr-overlay timing: first_frame={}ms asked_to_frame={}ms", crate::since_start(), self.asked.elapsed().as_millis());
        }
    }
}

fn key_name(key: &Key) -> Option<String> {
    Some(match key {
        Key::Named(NamedKey::Escape) => "escape".into(),
        Key::Named(NamedKey::Enter) => "enter".into(),
        Key::Named(NamedKey::Space) => "space".into(),
        Key::Named(NamedKey::ArrowLeft) => "left".into(),
        Key::Named(NamedKey::ArrowRight) => "right".into(),
        Key::Named(NamedKey::ArrowUp) => "up".into(),
        Key::Named(NamedKey::ArrowDown) => "down".into(),
        Key::Named(NamedKey::Backspace) => "backspace".into(),
        Key::Named(NamedKey::Tab) => "tab".into(),
        Key::Character(s) if s.chars().count() == 1 => s.to_lowercase(),
        _ => return None,
    })
}

impl ApplicationHandler<Request> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if let Some(first) = self.first.take() {
            self.start(el, first);
        }
        // The warm one makes its window now, hidden, on the main monitor.
        if !self.once && self.spare.is_none() && self.session.is_none() {
            let m = el.primary_monitor().or_else(|| el.available_monitors().next());
            let (at, size) = m.map_or(((0., 0.), (1024., 768.)), |m| {
                let (p, s) = (m.position().to_logical::<f64>(m.scale_factor()), m.size().to_logical::<f64>(m.scale_factor()));
                ((p.x, p.y), (s.width, s.height))
            });
            let from = Instant::now();
            self.spare = make_window(el, at, size, false);
            eprintln!("kadr-overlay timing: spare_window={}ms", from.elapsed().as_millis());
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, request: Request) {
        self.start(el, request);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(s) = self.session.as_mut() else { return };
        let scale = s.window.scale_factor();
        let mut outcome = None;
        match event {
            WindowEvent::RedrawRequested => return s.draw(),
            WindowEvent::CloseRequested => outcome = Some(Outcome::Done(None)),
            WindowEvent::ModifiersChanged(m) => s.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let p: LogicalPosition<f64> = PhysicalPosition::new(position.x, position.y).to_logical(scale);
                s.model.motion((p.x as f32, p.y as f32), s.mods.shift_key());
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => {
                    let p = s.model.cursor.unwrap_or_default();
                    let now = Instant::now();
                    let double = s.last_press.is_some_and(|(at, q)| now - at < DOUBLE_CLICK && (q.0 - p.0).abs() < 4. && (q.1 - p.1).abs() < 4.);
                    s.last_press = Some((now, p));
                    s.model.press(p, if double { 2 } else { 1 });
                }
                ElementState::Released => outcome = s.model.release(),
            },
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Right, .. } => outcome = Some(Outcome::Done(None)),
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key, state, repeat, .. }, .. } => {
                if let Some(name) = key_name(&logical_key) {
                    if state == ElementState::Pressed {
                        outcome = s.model.key(&name, s.mods.shift_key(), s.mods.alt_key(), repeat);
                    } else {
                        s.model.key_up(&name);
                    }
                }
            }
            _ => return,
        }
        if outcome.is_some() {
            return self.end(outcome, el);
        }
        s.update_cursor();
        s.window.request_redraw();
    }
}

/// macOS: over the menu bar and the Dock, on every Space, like the
/// system's own screenshot overlay.
#[cfg(target_os = "macos")]
fn above_everything(window: &Window) {
    use objc2_app_kit::{NSView, NSWindowCollectionBehavior};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: the live window's NSView, on the main thread.
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    if let Some(w) = view.window() {
        // NSScreenSaverWindowLevel.
        w.setLevel(1000);
        w.setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary);
    }
}
