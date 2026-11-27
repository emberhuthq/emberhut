//! A page of a room's chat (`GET /api/history`): older messages for "Show earlier", or what
//! came after a message the app already holds. The messages read as they do in the state's
//! `chat`, files and quotes included.

use std::sync::Arc;

use emberhut_api::chat::ChatPage;
use emberhut_core::message::HistoryQuery;
use emberhut_core::{Clock, RoomId};
use emberhut_db::{DbError, MessageRefStore, SnapshotStore};

use crate::services::chat::chat_lines;

/// Reads pages of a room's chat.
#[derive(Clone, Debug)]
pub struct HistoryService {
    snapshots: SnapshotStore,
    clock: Arc<dyn Clock>,
    refs: Option<MessageRefStore>,
}

impl HistoryService {
    /// A service over the snapshots' store, telling the messages' ages with `clock`.
    #[must_use]
    pub fn new(snapshots: SnapshotStore, clock: Arc<dyn Clock>) -> Self {
        Self {
            snapshots,
            clock,
            refs: None,
        }
    }

    /// The same service, putting the answers' chips (kept in `refs`) on their lines.
    #[must_use]
    pub fn with_refs(mut self, refs: MessageRefStore) -> Self {
        self.refs = Some(refs);
        self
    }

    /// The page of `room` (which must exist) that `query` asks for: at most 60 messages, the
    /// newest of the range, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the store fails.
    pub async fn page(&self, room: RoomId, query: HistoryQuery) -> Result<ChatPage, DbError> {
        let page = self.snapshots.history(room, query).await?;
        let mut chat = chat_lines(&page.chat, &page.quoted, &page.files, self.clock.now());
        if let Some(refs) = &self.refs {
            crate::services::refs::fill(refs, &mut chat).await?;
        }
        Ok(ChatPage { chat })
    }
}
