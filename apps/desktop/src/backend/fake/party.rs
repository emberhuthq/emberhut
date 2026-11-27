//! The fake's Party and Dark room: answers queued per route, with a plain default once a
//! queue is empty, and the events their streams send.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use emberhut_api::dark::{DarkEvent, DarkRoom};
use emberhut_api::party::{PartiesPage, Party, PartyEvent};

use crate::backend::Error;

/// What a Dark answer streams, then how it ends.
type Answer = (Vec<DarkEvent>, Result<(), Error>);

/// The queued answers and the streams' events.
#[derive(Debug, Default)]
pub(crate) struct PartyReplies {
    parties: Mutex<VecDeque<Result<PartiesPage, Error>>>,
    started: Mutex<VecDeque<Result<Party, Error>>>,
    acts: Mutex<VecDeque<Result<Option<Party>, Error>>>,
    /// What each party stream sends, in turn, before it ends.
    live: Mutex<VecDeque<Vec<PartyEvent>>>,
    /// How each party stream ends, in turn (then: the server closed it).
    live_ends: Mutex<VecDeque<Result<(), Error>>>,
    /// While set, a Dark answer waits for a word on it (or 5 s) before it streams.
    answer_gate: Mutex<Option<Receiver<()>>>,
    dark: Mutex<VecDeque<Result<DarkRoom, Error>>>,
    /// What each Dark answer streams, in turn, then how it ends.
    answers: Mutex<VecDeque<Answer>>,
    clears: Mutex<VecDeque<Result<(), Error>>>,
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

/// The answer of a server without the route.
fn not_found<T>() -> Result<T, Error> {
    Err(Error::Refused {
        status: Some(404),
        message: None,
    })
}

impl PartyReplies {
    /// Queues the answer to the next read of the parties (by default: 404).
    pub(crate) fn push_parties(&self, reply: Result<PartiesPage, Error>) {
        push(&self.parties, reply);
    }

    /// Queues the answer to the next new party (by default: 404).
    pub(crate) fn push_started(&self, reply: Result<Party, Error>) {
        push(&self.started, reply);
    }

    /// Queues the answer to the next control on a party (by default: no party).
    pub(crate) fn push_act(&self, reply: Result<Option<Party>, Error>) {
        push(&self.acts, reply);
    }

    /// Queues what the next party stream sends (by default: nothing).
    pub(crate) fn push_live(&self, events: Vec<PartyEvent>) {
        push(&self.live, events);
    }

    /// Queues how the next party stream ends (by default: the server closed it).
    pub(crate) fn push_live_end(&self, end: Result<(), Error>) {
        push(&self.live_ends, end);
    }

    /// Holds every Dark answer until the returned sender says to go on (or 5 s pass), as an
    /// answer the model takes long to write.
    pub(crate) fn hold_answers(&self) -> Sender<()> {
        let (go, gate) = mpsc::channel();
        *self
            .answer_gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(gate);
        go
    }

    /// Queues the answer to the next read of the Dark room (by default: 404).
    pub(crate) fn push_dark(&self, reply: Result<DarkRoom, Error>) {
        push(&self.dark, reply);
    }

    /// Queues what the next Dark answer streams and how it ends (by default: nothing, fine).
    pub(crate) fn push_answer(&self, events: Vec<DarkEvent>, end: Result<(), Error>) {
        push(&self.answers, (events, end));
    }

    /// Queues the answer to the next clearing of the Dark room (by default it goes through).
    pub(crate) fn push_clear(&self, reply: Result<(), Error>) {
        push(&self.clears, reply);
    }

    pub(super) fn parties(&self) -> Result<PartiesPage, Error> {
        pop(&self.parties).unwrap_or_else(not_found)
    }

    pub(super) fn started(&self) -> Result<Party, Error> {
        pop(&self.started).unwrap_or_else(not_found)
    }

    pub(super) fn act(&self) -> Result<Option<Party>, Error> {
        pop(&self.acts).unwrap_or(Ok(None))
    }

    pub(super) fn live(&self) -> Vec<PartyEvent> {
        pop(&self.live).unwrap_or_default()
    }

    pub(super) fn live_end(&self) -> Result<(), Error> {
        pop(&self.live_ends).unwrap_or(Ok(()))
    }

    /// Waits while the answers are held ([`PartyReplies::hold_answers`]).
    pub(super) fn wait_for_answer(&self) {
        let gate = self
            .answer_gate
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(gate) = gate
            && gate.recv_timeout(Duration::from_secs(5)).is_err()
        {
            log::warn!("a held Dark answer went on by itself");
        }
    }

    pub(super) fn dark(&self) -> Result<DarkRoom, Error> {
        pop(&self.dark).unwrap_or_else(not_found)
    }

    pub(super) fn answer(&self) -> Answer {
        pop(&self.answers).unwrap_or_else(|| (Vec::new(), Ok(())))
    }

    pub(super) fn clear(&self) -> Result<(), Error> {
        pop(&self.clears).unwrap_or(Ok(()))
    }
}
