//! The fake's answers about where the assistant works and what it uses: the environments and
//! their actions, the account picked, the background jobs and their actions. Queued per route,
//! with a plain default once a queue is empty.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use emberhut_api::environments::EnvPage;
use emberhut_api::jobs::JobsPage;
use emberhut_api::usage::{AssistantAccount, UseAccountReply, UseAccountRequest};

use crate::backend::Error;

/// The queued answers.
#[derive(Debug, Default)]
pub(crate) struct WorkingReplies {
    environments: Mutex<VecDeque<Result<EnvPage, Error>>>,
    environment_actions: Mutex<VecDeque<Result<EnvPage, Error>>>,
    accounts: Mutex<VecDeque<Result<UseAccountReply, Error>>>,
    jobs: Mutex<VecDeque<Result<JobsPage, Error>>>,
    job_actions: Mutex<VecDeque<Result<(), Error>>>,
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

/// A server without the route.
fn not_found<T>() -> Result<T, Error> {
    Err(Error::Refused {
        status: Some(404),
        message: None,
    })
}

impl WorkingReplies {
    /// Queues the answer to the next read of the environments (by default the server has
    /// none: 404).
    pub(crate) fn push_environments(&self, reply: Result<EnvPage, Error>) {
        push(&self.environments, reply);
    }

    /// Queues the answer to the next environment's action (by default: 404).
    pub(crate) fn push_environment_action(&self, reply: Result<EnvPage, Error>) {
        push(&self.environment_actions, reply);
    }

    /// Queues the answer to the next pick of the assistant's account (by default it goes
    /// through, with no use to show).
    pub(crate) fn push_use_account(&self, reply: Result<UseAccountReply, Error>) {
        push(&self.accounts, reply);
    }

    /// Queues the answer to the next read of the jobs (by default: 404).
    pub(crate) fn push_jobs(&self, reply: Result<JobsPage, Error>) {
        push(&self.jobs, reply);
    }

    /// Queues the answer to the next job's Stop or Clear (by default it goes through).
    pub(crate) fn push_job_action(&self, reply: Result<(), Error>) {
        push(&self.job_actions, reply);
    }

    pub(super) fn environments(&self) -> Result<EnvPage, Error> {
        pop(&self.environments).unwrap_or_else(not_found)
    }

    pub(super) fn environment_action(&self) -> Result<EnvPage, Error> {
        pop(&self.environment_actions).unwrap_or_else(not_found)
    }

    /// The queued answer, else the choice as asked.
    pub(super) fn use_account(
        &self,
        request: &UseAccountRequest,
    ) -> Result<UseAccountReply, Error> {
        pop(&self.accounts).unwrap_or_else(|| {
            let pin = request.pinned().map(str::to_owned);
            Ok(UseAccountReply {
                ok: true,
                account: AssistantAccount {
                    on: pin.clone(),
                    mode: if pin.is_some() { "pinned" } else { "auto" }.to_owned(),
                    pin,
                    pin_out: false,
                    rooms: Vec::new(),
                    waits: None,
                },
                usage: Vec::new(),
            })
        })
    }

    pub(super) fn jobs(&self) -> Result<JobsPage, Error> {
        pop(&self.jobs).unwrap_or_else(not_found)
    }

    pub(super) fn job_action(&self) -> Result<(), Error> {
        pop(&self.job_actions).unwrap_or(Ok(()))
    }
}
