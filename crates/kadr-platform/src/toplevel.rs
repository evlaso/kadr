//! A window by itself on Wayland, whatever covers it, through the standard
//! capture protocols Hyprland has: the window from ext-foreign-toplevel-list
//! (found by its app id and title), a capture source for it, and one frame
//! of ext-image-copy-capture copied into shared memory.

use std::os::fd::{AsFd, FromRawFd, OwnedFd};

use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{ext_foreign_toplevel_handle_v1 as handle, ext_foreign_toplevel_list_v1 as list};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1 as sources, ext_image_capture_source_v1 as source,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1 as frame, ext_image_copy_capture_manager_v1 as copy, ext_image_copy_capture_session_v1 as session,
};

#[derive(Default)]
struct State {
    list: Option<list::ExtForeignToplevelListV1>,
    sources: Option<sources::ExtForeignToplevelImageCaptureSourceManagerV1>,
    copy: Option<copy::ExtImageCopyCaptureManagerV1>,
    shm: Option<wl_shm::WlShm>,
    /// Each window: its handle, app id and title.
    windows: Vec<(handle::ExtForeignToplevelHandleV1, String, String)>,
    size: Option<(u32, u32)>,
    formats: Vec<wl_shm::Format>,
    constraints_done: bool,
    /// The frame came (true) or failed (false).
    ready: Option<bool>,
}

/// The window with `app_id` and `title`, in physical pixels, as RGBA.
pub fn grab(app_id: &str, title: &str) -> Option<image::RgbaImage> {
    let conn = Connection::connect_to_env().ok()?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state).ok()?;
    // The windows and their names arrive after the list is bound.
    queue.roundtrip(&mut state).ok()?;
    queue.roundtrip(&mut state).ok()?;
    let (sources, copy, shm) = (state.sources.clone()?, state.copy.clone()?, state.shm.clone()?);
    let target = state.windows.iter().find(|(_, a, t)| a == app_id && t == title).or_else(|| state.windows.iter().find(|(_, a, _)| a == app_id))?.0.clone();

    let src = sources.create_source(&target, &qh, ());
    let session = copy.create_session(&src, copy::Options::empty(), &qh, ());
    while !state.constraints_done {
        queue.blocking_dispatch(&mut state).ok()?;
        if state.ready == Some(false) {
            return None;
        }
    }
    let (w, h) = state.size?;
    let format = [wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888].into_iter().find(|f| state.formats.contains(f))?;
    let len = (w * h * 4) as usize;
    // SAFETY: an anonymous memory file of our own, sized before it's mapped.
    let fd = unsafe {
        let raw = libc::memfd_create(c"kadr-window".as_ptr(), libc::MFD_CLOEXEC);
        if raw < 0 {
            return None;
        }
        OwnedFd::from_raw_fd(raw)
    };
    // SAFETY: the file is ours and `len` long once truncated.
    let map = unsafe {
        if libc::ftruncate(std::os::fd::AsRawFd::as_raw_fd(&fd), len as libc::off_t) < 0 {
            return None;
        }
        let p = libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, std::os::fd::AsRawFd::as_raw_fd(&fd), 0);
        if p == libc::MAP_FAILED {
            return None;
        }
        p as *mut u8
    };
    let pool = shm.create_pool(fd.as_fd(), len as i32, &qh, ());
    let buffer = pool.create_buffer(0, w as i32, h as i32, (w * 4) as i32, format, &qh, ());
    let frame = session.create_frame(&qh, ());
    frame.attach_buffer(&buffer);
    frame.damage_buffer(0, 0, w as i32, h as i32);
    frame.capture();
    while state.ready.is_none() {
        if queue.blocking_dispatch(&mut state).is_err() {
            break;
        }
    }
    // SAFETY: the compositor is done writing once the frame is ready.
    let pixels = unsafe { std::slice::from_raw_parts(map, len) }.to_vec();
    // SAFETY: unmapping what was mapped above.
    unsafe { libc::munmap(map as *mut libc::c_void, len) };
    frame.destroy();
    session.destroy();
    src.destroy();
    buffer.destroy();
    pool.destroy();
    if state.ready != Some(true) {
        return None;
    }
    // BGRA (or BGRx) in memory, little-endian.
    let opaque = format == wl_shm::Format::Xrgb8888;
    let rgba = pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], if opaque { 255 } else { p[3] }]).collect();
    image::RgbaImage::from_raw(w, h, rgba)
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(state: &mut State, registry: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<State>) {
        let wl_registry::Event::Global { name, interface, version } = event else { return };
        match interface.as_str() {
            "ext_foreign_toplevel_list_v1" => state.list = Some(registry.bind(name, version.min(1), qh, ())),
            "ext_foreign_toplevel_image_capture_source_manager_v1" => state.sources = Some(registry.bind(name, version.min(1), qh, ())),
            "ext_image_copy_capture_manager_v1" => state.copy = Some(registry.bind(name, version.min(1), qh, ())),
            "wl_shm" => state.shm = Some(registry.bind(name, version.min(1), qh, ())),
            _ => {}
        }
    }
}

impl Dispatch<list::ExtForeignToplevelListV1, ()> for State {
    fn event(state: &mut State, _: &list::ExtForeignToplevelListV1, event: list::Event, _: &(), _: &Connection, _: &QueueHandle<State>) {
        if let list::Event::Toplevel { toplevel } = event {
            state.windows.push((toplevel, String::new(), String::new()));
        }
    }

    event_created_child!(State, list::ExtForeignToplevelListV1, [
        list::EVT_TOPLEVEL_OPCODE => (handle::ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<handle::ExtForeignToplevelHandleV1, ()> for State {
    fn event(state: &mut State, toplevel: &handle::ExtForeignToplevelHandleV1, event: handle::Event, _: &(), _: &Connection, _: &QueueHandle<State>) {
        let Some(entry) = state.windows.iter_mut().find(|(h, _, _)| h == toplevel) else { return };
        match event {
            handle::Event::AppId { app_id } => entry.1 = app_id,
            handle::Event::Title { title } => entry.2 = title,
            _ => {}
        }
    }
}

impl Dispatch<session::ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(state: &mut State, _: &session::ExtImageCopyCaptureSessionV1, event: session::Event, _: &(), _: &Connection, _: &QueueHandle<State>) {
        match event {
            session::Event::BufferSize { width, height } => state.size = Some((width, height)),
            session::Event::ShmFormat { format: WEnum::Value(f) } => state.formats.push(f),
            session::Event::Done => state.constraints_done = true,
            session::Event::Stopped => state.ready = Some(false),
            _ => {}
        }
    }
}

impl Dispatch<frame::ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(state: &mut State, _: &frame::ExtImageCopyCaptureFrameV1, event: frame::Event, _: &(), _: &Connection, _: &QueueHandle<State>) {
        match event {
            frame::Event::Ready => state.ready = Some(true),
            frame::Event::Failed { .. } => state.ready = Some(false),
            _ => {}
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut State, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<State>) {}
        }
    )*};
}
ignore!(sources::ExtForeignToplevelImageCaptureSourceManagerV1, source::ExtImageCaptureSourceV1, copy::ExtImageCopyCaptureManagerV1, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer);
