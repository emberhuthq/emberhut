//! The fake's answers about the assistant's browser: queued per route, with a plain default
//! once a queue is empty.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use emberhut_api::browser::{BrowserInfo, BrowserSession, StreamMode};

use crate::backend::Error;

/// The queued answers.
#[derive(Debug, Default)]
pub(crate) struct BrowserReplies {
    sessions: Mutex<VecDeque<Result<BrowserSession, Error>>>,
    inputs: Mutex<VecDeque<Result<BrowserInfo, Error>>>,
    uploads: Mutex<VecDeque<Result<BrowserInfo, Error>>>,
}

fn push<T>(queue: &Mutex<VecDeque<T>>, reply: T) {
    queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push_back(reply);
}

fn pop<T>(queue: &Mutex<VecDeque<T>>) -> Option<T> {
    queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
}

/// A view-only session on an example address.
#[must_use]
pub(crate) fn session(mode: StreamMode) -> BrowserSession {
    BrowserSession {
        url: Some("https://example.org/".to_owned()),
        user: "viewer".to_owned(),
        password: "example".to_owned(),
        mode,
        screen: None,
    }
}

impl BrowserReplies {
    /// Queues the answer to the next session (by default a view-only one).
    pub(crate) fn push_session(&self, reply: Result<BrowserSession, Error>) {
        push(&self.sessions, reply);
    }

    /// Queues the answer to the next input (by default a browser nobody drives).
    pub(crate) fn push_input(&self, reply: Result<BrowserInfo, Error>) {
        push(&self.inputs, reply);
    }

    /// Queues the answer to the next upload (by default it goes in).
    pub(crate) fn push_upload(&self, reply: Result<BrowserInfo, Error>) {
        push(&self.uploads, reply);
    }

    pub(super) fn session(&self) -> Result<BrowserSession, Error> {
        pop(&self.sessions).unwrap_or_else(|| Ok(session(StreamMode::View)))
    }

    pub(super) fn input(&self) -> Result<BrowserInfo, Error> {
        pop(&self.inputs).unwrap_or_else(|| Ok(BrowserInfo::default()))
    }

    pub(super) fn upload(&self) -> Result<BrowserInfo, Error> {
        pop(&self.uploads).unwrap_or_else(|| Ok(BrowserInfo::default()))
    }
}
