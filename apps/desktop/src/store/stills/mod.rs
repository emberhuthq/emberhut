//! A video attached in the composer goes with four still frames of it (at 5, 35, 65 and 95 %
//! of its length), so the assistant can see what happens in it: each is a JPEG picture
//! attachment of its own, named after the video.
//!
//! On macOS the frames come from `AVFoundation` (`avfoundation`), as the reference client
//! takes them; elsewhere, and for a video `AVFoundation` can't read, from the tools of the
//! `FFmpeg` project where they are installed ([`ffmpeg`]). Without either the video goes alone.
//! Removing the video's chip stops the work on its frames, and frames of a video no longer
//! attached are dropped.

#[cfg(target_os = "macos")]
mod avfoundation;
mod ffmpeg;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::AppContext as _;
use gpui_kit::{Context, Image, ImageFormat};

use super::attachments::{AttachmentId, StopFlag};
use super::{Store, update_or_log};
use crate::backend::UploadSource;

/// Where in a video the frames are taken, as shares of its length.
const SHARES: [f64; 4] = [0.05, 0.35, 0.65, 0.95];

/// The longest side of a frame, in pixels.
const LONGEST_SIDE: u32 = 1280;

/// Whether a file of this name is a video.
#[must_use]
pub(crate) fn is_video(name: &str) -> bool {
    let extension = Path::new(name)
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    matches!(
        extension.as_deref(),
        Some("mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm")
    )
}

/// What a video's still frames are named with, between the video's name and the frame's number.
pub(crate) const FRAME: &str = " · frame ";

/// The name of a video's `index`th frame (from 1).
#[must_use]
pub(crate) fn frame_name(video: &str, index: usize) -> String {
    format!("{video}{FRAME}{index}.jpg")
}

/// Whether the file called `name` is a video's still frame after its first: only the first
/// shows (as the video's picture), the others go to the assistant alone.
#[must_use]
pub(crate) fn is_later_frame(name: &str) -> bool {
    name.contains(FRAME) && !name.ends_with(&format!("{FRAME}1.jpg"))
}

/// The times to take frames at, in seconds, for a video `duration` seconds long.
#[must_use]
pub(crate) fn frame_times(duration: f64) -> Vec<f64> {
    if !duration.is_finite() || duration <= 0.0 {
        return Vec::new();
    }
    SHARES.iter().map(|share| duration * share).collect()
}

/// The still frames of the video at `path`, as JPEG files; none when it can't be read, or once
/// `stop` is set. On macOS `AVFoundation` takes them (`FFmpeg` then, for a video it can't read),
/// elsewhere `FFmpeg` where it is installed. It blocks: call it off the UI thread.
fn grab(path: &Path, stop: &StopFlag) -> Vec<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        let frames = avfoundation::grab(path);
        if !frames.is_empty() || stop.is_set() {
            return frames;
        }
    }
    ffmpeg::grab(path, stop)
}

/// A frame's bytes, shared with the picture that shows it.
struct FrameBytes(Arc<Image>);

impl AsRef<[u8]> for FrameBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.bytes()
    }
}

impl Store {
    /// Adds the still frames of the video at `path`, called `name` and attached as `video`,
    /// once they are made off the UI thread; Send waits meanwhile. They show as the
    /// video's chip (the first as its picture) and go when it is taken out.
    pub(super) fn attach_stills(
        &mut self,
        video: AttachmentId,
        path: PathBuf,
        name: String,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(stop) = self
            .attachments
            .get(video)
            .map(super::attachments::Attachment::stop_flag)
        else {
            return;
        };
        self.attachments.begin_adding();
        cx.spawn(async move |this, cx| {
            let frames = cx.background_spawn(async move { grab(&path, &stop) }).await;
            update_or_log(&this, cx, |store, cx| {
                store.attachments.added();
                if store.attachments.get(video).is_none() {
                    log::debug!("{name} was removed: its still frames go too");
                    cx.notify();
                    return;
                }
                for (index, frame) in frames.into_iter().enumerate() {
                    let size = u64::try_from(frame.len()).unwrap_or(u64::MAX);
                    let picture = Arc::new(Image::from_bytes(ImageFormat::Jpeg, frame));
                    let source = UploadSource::Bytes(Arc::new(FrameBytes(Arc::clone(&picture))));
                    let name = frame_name(&name, index + 1);
                    let Some(id) = store
                        .attachments
                        .add_still(video, name, size, source, picture)
                    else {
                        break;
                    };
                    store.start_upload(id, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn videos_are_told_by_their_extension() {
        assert!(is_video("clip.MOV"));
        assert!(is_video("screen.mp4"));
        assert!(!is_video("photo.jpg"));
        assert!(!is_video("notes"));
    }

    #[test]
    fn frames_spread_over_the_video() {
        let times = frame_times(100.0);
        assert_eq!(times.len(), 4);
        for (time, want) in times.iter().zip([5.0, 35.0, 65.0, 95.0]) {
            assert!((time - want).abs() < 1e-9, "{times:?}");
        }
        assert_eq!(frame_times(0.0), Vec::<f64>::new());
        assert_eq!(frame_times(f64::NAN), Vec::<f64>::new());
    }

    #[test]
    fn a_frame_is_named_after_its_video() {
        assert_eq!(frame_name("clip.mov", 2), "clip.mov · frame 2.jpg");
        assert!(is_later_frame(&frame_name("clip.mov", 2)));
        assert!(!is_later_frame(&frame_name("clip.mov", 1)));
        assert!(!is_later_frame("shot.jpg"));
    }
}
