//! The still frames on macOS, from `AVFoundation`, as the reference client takes them: the
//! asset's length, then `AVAssetImageGenerator` with the track's orientation applied and frames
//! at most [`LONGEST_SIDE`] pixels on their longest side, each made a JPEG by
//! `NSBitmapImageRep`.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::AllocAnyThread as _;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImageCompressionFactor};
use objc2_av_foundation::{AVAsset, AVAssetImageGenerator};
use objc2_core_graphics::CGImage;
use objc2_core_media::CMTime;
use objc2_foundation::{NSDictionary, NSError, NSNumber, NSSize, NSURL};

use super::{LONGEST_SIDE, frame_times};

/// How long one frame may take before it is given up on.
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);

/// The timescale of the times frames are asked at (the reference client's).
const TIMESCALE: i32 = 600;

/// A frame's JPEG quality, 0 to 1 (the reference client's).
const QUALITY: f64 = 0.8;

/// The still frames of the video at `path`, as JPEG files; none when `AVFoundation` can't read
/// it. It waits for each frame: call it off the UI thread.
#[expect(
    unsafe_code,
    reason = "AVFoundation's asset and generator calls are unsafe in objc2"
)]
pub(super) fn grab(path: &Path) -> Vec<Vec<u8>> {
    let Some(url) = NSURL::from_file_path(path) else {
        return Vec::new();
    };
    let side = f64::from(LONGEST_SIDE);
    // SAFETY: an asset of a file URL, its length (a plain value), and a generator over it,
    // set before any frame is asked for.
    let (duration, generator) = unsafe {
        let asset = AVAsset::assetWithURL(&url);
        let duration = asset.duration().seconds();
        let generator = AVAssetImageGenerator::assetImageGeneratorWithAsset(&asset);
        generator.setAppliesPreferredTrackTransform(true);
        generator.setMaximumSize(NSSize::new(side, side));
        (duration, generator)
    };
    let frames: Vec<Vec<u8>> = frame_times(duration)
        .into_iter()
        .filter_map(|at| frame(&generator, at))
        .collect();
    if frames.is_empty() {
        log::info!("AVFoundation made no still frames of {}", path.display());
    }
    frames
}

/// The frame `at` seconds in, as a JPEG, once the generator made it.
#[expect(
    unsafe_code,
    reason = "the generator's call and the frame it hands back are raw in objc2"
)]
fn frame(generator: &AVAssetImageGenerator, at: f64) -> Option<Vec<u8>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let handler = RcBlock::new(move |image: *mut CGImage, _: CMTime, error: *mut NSError| {
        // SAFETY: AVFoundation hands the frame (alive for this call) or null with an error.
        let made = if let Some(image) = unsafe { image.as_ref() } {
            jpeg(image)
        } else {
            // SAFETY: the error it hands with no frame, or null.
            let why =
                unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
            log::info!("no still frame: {}", why.unwrap_or_default());
            None
        };
        if sender.try_send(made).is_err() {
            log::debug!("a still frame came after it was given up on");
        }
    });
    // SAFETY: a time made from seconds, and a handler that only sends (it is Send).
    unsafe {
        generator.generateCGImageAsynchronouslyForTime_completionHandler(
            CMTime::with_seconds(at, TIMESCALE),
            &handler,
        );
    }
    receiver
        .recv_timeout(FRAME_TIMEOUT)
        .inspect_err(|error| log::info!("no still frame at {at:.1} s: {error}"))
        .ok()
        .flatten()
}

/// `image` as a JPEG file's bytes.
#[expect(
    unsafe_code,
    reason = "the JPEG representation takes a properties dictionary, unsafe in objc2"
)]
fn jpeg(image: &CGImage) -> Option<Vec<u8>> {
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), image);
    let quality = NSNumber::new_f64(QUALITY);
    let value: &AnyObject = &quality;
    // SAFETY: AppKit's own key, read once.
    let key = unsafe { NSImageCompressionFactor };
    let properties = NSDictionary::from_slices(&[key], &[value]);
    // SAFETY: the compression factor's value is a number, as AppKit wants.
    let data =
        unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::JPEG, &properties) };
    data.map(|data| data.to_vec())
}
