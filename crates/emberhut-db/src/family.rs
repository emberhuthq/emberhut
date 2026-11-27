//! Family members' rooms and phones.

use emberhut_core::message::Message;
use emberhut_core::push::{ApnsEnv, ApnsToken};
use emberhut_core::room::Room;
use emberhut_core::{MessageId, RoomId};
use jiff::Timestamp;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use crate::entity::{family_device, family_room, message, room};
use crate::rooms::from_row;
use crate::time::millis;
use crate::{DbError, begin_write};

/// The name of the room a family member gets first.
pub const HOME_ROOM: &str = "Home";

/// Reads and writes family members' rooms and phones.
#[derive(Clone, Debug)]
pub struct FamilyStore {
    conn: DatabaseConnection,
}

impl FamilyStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// `person`'s rooms, the oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn rooms_of(&self, person: &str) -> Result<Vec<Room>, DbError> {
        let ids: Vec<i64> = family_room::Entity::find()
            .filter(family_room::Column::Person.eq(person))
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| row.room_id)
            .collect();
        room::Entity::find()
            .filter(room::Column::Id.is_in(ids))
            .order_by_asc(room::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// `person`'s first room, made ([`HOME_ROOM`]) at `now` the first time they come.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn home_room(&self, person: &str, now: Timestamp) -> Result<Room, DbError> {
        if let Some(home) = self.rooms_of(person).await?.into_iter().next() {
            return Ok(home);
        }
        let txn = begin_write(&self.conn).await?;
        let made = room::ActiveModel {
            name: Set(HOME_ROOM.to_owned()),
            created_at: Set(millis(now)),
            seen_at: Set(None),
            seen_through: Set(None),
            ..Default::default()
        };
        let id = room::Entity::insert(made).exec(&txn).await?.last_insert_id;
        family_room::Entity::insert(family_room::ActiveModel {
            room_id: Set(id),
            person: Set(person.to_owned()),
        })
        .exec(&txn)
        .await?;
        let row = room::Entity::find_by_id(id).one(&txn).await?;
        txn.commit().await?;
        row.map(from_row)
            .transpose()?
            .ok_or(DbError::Query(sea_orm::DbErr::RecordNotInserted))
    }

    /// `room`'s conversation as a family member's app draws it: their words and the
    /// assistant's answers, not the server's event lines nor blank ones; the newest `limit`
    /// (older than `before`, when given), oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn conversation(
        &self,
        room: RoomId,
        before: Option<MessageId>,
        limit: u64,
    ) -> Result<Vec<Message>, DbError> {
        let mut select = message::Entity::find()
            .filter(message::Column::RoomId.eq(room.get()))
            .filter(
                message::Column::Author.is_in([message::Author::User, message::Author::Assistant]),
            )
            .filter(message::Column::Text.ne(""));
        if let Some(before) = before {
            select = select.filter(message::Column::Id.lt(before.get()));
        }
        let mut lines = select
            .order_by_desc(message::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(crate::messages::from_row)
            .collect::<Result<Vec<_>, _>>()?;
        lines.reverse();
        Ok(lines)
    }

    /// The family member `room` belongs to, if it is one of theirs.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn person_of(&self, room: RoomId) -> Result<Option<String>, DbError> {
        Ok(family_room::Entity::find_by_id(room.get())
            .one(&self.conn)
            .await?
            .map(|row| row.person))
    }

    /// Keeps `person`'s phone's token for `env`'s service, registered at `now`; the same token
    /// again takes the new service, person and time.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn register_device(
        &self,
        token: &ApnsToken,
        env: ApnsEnv,
        person: &str,
        now: Timestamp,
    ) -> Result<(), DbError> {
        let row = family_device::ActiveModel {
            token: Set(token.as_str().to_owned()),
            env: Set(env.as_str().to_owned()),
            person: Set(person.to_owned()),
            registered_at: Set(millis(now)),
        };
        family_device::Entity::insert(row)
            .on_conflict(
                OnConflict::column(family_device::Column::Token)
                    .update_columns([
                        family_device::Column::Env,
                        family_device::Column::Person,
                        family_device::Column::RegisteredAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// `person`'s phones: each token and its service's word, the longest registered first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn devices_of(&self, person: &str) -> Result<Vec<(String, String)>, DbError> {
        Ok(family_device::Entity::find()
            .filter(family_device::Column::Person.eq(person))
            .order_by_asc(family_device::Column::RegisteredAt)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| (row.token, row.env))
            .collect())
    }
}
