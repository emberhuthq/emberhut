//! Messages.

use emberhut_core::message::{Author, HistoryQuery, Message, NewMessage, PAGE_SIZE};
use emberhut_core::{FileId, MessageId, RoomId};
use jiff::Timestamp;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};

use crate::entity::message;
use crate::time::{instant, millis};
use crate::{DbError, begin_write, files};

/// Reads and writes messages.
#[derive(Clone, Debug)]
pub struct MessageStore {
    conn: DatabaseConnection,
}

impl MessageStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Stores `new`, said at `now`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], also when its room or the message it replies to does not exist.
    pub async fn append(&self, new: NewMessage, now: Timestamp) -> Result<Message, DbError> {
        let row = message::ActiveModel {
            room_id: Set(new.room.get()),
            author: Set(new.author.into()),
            text: Set(new.text),
            created_at: Set(millis(now)),
            reply_to_id: Set(new.reply_to.map(MessageId::get)),
            ..Default::default()
        }
        .insert(&self.conn)
        .await?;
        from_row(row)
    }

    /// Stores `new`, said at `now`, with `files` (stored already) attached in this order, in one
    /// transaction.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], also when its room, the message it replies to or one of the files
    /// does not exist; then nothing is stored.
    pub async fn append_with_files(
        &self,
        new: NewMessage,
        files: &[FileId],
        now: Timestamp,
    ) -> Result<Message, DbError> {
        let txn = begin_write(&self.conn).await?;
        let row = message::ActiveModel {
            room_id: Set(new.room.get()),
            author: Set(new.author.into()),
            text: Set(new.text),
            created_at: Set(millis(now)),
            reply_to_id: Set(new.reply_to.map(MessageId::get)),
            ..Default::default()
        }
        .insert(&txn)
        .await?;
        files::attach(&txn, MessageId::new(row.id), files).await?;
        txn.commit().await?;
        from_row(row)
    }

    /// A page of `room`'s history, oldest first (see [`HistoryQuery`]).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn page(&self, room: RoomId, query: HistoryQuery) -> Result<Vec<Message>, DbError> {
        page(&self.conn, room, query).await
    }

    /// The message with this id, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn get(&self, id: MessageId) -> Result<Option<Message>, DbError> {
        message::Entity::find_by_id(id.get())
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// The messages with these ids that exist, in id order.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn get_many(&self, ids: &[MessageId]) -> Result<Vec<Message>, DbError> {
        get_many(&self.conn, ids).await
    }

    /// The last thing said in `room`, by the person or the assistant (the server's own event
    /// lines are left out).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn last_said(&self, room: RoomId) -> Result<Option<Message>, DbError> {
        last_said(&self.conn, room).await
    }

    /// How many of the assistant's messages in `room` came after `seen_through` (all of them
    /// when the room was never seen).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn count_unread(
        &self,
        room: RoomId,
        seen_through: Option<MessageId>,
    ) -> Result<u64, DbError> {
        count_unread(&self.conn, room, seen_through).await
    }

    /// Whether `room` has messages older than `message`, for a "show earlier" control.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn has_before(&self, room: RoomId, message: MessageId) -> Result<bool, DbError> {
        has_before(&self.conn, room, message).await
    }

    /// What the person and the assistant said in `room` after `since` (text only: event lines
    /// and messages that are files alone are left out), the newest `limit`, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn said_since(
        &self,
        room: RoomId,
        since: Timestamp,
        limit: u64,
    ) -> Result<Vec<Message>, DbError> {
        let mut said = message::Entity::find()
            .filter(message::Column::RoomId.eq(room.get()))
            .filter(message::Column::Author.ne(message::Author::Event))
            .filter(message::Column::Text.ne(""))
            .filter(message::Column::CreatedAt.gt(millis(since)))
            .order_by_desc(message::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect::<Result<Vec<_>, _>>()?;
        said.reverse();
        Ok(said)
    }

    /// Everything said in `room` after `since`, the server's events included: the newest
    /// `limit` messages with text, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn conversation_since(
        &self,
        room: RoomId,
        since: Timestamp,
        limit: u64,
    ) -> Result<Vec<Message>, DbError> {
        let mut said = message::Entity::find()
            .filter(message::Column::RoomId.eq(room.get()))
            .filter(message::Column::Text.ne(""))
            .filter(message::Column::CreatedAt.gt(millis(since)))
            .order_by_desc(message::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect::<Result<Vec<_>, _>>()?;
        said.reverse();
        Ok(said)
    }

    /// The messages of every room after `since` whose text contains `needle`, the newest
    /// `limit`, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn naming(
        &self,
        needle: &str,
        since: Timestamp,
        limit: u64,
    ) -> Result<Vec<Message>, DbError> {
        let mut found = message::Entity::find()
            .filter(message::Column::Text.contains(needle))
            .filter(message::Column::CreatedAt.gt(millis(since)))
            .order_by_desc(message::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect::<Result<Vec<_>, _>>()?;
        found.reverse();
        Ok(found)
    }
}

/// [`MessageStore::page`] on `conn`.
pub(crate) async fn page(
    conn: &impl ConnectionTrait,
    room: RoomId,
    query: HistoryQuery,
) -> Result<Vec<Message>, DbError> {
    let mut select = message::Entity::find().filter(message::Column::RoomId.eq(room.get()));
    if let Some(before) = query.before {
        select = select.filter(message::Column::Id.lt(before.get()));
    }
    if let Some(after) = query.after {
        select = select.filter(message::Column::Id.gt(after.get()));
    }
    let mut page = select
        .order_by_desc(message::Column::Id)
        .limit(PAGE_SIZE)
        .all(conn)
        .await?
        .into_iter()
        .map(from_row)
        .collect::<Result<Vec<_>, _>>()?;
    page.reverse();
    Ok(page)
}

/// [`MessageStore::get_many`] on `conn`.
pub(crate) async fn get_many(
    conn: &impl ConnectionTrait,
    ids: &[MessageId],
) -> Result<Vec<Message>, DbError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    message::Entity::find()
        .filter(message::Column::Id.is_in(ids.iter().map(|id| id.get())))
        .order_by_asc(message::Column::Id)
        .all(conn)
        .await?
        .into_iter()
        .map(from_row)
        .collect()
}

/// [`MessageStore::last_said`] on `conn`.
pub(crate) async fn last_said(
    conn: &impl ConnectionTrait,
    room: RoomId,
) -> Result<Option<Message>, DbError> {
    message::Entity::find()
        .filter(message::Column::RoomId.eq(room.get()))
        .filter(message::Column::Author.ne(message::Author::Event))
        .order_by_desc(message::Column::Id)
        .one(conn)
        .await?
        .map(from_row)
        .transpose()
}

/// [`MessageStore::count_unread`] on `conn`.
pub(crate) async fn count_unread(
    conn: &impl ConnectionTrait,
    room: RoomId,
    seen_through: Option<MessageId>,
) -> Result<u64, DbError> {
    let mut unread = message::Entity::find()
        .filter(message::Column::RoomId.eq(room.get()))
        .filter(message::Column::Author.eq(message::Author::Assistant));
    if let Some(seen) = seen_through {
        unread = unread.filter(message::Column::Id.gt(seen.get()));
    }
    Ok(unread.count(conn).await?)
}

/// [`MessageStore::has_before`] on `conn`.
pub(crate) async fn has_before(
    conn: &impl ConnectionTrait,
    room: RoomId,
    message: MessageId,
) -> Result<bool, DbError> {
    let older = message::Entity::find()
        .select_only()
        .column(message::Column::Id)
        .filter(message::Column::RoomId.eq(room.get()))
        .filter(message::Column::Id.lt(message.get()))
        .into_tuple::<i64>()
        .one(conn)
        .await?;
    Ok(older.is_some())
}

pub(crate) fn from_row(row: message::Model) -> Result<Message, DbError> {
    Ok(Message {
        id: MessageId::new(row.id),
        room: RoomId::new(row.room_id),
        author: row.author.into(),
        text: row.text,
        created_at: instant(row.created_at)?,
        reply_to: row.reply_to_id.map(MessageId::new),
    })
}

impl From<Author> for message::Author {
    fn from(author: Author) -> Self {
        match author {
            Author::User => Self::User,
            Author::Assistant => Self::Assistant,
            Author::Event => Self::Event,
        }
    }
}

impl From<message::Author> for Author {
    fn from(author: message::Author) -> Self {
        match author {
            message::Author::User => Self::User,
            message::Author::Assistant => Self::Assistant,
            message::Author::Event => Self::Event,
        }
    }
}
