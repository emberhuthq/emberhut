//! The system's Reduce Transparency setting (macOS: System Settings › Accessibility › Display),
//! read when asked and watched through its change notification. Other platforms have no such
//! setting here: it reads as off and never changes.

use smol::channel::{self, Receiver};

/// Whether the system asks apps to reduce transparency now.
#[must_use]
pub(super) fn reduce_transparency() -> bool {
    platform::reduce_transparency()
}

/// The setting's changes, as they come; the watch ends when this is dropped.
#[derive(Debug)]
pub(crate) struct Changes {
    changed: Receiver<()>,
    /// The system's observer, removed on drop.
    _observer: platform::Observer,
}

impl Changes {
    /// Waits for the next change; `false` once no more can come.
    pub(crate) async fn next(&self) -> bool {
        self.changed.recv().await.is_ok()
    }
}

/// Starts watching the setting. Where the system has nothing to watch, the changes end at once.
#[must_use]
pub(super) fn watch() -> Changes {
    // One pending change is enough: whoever wakes reads the setting as it is then.
    let (sender, changed) = channel::bounded(1);
    let observer = platform::observe(move || match sender.try_send(()) {
        Ok(()) | Err(channel::TrySendError::Full(())) => {}
        Err(channel::TrySendError::Closed(())) => {
            log::debug!("a Reduce Transparency change came after the watch ended");
        }
    });
    Changes {
        changed,
        _observer: observer,
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::fmt;
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue,
    };

    pub(super) fn reduce_transparency() -> bool {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
    }

    /// A registration with the workspace's notification center, removed when dropped.
    pub(super) struct Observer {
        center: Retained<NSNotificationCenter>,
        token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    }

    impl fmt::Debug for Observer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Observer").finish_non_exhaustive()
        }
    }

    #[expect(
        unsafe_code,
        reason = "AppKit's notification name is an extern static, and observing takes a block"
    )]
    pub(super) fn observe(on_change: impl Fn() + Send + Sync + 'static) -> Observer {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let block = RcBlock::new(move |_: NonNull<NSNotification>| on_change());
        // The main queue: the block runs on the main thread, whoever posts.
        let queue = NSOperationQueue::mainQueue();
        // SAFETY: the name is a constant AppKit defines for the process's life. No object
        // filter is given, so there is no object of a wrong type. The main queue asks nothing
        // more of the block than to run on the main thread. The block is sendable, as the call
        // asks: all it holds is `on_change`, which is `Send + Sync`.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                None,
                Some(&queue),
                &block,
            )
        };
        Observer { center, token }
    }

    impl Drop for Observer {
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
mod platform {
    /// Nothing is observed off macOS.
    #[derive(Debug)]
    pub(super) struct Observer;

    pub(super) fn reduce_transparency() -> bool {
        false
    }

    /// Drops `on_change` (and so the changes' sender) at once: the changes end.
    pub(super) fn observe(_on_change: impl Fn() + Send + Sync + 'static) -> Observer {
        Observer
    }
}

// On macOS the setting is the Mac's own; the Mac build's check covers it (see the README).
#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn off_macos_the_setting_is_off_and_its_changes_end_at_once() {
        assert!(!reduce_transparency());
        assert!(!smol::block_on(watch().next()));
    }
}
