//! The workers' tokens: the bearer a worker's session calls the worker endpoint (and an ultra
//! lead's hook calls the gate) with. Each task's worker has one, made when its first session
//! starts and the same after; a token names its task, and the tools it reaches act on that
//! task. Held in memory only, as their hashes, like the rooms' tokens: a worker runs as a
//! child of the server, so a restart ends it and its token with it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use emberhut_core::task::TaskId;
use emberhut_core::{RandomError, RandomSource, SecretHash, Token};

/// The workers' tokens. Cheap to clone; clones share the tokens.
#[derive(Clone, Debug)]
pub struct WorkerTokens {
    random: Arc<dyn RandomSource>,
    inner: Arc<Mutex<Tokens>>,
}

#[derive(Debug, Default)]
struct Tokens {
    by_task: HashMap<TaskId, Token>,
    by_hash: HashMap<[u8; 32], TaskId>,
}

impl WorkerTokens {
    /// No tokens yet; new ones get their randomness from `random`.
    #[must_use]
    pub fn new(random: Arc<dyn RandomSource>) -> Self {
        Self {
            random,
            inner: Arc::default(),
        }
    }

    /// `task`'s token, made now if it has none.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when a new token's randomness fails.
    pub fn for_task(&self, task: TaskId) -> Result<Token, RandomError> {
        let mut tokens = self.tokens();
        if let Some(token) = tokens.by_task.get(&task) {
            return Ok(token.clone());
        }
        let token = Token::generate(self.random.as_ref())?;
        tokens.by_hash.insert(*token.hash().as_bytes(), task);
        tokens.by_task.insert(task, token.clone());
        Ok(token)
    }

    /// The task whose token `presented` is, if it is one.
    #[must_use]
    pub fn task_of(&self, presented: &str) -> Option<TaskId> {
        let hash = SecretHash::of(presented);
        self.tokens().by_hash.get(hash.as_bytes()).copied()
    }

    fn tokens(&self) -> MutexGuard<'_, Tokens> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use emberhut_core::OsRandom;

    use super::*;

    #[test]
    fn a_task_keeps_its_token_and_the_token_names_the_task() {
        let tokens = WorkerTokens::new(Arc::new(OsRandom));
        let one = tokens.for_task(TaskId::new(1)).unwrap();
        let two = tokens.for_task(TaskId::new(2)).unwrap();
        assert_eq!(
            tokens.for_task(TaskId::new(1)).unwrap().as_str(),
            one.as_str()
        );
        assert_eq!(tokens.task_of(two.as_str()), Some(TaskId::new(2)));
        assert_eq!(tokens.task_of("not a token"), None);
    }
}
