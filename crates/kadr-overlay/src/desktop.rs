//! The overlay's own window on macOS and Windows: a borderless window over
//! the whole monitor, above everything (on macOS at the screen saver's
//! level, over the menu bar and the Dock), drawn on the CPU through
//! softbuffer. It runs in the `kadr-overlay` program, since winit wants the
//! main thread, which the app's UI has.

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

/// Shows the overlay and runs it to its end; Err when it couldn't show
/// itself.
pub fn pick(screen: &Screen, options: &Options) -> Result<Option<Outcome>, ()> {
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        // No Dock icon, and the keys come here at once.
        builder.with_activation_policy(ActivationPolicy::Accessory).with_activate_ignoring_other_apps(true);
    }
    let event_loop = builder.build().map_err(drop)?;
    let mut model = Model::new((screen.logical.0 as f32, screen.logical.1 as f32), screen.scale as f32);
    model.windows = screen.windows.clone();
    model.previous = screen.previous;
    model.snap = options.snap;
    model.color = options.color;
    let mut app = App {
        screen,
        options,
        frozen: Frozen::new(&screen.frozen),
        model,
        window: None,
        surface: None,
        mods: ModifiersState::empty(),
        last_press: None,
        shown_cursor: None,
        outcome: None,
        failed: false,
        presented: false,
    };
    event_loop.run_app(&mut app).map_err(drop)?;
    if app.failed {
        return Err(());
    }
    Ok(app.outcome.flatten())
}

struct App<'a> {
    screen: &'a Screen,
    options: &'a Options,
    frozen: Frozen,
    model: Model,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    mods: ModifiersState,
    last_press: Option<(Instant, (f32, f32))>,
    shown_cursor: Option<Cursor>,
    outcome: Option<Option<Outcome>>,
    failed: bool,
    presented: bool,
}

impl App<'_> {
    fn end(&mut self, outcome: Option<Outcome>, el: &ActiveEventLoop) {
        if let Some(o) = outcome {
            self.outcome = Some(match o {
                Outcome::Done(None) => None,
                o => Some(o),
            });
            el.exit();
        }
    }

    fn update_cursor(&mut self) {
        let shape = self.model.cursor_shape();
        if self.shown_cursor == Some(shape) {
            return;
        }
        if let Some(w) = &self.window {
            w.set_cursor(match shape {
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
    }

    fn draw(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, self.surface.as_mut()) else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        // The window may come out a pixel off the monitor's mode: the frozen
        // screen follows it.
        if (self.frozen.width, self.frozen.height) != (size.width, size.height) {
            let resized = image::imageops::resize(self.screen.frozen.as_ref(), size.width, size.height, image::imageops::FilterType::Triangle);
            self.frozen = Frozen::new(&resized);
            self.model.scale = size.width as f32 / self.model.screen.0;
        }
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        // softbuffer's 0RGB words are B G R x in memory, as draw writes.
        let bytes = unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, buffer.len() * 4) };
        let words = Words { keys: draw::keys(&self.model, self.options.t), free: self.options.free.clone() };
        draw::frame(bytes, &self.frozen, &self.model, self.options.accent, &words);
        window.pre_present_notify();
        let _ = buffer.present();
        if !self.presented {
            self.presented = true;
            eprintln!("kadr-overlay timing: first_frame={}ms", crate::since_start());
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

impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (ox, oy) = self.screen.origin;
        #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
        let mut attrs = Window::default_attributes()
            .with_title("kadr-picker")
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(LogicalPosition::new(ox, oy))
            .with_inner_size(LogicalSize::new(self.screen.logical.0 as f64, self.screen.logical.1 as f64));
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs = attrs.with_skip_taskbar(true);
        }
        let Ok(window) = el.create_window(attrs) else {
            self.failed = true;
            return el.exit();
        };
        let window = Rc::new(window);
        let window_ms = crate::since_start();
        #[cfg(target_os = "macos")]
        above_everything(&window);
        let surface = softbuffer::Context::new(window.clone()).and_then(|ctx| softbuffer::Surface::new(&ctx, window.clone()));
        let Ok(surface) = surface else {
            self.failed = true;
            return el.exit();
        };
        eprintln!("kadr-overlay timing: window={window_ms}ms surface={}ms", crate::since_start());
        window.focus_window();
        self.window = Some(window.clone());
        self.surface = Some(surface);
        self.update_cursor();
        window.request_redraw();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let scale = self.window.as_ref().map_or(1., |w| w.scale_factor());
        match event {
            WindowEvent::RedrawRequested => return self.draw(),
            WindowEvent::CloseRequested => self.end(Some(Outcome::Done(None)), el),
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let p: LogicalPosition<f64> = PhysicalPosition::new(position.x, position.y).to_logical(scale);
                self.model.motion((p.x as f32, p.y as f32), self.mods.shift_key());
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => match state {
                ElementState::Pressed => {
                    let p = self.model.cursor.unwrap_or_default();
                    let now = Instant::now();
                    let double = self.last_press.is_some_and(|(at, q)| now - at < DOUBLE_CLICK && (q.0 - p.0).abs() < 4. && (q.1 - p.1).abs() < 4.);
                    self.last_press = Some((now, p));
                    self.model.press(p, if double { 2 } else { 1 });
                }
                ElementState::Released => {
                    let outcome = self.model.release();
                    self.end(outcome, el);
                }
            },
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Right, .. } => self.end(Some(Outcome::Done(None)), el),
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key, state, repeat, .. }, .. } => {
                if let Some(name) = key_name(&logical_key) {
                    if state == ElementState::Pressed {
                        let outcome = self.model.key(&name, self.mods.shift_key(), self.mods.alt_key(), repeat);
                        self.end(outcome, el);
                    } else {
                        self.model.key_up(&name);
                    }
                }
            }
            _ => return,
        }
        self.update_cursor();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
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
