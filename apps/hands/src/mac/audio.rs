//! Recording through `ScreenCaptureKit`: the Mac's sound (what a call plays, even into
//! headphones) and the owner's microphone as two streams, each written to its own AAC file by
//! `AVAssetWriter` and handed to the live transcript as it comes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAuthorizationStatus, AVCaptureDevice, AVFileTypeAppleM4A,
    AVMediaTypeAudio,
};
use objc2_core_media::{
    CMAudioFormatDescriptionGetStreamBasicDescription, CMClock, CMSampleBuffer, CMTime,
};
use objc2_foundation::{NSDictionary, NSError, NSNumber, NSObjectProtocol, NSString, NSURL};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate,
    SCStreamOutput, SCStreamOutputType,
};

use super::desktop::screen_recording;
use crate::fail::Fail;
use crate::record::live::first_channel;
use crate::record::send::Track;
use crate::record::{AudioSource, Capture, LiveSink, Lost, Mic};

/// How long starting the capture may take.
const STARTING: Duration = Duration::from_secs(15);
/// How long stopping it, and closing each file, may take.
const STOPPING: Duration = Duration::from_secs(10);
/// AAC (`kAudioFormatMPEG4AAC`, `'aac '`).
const AAC: u32 = 0x6161_6320;
/// The files' bit rate.
const BIT_RATE: u32 = 64_000;
/// `kAudioFormatFlagIsFloat`.
const FLOAT: u32 = 1;
/// `kAudioFormatFlagIsNonInterleaved`.
const NON_INTERLEAVED: u32 = 1 << 5;

/// The Mac's sound and microphone.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScreenAudio;

/// One track's file.
struct Writer {
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
}

/// What the output object keeps.
struct Output {
    dir: PathBuf,
    live: LiveSink,
    /// Told (on a thread of its own) when the Mac stops the capture.
    lost: Lost,
    /// The host clock's time when the capture started: both tracks are timed from it.
    origin: CMTime,
    writers: RefCell<HashMap<Track, Writer>>,
}

define_class!(
    /// Receives the stream's buffers (on its own serial queue) and its stop.
    #[unsafe(super(NSObject))]
    #[name = "EmberhutHandsAudioOutput"]
    #[ivars = Output]
    struct AudioOutput;

    unsafe impl NSObjectProtocol for AudioOutput {}

    unsafe impl SCStreamOutput for AudioOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        #[expect(non_snake_case, reason = "named for its selector, as the protocol is")]
        fn stream_didOutputSampleBuffer_ofType(
            &self,
            _stream: &SCStream,
            buffer: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            let track = if kind == SCStreamOutputType::Audio {
                Track::Sys
            } else if kind == SCStreamOutputType::Microphone {
                Track::Mic
            } else {
                return;
            };
            self.ivars().take(track, buffer);
        }
    }

    unsafe impl SCStreamDelegate for AudioOutput {
        #[unsafe(method(stream:didStopWithError:))]
        #[expect(non_snake_case, reason = "named for its selector, as the protocol is")]
        fn stream_didStopWithError(&self, _stream: &SCStream, error: &NSError) {
            tracing::warn!(
                error = %error.localizedDescription(),
                "macOS stopped the recording's capture"
            );
            // Off the capture's queue: stopping the recording closes the files on it.
            let lost = std::sync::Arc::clone(&self.ivars().lost);
            std::thread::spawn(move || lost());
        }
    }
);

impl AudioOutput {
    fn new(output: Output) -> Retained<Self> {
        let this = Self::alloc().set_ivars(output);
        // SAFETY: NSObject's init.
        #[expect(unsafe_code, reason = "msg_send init on a new object")]
        unsafe {
            msg_send![super(this), init]
        }
    }
}

impl Output {
    /// Writes one buffer of `track` to its file (made with the first buffer) and hands its
    /// samples to the live transcript.
    #[expect(
        unsafe_code,
        reason = "Core Media and AVFoundation calls are unsafe in objc2"
    )]
    fn take(&self, track: Track, buffer: &CMSampleBuffer) {
        // SAFETY: `buffer` is valid for this call.
        let (valid, count, time) = unsafe {
            (
                buffer.is_valid(),
                buffer.num_samples(),
                buffer.presentation_time_stamp(),
            )
        };
        let Ok(frames) = usize::try_from(count) else {
            return;
        };
        if !valid || frames == 0 {
            return;
        }
        // SAFETY: as above.
        let Some(format) = (unsafe { buffer.format_description() }) else {
            return;
        };
        // SAFETY: an audio buffer's format description; the pointer lives as long as it.
        let Some(basic) =
            (unsafe { CMAudioFormatDescriptionGetStreamBasicDescription(&format).as_ref() })
        else {
            return;
        };
        let mut writers = self.writers.borrow_mut();
        if let std::collections::hash_map::Entry::Vacant(entry) = writers.entry(track) {
            match self.writer(track, basic.mSampleRate, basic.mChannelsPerFrame, time) {
                Some(writer) => {
                    entry.insert(writer);
                }
                None => return,
            }
        }
        if let Some(writer) = writers.get(&track)
            // SAFETY: a live input and buffer.
            && unsafe { writer.input.isReadyForMoreMediaData() }
            && !unsafe { writer.input.appendSampleBuffer(buffer) }
        {
            tracing::debug!(track = track.name(), "a buffer was not written");
        }
        drop(writers);
        let float = basic.mFormatFlags & FLOAT != 0 && basic.mBitsPerChannel == 32;
        if !float {
            return;
        }
        // SAFETY: as above.
        let Some(data) = (unsafe { buffer.data_buffer() }) else {
            return;
        };
        // SAFETY: a live block buffer.
        let length = unsafe { data.data_length() };
        let mut bytes = vec![0_u8; length];
        let Some(into) = std::ptr::NonNull::new(bytes.as_mut_ptr().cast()) else {
            return;
        };
        // SAFETY: `bytes` holds `length` bytes.
        if unsafe { data.copy_data_bytes(0, length, into) } != 0 {
            return;
        }
        let channels = usize::try_from(basic.mChannelsPerFrame).unwrap_or(1);
        let interleaved = basic.mFormatFlags & NON_INTERLEAVED == 0;
        let samples = first_channel(&bytes, frames, channels, interleaved);
        // SAFETY: two valid times.
        let at = unsafe { time.seconds() - self.origin.seconds() };
        (self.live)(track, at.max(0.0), basic.mSampleRate, &samples);
    }

    /// A new AAC file for `track`, its session starting at the capture's start (or the first
    /// buffer's time, when it comes from another clock).
    #[expect(unsafe_code, reason = "AVAssetWriter's calls are unsafe in objc2")]
    fn writer(&self, track: Track, rate: f64, channels: u32, first: CMTime) -> Option<Writer> {
        let path = self.dir.join(format!("{}.m4a", track.name()));
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // SAFETY: the framework's own constants.
        let (m4a, audio) = unsafe { (AVFileTypeAppleM4A?, AVMediaTypeAudio?) };
        // SAFETY: a file URL and a file type.
        let writer = unsafe { AVAssetWriter::assetWriterWithURL_fileType_error(&url, m4a) }
            .map_err(|error| {
                tracing::warn!(error = %error.localizedDescription(), "a track's file was not made");
            })
            .ok()?;
        let keys = [
            NSString::from_str("AVFormatIDKey"),
            NSString::from_str("AVSampleRateKey"),
            NSString::from_str("AVNumberOfChannelsKey"),
            NSString::from_str("AVEncoderBitRateKey"),
        ];
        let values = [
            NSNumber::new_u32(AAC),
            NSNumber::new_f64(rate),
            NSNumber::new_u32(channels.clamp(1, 2)),
            NSNumber::new_u32(BIT_RATE),
        ];
        let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
        let value_refs: Vec<&AnyObject> = values.iter().map(AsRef::as_ref).collect();
        let settings: Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_slices(&key_refs, &value_refs);
        // SAFETY: the audio media type and AAC settings.
        let input = unsafe {
            AVAssetWriterInput::initWithMediaType_outputSettings(
                AVAssetWriterInput::alloc(),
                audio,
                Some(&settings),
            )
        };
        // SAFETY: a new writer and input.
        unsafe {
            input.setExpectsMediaDataInRealTime(true);
            if !writer.canAddInput(&input) {
                return None;
            }
            writer.addInput(&input);
            if !writer.startWriting() {
                return None;
            }
            let offset = first.seconds() - self.origin.seconds();
            let start = if (-1.0..600.0).contains(&offset) {
                self.origin
            } else {
                first
            };
            writer.startSessionAtSourceTime(start);
        }
        tracing::info!(track = track.name(), rate, channels, "recording a track");
        Some(Writer { writer, input })
    }

    /// Closes every file.
    #[expect(unsafe_code, reason = "AVAssetWriter's calls are unsafe in objc2")]
    fn close(&self) {
        for (track, writer) in self.writers.borrow_mut().drain() {
            let (done, closed) = mpsc::channel::<()>();
            let handler = RcBlock::new(move || {
                let _sent = done.send(());
            });
            // SAFETY: a live writer and input; the handler only signals.
            unsafe {
                writer.input.markAsFinished();
                writer.writer.finishWritingWithCompletionHandler(&handler);
            }
            if closed.recv_timeout(STOPPING).is_err() {
                tracing::warn!(track = track.name(), "a track's file did not close in time");
            }
        }
    }
}

/// The capture running.
struct ScreenCapture {
    stream: Retained<SCStream>,
    output: Retained<AudioOutput>,
    queue: DispatchRetained<DispatchQueue>,
}

// SAFETY: the stream and the output are only touched through their serial queue (the buffers)
// and the stop below, which waits for that queue.
#[expect(
    unsafe_code,
    reason = "the capture is stopped from the request worker's thread"
)]
unsafe impl Send for ScreenCapture {}

/// The output, handed to its own queue to close its files there.
struct OnQueue(Retained<AudioOutput>);

// SAFETY: the output is used on its serial queue only, where `exec_sync` runs this.
#[expect(unsafe_code, reason = "the output is closed on its own queue")]
unsafe impl Send for OnQueue {}

impl OnQueue {
    fn close(&self) {
        self.0.ivars().close();
    }
}

impl Capture for ScreenCapture {
    #[expect(unsafe_code, reason = "SCStream's calls are unsafe in objc2")]
    fn finish(self: Box<Self>) {
        let (done, stopped) = mpsc::channel::<()>();
        let handler = RcBlock::new(move |_error: *mut NSError| {
            let _sent = done.send(());
        });
        // SAFETY: a running stream; the handler only signals.
        unsafe { self.stream.stopCaptureWithCompletionHandler(Some(&handler)) };
        if stopped.recv_timeout(STOPPING).is_err() {
            tracing::warn!("the recording's capture did not stop in time");
        }
        let output = OnQueue(self.output.clone());
        self.queue.exec_sync(move || output.close());
    }
}

impl AudioSource for ScreenAudio {
    fn permitted(&self) -> bool {
        screen_recording()
    }

    #[expect(unsafe_code, reason = "AVCaptureDevice's calls are unsafe in objc2")]
    fn microphone(&self) -> Mic {
        // SAFETY: the framework's own constant.
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            return Mic::Refused;
        };
        // SAFETY: the audio media type.
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) };
        if status == AVAuthorizationStatus::Authorized {
            Mic::Allowed
        } else if status == AVAuthorizationStatus::NotDetermined {
            // The Mac asks the owner once, now; this recording goes on without it.
            let handler = RcBlock::new(|allowed: Bool| {
                tracing::info!(allowed = allowed.as_bool(), "the microphone was answered");
            });
            // SAFETY: the audio media type and a handler that only logs.
            unsafe {
                AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler);
            };
            Mic::Asking
        } else {
            Mic::Refused
        }
    }

    #[expect(unsafe_code, reason = "ScreenCaptureKit's calls are unsafe in objc2")]
    fn start(
        &self,
        dir: &Path,
        mic: bool,
        live: LiveSink,
        lost: Lost,
    ) -> Result<Box<dyn Capture>, Fail> {
        let display = first_display()?;
        // SAFETY: a new configuration, set before the stream takes it.
        let configuration = unsafe {
            let c = SCStreamConfiguration::new();
            c.setCapturesAudio(true);
            c.setExcludesCurrentProcessAudio(true);
            c.setSampleRate(48_000);
            c.setChannelCount(1);
            c.setCaptureMicrophone(mic);
            // No picture is wanted: the smallest frames, once a second.
            c.setWidth(2);
            c.setHeight(2);
            c.setMinimumFrameInterval(CMTime::new(1, 1));
            c.setShowsCursor(false);
            c
        };
        // SAFETY: a display and no excluded windows.
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &display,
                &objc2_foundation::NSArray::new(),
            )
        };
        // SAFETY: the host clock's time now.
        let origin = unsafe { CMClock::host_time_clock().time() };
        let output = AudioOutput::new(Output {
            dir: dir.to_path_buf(),
            live,
            lost,
            origin,
            writers: RefCell::new(HashMap::new()),
        });
        let queue = DispatchQueue::new("com.emberhut.hands.record", None);
        let delegate = ProtocolObject::from_ref(&*output);
        // SAFETY: a filter, a configuration and a delegate kept alive with the stream.
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &configuration,
                Some(delegate),
            )
        };
        let as_output = ProtocolObject::from_ref(&*output);
        let mut kinds = vec![SCStreamOutputType::Screen, SCStreamOutputType::Audio];
        if mic {
            kinds.push(SCStreamOutputType::Microphone);
        }
        for kind in kinds {
            // SAFETY: the output is kept alive with the stream; the queue is serial.
            unsafe {
                stream.addStreamOutput_type_sampleHandlerQueue_error(as_output, kind, Some(&queue))
            }
            .map_err(|error| {
                Fail::broke(format!(
                    "Couldn't start recording: {}",
                    error.localizedDescription()
                ))
            })?;
        }
        let (done, started) = mpsc::channel::<Option<String>>();
        let handler = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: null or a valid error for the block's call.
            let error = unsafe { error.as_ref() };
            let _sent = done.send(error.map(|e| e.localizedDescription().to_string()));
        });
        // SAFETY: a configured stream; the handler only signals.
        unsafe { stream.startCaptureWithCompletionHandler(Some(&handler)) };
        match started.recv_timeout(STARTING) {
            Ok(None) => Ok(Box::new(ScreenCapture {
                stream,
                output,
                queue,
            })),
            Ok(Some(error)) => Err(Fail::broke(format!("Couldn't start recording: {error}"))),
            Err(_) => {
                // It may still start later: stop it, so nothing records unseen.
                // SAFETY: a stream whose start was asked for.
                unsafe { stream.stopCaptureWithCompletionHandler(None) };
                Err(Fail::broke("Recording didn't start in 15 s"))
            }
        }
    }
}

/// The first display, for the Mac's sound (`ScreenCaptureKit` captures audio with a display).
#[expect(unsafe_code, reason = "SCShareableContent's calls are unsafe in objc2")]
fn first_display() -> Result<Retained<objc2_screen_capture_kit::SCDisplay>, Fail> {
    let (done, got) = mpsc::channel::<Option<Retained<SCShareableContent>>>();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, _error: *mut NSError| {
            // SAFETY: null or a valid content object for the block's call; retained to keep it.
            let content = unsafe { Retained::retain(content) };
            let _sent = done.send(content);
        },
    );
    // SAFETY: a completion handler called once.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            false, true, &handler,
        );
    }
    let content = got
        .recv_timeout(STARTING)
        .ok()
        .flatten()
        .ok_or_else(|| Fail::broke("No display to capture the Mac's sound from"))?;
    // SAFETY: a valid content object.
    unsafe { content.displays() }
        .firstObject()
        .ok_or_else(|| Fail::broke("No display to capture the Mac's sound from"))
}
