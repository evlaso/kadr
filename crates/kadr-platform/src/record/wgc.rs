//! Recording on Windows: Windows.Graphics.Capture hands the monitor's
//! frames over as Direct3D textures; the area is copied out of each into a
//! texture the CPU can read, and a Media Foundation sink writer encodes it
//! (H.264, on the GPU where there's an encoder for it) into an MP4. What the
//! computer plays comes from WASAPI's loopback and the microphone from its
//! capture endpoint, both as 16-bit 48 kHz stereo, encoded as AAC. Silence
//! is written where the system plays nothing, so the tracks stay in step.
//! Windows sends a frame only when the screen changes; between them the
//! last one is written again, so the file has a steady frame rate like the
//! other systems'. A pause leaves frames out and shifts the later ones back.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, POINT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient, IAudioClient,
    IMMDeviceEnumerator, MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX, eCapture, eConsole, eRender,
};
use windows::Win32::Media::MediaFoundation::{
    IMFSinkWriter, MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
    MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_VERSION, MFAudioFormat_AAC, MFAudioFormat_PCM, MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFCreateSinkWriterFromURL, MFMediaType_Audio, MFMediaType_Video, MFSTARTUP_FULL, MFStartup, MFVideoFormat_H264, MFVideoFormat_RGB32, MFVideoInterlace_Progressive,
};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{HSTRING, Interface};

use super::Spec;

/// Media Foundation's time unit: 100 ns.
const SECOND: i64 = 10_000_000;
const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// Audio is read this often.
const TICK: Duration = Duration::from_millis(10);

/// The writer and what the capture threads share of it.
struct Writer {
    sink: IMFSinkWriter,
    video: u32,
    system: Option<u32>,
    mic: Option<u32>,
    started: Option<Instant>,
    finished: bool,
    paused: Option<Instant>,
    /// Paused time so far, taken off every later sample.
    offset: Duration,
    ended: Option<Box<dyn FnOnce() + Send>>,
    /// Samples written, and the first write that failed, for the log.
    written: u64,
    failed: Option<String>,
}

impl Writer {
    /// Where `at` falls in the file, or None while paused or not started.
    fn time(&self, at: Instant) -> Option<i64> {
        let start = self.started?;
        if self.paused.is_some() || self.finished {
            return None;
        }
        Some((at.saturating_duration_since(start).saturating_sub(self.offset)).as_nanos() as i64 / 100)
    }

    fn write(&mut self, stream: u32, bytes: &[u8], time: i64, duration: i64) {
        self.written += 1;
        // SAFETY: Media Foundation's buffers, filled before they're handed over.
        unsafe {
            let Ok(buffer) = MFCreateMemoryBuffer(bytes.len() as u32) else { return };
            let mut data = std::ptr::null_mut();
            if buffer.Lock(&mut data, None, None).is_err() {
                return;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
            let _ = buffer.Unlock();
            let _ = buffer.SetCurrentLength(bytes.len() as u32);
            let Ok(sample) = MFCreateSample() else { return };
            let _ = sample.AddBuffer(&buffer);
            let _ = sample.SetSampleTime(time);
            let _ = sample.SetSampleDuration(duration);
            if let Err(e) = self.sink.WriteSample(stream, &sample) {
                self.failed.get_or_insert(format!("WriteSample: {e}"));
            }
        }
    }
}

/// SAFETY: the sink writer is used from one thread at a time, behind the
/// mutex.
struct Shared(Mutex<Writer>);
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    fn end(&self) {
        let ended = self.0.lock().unwrap().ended.take();
        if let Some(ended) = ended {
            ended();
        }
    }
}

pub struct Session {
    shared: Arc<Shared>,
    capture: Arc<Capture>,
}

/// What's stopped when the recording ends: the capture and the threads.
struct Capture {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    stop: Arc<AtomicBool>,
    audio: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

/// SAFETY: as Session's.
unsafe impl Send for Capture {}
unsafe impl Sync for Capture {}

impl Capture {
    fn close(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.session.Close();
        let _ = self.pool.Close();
        for t in self.audio.lock().unwrap().drain(..) {
            let _ = t.join();
        }
    }
}

/// SAFETY: the frame pool is free-threaded and the session's Close may come
/// from any thread.
unsafe impl Send for Session {}

impl Session {
    pub fn start(spec: &Spec, ended: Box<dyn FnOnce() + Send>) -> Result<Session, String> {
        let log = |e: &dyn std::fmt::Display| {
            let _ = std::fs::write(&spec.log, format!("{e}\n"));
            e.to_string()
        };
        // SAFETY: COM, Direct3D, WinRT and Media Foundation calls on objects
        // made here; each frame's texture is read before it's let go.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| log(&format!("Media Foundation: {e}")))?;
            let (device, context) = device().map_err(|e| log(&format!("Direct3D: {e}")))?;
            let dxgi: IDXGIDevice = device.cast().map_err(|e| log(&e))?;
            let d3d: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi).and_then(|i| i.cast()).map_err(|e| log(&e))?;

            let m = &spec.monitor;
            let at = POINT { x: (m.x as f64 * m.scale).round() as i32 + 1, y: (m.y as f64 * m.scale).round() as i32 + 1 };
            let monitor = MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST);
            let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(|e| log(&e))?;
            let item: GraphicsCaptureItem = interop.CreateForMonitor(monitor).map_err(|e| log(&format!("Windows.Graphics.Capture: {e}")))?;
            let full = item.Size().map_err(|e| log(&e))?;
            // The area on the monitor, in its pixels, even in size.
            let even = |n: f64| ((n.round() as i32) / 2 * 2).max(2);
            let (left, top, width, height) = match spec.area {
                Some((x, y, w, h)) => (((x - m.x) as f64 * m.scale).round() as i32, ((y - m.y) as f64 * m.scale).round() as i32, even(w as f64 * m.scale), even(h as f64 * m.scale)),
                None => (0, 0, full.Width / 2 * 2, full.Height / 2 * 2),
            };
            let left = left.clamp(0, (full.Width - width).max(0));
            let top = top.clamp(0, (full.Height - height).max(0));
            let staging = staging(&device, width as u32, height as u32).map_err(|e| log(&e))?;

            let _ = std::fs::remove_file(&spec.out);
            let sink = sink(spec, width as u32, height as u32).map_err(|e| log(&format!("Media Foundation: {e}")))?;
            let shared = Arc::new(Shared(Mutex::new(Writer {
                sink: sink.0,
                video: sink.1,
                system: sink.2,
                mic: sink.3,
                started: None,
                finished: false,
                paused: None,
                offset: Duration::ZERO,
                ended: Some(ended),
                written: 0,
                failed: None,
            })));

            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, full).map_err(|e| log(&e))?;
            let session = pool.CreateCaptureSession(&item).map_err(|e| log(&e))?;
            let _ = session.SetIsCursorCaptureEnabled(spec.cursor);
            // Windows 11: no yellow border around what's recorded.
            let _ = session.SetIsBorderRequired(false);
            let frame_time = SECOND / spec.fps.max(1) as i64;
            let (first_tx, first_rx) = std::sync::mpsc::channel();
            let first = Mutex::new(Some(first_tx));
            let frames = shared.clone();
            // The last frame and its time, shared with the repeater below.
            let last = Arc::new(Mutex::new(None::<i64>));
            let bytes = Arc::new(Mutex::new(vec![0u8; (width * height * 4) as usize]));
            let (frame_last, frame_bytes) = (last.clone(), bytes.clone());
            pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, windows::core::IInspectable>::new(move |pool, _| {
                let Some(pool) = pool.as_ref() else { return Ok(()) };
                let frame = pool.TryGetNextFrame()?;
                let now = Instant::now();
                let mut w = frames.0.lock().unwrap();
                if w.started.is_none() {
                    w.started = Some(now);
                    let _ = w.sink.BeginWriting();
                    if let Some(tx) = first.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                }
                let Some(time) = w.time(now) else { return Ok(()) };
                // No more than the frame rate.
                let mut last = frame_last.lock().unwrap();
                if last.is_some_and(|l| time - l < frame_time * 9 / 10) {
                    return Ok(());
                }
                *last = Some(time);
                let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
                let texture: ID3D11Texture2D = access.GetInterface()?;
                let area = D3D11_BOX { left: left as u32, top: top as u32, front: 0, right: (left + width) as u32, bottom: (top + height) as u32, back: 1 };
                context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &texture, 0, Some(&area));
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                let mut out = frame_bytes.lock().unwrap();
                let row = width as usize * 4;
                for y in 0..height as usize {
                    let src = std::slice::from_raw_parts((mapped.pData as *const u8).add(y * mapped.RowPitch as usize), row);
                    out[y * row..(y + 1) * row].copy_from_slice(src);
                }
                context.Unmap(&staging, 0);
                let video = w.video;
                w.write(video, &out, time, frame_time);
                Ok(())
            }))
            .map_err(|e| log(&e))?;
            session.StartCapture().map_err(|e| log(&format!("Windows.Graphics.Capture: {e}")))?;

            let stop = Arc::new(AtomicBool::new(false));
            let mut audio = Vec::new();
            let streams = { let w = shared.0.lock().unwrap(); (w.system, w.mic) };
            if let Some(stream) = streams.0 {
                audio.push(audio_thread(shared.clone(), stop.clone(), stream, true));
            }
            if let Some(stream) = streams.1 {
                audio.push(audio_thread(shared.clone(), stop.clone(), stream, false));
            }
            audio.push(repeater(shared.clone(), stop.clone(), last, bytes, frame_time));
            let session = Session { shared, capture: Arc::new(Capture { pool, session, stop, audio: Mutex::new(audio) }) };
            if first_rx.recv_timeout(Duration::from_secs(5)).is_err() {
                session.shared.0.lock().unwrap().ended = None;
                session.capture.close();
                let _ = std::fs::remove_file(&spec.out);
                return Err(log(&"Windows.Graphics.Capture sent no frames"));
            }
            Ok(session)
        }
    }

    pub fn set_paused(&self, paused: bool) {
        let mut w = self.shared.0.lock().unwrap();
        match (w.paused, paused) {
            (None, true) => w.paused = Some(Instant::now()),
            (Some(at), false) => {
                w.offset += at.elapsed();
                w.paused = None;
            }
            _ => {}
        }
    }

    /// Stops on a thread of its own (closing the capture waits for its
    /// threads and the encoder), then writes the file out.
    pub fn stop(&self) {
        let (shared, capture) = (self.shared.clone(), self.capture.clone());
        std::thread::spawn(move || {
            capture.close();
            {
                let mut w = shared.0.lock().unwrap();
                w.finished = true;
                if w.started.is_some() {
                    // SAFETY: the writer, under the lock, after the last sample.
                    let done = unsafe { w.sink.Finalize() };
                    eprintln!("kadr: recording: {} samples, first failure {:?}, finalize {:?}", w.written, w.failed, done.err());
                }
            }
            shared.end();
        });
    }
}

/// Writes the last frame again whenever a frame's time has gone by without
/// a new one (the screen didn't change).
fn repeater(shared: Arc<Shared>, stop: Arc<AtomicBool>, last: Arc<Mutex<Option<i64>>>, bytes: Arc<Mutex<Vec<u8>>>, frame_time: i64) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let tick = Duration::from_nanos(frame_time as u64 * 100 / 2);
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(tick);
            let mut w = shared.0.lock().unwrap();
            let Some(now) = w.time(Instant::now()) else { continue };
            let mut last = last.lock().unwrap();
            let Some(at) = *last else { continue };
            if now - at >= frame_time {
                let at = at + frame_time;
                let video = w.video;
                w.write(video, &bytes.lock().unwrap(), at, frame_time);
                *last = Some(at);
            }
        }
    })
}

/// A window by itself, whatever covers it: one frame of it through
/// Windows.Graphics.Capture (its own frame pool, free-threaded, so nothing
/// waits on a UI thread), within two seconds. Physical pixels, the DWM's
/// frame (the invisible resize border cut off).
pub(crate) fn window_shot(hwnd: isize) -> Option<image::RgbaImage> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    // SAFETY: as in `Session::start`: objects made here, each frame read
    // before it's let go.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let window = HWND(hwnd as *mut _);
        let (device, context) = device().ok()?;
        let dxgi: IDXGIDevice = device.cast().ok()?;
        let d3d: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi).and_then(|i| i.cast()).ok()?;
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().ok()?;
        let item: GraphicsCaptureItem = interop.CreateForWindow(window).ok()?;
        let size = item.Size().ok()?;
        // Where the DWM's frame sits in the captured surface.
        let (mut outer, mut frame) = (RECT::default(), RECT::default());
        let _ = GetWindowRect(window, &mut outer);
        let framed = DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &mut frame as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_ok();
        let (left, top) = if framed { ((frame.left - outer.left).max(0), (frame.top - outer.top).max(0)) } else { (0, 0) };
        let (width, height) = if framed { (frame.right - frame.left, frame.bottom - frame.top) } else { (size.Width, size.Height) };
        let (width, height) = (width.min(size.Width - left).max(1), height.min(size.Height - top).max(1));
        let staging = staging(&device, width as u32, height as u32).ok()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d, DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, size).ok()?;
        let session = pool.CreateCaptureSession(&item).ok()?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetIsBorderRequired(false);
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(Some(tx));
        pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, windows::core::IInspectable>::new(move |pool, _| {
            let Some(pool) = pool.as_ref() else { return Ok(()) };
            let frame = pool.TryGetNextFrame()?;
            let Some(tx) = tx.lock().unwrap().take() else { return Ok(()) };
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
            let texture: ID3D11Texture2D = access.GetInterface()?;
            let area = D3D11_BOX { left: left as u32, top: top as u32, front: 0, right: (left + width) as u32, bottom: (top + height) as u32, back: 1 };
            context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &texture, 0, Some(&area));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let row = width as usize * 4;
            let mut rgba = Vec::with_capacity(row * height as usize);
            for y in 0..height as usize {
                let src = std::slice::from_raw_parts((mapped.pData as *const u8).add(y * mapped.RowPitch as usize), row);
                rgba.extend(src.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]));
            }
            context.Unmap(&staging, 0);
            let _ = tx.send(rgba);
            Ok(())
        }))
        .ok()?;
        session.StartCapture().ok()?;
        let got = rx.recv_timeout(Duration::from_secs(2)).ok();
        let _ = session.Close();
        let _ = pool.Close();
        image::RgbaImage::from_raw(width as u32, height as u32, got?)
    }
}

/// A hardware Direct3D 11 device (WARP where there's no GPU) with its context.
unsafe fn device() -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    unsafe {
        let (mut device, mut context) = (None, None);
        let made = D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context));
        if made.is_err() {
            D3D11CreateDevice(None, windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))?;
        }
        Ok((device.unwrap(), context.unwrap()))
    }
}

/// A texture the CPU reads the area from.
unsafe fn staging(device: &ID3D11Device, width: u32, height: u32) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    Ok(texture.unwrap())
}

/// The sink writer with its streams: video, then the system's audio and the
/// microphone's.
unsafe fn sink(spec: &Spec, width: u32, height: u32) -> windows::core::Result<(IMFSinkWriter, u32, Option<u32>, Option<u32>)> {
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.unwrap();
        attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        let sink = MFCreateSinkWriterFromURL(&HSTRING::from(spec.out.as_os_str()), None, &attrs)?;
        let pack = |a: u32, b: u32| ((a as u64) << 32) | b as u64;
        let fps = spec.fps.max(1);
        let bpp = [0.05, 0.08, 0.12, 0.2][spec.quality.min(3)];

        let out = MFCreateMediaType()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        out.SetUINT32(&MF_MT_AVG_BITRATE, (width as f64 * height as f64 * fps as f64 * bpp) as u32)?;
        out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        out.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        out.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
        out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        let video = sink.AddStream(&out)?;
        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        input.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        input.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
        input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        // Top row first.
        input.SetUINT32(&MF_MT_DEFAULT_STRIDE, width * 4)?;
        sink.SetInputMediaType(video, &input, None)?;

        let audio = |sink: &IMFSinkWriter| -> windows::core::Result<u32> {
            let out = MFCreateMediaType()?;
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            out.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            out.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
            out.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS as u32)?;
            out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 20_000)?;
            let stream = sink.AddStream(&out)?;
            let input = MFCreateMediaType()?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            input.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, RATE)?;
            input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, CHANNELS as u32)?;
            input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, CHANNELS as u32 * 2)?;
            input.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, RATE * CHANNELS as u32 * 2)?;
            sink.SetInputMediaType(stream, &input, None)?;
            Ok(stream)
        };
        let system = if spec.system_audio { Some(audio(&sink)?) } else { None };
        let mic = if spec.mic.is_some() { Some(audio(&sink)?) } else { None };
        Ok((sink, video, system, mic))
    }
}

/// Reads the system's sound (`loopback`) or the default microphone into
/// `stream` until `stop`, filling what the device doesn't deliver with
/// silence so the track keeps the video's time.
fn audio_thread(shared: Arc<Shared>, stop: Arc<AtomicBool>, stream: u32, loopback: bool) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // SAFETY: WASAPI on this thread's own COM apartment; the packets are
        // copied before they're released.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let capture = (|| -> windows::core::Result<(IAudioClient, IAudioCaptureClient)> {
                let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let device = devices.GetDefaultAudioEndpoint(if loopback { eRender } else { eCapture }, eConsole)?;
                let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
                let format = WAVEFORMATEX {
                    wFormatTag: WAVE_FORMAT_PCM as u16,
                    nChannels: CHANNELS,
                    nSamplesPerSec: RATE,
                    nAvgBytesPerSec: RATE * CHANNELS as u32 * 2,
                    nBlockAlign: CHANNELS * 2,
                    wBitsPerSample: 16,
                    cbSize: 0,
                };
                let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY | if loopback { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
                client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, SECOND / 5, 0, &format, None)?;
                let capture: IAudioCaptureClient = client.GetService()?;
                client.Start()?;
                Ok((client, capture))
            })();
            let device = capture.ok();
            let frame = CHANNELS as usize * 2;
            // Frames written so far, by the file's clock.
            let mut written: i64 = 0;
            let mut chunk = Vec::new();
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(TICK);
                chunk.clear();
                if let Some((_, capture)) = &device {
                    while capture.GetNextPacketSize().unwrap_or(0) > 0 {
                        let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0, 0);
                        if capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None).is_err() {
                            break;
                        }
                        let len = frames as usize * frame;
                        // AUDCLNT_BUFFERFLAGS_SILENT
                        if flags & 2 != 0 || data.is_null() {
                            chunk.resize(chunk.len() + len, 0);
                        } else {
                            chunk.extend_from_slice(std::slice::from_raw_parts(data, len));
                        }
                        let _ = capture.ReleaseBuffer(frames);
                    }
                }
                let mut w = shared.0.lock().unwrap();
                let Some(now) = w.time(Instant::now()) else {
                    // Paused (or not started): what came in is left out.
                    continue;
                };
                let due = now * RATE as i64 / SECOND;
                // Behind the clock by more than a tick: silence up to it.
                let have = written + (chunk.len() / frame) as i64;
                if have < due - (RATE as i64 / 20) {
                    chunk.resize(chunk.len() + (due - have) as usize * frame, 0);
                }
                if chunk.is_empty() {
                    continue;
                }
                let frames = (chunk.len() / frame) as i64;
                w.write(stream, &chunk, written * SECOND / RATE as i64, frames * SECOND / RATE as i64);
                written += frames;
            }
            if let Some((client, _)) = device {
                let _ = client.Stop();
            }
        }
    })
}

/// The microphones there are: the system's default only, for now.
pub fn microphones() -> Vec<(String, String)> {
    Vec::new()
}
