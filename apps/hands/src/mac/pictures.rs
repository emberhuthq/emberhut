//! Pictures: a window's (covered or hidden) or the main screen's, taken by the system's
//! `screencapture` and sized and made JPEG by `sips`, both run by absolute path.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use objc2_core_graphics::{CGDisplayBounds, CGMainDisplayID};

use crate::desktop::ScreenPicture;

/// The system's screen capture tool.
const SCREENCAPTURE: &str = "/usr/sbin/screencapture";
/// The system's image tool.
const SIPS: &str = "/usr/bin/sips";
/// The JPEG quality, in percent.
const QUALITY: &str = "80";

/// A new private file for a picture, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Option<Self> {
        let mut bytes = [0_u8; 8];
        getrandom::getrandom(&mut bytes).ok()?;
        let name: String = bytes.iter().map(|b| char::from(b'a' + b % 26)).collect();
        Some(Self(
            std::env::temp_dir().join(format!("emberhut-hands-{name}.png")),
        ))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _gone = std::fs::remove_file(&self.0);
    }
}

/// Runs a tool quietly; whether it succeeded.
fn quiet(command: &mut Command) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// `file` as a JPEG, resized as `size` says (`-Z max`, or `-z height width`).
fn jpeg(file: &Path, size: &[String]) -> Option<Vec<u8>> {
    let out = Scratch::new()?;
    let made = quiet(
        Command::new(SIPS)
            .args(["-s", "format", "jpeg", "-s", "formatOptions", QUALITY])
            .args(size)
            .arg(file)
            .arg("--out")
            .arg(&out.0),
    );
    if !made {
        return None;
    }
    std::fs::read(&out.0).ok()
}

/// A JPEG of window `id`, its longest side at most `max` pixels.
#[must_use]
pub(super) fn window(id: u32, max: u32) -> Option<Vec<u8>> {
    let file = Scratch::new()?;
    let taken = quiet(
        Command::new(SCREENCAPTURE)
            .args(["-x", "-o", "-l"])
            .arg(id.to_string())
            .arg(&file.0),
    );
    if !taken {
        return None;
    }
    jpeg(&file.0, &["-Z".to_owned(), max.to_string()])
}

/// A JPEG of the main screen, one pixel a point (the points clicks use).
#[must_use]
pub(super) fn screen() -> Option<ScreenPicture> {
    let bounds = CGDisplayBounds(CGMainDisplayID());
    let file = Scratch::new()?;
    if !quiet(
        Command::new(SCREENCAPTURE)
            .args(["-x", "-m", "-C"])
            .arg(&file.0),
    ) {
        return None;
    }
    let (width, height) = (bounds.size.width, bounds.size.height);
    // Screen sizes in points are small whole numbers.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a screen's size in points"
    )]
    let size = [
        "-z".to_owned(),
        (height.round().max(1.0) as u32).to_string(),
        (width.round().max(1.0) as u32).to_string(),
    ];
    Some(ScreenPicture {
        jpeg: jpeg(&file.0, &size)?,
        width,
        height,
    })
}
