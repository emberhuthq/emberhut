//! The fake's answers about the agent's provider: what is saved, a check, a save and a removal.
//! Queued per route; once a queue is empty nothing is saved, a check passes with no models, a
//! save keeps what was sent (a key by its last four characters), and a removal goes through.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use emberhut_api::agent_provider::{AgentProvider, ProviderKind, ProviderRequest, ProviderTest};

use crate::backend::Error;

/// The queued answers.
#[derive(Debug, Default)]
pub(crate) struct ProviderReplies {
    status: Mutex<VecDeque<Result<AgentProvider, Error>>>,
    tests: Mutex<VecDeque<Result<ProviderTest, Error>>>,
    saves: Mutex<VecDeque<Result<AgentProvider, Error>>>,
    forgets: Mutex<VecDeque<Result<AgentProvider, Error>>>,
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

impl ProviderReplies {
    /// Queues the answer to the next read of what is saved.
    pub(crate) fn push_status(&self, reply: Result<AgentProvider, Error>) {
        push(&self.status, reply);
    }

    /// Queues the answer to the next check.
    pub(crate) fn push_test(&self, reply: Result<ProviderTest, Error>) {
        push(&self.tests, reply);
    }

    /// Queues the answer to the next save.
    pub(crate) fn push_save(&self, reply: Result<AgentProvider, Error>) {
        push(&self.saves, reply);
    }

    /// Queues the answer to the next removal.
    pub(crate) fn push_forget(&self, reply: Result<AgentProvider, Error>) {
        push(&self.forgets, reply);
    }

    pub(super) fn status(&self) -> Result<AgentProvider, Error> {
        pop(&self.status).unwrap_or(Ok(AgentProvider::NONE))
    }

    pub(super) fn test(&self) -> Result<ProviderTest, Error> {
        pop(&self.tests).unwrap_or_else(|| {
            Ok(ProviderTest {
                checked: true,
                models: Vec::new(),
            })
        })
    }

    pub(super) fn save(&self, request: &ProviderRequest) -> Result<AgentProvider, Error> {
        pop(&self.saves).unwrap_or_else(|| Ok(kept(request)))
    }

    pub(super) fn forget(&self) -> Result<AgentProvider, Error> {
        pop(&self.forgets).unwrap_or(Ok(AgentProvider::NONE))
    }
}

/// What a server keeps of `request`, as it answers a save: the key by its last four
/// characters, and in use unless the provider needs the bridge to `OpenAI`'s API.
fn kept(request: &ProviderRequest) -> AgentProvider {
    let key = request.api_key.as_deref();
    let usable = !matches!(
        request.provider,
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible
    );
    AgentProvider {
        provider: Some(request.provider.clone()),
        key_set: key.is_some(),
        key_hint: key.map(|key| {
            key.chars()
                .skip(key.chars().count().saturating_sub(4))
                .collect()
        }),
        base_url: request.base_url.clone(),
        model: request.model.clone(),
        fast_model: request.fast_model.clone(),
        checked_at: None,
        usable,
        in_use: usable,
    }
}
