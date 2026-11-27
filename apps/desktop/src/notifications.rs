//! Asking to show notifications. On macOS the app asks at launch for alerts and sounds, as Mac
//! apps do: the system asks the user the first time only and keeps the answer, later asks
//! return it. `--no-prompt` skips the ask (tests, screenshots). The app posts no notification
//! yet: the server pushes to phones, not to the desktop. Elsewhere there is nothing to ask.

use gpui_kit::App;
use gpui_kit::AppContext as _;

/// Asks for permission to show notifications, unless `skip`.
pub(crate) fn ask(skip: bool, cx: &mut App) {
    if skip {
        return;
    }
    cx.background_spawn(async { platform::ask() }).detach();
}

#[cfg(target_os = "macos")]
mod platform {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError};
    use objc2_user_notifications::{UNAuthorizationOptions, UNUserNotificationCenter};

    pub(super) fn ask() {
        // The notification center exists only for an app bundle: a binary run on its own
        // (`cargo run`) would stop with an exception.
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            log::info!("not asking to show notifications: the app isn't running from its bundle");
            return;
        }
        let answered = RcBlock::new(|granted: Bool, _error: *mut NSError| {
            log::info!("showing notifications: {}", granted.as_bool());
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &answered,
            );
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    /// Nothing to ask off macOS.
    pub(super) fn ask() {}
}
