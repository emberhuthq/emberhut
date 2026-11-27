//! The fake's answers to the user's changes and the pages behind Settings: queued per route,
//! with a plain default once a queue is empty.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use emberhut_api::chrome::ChromeReply;
use emberhut_api::facts::FactsPage;
use emberhut_api::switches::SwitchPage;

use crate::backend::Error;

/// The queued answers.
#[derive(Debug, Default)]
pub(crate) struct SettingsReplies {
    settings: Mutex<VecDeque<Result<(), Error>>>,
    switches: Mutex<VecDeque<Result<SwitchPage, Error>>>,
    facts: Mutex<VecDeque<Result<FactsPage, Error>>>,
    chrome: Mutex<VecDeque<Result<ChromeReply, Error>>>,
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

impl SettingsReplies {
    /// Queues the answer to the next change (by default they go through).
    pub(crate) fn push_settings(&self, reply: Result<(), Error>) {
        push(&self.settings, reply);
    }

    /// Queues the answer to the next read or change of the switches (by default the server
    /// has none: 404).
    pub(crate) fn push_switches(&self, reply: Result<SwitchPage, Error>) {
        push(&self.switches, reply);
    }

    /// Queues the answer to the next read of the facts (by default there are none).
    pub(crate) fn push_facts(&self, reply: Result<FactsPage, Error>) {
        push(&self.facts, reply);
    }

    /// Queues the answer to the next Chrome change (by default no profile and no link).
    pub(crate) fn push_chrome(&self, reply: Result<ChromeReply, Error>) {
        push(&self.chrome, reply);
    }

    pub(super) fn settings(&self) -> Result<(), Error> {
        pop(&self.settings).unwrap_or(Ok(()))
    }

    pub(super) fn switches(&self) -> Result<SwitchPage, Error> {
        pop(&self.switches).unwrap_or_else(|| {
            Err(Error::Refused {
                status: Some(404),
                message: None,
            })
        })
    }

    pub(super) fn facts(&self) -> Result<FactsPage, Error> {
        pop(&self.facts).unwrap_or_else(|| Ok(FactsPage::default()))
    }

    pub(super) fn chrome(&self) -> Result<ChromeReply, Error> {
        pop(&self.chrome).unwrap_or_else(|| {
            Ok(ChromeReply {
                ok: true,
                chrome: Vec::new(),
                chrome_pairing: Vec::new(),
                chrome_latest: None,
                pairing: None,
            })
        })
    }
}
