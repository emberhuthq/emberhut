//! The chips under the assistant's answers (what each is about), kept per message as the JSON
//! list the chat shows.

use std::collections::BTreeMap;

use emberhut_core::MessageId;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::DbError;
use crate::entity::message_ref;

/// Writes and reads the answers' chips.
#[derive(Clone, Debug)]
pub struct MessageRefStore {
    conn: DatabaseConnection,
}

impl MessageRefStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Keeps `refs` (a JSON list) as `message`'s chips, in place of any it had.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], also when the message does not exist.
    pub async fn set(&self, message: MessageId, refs: String) -> Result<(), DbError> {
        message_ref::Entity::insert(message_ref::ActiveModel {
            message_id: Set(message.get()),
            refs: Set(refs),
        })
        .on_conflict(
            OnConflict::column(message_ref::Column::MessageId)
                .update_column(message_ref::Column::Refs)
                .to_owned(),
        )
        .exec_without_returning(&self.conn)
        .await?;
        Ok(())
    }

    /// The chips of each of these messages that has some.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn of_messages(
        &self,
        messages: &[MessageId],
    ) -> Result<BTreeMap<MessageId, String>, DbError> {
        if messages.is_empty() {
            return Ok(BTreeMap::new());
        }
        Ok(message_ref::Entity::find()
            .filter(message_ref::Column::MessageId.is_in(messages.iter().map(|id| id.get())))
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| (MessageId::new(row.message_id), row.refs))
            .collect())
    }
}
