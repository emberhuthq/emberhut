//! The assistant's own WhatsApp number ([`WhatsAppStore`]): its chats and allow-list, the
//! allowed chats' recent messages, and the log of what it sent.

use emberhut_core::RoomId;
use emberhut_core::messaging::whatsapp::{ChatKind, ChatStatus, WaChat, WaMessage, WaSent};
use jiff::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, TryInsertResult,
};

use crate::entity::{wa_chat, wa_msg, wa_sent};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// A change to a chat: each field set is changed, and its time with it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatChange {
    /// Its name.
    pub name: Option<String>,
    /// Where it is on the allow-list.
    pub status: Option<ChatStatus>,
    /// The id its messages come from.
    pub via: Option<String>,
    /// The task the owner was asked on.
    pub task: Option<i64>,
    /// The room its asks go to (`Some(None)`: the WhatsApp room).
    pub room: Option<Option<RoomId>>,
    /// What the owner allowed there (`Some(None)`: nothing).
    pub topics: Option<Option<String>>,
}

/// A message the assistant sent, to log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewWaSent {
    /// The chat's jid.
    pub chat: String,
    /// The chat's name now.
    pub name: Option<String>,
    /// When it was sent.
    pub at: Timestamp,
    /// What it said.
    pub text: String,
    /// WhatsApp's id of it.
    pub msg: Option<String>,
    /// The message it answered.
    pub reply_to: Option<String>,
    /// The room it was shown in.
    pub room: Option<RoomId>,
}

/// Keeps the assistant's own WhatsApp chats, their messages and what it sent.
#[derive(Clone, Debug)]
pub struct WhatsAppStore {
    conn: DatabaseConnection,
}

impl WhatsAppStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// The chat `jid`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn chat(&self, jid: &str) -> Result<Option<WaChat>, DbError> {
        wa_chat::Entity::find_by_id(jid)
            .one(&self.conn)
            .await?
            .map(chat_from_row)
            .transpose()
    }

    /// The chat whose messages come from `via`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn chat_via(&self, via: &str) -> Result<Option<WaChat>, DbError> {
        wa_chat::Entity::find()
            .filter(wa_chat::Column::Via.eq(via))
            .one(&self.conn)
            .await?
            .map(chat_from_row)
            .transpose()
    }

    /// The chats called `name`, in any case.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn chats_named(&self, name: &str) -> Result<Vec<WaChat>, DbError> {
        let lower = name.to_lowercase();
        Ok(self
            .chats()
            .await?
            .into_iter()
            .filter(|chat| {
                chat.name
                    .as_deref()
                    .is_some_and(|n| n.to_lowercase() == lower)
            })
            .collect())
    }

    /// The chat still waiting for the owner's answer on `task`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn pending_on(&self, task: i64) -> Result<Option<WaChat>, DbError> {
        wa_chat::Entity::find()
            .filter(wa_chat::Column::Task.eq(task))
            .filter(wa_chat::Column::Status.eq(ChatStatus::Pending.as_str()))
            .one(&self.conn)
            .await?
            .map(chat_from_row)
            .transpose()
    }

    /// Every chat: the allowed ones first, then by name.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn chats(&self) -> Result<Vec<WaChat>, DbError> {
        let mut chats = wa_chat::Entity::find()
            .all(&self.conn)
            .await?
            .into_iter()
            .map(chat_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        chats.sort_by(|a, b| {
            (b.status == ChatStatus::Allowed)
                .cmp(&(a.status == ChatStatus::Allowed))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(chats)
    }

    /// Adds `chat`, unless one with its jid is known; whether it was new.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn add_chat(&self, chat: &WaChat) -> Result<bool, DbError> {
        let added = wa_chat::Entity::insert(wa_chat::ActiveModel {
            jid: Set(chat.jid.clone()),
            kind: Set(chat.kind.as_str().to_owned()),
            name: Set(chat.name.clone()),
            status: Set(chat.status.as_str().to_owned()),
            via: Set(chat.via.clone()),
            task: Set(chat.task),
            room: Set(chat.room.map(RoomId::get)),
            topics: Set(chat.topics.clone()),
            by: Set(chat.by.clone()),
            added: Set(millis(chat.added)),
            updated: Set(millis(chat.updated)),
        })
        .on_conflict_do_nothing_on([wa_chat::Column::Jid])
        .exec_without_returning(&self.conn)
        .await?;
        Ok(matches!(added, TryInsertResult::Inserted(n) if n > 0))
    }

    /// Changes chat `jid` as `change` says, at `now`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn change(
        &self,
        jid: &str,
        change: ChatChange,
        now: Timestamp,
    ) -> Result<(), DbError> {
        let mut update = wa_chat::Entity::update_many()
            .col_expr(wa_chat::Column::Updated, Expr::value(millis(now)))
            .filter(wa_chat::Column::Jid.eq(jid));
        if let Some(name) = change.name {
            update = update.col_expr(wa_chat::Column::Name, Expr::value(name));
        }
        if let Some(status) = change.status {
            update = update.col_expr(wa_chat::Column::Status, Expr::value(status.as_str()));
        }
        if let Some(via) = change.via {
            update = update.col_expr(wa_chat::Column::Via, Expr::value(via));
        }
        if let Some(task) = change.task {
            update = update.col_expr(wa_chat::Column::Task, Expr::value(task));
        }
        if let Some(room) = change.room {
            update = update.col_expr(wa_chat::Column::Room, Expr::value(room.map(RoomId::get)));
        }
        if let Some(topics) = change.topics {
            update = update.col_expr(wa_chat::Column::Topics, Expr::value(topics));
        }
        update.exec(&self.conn).await?;
        Ok(())
    }

    /// Keeps `message` (one known already stays as it was), then only the chat's newest
    /// `keep`; whether it was new.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn keep(&self, message: &WaMessage, keep: u64) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        let added = wa_msg::Entity::insert(wa_msg::ActiveModel {
            chat: Set(message.chat.clone()),
            id: Set(message.id.clone()),
            sender: Set(message.sender.clone()),
            who: Set(message.who.clone()),
            text: Set(message.text.clone()),
            at: Set(millis(message.at)),
            from_me: Set(message.from_me),
            asked: Set(message.asked),
            answered: Set(message.answered),
        })
        .on_conflict_do_nothing_on([wa_msg::Column::Chat, wa_msg::Column::Id])
        .exec_without_returning(&txn)
        .await?;
        let older = wa_msg::Entity::find()
            .select_only()
            .column(wa_msg::Column::Id)
            .filter(wa_msg::Column::Chat.eq(message.chat.as_str()))
            .order_by_desc(wa_msg::Column::At)
            .offset(keep)
            // SQLite takes an offset only with a limit.
            .limit(u64::from(u32::MAX))
            .into_tuple::<String>()
            .all(&txn)
            .await?;
        if !older.is_empty() {
            wa_msg::Entity::delete_many()
                .filter(wa_msg::Column::Chat.eq(message.chat.as_str()))
                .filter(wa_msg::Column::Id.is_in(older))
                .exec(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(matches!(added, TryInsertResult::Inserted(n) if n > 0))
    }

    /// How many messages asked the assistant after `since`, in every chat.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn asks_since(&self, since: Timestamp) -> Result<u64, DbError> {
        Ok(wa_msg::Entity::find()
            .filter(wa_msg::Column::Asked.eq(true))
            .filter(wa_msg::Column::At.gt(millis(since)))
            .count(&self.conn)
            .await?)
    }

    /// `chat`'s newest `limit` messages, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn messages(&self, chat: &str, limit: u64) -> Result<Vec<WaMessage>, DbError> {
        let mut rows = wa_msg::Entity::find()
            .filter(wa_msg::Column::Chat.eq(chat))
            .order_by_desc(wa_msg::Column::At)
            .limit(limit)
            .all(&self.conn)
            .await?;
        rows.reverse();
        rows.into_iter().map(msg_from_row).collect()
    }

    /// `chat`'s messages after `since` that asked the assistant and are not answered yet,
    /// the newest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn open_asks(&self, chat: &str, since: Timestamp) -> Result<Vec<WaMessage>, DbError> {
        wa_msg::Entity::find()
            .filter(wa_msg::Column::Chat.eq(chat))
            .filter(wa_msg::Column::Asked.eq(true))
            .filter(wa_msg::Column::Answered.eq(false))
            .filter(wa_msg::Column::At.gt(millis(since)))
            .order_by_desc(wa_msg::Column::At)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(msg_from_row)
            .collect()
    }

    /// Whether message `id` in `chat` was answered.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn is_answered(&self, chat: &str, id: &str) -> Result<bool, DbError> {
        Ok(wa_msg::Entity::find()
            .filter(wa_msg::Column::Chat.eq(chat))
            .filter(wa_msg::Column::Id.eq(id))
            .filter(wa_msg::Column::Answered.eq(true))
            .count(&self.conn)
            .await?
            > 0)
    }

    /// Marks message `id` in `chat` answered (or not again); whether it was unanswered (for
    /// `answered`, then two claims can't both win).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn set_answered(
        &self,
        chat: &str,
        id: &str,
        answered: bool,
    ) -> Result<bool, DbError> {
        let updated = wa_msg::Entity::update_many()
            .col_expr(wa_msg::Column::Answered, Expr::value(answered))
            .filter(wa_msg::Column::Chat.eq(chat))
            .filter(wa_msg::Column::Id.eq(id))
            .filter(wa_msg::Column::Answered.eq(!answered))
            .exec(&self.conn)
            .await?;
        Ok(updated.rows_affected > 0)
    }

    /// Logs a message the assistant sent.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn log_sent(&self, sent: &NewWaSent) -> Result<WaSent, DbError> {
        let row = wa_sent::Entity::insert(wa_sent::ActiveModel {
            chat: Set(sent.chat.clone()),
            name: Set(sent.name.clone()),
            at: Set(millis(sent.at)),
            text: Set(sent.text.clone()),
            msg: Set(sent.msg.clone()),
            reply_to: Set(sent.reply_to.clone()),
            room: Set(sent.room.map(RoomId::get)),
            ..Default::default()
        })
        .exec_with_returning(&self.conn)
        .await?;
        sent_from_row(row)
    }

    /// How many messages the assistant sent, ever.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn sent_count(&self) -> Result<u64, DbError> {
        Ok(wa_sent::Entity::find().count(&self.conn).await?)
    }

    /// How many it sent after `since`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn sent_since(&self, since: Timestamp) -> Result<u64, DbError> {
        Ok(wa_sent::Entity::find()
            .filter(wa_sent::Column::At.gt(millis(since)))
            .count(&self.conn)
            .await?)
    }

    /// When it last sent in `chat`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn last_sent_in(&self, chat: &str) -> Result<Option<Timestamp>, DbError> {
        let last = wa_sent::Entity::find()
            .select_only()
            .column_as(wa_sent::Column::At.max(), "last")
            .filter(wa_sent::Column::Chat.eq(chat))
            .into_tuple::<Option<i64>>()
            .one(&self.conn)
            .await?
            .flatten();
        last.map(instant).transpose()
    }

    /// The newest `limit` messages it sent, newest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn recent_sent(&self, limit: u64) -> Result<Vec<WaSent>, DbError> {
        wa_sent::Entity::find()
            .order_by_desc(wa_sent::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(sent_from_row)
            .collect()
    }
}

fn chat_from_row(row: wa_chat::Model) -> Result<WaChat, DbError> {
    Ok(WaChat {
        kind: ChatKind::from_word(&row.kind),
        status: ChatStatus::from_word(&row.status),
        room: row.room.map(RoomId::new),
        added: instant(row.added)?,
        updated: instant(row.updated)?,
        jid: row.jid,
        name: row.name,
        via: row.via,
        task: row.task,
        topics: row.topics,
        by: row.by,
    })
}

fn msg_from_row(row: wa_msg::Model) -> Result<WaMessage, DbError> {
    Ok(WaMessage {
        at: instant(row.at)?,
        chat: row.chat,
        id: row.id,
        sender: row.sender,
        who: row.who,
        text: row.text,
        from_me: row.from_me,
        asked: row.asked,
        answered: row.answered,
    })
}

fn sent_from_row(row: wa_sent::Model) -> Result<WaSent, DbError> {
    Ok(WaSent {
        at: instant(row.at)?,
        room: row.room.map(RoomId::new),
        id: row.id,
        chat: row.chat,
        name: row.name,
        text: row.text,
        msg: row.msg,
        reply_to: row.reply_to,
    })
}
