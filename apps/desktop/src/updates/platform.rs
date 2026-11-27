//! What the updater asks of the system: how long the Mac has been without input, the old copy
//! stepping out of sight while it watches the new one (no Dock icon, no menu bar), a Dock click
//! passed on to the new copy, and the notice of a logout or power-off. On macOS through `AppKit`
//! and Core Graphics; elsewhere there is no updater, and these do nothing.

#[cfg(target_os = "macos")]
pub(crate) use mac::{PowerOffWatch, activate, idle, step_aside, watch_power_off};
#[cfg(not(target_os = "macos"))]
pub(crate) use other::{PowerOffWatch, activate, idle, step_aside, watch_power_off};

#[cfg(target_os = "macos")]
mod mac {
    use std::fmt;
    use std::ptr::NonNull;
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy,
        NSRunningApplication, NSWorkspace, NSWorkspaceWillPowerOffNotification,
    };
    use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue,
    };

    /// How long since the last keyboard, mouse or trackpad input to any app.
    pub(crate) fn idle() -> Option<Duration> {
        // Every kind of input (`kCGAnyInputEventType`), from every source in the session.
        let seconds = CGEventSource::seconds_since_last_event_type(
            CGEventSourceStateID::CombinedSessionState,
            CGEventType(u32::MAX),
        );
        Duration::try_from_secs_f64(seconds).ok()
    }

    /// Leaves the Dock, the menu bar and ⌘-Tab (the accessory activation policy).
    pub(crate) fn step_aside() {
        let Some(main) = MainThreadMarker::new() else {
            log::warn!("can't leave the Dock off the main thread");
            return;
        };
        let app = NSApplication::sharedApplication(main);
        if !app.setActivationPolicy(NSApplicationActivationPolicy::Accessory) {
            log::warn!("the system kept this copy in the Dock");
        }
    }

    /// Brings the app with process `pid` to the front; whether it could.
    pub(crate) fn activate(pid: u32) -> bool {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .is_some_and(|app| app.activateWithOptions(NSApplicationActivationOptions::empty()))
    }

    /// A registration with the workspace's notification center, removed when dropped.
    pub(crate) struct PowerOffWatch {
        center: Retained<NSNotificationCenter>,
        token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    }

    impl fmt::Debug for PowerOffWatch {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PowerOffWatch").finish_non_exhaustive()
        }
    }

    #[expect(
        unsafe_code,
        reason = "AppKit's notification name is an extern static, and observing takes a block"
    )]
    /// Calls `on` when the user logs out, restarts or shuts down, until dropped.
    pub(crate) fn watch_power_off(on: impl Fn() + Send + Sync + 'static) -> PowerOffWatch {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let block = RcBlock::new(move |_: NonNull<NSNotification>| on());
        let queue = NSOperationQueue::mainQueue();
        // SAFETY: the name is a constant AppKit defines for the process's life. No object
        // filter is given, so there is no object of a wrong type. The main queue asks nothing
        // more of the block than to run on the main thread. The block is sendable, as the call
        // asks: all it holds is `on`, which is `Send + Sync`.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceWillPowerOffNotification),
                None,
                Some(&queue),
                &block,
            )
        };
        PowerOffWatch { center, token }
    }

    impl Drop for PowerOffWatch {
        #[expect(
            unsafe_code,
            reason = "removing an observer is unsafe in objc2's binding"
        )]
        fn drop(&mut self) {
            let observer: &AnyObject = (*self.token).as_ref();
            // SAFETY: `observer` is the very observer this center returned for the registration.
            unsafe { self.center.removeObserver(observer) };
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use std::time::Duration;

    /// Nothing is watched off macOS.
    #[derive(Debug)]
    pub(crate) struct PowerOffWatch;

    /// The system doesn't say off macOS.
    pub(crate) fn idle() -> Option<Duration> {
        None
    }

    /// Nothing to step out of off macOS.
    pub(crate) fn step_aside() {}

    /// Nothing to bring forward off macOS.
    pub(crate) fn activate(_pid: u32) -> bool {
        false
    }

    /// Nothing to watch off macOS.
    pub(crate) fn watch_power_off(_on: impl Fn() + Send + Sync + 'static) -> PowerOffWatch {
        PowerOffWatch
    }
}
