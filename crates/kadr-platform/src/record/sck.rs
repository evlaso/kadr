//! Recording on macOS: ScreenCaptureKit hands the display's frames (and
//! what the computer plays, and from macOS 15 the microphone) to an
//! AVAssetWriter, which encodes H.264 on the media engine and AAC into an
//! MP4. A pause leaves samples out and shifts the later ones back by the
//! time it took, so the file has no gap.

use std::ptr::NonNull;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterStatus, AVFileTypeMPEG4, AVMediaTypeAudio, AVMediaTypeVideo, AVVideoAverageBitRateKey, AVVideoCodecKey,
    AVVideoCodecTypeH264, AVVideoCompressionPropertiesKey, AVVideoExpectedSourceFrameRateKey, AVVideoHeightKey, AVVideoMaxKeyFrameIntervalKey, AVVideoWidthKey,
};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_media::{CMSampleBuffer, CMSampleTimingInfo, CMTime};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_screen_capture_kit::{SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput, SCStreamOutputType};

use super::Spec;

/// How long starting, the first frame and finishing may take.
const WAIT: Duration = Duration::from_secs(5);
/// 2-plane "video" range YCbCr 4:2:0: what the H.264 encoder takes as is.
const YUV_420V: u32 = u32::from_be_bytes(*b"420v");

/// The writer and what the stream's queue and the app share of it.
struct Writer {
    writer: Retained<AVAssetWriter>,
    video: Retained<AVAssetWriterInput>,
    system: Option<Retained<AVAssetWriterInput>>,
    mic: Option<Retained<AVAssetWriterInput>>,
    /// Told once the first frame started the file.
    first: Option<mpsc::Sender<()>>,
    started: bool,
    finished: bool,
    paused: bool,
    /// The last sample's time, where a pause starts; set back on resuming.
    last: Option<CMTime>,
    paused_at: Option<CMTime>,
    /// Paused time so far, taken off every later sample.
    offset: Option<CMTime>,
    ended: Option<Box<dyn FnOnce() + Send>>,
}

/// SAFETY: AVFoundation's writer and inputs may be used from any thread
/// one at a time, which the mutex makes so.
struct Shared(Mutex<Writer>);
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

impl Shared {
    fn take(&self, buffer: &CMSampleBuffer, kind: SCStreamOutputType) {
        let mut w = self.0.lock().unwrap();
        // SAFETY: a live sample buffer from the stream, read in its handler.
        unsafe {
            if w.finished || !buffer.is_valid() || !buffer.data_is_ready() {
                return;
            }
            // Frames with nothing new (the screen didn't change) have no picture.
            if kind == SCStreamOutputType::Screen && buffer.image_buffer().is_none() {
                return;
            }
            let pts = buffer.presentation_time_stamp();
            if !w.started {
                if kind != SCStreamOutputType::Screen || !w.writer.startWriting() {
                    return;
                }
                w.writer.startSessionAtSourceTime(pts);
                w.started = true;
                if let Some(first) = w.first.take() {
                    let _ = first.send(());
                }
            }
            if w.paused {
                return;
            }
            // The first sample after a pause: the pause's length goes off
            // this and every later one.
            if let Some(at) = w.paused_at.take() {
                let gap = pts.subtract(at);
                w.offset = Some(w.offset.map_or(gap, |o| o.add(gap)));
            }
            w.last = Some(pts);
            let input = match kind {
                SCStreamOutputType::Screen => Some(&w.video),
                SCStreamOutputType::Audio => w.system.as_ref(),
                SCStreamOutputType::Microphone => w.mic.as_ref(),
                _ => None,
            };
            let Some(input) = input else { return };
            if !input.isReadyForMoreMediaData() {
                return;
            }
            match w.offset {
                Some(by) => {
                    if let Some(shifted) = shifted(buffer, by) {
                        input.appendSampleBuffer(&shifted);
                    }
                }
                None => {
                    input.appendSampleBuffer(buffer);
                }
            }
        }
    }

    fn end(&self) {
        if let Some(ended) = self.0.lock().unwrap().ended.take() {
            ended();
        }
    }
}

/// `buffer` with its times moved `by` earlier.
unsafe fn shifted(buffer: &CMSampleBuffer, by: CMTime) -> Option<CFRetained<CMSampleBuffer>> {
    unsafe {
        let mut count = 0;
        buffer.sample_timing_info_array(0, std::ptr::null_mut(), &mut count);
        let mut timing = vec![CMSampleTimingInfo { duration: CMTime::new(0, 1), presentationTimeStamp: CMTime::new(0, 1), decodeTimeStamp: CMTime::new(0, 1) }; count as usize];
        buffer.sample_timing_info_array(count, timing.as_mut_ptr(), &mut count);
        for t in &mut timing {
            t.presentationTimeStamp = t.presentationTimeStamp.subtract(by);
            if t.decodeTimeStamp.flags.contains(objc2_core_media::CMTimeFlags::Valid) {
                t.decodeTimeStamp = t.decodeTimeStamp.subtract(by);
            }
        }
        let mut out: *mut CMSampleBuffer = std::ptr::null_mut();
        CMSampleBuffer::create_copy_with_new_timing(None, buffer, count, timing.as_ptr(), NonNull::from(&mut out));
        NonNull::new(out).map(|p| CFRetained::from_raw(p))
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; Output doesn't
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "KadrRecordOutput"]
    #[ivars = Arc<Shared>]
    struct Output;

    unsafe impl NSObjectProtocol for Output {}

    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &SCStream, buffer: &CMSampleBuffer, kind: SCStreamOutputType) {
            self.ivars().take(buffer, kind);
        }
    }
);

impl Output {
    fn new(shared: Arc<Shared>) -> Retained<Output> {
        let this = Output::alloc().set_ivars(shared);
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

pub struct Session {
    stream: Retained<SCStream>,
    shared: Arc<Shared>,
    /// Kept alive while the stream calls into it.
    _output: Retained<Output>,
    _queue: DispatchRetained<DispatchQueue>,
}

/// SAFETY: SCStream's start, stop and the writer behind the mutex may be
/// used from any thread.
unsafe impl Send for Session {}

impl Session {
    pub fn start(spec: &Spec, ended: Box<dyn FnOnce() + Send>) -> Result<Session, String> {
        let log = |e: &str| {
            let _ = std::fs::write(&spec.log, format!("{e}\n"));
            e.to_owned()
        };
        // SAFETY: ScreenCaptureKit and AVFoundation calls on live objects,
        // their handlers retained by the calls.
        unsafe {
            let display = display(spec).ok_or_else(|| log("ScreenCaptureKit: no such display, or no permission to record the screen"))?;
            let filter = SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &NSArray::new());
            let m = &spec.monitor;
            let (points, rect) = match spec.area {
                Some((x, y, w, h)) => ((w, h), Some(CGRect::new(CGPoint::new((x - m.x) as f64, (y - m.y) as f64), CGSize::new(w as f64, h as f64)))),
                None => ((m.w, m.h), None),
            };
            let even = |n: f64| ((n.round() as usize) / 2 * 2).max(2);
            let size = (even(points.0 as f64 * m.scale), even(points.1 as f64 * m.scale));
            let config = SCStreamConfiguration::new();
            config.setWidth(size.0);
            config.setHeight(size.1);
            if let Some(rect) = rect {
                config.setSourceRect(rect);
            }
            config.setMinimumFrameInterval(CMTime::new(1, spec.fps.max(1) as i32));
            config.setShowsCursor(spec.cursor);
            config.setPixelFormat(YUV_420V);
            config.setQueueDepth(6);
            config.setCapturesAudio(spec.system_audio);
            config.setSampleRate(48_000);
            config.setChannelCount(2);
            config.setExcludesCurrentProcessAudio(true);
            // The microphone through the stream: macOS 15 and later.
            let mic = spec.mic.is_some() && msg_send![&config, respondsToSelector: sel!(setCaptureMicrophone:)];
            if mic {
                config.setCaptureMicrophone(true);
                if let Some(id) = spec.mic.as_deref().filter(|id| !id.is_empty()) {
                    config.setMicrophoneCaptureDeviceID(Some(&NSString::from_str(id)));
                }
            }

            let _ = std::fs::remove_file(&spec.out);
            let url = NSURL::from_file_path(&spec.out).ok_or_else(|| log("bad output path"))?;
            let writer = AVAssetWriter::initWithURL_fileType_error(AVAssetWriter::alloc(), &url, AVFileTypeMPEG4.unwrap()).map_err(|e| log(&e.localizedDescription().to_string()))?;
            let video = AVAssetWriterInput::initWithMediaType_outputSettings(AVAssetWriterInput::alloc(), AVMediaTypeVideo.unwrap(), Some(&video_settings(size, spec.fps, spec.quality)));
            video.setExpectsMediaDataInRealTime(true);
            writer.addInput(&video);
            let audio_input = || {
                let input = AVAssetWriterInput::initWithMediaType_outputSettings(AVAssetWriterInput::alloc(), AVMediaTypeAudio.unwrap(), Some(&audio_settings()));
                input.setExpectsMediaDataInRealTime(true);
                writer.addInput(&input);
                input
            };
            // The system's track first, then the microphone's.
            let system = spec.system_audio.then(audio_input);
            let mic_input = mic.then(audio_input);

            let (first_tx, first_rx) = mpsc::channel();
            let shared = Arc::new(Shared(Mutex::new(Writer {
                writer,
                video,
                system,
                mic: mic_input,
                first: Some(first_tx),
                started: false,
                finished: false,
                paused: false,
                last: None,
                paused_at: None,
                offset: None,
                ended: Some(ended),
            })));
            let output = Output::new(shared.clone());
            let queue = DispatchQueue::new("kadr.record", DispatchQueueAttr::SERIAL);
            let stream = SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None);
            let as_output = ProtocolObject::from_ref(&*output);
            let kinds = [Some(SCStreamOutputType::Screen), spec.system_audio.then_some(SCStreamOutputType::Audio), mic.then_some(SCStreamOutputType::Microphone)];
            for kind in kinds.into_iter().flatten() {
                stream.addStreamOutput_type_sampleHandlerQueue_error(as_output, kind, Some(&queue)).map_err(|e| log(&e.localizedDescription().to_string()))?;
            }
            let (tx, rx) = mpsc::channel();
            let handler = RcBlock::new(move |error: *mut NSError| {
                let _ = tx.send(error.as_ref().map(|e| e.localizedDescription().to_string()));
            });
            stream.startCaptureWithCompletionHandler(Some(&handler));
            match rx.recv_timeout(WAIT) {
                Ok(None) => {}
                Ok(Some(e)) => return Err(log(&format!("ScreenCaptureKit: {e}"))),
                Err(_) => return Err(log("ScreenCaptureKit didn't start")),
            }
            let session = Session { stream, shared, _output: output, _queue: queue };
            if first_rx.recv_timeout(WAIT).is_err() {
                session.shared.0.lock().unwrap().ended = None;
                session.stream.stopCaptureWithCompletionHandler(None);
                let _ = std::fs::remove_file(&spec.out);
                return Err(log("ScreenCaptureKit sent no frames"));
            }
            Ok(session)
        }
    }

    pub fn set_paused(&self, paused: bool) {
        let mut w = self.shared.0.lock().unwrap();
        if w.paused == paused {
            return;
        }
        w.paused = paused;
        // Resuming: the next sample measures the pause from the last one.
        if !paused {
            w.paused_at = w.last;
        }
    }

    pub fn stop(&self) {
        let shared = self.shared.clone();
        let handler = RcBlock::new(move |_: *mut NSError| finish(&shared));
        // SAFETY: stopping a running stream; the handler is retained.
        unsafe { self.stream.stopCaptureWithCompletionHandler(Some(&handler)) };
        // Whatever happens, the app hears of the end.
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            std::thread::sleep(WAIT * 2);
            shared.end();
        });
    }
}

/// The stream has stopped: close the inputs and write the file out.
fn finish(shared: &Arc<Shared>) {
    let mut w = shared.0.lock().unwrap();
    if std::mem::replace(&mut w.finished, true) {
        return;
    }
    // SAFETY: the writer and its inputs, under the lock.
    unsafe {
        if !w.started || w.writer.status() != AVAssetWriterStatus::Writing {
            drop(w);
            return shared.end();
        }
        w.video.markAsFinished();
        for input in [&w.system, &w.mic].into_iter().flatten() {
            input.markAsFinished();
        }
        let done = shared.clone();
        let handler = RcBlock::new(move || done.end());
        w.writer.finishWritingWithCompletionHandler(&handler);
    }
}

/// The display the spec's monitor is: the one at its top left.
unsafe fn display(spec: &Spec) -> Option<Retained<objc2_screen_capture_kit::SCDisplay>> {
    unsafe {
        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |content: *mut SCShareableContent, _: *mut NSError| {
            let _ = tx.send(Retained::retain(content));
        });
        SCShareableContent::getShareableContentWithCompletionHandler(&handler);
        let content = rx.recv_timeout(WAIT).ok().flatten()?;
        let m = &spec.monitor;
        content.displays().iter().find(|d| {
            let f = d.frame();
            (f.origin.x - m.x as f64).abs() < 1. && (f.origin.y - m.y as f64).abs() < 1.
        })
    }
}

/// H.264 at `size`, its bit rate from the quality: bits per pixel per frame.
fn video_settings(size: (usize, usize), fps: u32, quality: usize) -> Retained<NSDictionary<NSString, AnyObject>> {
    let bpp = [0.05, 0.08, 0.12, 0.2][quality.min(3)];
    let rate = (size.0 * size.1) as f64 * fps.max(1) as f64 * bpp;
    // SAFETY: AVFoundation's constant keys.
    unsafe {
        let props = NSDictionary::<NSString, AnyObject>::from_slices(
            &[AVVideoAverageBitRateKey.unwrap(), AVVideoExpectedSourceFrameRateKey.unwrap(), AVVideoMaxKeyFrameIntervalKey.unwrap()],
            &[NSNumber::new_f64(rate).as_ref(), NSNumber::new_u32(fps).as_ref(), NSNumber::new_u32(fps * 2).as_ref()],
        );
        NSDictionary::from_slices(
            &[AVVideoCodecKey.unwrap(), AVVideoWidthKey.unwrap(), AVVideoHeightKey.unwrap(), AVVideoCompressionPropertiesKey.unwrap()],
            &[AVVideoCodecTypeH264.unwrap().as_ref(), NSNumber::new_usize(size.0).as_ref(), NSNumber::new_usize(size.1).as_ref(), props.as_ref()],
        )
    }
}

/// AAC, 48 kHz stereo. The keys are AVFAudio's, whose values are their names.
fn audio_settings() -> Retained<NSDictionary<NSString, AnyObject>> {
    // kAudioFormatMPEG4AAC.
    let aac = u32::from_be_bytes(*b"aac ");
    let keys = ["AVFormatIDKey", "AVSampleRateKey", "AVNumberOfChannelsKey", "AVEncoderBitRateKey"].map(NSString::from_str);
    let values = [NSNumber::new_u32(aac), NSNumber::new_f64(48_000.), NSNumber::new_u32(2), NSNumber::new_u32(160_000)];
    NSDictionary::from_slices(&keys.each_ref().map(|k| &**k), &values.each_ref().map(|v| v.as_ref()))
}

/// The microphones there are: the system's default only, for now.
pub fn microphones() -> Vec<(String, String)> {
    Vec::new()
}
