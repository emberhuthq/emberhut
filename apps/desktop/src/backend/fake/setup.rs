//! The fake's answers about setup: the checklist (`GET /api/setup`) and whether the server
//! answers its health check. Queued; once the queue is empty the checklist says the ember is
//! lit, and the health check goes unanswered unless told otherwise.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use emberhut_api::setup::{SetupItem, SetupItemId, SetupPage, SetupState};

use crate::backend::Error;

/// The queued answers.
#[derive(Debug, Default)]
pub(crate) struct SetupReplies {
    pages: Mutex<VecDeque<Result<SetupPage, Error>>>,
    answers: AtomicBool,
}

impl SetupReplies {
    /// Queues the answer to the next read of the checklist.
    pub(crate) fn push_page(&self, reply: Result<SetupPage, Error>) {
        self.pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    /// Whether the server answers its health check from now on.
    pub(crate) fn set_answers(&self, answers: bool) {
        self.answers.store(answers, Ordering::SeqCst);
    }

    pub(super) fn page(&self) -> Result<SetupPage, Error> {
        self.pages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Ok(page(&[(SetupItemId::Provider, SetupState::Ready)])))
    }

    pub(super) fn answers(&self) -> bool {
        self.answers.load(Ordering::SeqCst)
    }
}

/// A checklist of `items`, each with its state and no detail.
pub(crate) fn page(items: &[(SetupItemId, SetupState)]) -> SetupPage {
    SetupPage {
        items: items
            .iter()
            .map(|(id, state)| SetupItem {
                id: id.clone(),
                state: state.clone(),
                detail: None,
            })
            .collect(),
    }
}
