//! The fake's release feed and payloads: queued, with "nothing published" and "not there" once
//! a queue is empty. A payload is written as a server would send it, cut off past the cap.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Mutex, PoisonError};

use emberhut_api::releases::ReleaseFeed;
use sha2::{Digest as _, Sha256};

use crate::backend::{Error, PayloadDigest, TooLarge};

/// The queued answers.
#[derive(Debug, Default)]
pub(crate) struct ReleaseReplies {
    feeds: Mutex<VecDeque<Result<Option<ReleaseFeed>, Error>>>,
    payloads: Mutex<VecDeque<Result<Vec<u8>, Error>>>,
}

impl ReleaseReplies {
    /// Queues the answer to the next read of the feed (by default nothing is published).
    pub(crate) fn push_feed(&self, reply: Result<Option<ReleaseFeed>, Error>) {
        self.feeds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next payload download (by default it isn't there).
    pub(crate) fn push_payload(&self, reply: Result<Vec<u8>, Error>) {
        self.payloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn feed(&self) -> Result<Option<ReleaseFeed>, Error> {
        self.feeds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(None))
    }

    pub(super) fn payload(&self, to: &mut dyn Write, max_len: u64) -> Result<PayloadDigest, Error> {
        let bytes = self
            .payloads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| {
                Err(Error::Refused {
                    status: Some(404),
                    message: None,
                })
            })?;
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let kept = usize::try_from(max_len).map_or(bytes.len(), |cap| cap.min(bytes.len()));
        to.write_all(&bytes[..kept])
            .map_err(|error| Error::Local(Box::new(error)))?;
        if size > max_len {
            return Err(Error::Local(Box::new(TooLarge {
                size,
                limit: max_len,
            })));
        }
        Ok(PayloadDigest {
            size,
            sha256: Sha256::digest(&bytes).into(),
        })
    }
}
