//! A configured agent's tasks: the tasks made in its room, and only those. It makes them for
//! the person who asked (`requested_by`, shown only: it grants nothing, since the agent writes
//! it), and passes a message on to one's worker as itself, never as the person.

use emberhut_core::RoomId;
use emberhut_core::task::{Owner, ProjectId, Status, Task, TaskChange, TaskId, Who};
use emberhut_db::NewTask;

use super::{TaskError, TaskService};

/// The most of a room's tasks read at once.
pub const ROOM_TASKS: u64 = 60;

/// A task an agent makes in its room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomTask {
    /// The room it is made in: the agent's.
    pub room: RoomId,
    /// The agent's name, as the task's history says it.
    pub agent: String,
    /// A few words.
    pub title: String,
    /// Everything its worker needs.
    pub brief: String,
    /// Who asked for it.
    pub requested_by: String,
    /// The project it is in: the agent's.
    pub project: Option<ProjectId>,
}

impl TaskService {
    /// Makes `new`, the assistant's to work, in its agent's room, written with its room in one
    /// insert (it is never an owner's task, not for a moment): who asked is kept for showing
    /// (`requested_by`, as the agent wrote it), never a grant; its history says whose task it is.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn create_in_room(&self, new: RoomTask) -> Result<Task, TaskError> {
        let requested_by = serde_json::json!({
            "who": new.requested_by,
            "at": self.iso(),
            "how": format!("asked {} in its room", new.agent),
        });
        let made = self
            .create(
                NewTask {
                    title: new.title,
                    body: Some(new.brief),
                    owner: Some(Owner::Assistant),
                    pid: new.project,
                    room: Some(new.room),
                    requested_by: Some(requested_by.to_string()),
                    ..NewTask::default()
                },
                Who::Assistant,
            )
            .await?;
        self.note(
            made.id,
            Who::Assistant,
            &format!(
                "Made by the agent {} for {}, in its room: its final steps are decided by the \
                 agent's policies, and its worker hands back to the agent, never posting itself",
                new.agent, new.requested_by
            ),
            false,
        )
        .await?;
        Ok(made)
    }

    /// The room the task `id` was made in, when one was recorded.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn room_of(&self, id: TaskId) -> Result<Option<RoomId>, TaskError> {
        Ok(self.tasks.room_of(id).await?)
    }

    /// The tasks made in `room`, the most recently changed first.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn in_room(&self, room: RoomId) -> Result<Vec<Task>, TaskError> {
        Ok(self.tasks.in_room(room, ROOM_TASKS).await?)
    }

    /// Passes `text` from the agent `from` to the task `id`'s worker: noted on the task as the
    /// assistant's, and carried on with by its worker (queued while one runs). Whether a
    /// worker was running.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn pass_to_worker(
        &self,
        id: TaskId,
        from: &str,
        text: &str,
    ) -> Result<bool, TaskError> {
        let task = self.task(id).await?;
        let running = task.worker.is_some();
        self.note(id, Who::Assistant, &format!("From {from}: {text}"), false)
            .await?;
        self.tasks
            .queue_next(id, &format!("(From {from}) {text}"), !running, self.now())
            .await?;
        Ok(running)
    }

    /// Cancels the task `id`, its worker stopped, saying why.
    ///
    /// # Errors
    ///
    /// [`TaskError`].
    pub async fn cancel(&self, id: TaskId, from: &str, why: &str) -> Result<(), TaskError> {
        self.stop_worker(id).await?;
        self.update(
            id,
            TaskChange {
                status: Some(Status::Canceled),
                ..TaskChange::default()
            },
            Who::Assistant,
            None,
        )
        .await?;
        self.note(
            id,
            Who::Assistant,
            &format!("Canceled by {from}: {why}"),
            false,
        )
        .await
    }
}
