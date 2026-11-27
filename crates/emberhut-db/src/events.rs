//! Events: what programs and the server tell the assistant, per room.

use emberhut_core::event::{Event, NewEvent};
use emberhut_core::{EventId, RoomId};
use jiff::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, TryInsertResult,
};

use crate::DbError;
use crate::entity::event;
use crate::time::{instant, millis};

/// Writes and reads events.
#[derive(Clone, Debug)]
pub struct EventStore {
    conn: DatabaseConnection,
}

impl EventStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Stores `new` as told at `at`; none when its key was used already (the event happened).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], also when its room does not exist.
    pub async fn add(&self, new: NewEvent, at: Timestamp) -> Result<Option<Event>, DbError> {
        let row = event::ActiveModel {
            room_id: Set(new.room.get()),
            kind: Set(new.kind),
            text: Set(new.text),
            at: Set(millis(at)),
            key: Set(new.key),
            handled: Set(new.handled),
            ..Default::default()
        };
        let inserted = event::Entity::insert(row)
            .on_conflict_do_nothing_on([event::Column::Key])
            .exec(&self.conn)
            .await?;
        let TryInsertResult::Inserted(inserted) = inserted else {
            return Ok(None);
        };
        event::Entity::find_by_id(inserted.last_insert_id)
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// Whether an event with this key happened.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn exists(&self, key: &str) -> Result<bool, DbError> {
        Ok(event::Entity::find()
            .filter(event::Column::Key.eq(key))
            .count(&self.conn)
            .await?
            > 0)
    }

    /// `room`'s events not read yet, oldest first: the `limit` oldest of them.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn waiting(&self, room: RoomId, limit: u64) -> Result<Vec<Event>, DbError> {
        event::Entity::find()
            .filter(event::Column::RoomId.eq(room.get()))
            .filter(event::Column::Handled.eq(false))
            .order_by_asc(event::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// How many events, in every room, are not read yet.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn count_waiting(&self) -> Result<u64, DbError> {
        Ok(event::Entity::find()
            .filter(event::Column::Handled.eq(false))
            .count(&self.conn)
            .await?)
    }

    /// Marks `room`'s events up to and including `through` read; how many were waiting.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn handle_through(&self, room: RoomId, through: EventId) -> Result<u64, DbError> {
        let updated = event::Entity::update_many()
            .col_expr(event::Column::Handled, Expr::value(true))
            .filter(event::Column::RoomId.eq(room.get()))
            .filter(event::Column::Id.lte(through.get()))
            .filter(event::Column::Handled.eq(false))
            .exec(&self.conn)
            .await?;
        Ok(updated.rows_affected)
    }

    /// Marks every waiting event of `kind`, in every room, read; how many there were.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn handle_kind(&self, kind: &str) -> Result<u64, DbError> {
        let updated = event::Entity::update_many()
            .col_expr(event::Column::Handled, Expr::value(true))
            .filter(event::Column::Kind.eq(kind))
            .filter(event::Column::Handled.eq(false))
            .exec(&self.conn)
            .await?;
        Ok(updated.rows_affected)
    }
}

fn from_row(row: event::Model) -> Result<Event, DbError> {
    Ok(Event {
        id: EventId::new(row.id),
        room: RoomId::new(row.room_id),
        kind: row.kind,
        text: row.text,
        at: instant(row.at)?,
        key: row.key,
        handled: row.handled,
    })
}
