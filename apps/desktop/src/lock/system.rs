//! Asking the system who is at the computer: on macOS the device-owner check of
//! `LocalAuthentication` (Touch ID, or the Mac's password when there is no Touch ID or the user
//! prefers it). Elsewhere there is nothing to ask with, so the lock never closes.

use std::future::Future;
use std::pin::Pin;

/// An answer on its way: whether the user proved it is them.
pub(super) type Asking = Pin<Box<dyn Future<Output = bool>>>;

/// Why the system can't ask now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CantAsk {
    /// There is nothing to ask with: no password is set on this Mac, or it isn't a Mac. Only
    /// then does the lock open without asking.
    NothingToAskWith,
    /// The system can't ask right now, for this reason (Touch ID locked out, no way to show the
    /// sheet, an error it may give in the future): the lock stays shut.
    #[cfg_attr(
        all(not(target_os = "macos"), not(test)),
        expect(dead_code, reason = "only the Mac's check fails this way")
    )]
    Failed(String),
}

#[cfg(target_os = "macos")]
pub(super) use mac::{AVAILABLE, ask};

#[cfg(not(target_os = "macos"))]
pub(super) use other::{AVAILABLE, ask};

#[cfg(target_os = "macos")]
mod mac {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    use super::{Asking, CantAsk};

    /// The Mac can ask.
    pub(in crate::lock) const AVAILABLE: bool = true;

    /// `LocalAuthentication`'s error domain.
    const LA_ERROR_DOMAIN: &str = "com.apple.LocalAuthentication";

    /// `LAError.passcodeNotSet`: no password is set on this Mac.
    const PASSCODE_NOT_SET: isize = -5;

    /// What a failed `canEvaluatePolicy` means for the lock: only "no password set" is nothing
    /// to ask with; any other error keeps it shut.
    fn cant_ask(error: &NSError) -> CantAsk {
        if error.domain().to_string() == LA_ERROR_DOMAIN && error.code() == PASSCODE_NOT_SET {
            CantAsk::NothingToAskWith
        } else {
            CantAsk::Failed(error.localizedDescription().to_string())
        }
    }

    /// Shows the system's Touch ID or password sheet with `reason` ("Emberhut is trying to …").
    ///
    /// # Errors
    ///
    /// Why the system can't ask now ([`CantAsk`]).
    pub(in crate::lock) fn ask(reason: &str) -> Result<Asking, CantAsk> {
        let policy = LAPolicy::DeviceOwnerAuthentication;
        #[expect(
            unsafe_code,
            reason = "objc2-local-authentication marks every LAContext call unsafe; these take \
                      owned, typed arguments and the context lives until its reply"
        )]
        let context = unsafe { LAContext::new() };
        #[expect(
            unsafe_code,
            reason = "as above: a typed policy, the error is returned"
        )]
        let can = unsafe { context.canEvaluatePolicy_error(policy) };
        if let Err(error) = can {
            return Err(cant_ask(&error));
        }
        let (answer, answered) = smol::channel::bounded(1);
        // LocalAuthentication replies once, on a queue of its own.
        let reply = RcBlock::new(move |opened: Bool, _error: *mut NSError| {
            if answer.try_send(opened.as_bool()).is_err() {
                log::warn!("the app lock's answer came after it stopped waiting");
            }
        });
        #[expect(
            unsafe_code,
            reason = "as above: the reason is a live NSString and the reply block is retained by \
                      the call"
        )]
        unsafe {
            context.evaluatePolicy_localizedReason_reply(
                policy,
                &NSString::from_str(reason),
                &reply,
            );
        }
        Ok(Box::pin(async move {
            let opened = answered.recv().await.unwrap_or(false);
            // The context is kept until it has answered: dropping it would cancel the sheet.
            drop(context);
            opened
        }))
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use super::{Asking, CantAsk};

    /// Nothing to ask with off macOS.
    pub(in crate::lock) const AVAILABLE: bool = false;

    /// Nothing to ask with: the lock opens.
    ///
    /// # Errors
    ///
    /// Always [`CantAsk::NothingToAskWith`].
    pub(in crate::lock) fn ask(_reason: &str) -> Result<Asking, CantAsk> {
        Err(CantAsk::NothingToAskWith)
    }
}
