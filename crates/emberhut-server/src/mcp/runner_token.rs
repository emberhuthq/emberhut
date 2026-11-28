//! The runner sessions' token: the bearer every runner session calls `POST`
//! [`emberhut_api::paths::MCP_RUNNER`] with, a task's plan and the task and nothing else.
//!
//! One token for the whole server process, made the first time the runners' shared MCP list
//! hands it out (`GET /api/runner/mcp`, which only a runner's own token reaches), and the same
//! after. It names no room and no task: which task a call may write is said by the session its
//! request names. Held in memory only, as its hash next to the text handed out, like the rooms'
//! tokens; after a restart the list hands out a new one, and the old one is a stranger's.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use emberhut_core::{RandomError, RandomSource, SecretHash, Token};

/// The runner sessions' token. Cheap to clone; clones share the token.
#[derive(Clone, Debug)]
pub struct RunnerToken {
    random: Arc<dyn RandomSource>,
    inner: Arc<Mutex<Option<Held>>>,
}

/// The token handed out, and its hash, which a presented token is checked against.
#[derive(Debug)]
struct Held {
    token: Token,
    hash: [u8; 32],
}

impl RunnerToken {
    /// No token yet; it gets its randomness from `random`.
    #[must_use]
    pub fn new(random: Arc<dyn RandomSource>) -> Self {
        Self {
            random,
            inner: Arc::default(),
        }
    }

    /// The token, made now if there is none yet: what the runners' sessions send as
    /// `Authorization: Bearer <token>`.
    ///
    /// # Errors
    ///
    /// [`RandomError`] when the new token's randomness fails.
    pub fn get(&self) -> Result<Token, RandomError> {
        let mut held = self.held();
        if let Some(held) = held.as_ref() {
            return Ok(held.token.clone());
        }
        let token = Token::generate(self.random.as_ref())?;
        *held = Some(Held {
            hash: *token.hash().as_bytes(),
            token: token.clone(),
        });
        Ok(token)
    }

    /// `presented` is the token (compared by hash, never as text).
    #[must_use]
    pub fn is(&self, presented: &str) -> bool {
        let hash = SecretHash::of(presented);
        self.held()
            .as_ref()
            .is_some_and(|held| &held.hash == hash.as_bytes())
    }

    fn held(&self) -> MutexGuard<'_, Option<Held>> {
        // Every change leaves the value whole, so a panic elsewhere left it usable.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use emberhut_core::OsRandom;

    use super::*;

    #[test]
    fn the_token_is_made_once_and_only_it_is_known() {
        let token = RunnerToken::new(Arc::new(OsRandom));
        assert!(!token.is(""), "no token before one is handed out");
        let first = token.get().unwrap();
        assert_eq!(token.get().unwrap().as_str(), first.as_str());
        assert!(token.is(first.as_str()));
        assert!(!token.is("not the token"));
        let restarted = RunnerToken::new(Arc::new(OsRandom));
        let second = restarted.get().unwrap();
        assert_ne!(second.as_str(), first.as_str());
        assert!(
            !restarted.is(first.as_str()),
            "an old process's token is a stranger's"
        );
    }
}
