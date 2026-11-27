//! The owner's "send it" in the chat as the OK for a message, and their yes in the chat about
//! a task ([`emberhut_core::guard::chat_ok`]'s rules over a room's lines). One message per OK: the
//! messages used are kept (`chatOksUsed`, the last 200).

use emberhut_core::RoomId;
use emberhut_core::guard::chat_ok::{self as rules, ChatLine, ChatOkMsg};
use emberhut_core::message::Author;
use emberhut_core::task::{Status, TaskChange, TaskId, Who};
use emberhut_db::{DbError, SettingKey};
use jiff::Timestamp;

use super::Guard;

pub(crate) const USED: SettingKey<Vec<i64>> = SettingKey::new("chatOksUsed");

impl Guard {
    /// A room's lines since `since`, oldest first, as the rules read them.
    async fn lines(&self, room: RoomId, since: Timestamp) -> Result<Vec<ChatLine>, DbError> {
        Ok(self
            .inner
            .messages
            .said_since(room, since, 500)
            .await?
            .into_iter()
            .map(|m| ChatLine {
                id: m.id.get(),
                at: m.created_at,
                text: m.text,
                mine: m.author == Author::User,
                reply_to: m.reply_to.map(emberhut_core::MessageId::get),
            })
            .collect())
    }

    /// The owner's OK in `room` for exactly `text` to `to` (`to_self`: their own chat), after
    /// the card asked at `asked` when there is one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn find_chat_ok(
        &self,
        room: RoomId,
        text: &str,
        to: &str,
        to_self: bool,
        asked: Option<Timestamp>,
    ) -> Result<Option<ChatOkMsg>, DbError> {
        let now = self.now();
        let lines = self
            .lines(room, now - rules::WINDOW - rules::WINDOW)
            .await?;
        let used = self.inner.settings.get_or(&USED, Vec::new()).await?;
        Ok(rules::find(&lines, text, to, to_self, asked, &used, now))
    }

    /// The owner's yes about the task in one of `rooms`, when they gave one in the window.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn task_chat_ok(
        &self,
        task: TaskId,
        rooms: &[RoomId],
    ) -> Result<Option<ChatOkMsg>, DbError> {
        let now = self.now();
        let key = task.key();
        let mut seen = Vec::new();
        for room in rooms {
            if seen.contains(room) {
                continue;
            }
            seen.push(*room);
            let lines = self
                .lines(*room, now - rules::WINDOW - rules::WINDOW)
                .await?;
            if let Some(ok) = rules::task_ok(&lines, &key, now) {
                return Ok(Some(ok));
            }
        }
        Ok(None)
    }

    /// Spends the owner's message `id` as an OK: one message per OK. `false` when it was
    /// spent already (another call got there first).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn spend(&self, id: i64) -> Result<bool, DbError> {
        self.inner
            .settings
            .update(&USED, Vec::new(), |used| {
                if used.contains(&id) {
                    return false;
                }
                used.push(id);
                let extra = used.len().saturating_sub(200);
                used.drain(..extra);
                true
            })
            .await
    }

    /// The task's line on which message of the owner's gave the OK.
    pub(crate) async fn ok_line(&self, ok: &ChatOkMsg, room: RoomId) -> String {
        let name = self.room_name(room).await;
        rules::ok_line(ok, &self.hhmm(ok.at), &name)
    }

    /// The card `question` still open on the task is answered by an OK given another way: it
    /// leaves the owner's Needs you, as their tap would.
    ///
    /// # Errors
    ///
    /// [`super::GuardError`].
    pub async fn close_card(&self, task: TaskId, question: &str) -> Result<(), super::GuardError> {
        let Some(t) = self.task(task).await? else {
            return Ok(());
        };
        let open = t.status == Status::Waiting
            && t.decision
                .as_deref()
                .and_then(|d| serde_json::from_str::<emberhut_api::tasks::Decision>(d).ok())
                .is_some_and(|d| d.question == question);
        if open {
            let change = TaskChange {
                action: Some(None),
                ..TaskChange::default()
            };
            self.inner
                .tasks
                .update(task, change, Who::Assistant, None)
                .await?;
        }
        Ok(())
    }
}
