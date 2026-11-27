//! The Mac itself, through objc2's bindings to its frameworks: Accessibility (`ax`), the apps
//! (`apps`), the window list (`windows`), input events (`input`), pictures (`pictures`), the
//! lock (`session`), the bar with Stop (`bar`), recording (`audio`) and sending to the server
//! (`upload`). Every unsafe call sits in these small modules, each with its reason.

mod apps;
mod audio;
mod ax;
mod bar;
mod desktop;
mod input;
mod pictures;
mod session;
mod upload;
mod windows;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use objc2_core_graphics::CGRequestScreenCaptureAccess;

pub use self::audio::ScreenAudio;
pub use self::ax::Element;
pub use self::bar::on_stop;
pub use self::desktop::MacDesktop;
pub use self::upload::SessionUploader;

/// The grants the helper holds: Accessibility, and Screen Recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grants {
    /// Accessibility.
    pub accessibility: bool,
    /// Screen Recording.
    pub screen_recording: bool,
}

/// The grants, asking the Mac to list the helper under each one it lacks (System Settings →
/// Privacy & Security), for the owner to turn on.
#[must_use]
pub fn ask_for_grants() -> Grants {
    let accessibility = ax::ask_trust();
    let screen_recording = desktop::screen_recording() || CGRequestScreenCaptureAccess();
    Grants {
        accessibility,
        screen_recording,
    }
}

/// The helper is not on the main thread.
#[derive(Debug, thiserror::Error)]
#[error("the Mac's app loop must run on the main thread")]
pub struct NotMainThread;

/// Runs the app loop on the main thread, as an accessory (no Dock icon, never in front), so the
/// bar can be shown and `ScreenCaptureKit` has its window server connection. It returns only
/// when the app ends.
///
/// # Errors
///
/// [`NotMainThread`] off the main thread.
pub fn run_app() -> Result<(), NotMainThread> {
    let mtm = MainThreadMarker::new().ok_or(NotMainThread)?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.run();
    Ok(())
}
