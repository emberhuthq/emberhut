//! Rooms.

use emberhut_core::room::Room;
use emberhut_core::task::ProjectId;
use emberhut_core::{MessageId, RoomId};
use jiff::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};

use crate::entity::{family_room, message, room};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// Reads and writes rooms.
#[derive(Clone, Debug)]
pub struct RoomStore {
    conn: DatabaseConnection,
}

impl RoomStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// The room with this id, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn get(&self, id: RoomId) -> Result<Option<Room>, DbError> {
        room::Entity::find_by_id(id.get())
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// Every room, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn list(&self) -> Result<Vec<Room>, DbError> {
        room::Entity::find()
            .order_by_asc(room::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// The owner's rooms: every room but the family members' own, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn owners(&self) -> Result<Vec<Room>, DbError> {
        owners_rows(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// Notes that the user looked at room `id` at `now`, seeing every message in it so far;
    /// `false` when there is no such room.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn mark_seen(&self, id: RoomId, now: Timestamp) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        let newest = message::Entity::find()
            .select_only()
            .column_as(message::Column::Id.max(), "newest")
            .filter(message::Column::RoomId.eq(id.get()))
            .into_tuple::<Option<i64>>()
            .one(&txn)
            .await?
            .flatten();
        let updated = room::Entity::update_many()
            .col_expr(room::Column::SeenAt, Expr::value(millis(now)))
            .col_expr(room::Column::SeenThrough, Expr::value(newest))
            .filter(room::Column::Id.eq(id.get()))
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(updated.rows_affected > 0)
    }

    /// Makes a room called `name` at `now`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn create(&self, name: &str, now: Timestamp) -> Result<Room, DbError> {
        let row = room::ActiveModel {
            name: Set(name.to_owned()),
            created_at: Set(millis(now)),
            ..Default::default()
        }
        .insert(&self.conn)
        .await?;
        from_row(row)
    }
}

pub(crate) fn from_row(row: room::Model) -> Result<Room, DbError> {
    Ok(Room {
        id: RoomId::new(row.id),
        name: row.name,
        created_at: instant(row.created_at)?,
        seen_at: row.seen_at.map(instant).transpose()?,
        seen_through: row.seen_through.map(MessageId::new),
        project: row.project_id.map(ProjectId::new),
    })
}

/// The rows of the owner's rooms (every room but the family members' own), oldest first.
pub(crate) async fn owners_rows<C: ConnectionTrait>(conn: &C) -> Result<Vec<room::Model>, DbError> {
    let family: Vec<i64> = family_room::Entity::find()
        .select_only()
        .column(family_room::Column::RoomId)
        .into_tuple()
        .all(conn)
        .await?;
    Ok(room::Entity::find()
        .filter(room::Column::Id.is_not_in(family))
        .order_by_asc(room::Column::Id)
        .all(conn)
        .await?)
}
