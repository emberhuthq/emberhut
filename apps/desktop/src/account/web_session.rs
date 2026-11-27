//! The sign-in through the system's web authentication session (macOS 14.4 and later): an
//! `ASWebAuthenticationSession` at the provider's authorization page, which comes back only
//! through the HTTPS callback `https://emberhut.com/oauth/callback`, a universal link that only the
//! app named by emberhut.com's associated domain receives. The model uses it only in a build that
//! claims that domain ([`super::switch`]); every other build signs in with the device grant.
//!
//! Its calls are unsafe, so they sit here, in the one module allowed to make them. Elsewhere
//! than macOS there is no such session, and [`available`] is false.

use std::future::Future;
use std::pin::Pin;

/// The callback URL a session came back with, or why there is none.
pub(crate) type Answer = Pin<Box<dyn Future<Output = Result<String, Ended>>>>;

/// Why a session ended without a callback.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Ended {
    /// The person closed the sign-in window.
    #[error("Sign-in was cancelled.")]
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "only the Mac's window is closed this way")
    )]
    Cancelled,
    /// The system couldn't show it, or failed, in its own words.
    #[error("The sign-in window couldn't finish: {0}")]
    Failed(String),
}

#[cfg(target_os = "macos")]
pub(crate) use mac::{WebSession, available, start};

#[cfg(not(target_os = "macos"))]
pub(crate) use other::{WebSession, available, start};

#[cfg(target_os = "macos")]
#[expect(
    unsafe_code,
    reason = "AuthenticationServices marks its calls unsafe, and the context provider is an \
              Objective-C class of our own; each call takes live, typed objects"
)]
mod mac {
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AnyThread as _, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{NSApplication, NSWindow};
    use objc2_authentication_services::{
        ASPresentationAnchor, ASWebAuthenticationPresentationContextProviding,
        ASWebAuthenticationSession, ASWebAuthenticationSessionCallback,
        ASWebAuthenticationSessionErrorCode, ASWebAuthenticationSessionErrorDomain,
    };
    use objc2_foundation::{NSError, NSString, NSURL};

    use super::{Answer, Ended};
    use crate::account::oidc::CALLBACK;
    use crate::account::switch::CALLBACK_HOST;

    define_class!(
        /// The window the session's sheet is shown over: the app's key window.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "EmberhutWebAuthenticationAnchor"]
        struct Anchor;

        unsafe impl NSObjectProtocol for Anchor {}

        unsafe impl ASWebAuthenticationPresentationContextProviding for Anchor {
            #[unsafe(method_id(presentationAnchorForWebAuthenticationSession:))]
            fn anchor(
                &self,
                _session: &ASWebAuthenticationSession,
            ) -> Retained<ASPresentationAnchor> {
                let mtm = self.mtm();
                let app = NSApplication::sharedApplication(mtm);
                let window = app
                    .keyWindow()
                    .or_else(|| app.mainWindow())
                    .or_else(|| app.windows().firstObject())
                    // SAFETY: a new, empty window made on the main thread, as `mtm` proves; it
                    // is only the sheet's anchor when the app has no window of its own.
                    .unwrap_or_else(|| unsafe { NSWindow::new(mtm) });
                Retained::into_super(Retained::into_super(window))
            }
        }
    );

    impl Anchor {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            // SAFETY: `init` on a fresh allocation of a class with no instance variables.
            unsafe { msg_send![Self::alloc(mtm), init] }
        }
    }

    /// Whether `error` is the session's own "the person closed the window".
    fn is_cancelled(error: &NSError) -> bool {
        // SAFETY: a constant NSString the framework exports, alive for the process.
        let domain = unsafe { ASWebAuthenticationSessionErrorDomain };
        error.domain().isEqualToString(domain)
            && error.code() == ASWebAuthenticationSessionErrorCode::CanceledLogin.0
    }

    /// A session under way: kept until it answers, so it isn't dropped mid-sign-in.
    pub(crate) struct WebSession {
        session: Retained<ASWebAuthenticationSession>,
        /// The session holds its context provider weakly.
        _anchor: Retained<Anchor>,
    }

    impl std::fmt::Debug for WebSession {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("WebSession")
                .field("session", &self.session)
                .finish_non_exhaustive()
        }
    }

    impl WebSession {
        /// Closes the sign-in window; the answer is [`Ended::Cancelled`].
        pub(crate) fn cancel(&self) {
            unsafe {
                self.session.cancel();
            }
        }
    }

    /// Whether this Mac has the session with an HTTPS callback (macOS 14.4 and later, where
    /// `ASWebAuthenticationSessionCallback` exists).
    #[must_use]
    pub(crate) fn available() -> bool {
        AnyClass::get(c"ASWebAuthenticationSessionCallback").is_some()
    }

    /// Opens the session at `url` over the key window. Call it on the main thread.
    ///
    /// # Errors
    ///
    /// The session couldn't start, in words.
    pub(crate) fn start(url: &str) -> Result<(WebSession, Answer), Ended> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| Ended::Failed("not on the main thread".to_owned()))?;
        let page = NSURL::URLWithString(&NSString::from_str(url))
            .ok_or_else(|| Ended::Failed("the sign-in page's address".to_owned()))?;
        let path = CALLBACK
            .strip_prefix(&format!("https://{CALLBACK_HOST}"))
            .unwrap_or(CALLBACK);
        let callback = unsafe {
            ASWebAuthenticationSessionCallback::callbackWithHTTPSHost_path(
                &NSString::from_str(CALLBACK_HOST),
                &NSString::from_str(path),
            )
        };
        let (answer, answered) = smol::channel::bounded(1);
        // The session calls this once, with the callback URL or an error.
        let done = RcBlock::new(move |url: *mut NSURL, error: *mut NSError| {
            // SAFETY: the session passes a valid NSURL or nil, and a valid NSError or nil.
            let (url, error) = unsafe { (url.as_ref(), error.as_ref()) };
            let result = match (url.and_then(NSURL::absoluteString), error) {
                (Some(url), _) => Ok(url.to_string()),
                (None, Some(error)) if is_cancelled(error) => Err(Ended::Cancelled),
                (None, Some(error)) => Err(Ended::Failed(error.localizedDescription().to_string())),
                (None, None) => Err(Ended::Failed("no answer".to_owned())),
            };
            if answer.try_send(result).is_err() {
                log::warn!("the sign-in window answered after the app stopped waiting");
            }
        });
        let anchor = Anchor::new(mtm);
        let session = unsafe {
            let session = ASWebAuthenticationSession::initWithURL_callback_completionHandler(
                ASWebAuthenticationSession::alloc(),
                &page,
                &callback,
                RcBlock::as_ptr(&done),
            );
            session.setPresentationContextProvider(Some(ProtocolObject::from_ref(&*anchor)));
            session
        };
        let started = unsafe { session.start() };
        if !started {
            return Err(Ended::Failed("the system didn't open it".to_owned()));
        }
        let kept = WebSession {
            session,
            _anchor: anchor,
        };
        Ok((
            kept,
            Box::pin(async move {
                answered
                    .recv()
                    .await
                    .unwrap_or(Err(Ended::Failed("no answer".to_owned())))
            }),
        ))
    }
}

#[cfg(not(target_os = "macos"))]
mod other {
    use super::{Answer, Ended};

    /// No session off macOS.
    #[derive(Debug)]
    pub(crate) struct WebSession;

    impl WebSession {
        /// Nothing to close.
        #[expect(
            clippy::unused_self,
            reason = "the Mac's session is closed through itself"
        )]
        pub(crate) fn cancel(&self) {}
    }

    /// No session off macOS.
    #[must_use]
    pub(crate) fn available() -> bool {
        false
    }

    /// No session off macOS.
    ///
    /// # Errors
    ///
    /// Always.
    pub(crate) fn start(_url: &str) -> Result<(WebSession, Answer), Ended> {
        Err(Ended::Failed(
            "this system has no sign-in window".to_owned(),
        ))
    }
}
