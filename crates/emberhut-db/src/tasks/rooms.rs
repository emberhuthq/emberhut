//! The room a task was made in (`NewTask::room`, written with the task in one insert), when
//! that isn't its project's: an agent's tasks are the tasks made in its room, and only those
//! are its own.

use emberhut_core::RoomId;
use emberhut_core::task::{Task, TaskId};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use super::{TaskStore, rows};
use crate::DbError;
use crate::entity::issue;

impl TaskStore {
    /// The room the task `id` was made in, when one was recorded.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn room_of(&self, id: TaskId) -> Result<Option<RoomId>, DbError> {
        Ok(issue::Entity::find_by_id(id.get())
            .one(&self.conn)
            .await?
            .and_then(|row| row.room)
            .map(RoomId::new))
    }

    /// The tasks made in `room`, the most recently changed first, at most `limit`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn in_room(&self, room: RoomId, limit: u64) -> Result<Vec<Task>, DbError> {
        rows(
            issue::Entity::find()
                .filter(issue::Column::Room.eq(room.get()))
                .order_by_desc(issue::Column::Updated)
                .order_by_desc(issue::Column::Id)
                .limit(limit)
                .all(&self.conn)
                .await?,
        )
    }
}
