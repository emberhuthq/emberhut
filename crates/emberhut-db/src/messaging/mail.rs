//! The owner's mail ([`MailStore`]).

use emberhut_core::messaging::mail::{MailConversation, MailMessage};
use jiff::Timestamp;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, TryInsertResult,
};

use crate::DbError;
use crate::entity::mail_message;
use crate::time::{instant, millis};

/// The most conversations a list of them gives.
const CONVERSATIONS: usize = 60;

/// Keeps and reads the owner's mail.
#[derive(Clone, Debug)]
pub struct MailStore {
    conn: DatabaseConnection,
}

impl MailStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Keeps `message` unless one with its id is kept already; whether it was new.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn add(&self, message: &MailMessage) -> Result<bool, DbError> {
        let added = mail_message::Entity::insert(mail_message::ActiveModel {
            id: Set(message.id.clone()),
            account: Set(message.account.clone()),
            chat: Set(message.chat.clone()),
            who: Set(message.who.clone()),
            sender: Set(message.sender.clone()),
            from_me: Set(message.from_me),
            subject: Set(message.subject.clone()),
            body: Set(message.body.clone()),
            at: Set(millis(message.at)),
            got: Set(millis(message.got)),
            loud: Set(message.loud),
            thread: Set(message.thread.clone()),
        })
        .on_conflict_do_nothing_on([mail_message::Column::Id])
        .exec_without_returning(&self.conn)
        .await?;
        Ok(matches!(added, TryInsertResult::Inserted(n) if n > 0))
    }

    /// Whether a message with this id is kept.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn has(&self, id: &str) -> Result<bool, DbError> {
        Ok(mail_message::Entity::find_by_id(id)
            .count(&self.conn)
            .await?
            > 0)
    }

    /// How many messages of `account` are kept.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn count(&self, account: &str) -> Result<u64, DbError> {
        Ok(mail_message::Entity::find()
            .filter(mail_message::Column::Account.eq(account))
            .count(&self.conn)
            .await?)
    }

    /// The correspondents with loud mail sent after `since`, the newest first (at most 60).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn conversations(&self, since: Timestamp) -> Result<Vec<MailConversation>, DbError> {
        let mut conversations = self.grouped(mail_message::Column::At, since).await?;
        conversations.sort_by_key(|c| std::cmp::Reverse(c.last_at));
        conversations.truncate(CONVERSATIONS);
        Ok(conversations)
    }

    /// The correspondents with loud mail the server got after `since`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn changes(&self, since: Timestamp) -> Result<Vec<MailConversation>, DbError> {
        self.grouped(mail_message::Column::Got, since).await
    }

    async fn grouped(
        &self,
        by: mail_message::Column,
        since: Timestamp,
    ) -> Result<Vec<MailConversation>, DbError> {
        let rows = mail_message::Entity::find()
            .select_only()
            .column(mail_message::Column::Chat)
            .column(mail_message::Column::Account)
            .column_as(mail_message::Column::Who.max(), "who")
            .column_as(mail_message::Column::Id.count(), "n")
            .column_as(mail_message::Column::FromMe.sum(), "mine")
            .column_as(mail_message::Column::At.max(), "last")
            .filter(by.gt(millis(since)))
            .filter(mail_message::Column::Loud.eq(true))
            .group_by(mail_message::Column::Chat)
            .group_by(mail_message::Column::Account)
            .into_tuple::<(String, String, String, i64, Option<i64>, i64)>()
            .all(&self.conn)
            .await?;
        rows.into_iter()
            .map(|(chat, account, who, n, mine, last)| {
                Ok(MailConversation {
                    chat,
                    account,
                    who,
                    messages: u64::try_from(n).unwrap_or(0),
                    mine: u64::try_from(mine.unwrap_or(0)).unwrap_or(0),
                    last_at: instant(last)?,
                })
            })
            .collect()
    }

    /// `chat`'s newest `limit` messages, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn messages(&self, chat: &str, limit: u64) -> Result<Vec<MailMessage>, DbError> {
        self.newest(
            Condition::all().add(mail_message::Column::Chat.eq(chat)),
            limit,
        )
        .await
    }

    /// `chat`'s newest `limit` messages the server got after `since`, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn new_since(
        &self,
        chat: &str,
        since: Timestamp,
        limit: u64,
    ) -> Result<Vec<MailMessage>, DbError> {
        self.newest(
            Condition::all()
                .add(mail_message::Column::Chat.eq(chat))
                .add(mail_message::Column::Got.gt(millis(since))),
            limit,
        )
        .await
    }

    async fn newest(&self, filter: Condition, limit: u64) -> Result<Vec<MailMessage>, DbError> {
        let mut rows = mail_message::Entity::find()
            .filter(filter)
            .order_by_desc(mail_message::Column::At)
            .limit(limit)
            .all(&self.conn)
            .await?;
        rows.reverse();
        rows.into_iter().map(from_row).collect()
    }

    /// `account`'s newest `limit` messages sent after `since` that hold `words` (in any case)
    /// in their text, subject, sender, correspondent or chat, newest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn search(
        &self,
        account: &str,
        words: &str,
        since: Timestamp,
        limit: u64,
    ) -> Result<Vec<MailMessage>, DbError> {
        mail_message::Entity::find()
            .filter(mail_message::Column::Account.eq(account))
            .filter(mail_message::Column::At.gt(millis(since)))
            .filter(
                Condition::any()
                    .add(mail_message::Column::Body.contains(words))
                    .add(mail_message::Column::Subject.contains(words))
                    .add(mail_message::Column::Sender.contains(words))
                    .add(mail_message::Column::Who.contains(words))
                    .add(mail_message::Column::Chat.contains(words)),
            )
            .order_by_desc(mail_message::Column::At)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }
}

fn from_row(row: mail_message::Model) -> Result<MailMessage, DbError> {
    Ok(MailMessage {
        id: row.id,
        account: row.account,
        chat: row.chat,
        who: row.who,
        sender: row.sender,
        from_me: row.from_me,
        subject: row.subject,
        body: row.body,
        at: instant(row.at)?,
        got: instant(row.got)?,
        loud: row.loud,
        thread: row.thread,
    })
}
