//! The fake's runner sessions: transcripts by session, scripted live streams, and the answers to
//! messages, Stop, settings, new sessions, folders, branches and external threads' steps, queued per route with
//! a plain default once a queue is empty.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};

use emberhut_api::ids::{SessionId, TurnId};
use emberhut_api::runner::actions::SentToSession;
use emberhut_api::runner::new_session::{BranchList, FolderList, OpenedSession};
use emberhut_api::runner::transcript::{SessionLive, Transcript};
use emberhut_api::threads::ThreadActivity;

use crate::backend::Error;

/// The queued answers.
#[derive(Debug, Default)]
pub(crate) struct SessionReplies {
    /// The transcripts, by session (any other is the runner not answering).
    transcripts: Mutex<HashMap<SessionId, Transcript>>,
    /// What each live stream sends in turn, before it waits for `stop` (then it sends nothing).
    streams: Mutex<VecDeque<Result<Vec<SessionLive>, Error>>>,
    sends: Mutex<VecDeque<Result<SentToSession, Error>>>,
    /// Stop, Stop this turn and settings, in turn.
    actions: Mutex<VecDeque<Result<(), Error>>>,
    opens: Mutex<VecDeque<Result<OpenedSession, Error>>>,
    folders: Mutex<VecDeque<Result<FolderList, Error>>>,
    branches: Mutex<VecDeque<Result<BranchList, Error>>>,
    t3: Mutex<VecDeque<Result<ThreadActivity, Error>>>,
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

/// The runner not answering, as the server says it.
fn no_runner() -> Error {
    Error::Refused {
        status: Some(502),
        message: Some("the runner didn't answer".to_owned()),
    }
}

impl SessionReplies {
    /// Serves `transcript` as its session's (replacing one served before).
    pub(crate) fn put_transcript(&self, transcript: Transcript) {
        self.transcripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(transcript.session.id.clone(), transcript);
    }

    /// Queues what the next live stream sends (or its failure).
    pub(crate) fn push_stream(&self, events: Result<Vec<SessionLive>, Error>) {
        push(&self.streams, events);
    }

    /// Queues the answer to the next message (by default it goes through, not queued).
    pub(crate) fn push_send(&self, reply: Result<SentToSession, Error>) {
        push(&self.sends, reply);
    }

    /// Queues the answer to the next Stop, Stop this turn or settings change (by default they
    /// go through).
    pub(crate) fn push_action(&self, reply: Result<(), Error>) {
        push(&self.actions, reply);
    }

    /// Queues the answer to the next new session (by default `r-new` opens).
    pub(crate) fn push_open(&self, reply: Result<OpenedSession, Error>) {
        push(&self.opens, reply);
    }

    /// Queues the answer to the next folder listing (by default an empty one).
    pub(crate) fn push_folders(&self, reply: Result<FolderList, Error>) {
        push(&self.folders, reply);
    }

    /// Queues the answer to the next branch listing (by default the runner doesn't answer).
    pub(crate) fn push_branches(&self, reply: Result<BranchList, Error>) {
        push(&self.branches, reply);
    }

    /// Queues the answer to the next external thread's steps (by default the app doesn't answer).
    pub(crate) fn push_t3(&self, reply: Result<ThreadActivity, Error>) {
        push(&self.t3, reply);
    }

    pub(super) fn transcript(&self, id: &SessionId) -> Result<Transcript, Error> {
        self.transcripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
            .ok_or_else(no_runner)
    }

    pub(super) fn stream(&self) -> Result<Vec<SessionLive>, Error> {
        pop(&self.streams).unwrap_or_else(|| Ok(Vec::new()))
    }

    pub(super) fn send(&self) -> Result<SentToSession, Error> {
        pop(&self.sends).unwrap_or_else(|| {
            Ok(SentToSession {
                ok: true,
                turn: None,
                queued: false,
            })
        })
    }

    pub(super) fn action(&self) -> Result<(), Error> {
        pop(&self.actions).unwrap_or(Ok(()))
    }

    pub(super) fn open(&self) -> Result<OpenedSession, Error> {
        pop(&self.opens).unwrap_or_else(|| {
            Ok(OpenedSession {
                ok: true,
                id: SessionId::from("r-new"),
                turn: Some(TurnId::from("r-new:1")),
                session: None,
            })
        })
    }

    pub(super) fn folders(&self) -> Result<FolderList, Error> {
        pop(&self.folders).unwrap_or_else(|| Ok(FolderList::default()))
    }

    pub(super) fn branches(&self) -> Result<BranchList, Error> {
        pop(&self.branches).unwrap_or_else(|| Err(no_runner()))
    }

    pub(super) fn t3(&self) -> Result<ThreadActivity, Error> {
        pop(&self.t3).unwrap_or_else(|| Err(no_runner()))
    }
}
