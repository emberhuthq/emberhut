//! The fake's answers for the account: the server on this computer signing a link statement,
//! and a pairing from a link. Queued; once a queue is empty a link statement is refused (404,
//! a server without the route) and a pairing goes through.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use emberhut_api::connect::SignedLink;

use crate::backend::Error;

/// The queued answers.
#[derive(Debug, Default)]
pub(crate) struct AccountReplies {
    link_statements: Mutex<VecDeque<Result<SignedLink, Error>>>,
    pairings: Mutex<VecDeque<Result<(), Error>>>,
}

impl AccountReplies {
    /// Queues the answer to the next link statement asked for.
    pub(crate) fn push_link_statement(&self, reply: Result<SignedLink, Error>) {
        self.link_statements
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Queues the answer to the next pairing from a link.
    pub(crate) fn push_pairing(&self, reply: Result<(), Error>) {
        self.pairings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn link_statement(&self) -> Result<SignedLink, Error> {
        self.link_statements
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Err(Error::Refused {
                status: Some(404),
                message: None,
            }))
    }

    pub(super) fn pairing(&self) -> Result<(), Error> {
        self.pairings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(()))
    }
}
