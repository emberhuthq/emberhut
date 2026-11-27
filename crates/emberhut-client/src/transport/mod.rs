//! How a request reaches a machine: the [`Transport`] trait, the request and answer it carries,
//! and [`Direct`], the transport to one base URL.
//!
//! [`Client`](crate::Client) builds every request as a [`Request`] (a method, a path and query
//! on the machine, headers and a body) and hands it to its transport, which gives back an
//! [`Answer`] whose body is read as it arrives. Nothing in [`Client`](crate::Client) knows how
//! the bytes travel, so another transport can stand in for [`Direct`], or wrap one and change
//! whole requests and answers on their way: their headers, their bodies, or both, streams
//! included.
//!
//! The types are this crate's own, over the [`http`] crate's parts (re-exported here, so an
//! implementation uses the same version). No HTTP client library's type is part of them:
//! `ureq` is [`Direct`]'s own business.
//!
//! ```
//! use std::borrow::Cow;
//! use std::io::Read;
//!
//! use emberhut_client::transport::http::{HeaderMap, Method, StatusCode};
//! use emberhut_client::transport::{Answer, Request, Timeouts, Transport, TransportError};
//!
//! /// Answers every request with an empty 204.
//! #[derive(Debug)]
//! struct Nothing;
//!
//! impl Transport for Nothing {
//!     fn send(&self, _: Request<'_>, _: Timeouts) -> Result<Answer, TransportError> {
//!         Ok(Answer::new(StatusCode::NO_CONTENT, HeaderMap::new(), std::io::empty()))
//!     }
//!
//!     fn base_url(&self) -> Cow<'_, str> {
//!         Cow::Borrowed("http://nothing.invalid")
//!     }
//! }
//!
//! let request = Request::new(Method::GET, "/api/machines".parse()?);
//! let mut answer = Nothing.send(request, Timeouts::default())?;
//! assert_eq!(answer.status(), StatusCode::NO_CONTENT);
//! assert_eq!(answer.body_mut().read(&mut [0; 8])?, 0);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod answer;
mod direct;
mod error;
mod request;
mod timeouts;

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

pub use ::http;

pub use answer::Answer;
pub use direct::Direct;
pub use error::{TransportError, TransportErrorKind};
pub use request::{Body, Request};
pub use timeouts::Timeouts;

/// One way to send a request to a machine and read its answer.
///
/// An implementation sends the request as it is given and hands back every answer, of any
/// status, as an [`Answer`]: what a status means is the caller's business. It fails only when
/// the request didn't get through or no answer came. Each call blocks until the answer's head
/// has arrived; its body is then read from [`Answer::body_mut`] as it arrives.
///
/// # Sending again
///
/// A transport that may send one request more than once (on another route, after a failed
/// connection) keeps a copy made with [`Request::try_clone`] before the first attempt, and
/// sends the copy only when the attempt failed with [`TransportErrorKind::NotSent`]: after
/// any other failure the machine may have acted on the request. A request with a
/// [`Body::Stream`] has no copy, since its body can be read only once: it is sent once, and
/// its failure goes back to the caller.
///
/// # Threads
///
/// A request's body isn't `Send`: an upload's body reads through the caller's progress
/// callback, which [`Client::upload`](crate::Client::upload) doesn't require to be `Send`. A
/// transport reads it on the thread that called [`send`](Self::send). The answer's body is
/// `Send`, so it can be read anywhere.
pub trait Transport: Send + Sync + fmt::Debug {
    /// Sends `request` (a path and query on the machine) and returns the answer's head, whose
    /// body is read as it arrives.
    ///
    /// # Errors
    ///
    /// The request didn't get through, or no answer came within `timeouts`; the error's
    /// [kind](TransportError::kind) says how far it got.
    fn send(&self, request: Request<'_>, timeouts: Timeouts) -> Result<Answer, TransportError>;

    /// The base URL requests go to now (it may change as the route does), for display and
    /// logs only: nothing builds a request from it.
    fn base_url(&self) -> Cow<'_, str>;
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn send(&self, request: Request<'_>, timeouts: Timeouts) -> Result<Answer, TransportError> {
        (**self).send(request, timeouts)
    }

    fn base_url(&self) -> Cow<'_, str> {
        (**self).base_url()
    }
}

impl<T: Transport + ?Sized> Transport for Arc<T> {
    fn send(&self, request: Request<'_>, timeouts: Timeouts) -> Result<Answer, TransportError> {
        (**self).send(request, timeouts)
    }

    fn base_url(&self) -> Cow<'_, str> {
        (**self).base_url()
    }
}
