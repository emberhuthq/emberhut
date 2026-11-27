//! A video playing in its chat row or its result tile, as the reference client plays one in
//! place. On macOS a click on its still fetches the file (as the viewer does) and plays it
//! there: `AVFoundation` decodes it and each new frame is drawn by GPUI as a surface, so it is
//! clipped and scrolled with the chat like anything else (no native view over the app's own).
//! One plays at a time; it pauses once it is out of sight. Its bar: play or pause, how far in
//! of how long, a track to click to a place, and full screen (the file in a window of its own,
//! in `AVKit`'s player). Elsewhere a click opens the video in the viewer.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod view;

#[cfg(target_os = "macos")]
pub(crate) use self::view::{play, playing};

/// The size a video plays at in a box at most `most` (width, height) big, `natural` its own
/// size (its still's; none: 16 by 9).
#[must_use]
pub(crate) fn fit(natural: Option<(f32, f32)>, most: (f32, f32)) -> (f32, f32) {
    let natural = natural.filter(|(width, height)| *width > 0. && *height > 0.);
    let (width, height) = natural.unwrap_or((16., 9.));
    let scale = (most.0 / width).min(most.1 / height);
    // A still is never drawn bigger than it is; the stand-in shape fills the box.
    let scale = if natural.is_some() {
        scale.min(1.)
    } else {
        scale
    };
    (width * scale, height * scale)
}

/// The size `still` is drawn at by default, in logical pixels: a video's own shape.
#[must_use]
pub(crate) fn still_size(still: &gpui_kit::RenderImage) -> (f32, f32) {
    let size = still.size(0);
    let logical = |n| f32::from(gpui_kit::Pixels::from(u32::from(n)));
    (logical(size.width), logical(size.height))
}

/// How far in a video is, 0 to 1 (0 while its length isn't known).
#[cfg(any(target_os = "macos", test))]
#[must_use]
fn share(position: f64, duration: Option<f64>) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a share from 0 to 1 fits an f32"
    )]
    duration
        .filter(|duration| *duration > 0.)
        .map_or(0., |duration| (position / duration).clamp(0., 1.) as f32)
}

/// A time as a player shows it: `0:07`, `12:30`, `1:02:03`.
#[cfg(any(target_os = "macos", test))]
#[must_use]
fn clock(seconds: f64) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "whole seconds of a video, never negative here"
    )]
    let whole = seconds.max(0.).floor() as u64;
    let (hours, minutes, seconds) = (whole / 3600, whole / 60 % 60, whole % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// How far in of how long: `0:12 / 1:03` (`0:12` while its length isn't known).
#[cfg(any(target_os = "macos", test))]
#[must_use]
fn clock_line(position: f64, duration: Option<f64>) -> String {
    match duration {
        Some(duration) => format!("{} / {}", clock(position), clock(duration)),
        None => clock(position),
    }
}

/// Plays `file` in place; here it can't (`false`): the caller opens it in the viewer.
#[cfg(not(target_os = "macos"))]
pub(crate) fn play(
    _store: &gpui_kit::Entity<crate::store::Store>,
    _file: &crate::store::files::RemoteFile,
    _cx: &mut gpui_kit::App,
) -> bool {
    false
}

/// The video in place; here none plays in place.
#[cfg(not(target_os = "macos"))]
pub(crate) fn playing(
    _file: &crate::store::files::RemoteFile,
    _size: (f32, f32),
    _store: &gpui_kit::Entity<crate::store::Store>,
    _cx: &gpui_kit::App,
) -> Option<gpui_kit::AnyElement> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_video_fits_its_box_keeping_its_shape() {
        assert_eq!(fit(Some((1920., 1080.)), (420., 320.)), (420., 236.25));
        assert_eq!(fit(Some((1080., 1920.)), (420., 320.)), (180., 320.));
        assert_eq!(
            fit(Some((200., 100.)), (420., 320.)),
            (200., 100.),
            "never bigger"
        );
        assert_eq!(
            fit(None, (320., 320.)),
            (320., 180.),
            "16 by 9 without a still"
        );
    }

    #[test]
    fn the_bar_reads_how_far_in_of_how_long() {
        assert_eq!(clock(7.9), "0:07");
        assert_eq!(clock(750.), "12:30");
        assert_eq!(clock(3723.), "1:02:03");
        assert_eq!(clock(-3.), "0:00");
        assert_eq!(clock_line(12., Some(63.)), "0:12 / 1:03");
        assert_eq!(clock_line(12., None), "0:12");
        assert!((share(30., Some(60.)) - 0.5).abs() < f32::EPSILON);
        assert!(share(30., None).abs() < f32::EPSILON);
        assert!((share(90., Some(60.)) - 1.).abs() < f32::EPSILON);
    }
}
