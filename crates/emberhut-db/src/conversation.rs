//! The conversation as the assistant reads it back: what the user and the assistant said, in
//! every room (the server's own events left out), for the chat log and recall.

use std::collections::HashMap;

use emberhut_core::{MessageId, RoomId};
use jiff::Timestamp;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::DbError;
use crate::entity::message::{self, Author};
use crate::entity::room;
use crate::time::{instant, millis};

/// One message of the conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Said {
    /// The message's id.
    pub id: MessageId,
    /// Its room.
    pub room: RoomId,
    /// The room's name.
    pub room_name: String,
    /// The user said it (else the assistant did).
    pub mine: bool,
    /// What was said (never empty).
    pub text: String,
    /// When.
    pub at: Timestamp,
}

/// Reads the conversation back.
#[derive(Clone, Debug)]
pub struct ConversationStore {
    conn: DatabaseConnection,
}

impl ConversationStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// What was said after `after`, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn said_since(&self, after: Timestamp) -> Result<Vec<Said>, DbError> {
        let rows = said()
            .filter(message::Column::CreatedAt.gt(millis(after)))
            .order_by_asc(message::Column::CreatedAt)
            .order_by_asc(message::Column::Id)
            .all(&self.conn)
            .await?;
        self.named(rows).await
    }

    /// At most `limit` messages said after the message `after` (none: from the first), in
    /// order.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn said_after(
        &self,
        after: Option<MessageId>,
        limit: u64,
    ) -> Result<Vec<Said>, DbError> {
        let mut query = said();
        if let Some(after) = after {
            query = query.filter(message::Column::Id.gt(after.get()));
        }
        let rows = query
            .order_by_asc(message::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?;
        self.named(rows).await
    }

    async fn named(&self, rows: Vec<message::Model>) -> Result<Vec<Said>, DbError> {
        let names: HashMap<i64, String> = room::Entity::find()
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|room| (room.id, room.name))
            .collect();
        rows.into_iter()
            .map(|row| {
                Ok(Said {
                    id: MessageId::new(row.id),
                    room: RoomId::new(row.room_id),
                    room_name: names.get(&row.room_id).cloned().unwrap_or_default(),
                    mine: row.author == Author::User,
                    at: instant(row.created_at)?,
                    text: row.text,
                })
            })
            .collect()
    }
}

/// The user's and the assistant's messages with something in them.
fn said() -> sea_orm::Select<message::Entity> {
    message::Entity::find()
        .filter(message::Column::Author.is_in([Author::User, Author::Assistant]))
        .filter(message::Column::Text.ne(""))
}
