//! The player on macOS: `AVPlayer` over the kept file, with an `AVPlayerItemVideoOutput` that
//! hands each new frame as a Core Video pixel buffer in the format GPUI's surface draws
//! (bi-planar 4:2:0, full range, Metal-compatible).

use std::path::Path;
use std::ptr::{self, NonNull};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread as _, MainThreadMarker};
use objc2_av_foundation::{AVPlayer, AVPlayerItem, AVPlayerItemVideoOutput};
use objc2_core_foundation::CFRetained;
use objc2_core_media::CMTime;
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetHeight, CVPixelBufferGetWidth,
    kCVPixelBufferMetalCompatibilityKey, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

/// The timescale of the times the player is sent to.
const TIMESCALE: i32 = 600;

/// A video playing, and its newest frame.
pub(super) struct Player {
    av: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    output: Retained<AVPlayerItemVideoOutput>,
    /// The newest frame taken, drawn again until a newer one comes.
    frame: Option<CFRetained<CVPixelBuffer>>,
}

impl Player {
    /// A player of the video kept at `path`, playing; `None` off the main thread or for a
    /// path that is no file URL.
    #[expect(
        unsafe_code,
        reason = "AVFoundation's player calls and Core Video's keys are unsafe in objc2"
    )]
    pub(super) fn open(path: &Path) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let url = NSURL::from_file_path(path)?;
        // SAFETY: Core Video's own keys (toll-free bridged to NSString), read once.
        let (format_key, metal_key): (&NSString, &NSString) = unsafe {
            (
                kCVPixelBufferPixelFormatTypeKey.as_ref(),
                kCVPixelBufferMetalCompatibilityKey.as_ref(),
            )
        };
        let format = NSNumber::new_u32(kCVPixelFormatType_420YpCbCr8BiPlanarFullRange);
        let metal = NSNumber::new_bool(true);
        let values: [&AnyObject; 2] = [&format, &metal];
        let attributes = NSDictionary::from_slices(&[format_key, metal_key], &values);
        // SAFETY: an item of a file URL, an output asking for pixel buffers of a format and
        // Metal compatibility (numbers, as Core Video wants), added to the item, and a player
        // of the item, playing.
        let (player, item, output) = unsafe {
            let item = AVPlayerItem::playerItemWithURL(&url, mtm);
            let output = AVPlayerItemVideoOutput::initWithPixelBufferAttributes(
                AVPlayerItemVideoOutput::alloc(),
                Some(&*attributes),
            );
            item.addOutput(&output);
            let player = AVPlayer::playerWithPlayerItem(Some(&*item), mtm);
            player.play();
            (player, item, output)
        };
        Some(Self {
            av: player,
            item,
            output,
            frame: None,
        })
    }

    /// The newest frame, taken from the output when it has a new one; with its size in pixels.
    #[expect(
        unsafe_code,
        reason = "the output's calls are unsafe in objc2, and its buffer changes owner type"
    )]
    pub(super) fn frame(&mut self) -> Option<(CFRetained<CVPixelBuffer>, (usize, usize))> {
        // SAFETY: the player's time (a plain value), asked of its own output; a null pointer for
        // the display time, which isn't wanted.
        let fresh = unsafe {
            let now = self.av.currentTime();
            if self.output.hasNewPixelBufferForItemTime(now) {
                self.output
                    .copyPixelBufferForItemTime_itemTimeForDisplay(now, ptr::null_mut())
            } else {
                None
            }
        };
        if let Some(fresh) = fresh {
            let raw = NonNull::new(Retained::into_raw(fresh));
            // SAFETY: the copied buffer is owned (+1), handed from objc2's owner to Core
            // Foundation's, which releases it the same way.
            self.frame = raw.map(|raw| unsafe { CFRetained::from_raw(raw) });
        }
        let frame = self.frame.clone()?;
        let size = (
            CVPixelBufferGetWidth(&frame),
            CVPixelBufferGetHeight(&frame),
        );
        Some((frame, size))
    }

    /// Whether it is playing (not paused, not at its end).
    #[expect(unsafe_code, reason = "AVPlayer's rate is unsafe in objc2")]
    pub(super) fn is_playing(&self) -> bool {
        // SAFETY: a plain value of the live player.
        unsafe { self.av.rate().abs() > f32::EPSILON }
    }

    /// How far in it is, in seconds.
    #[expect(unsafe_code, reason = "AVPlayer's time is unsafe in objc2")]
    pub(super) fn position(&self) -> f64 {
        // SAFETY: a plain value of the live player.
        let seconds = unsafe { self.av.currentTime().seconds() };
        if seconds.is_finite() { seconds } else { 0.0 }
    }

    /// How long it is, in seconds, once known.
    #[expect(unsafe_code, reason = "AVPlayerItem's duration is unsafe in objc2")]
    pub(super) fn duration(&self) -> Option<f64> {
        // SAFETY: a plain value of the live item (indefinite until it is loaded).
        let seconds = unsafe { self.item.duration().seconds() };
        (seconds.is_finite() && seconds > 0.0).then_some(seconds)
    }

    /// Plays, from the start again when it is at its end.
    #[expect(unsafe_code, reason = "AVPlayer's calls are unsafe in objc2")]
    pub(super) fn play(&self) {
        if self
            .duration()
            .is_some_and(|duration| self.position() >= duration - 0.05)
        {
            self.seek(0.0);
        }
        // SAFETY: the live player.
        unsafe { self.av.play() }
    }

    /// Pauses.
    #[expect(unsafe_code, reason = "AVPlayer's calls are unsafe in objc2")]
    pub(super) fn pause(&self) {
        // SAFETY: the live player.
        unsafe { self.av.pause() }
    }

    /// Goes to `seconds` in.
    #[expect(
        unsafe_code,
        reason = "AVPlayer's calls and Core Media's times are unsafe in objc2"
    )]
    pub(super) fn seek(&self, seconds: f64) {
        // SAFETY: a time made from seconds, sent to the live player.
        unsafe {
            self.av
                .seekToTime(CMTime::with_seconds(seconds.max(0.0), TIMESCALE));
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.pause();
    }
}
