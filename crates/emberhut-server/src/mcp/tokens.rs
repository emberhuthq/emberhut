//! The rooms' agent tokens: the bearer a room's agent session calls the MCP endpoint with.
//!
//! Each room has one token, made the first time its session asks for it and the same after,
//! so the session's MCP configuration stays valid across its turns. A token names its room:
//! the tools it reaches act in that room. Tokens are held in memory only, as their hashes
//! (looked up by hash, never compared as text) next to the text handed out; after a restart a
//! room's next session start asks for a new one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use emberhut_core::{RandomError, RandomSource, RoomId, SecretHash, Token};

/// The rooms' agent tokens. Cheap to clone; clones share the tokens.
#[derive(Clone, Debug)]
pub struct AgentTokens {
    random: Arc<dyn RandomSource>,
    inner: Arc<Mutex<Tokens>>,
}

#[derive(Debug, Default)]
struct Tokens {
    by_room: HashMap<RoomId, Token>,
    by_hash: HashMap<[u8; 32], RoomId>,
}

impl AgentTokens {
    /// No tokens yet; new ones get their randomness from `random`.
    #[must_use]
    pub fn new(random: Arc<dyn RandomSource>) -> Self {
        Self {
            random,
            inner: Arc::default(),
        }
    }

    /// `room`'s token, made now if it has none: what its agent session sends as
    /// `Authorization: Bearer <token>` to `POST` [`emberhut_api::paths::MCP_ROOM`].
    ///
    /// # Errors
    ///
    /// [`RandomError`] when a new token's randomness fails.
    pub fn for_room(&self, room: RoomId) -> Result<Token, RandomError> {
        let mut tokens = self.tokens();
        if let Some(token) = tokens.by_room.get(&room) {
            return Ok(token.clone());
        }
        let token = Token::generate(self.random.as_ref())?;
        tokens.by_hash.insert(*token.hash().as_bytes(), room);
        tokens.by_room.insert(room, token.clone());
        Ok(token)
    }

    /// The room whose token `presented` is, if it is one.
    #[must_use]
    pub fn room_of(&self, presented: &str) -> Option<RoomId> {
        let hash = SecretHash::of(presented);
        self.tokens().by_hash.get(hash.as_bytes()).copied()
    }

    /// Stops `room`'s token, if it has one (a room deleted, a session ended for good).
    pub fn revoke(&self, room: RoomId) {
        let mut tokens = self.tokens();
        if let Some(token) = tokens.by_room.remove(&room) {
            tokens.by_hash.remove(token.hash().as_bytes());
        }
    }

    fn tokens(&self) -> MutexGuard<'_, Tokens> {
        // Every change leaves the maps consistent, so a panic elsewhere left them usable.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use emberhut_core::OsRandom;

    use super::*;

    #[test]
    fn a_room_keeps_its_token_and_the_token_names_the_room() {
        let tokens = AgentTokens::new(Arc::new(OsRandom));
        let main = tokens.for_room(RoomId::MAIN).unwrap();
        let other = tokens.for_room(RoomId::new(2)).unwrap();
        assert_eq!(
            tokens.for_room(RoomId::MAIN).unwrap().as_str(),
            main.as_str()
        );
        assert_ne!(main.as_str(), other.as_str());
        assert_eq!(tokens.room_of(main.as_str()), Some(RoomId::MAIN));
        assert_eq!(tokens.room_of(other.as_str()), Some(RoomId::new(2)));
        assert_eq!(tokens.room_of("not a token"), None);
    }

    #[test]
    fn a_revoked_token_names_no_room() {
        let tokens = AgentTokens::new(Arc::new(OsRandom));
        let main = tokens.for_room(RoomId::MAIN).unwrap();
        tokens.revoke(RoomId::MAIN);
        tokens.revoke(RoomId::MAIN);
        assert_eq!(tokens.room_of(main.as_str()), None);
        assert_ne!(
            tokens.for_room(RoomId::MAIN).unwrap().as_str(),
            main.as_str()
        );
    }
}
