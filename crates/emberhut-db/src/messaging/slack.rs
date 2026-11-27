//! The owner's Slack ([`SlackStore`]).

use emberhut_core::messaging::slack::{SlackConversation, SlackMessage};
use jiff::Timestamp;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, TryInsertResult,
};

use crate::DbError;
use crate::entity::slack_message;
use crate::time::{instant, millis};

/// Keeps and reads the owner's Slack messages.
#[derive(Clone, Debug)]
pub struct SlackStore {
    conn: DatabaseConnection,
}

impl SlackStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Keeps `message`; one already kept gets its body brought up to date (an edit, or a file
    /// a first look only half named).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn keep(&self, message: &SlackMessage) -> Result<(), DbError> {
        slack_message::Entity::insert(row(message))
            .on_conflict(
                OnConflict::column(slack_message::Column::Id)
                    .update_column(slack_message::Column::Body)
                    .to_owned(),
            )
            .exec_without_returning(&self.conn)
            .await?;
        Ok(())
    }

    /// Keeps `message` unless one with its id is kept already; whether it was new.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn add(&self, message: &SlackMessage) -> Result<bool, DbError> {
        let added = slack_message::Entity::insert(row(message))
            .on_conflict_do_nothing_on([slack_message::Column::Id])
            .exec_without_returning(&self.conn)
            .await?;
        Ok(matches!(added, TryInsertResult::Inserted(n) if n > 0))
    }

    /// The message `id`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn get(&self, id: &str) -> Result<Option<SlackMessage>, DbError> {
        slack_message::Entity::find_by_id(id)
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// When the owner's newest message read from Slack in `workspace` was sent.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn last_mine(&self, workspace: &str) -> Result<Option<Timestamp>, DbError> {
        let last = slack_message::Entity::find()
            .select_only()
            .column_as(slack_message::Column::At.max(), "last")
            .filter(slack_message::Column::Workspace.eq(workspace))
            .filter(slack_message::Column::FromMe.eq(true))
            .filter(slack_message::Column::Channel.is_not_null())
            .into_tuple::<Option<i64>>()
            .one(&self.conn)
            .await?
            .flatten();
        last.map(instant).transpose()
    }

    /// When the newest notification (a message with no channel) was sent.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn last_notification(&self) -> Result<Option<Timestamp>, DbError> {
        let last = slack_message::Entity::find()
            .select_only()
            .column_as(slack_message::Column::At.max(), "last")
            .filter(slack_message::Column::Channel.is_null())
            .into_tuple::<Option<i64>>()
            .one(&self.conn)
            .await?
            .flatten();
        last.map(instant).transpose()
    }

    /// The channel ids `who`'s messages in `workspace` came with, each once.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn channels(&self, workspace: &str, who: &str) -> Result<Vec<String>, DbError> {
        let channels = slack_message::Entity::find()
            .select_only()
            .column(slack_message::Column::Channel)
            .distinct()
            .filter(slack_message::Column::Workspace.eq(workspace))
            .filter(slack_message::Column::Who.eq(who))
            .filter(slack_message::Column::Channel.is_not_null())
            .into_tuple::<Option<String>>()
            .all(&self.conn)
            .await?;
        Ok(channels.into_iter().flatten().collect())
    }

    /// Every conversation with messages sent after `since`, the newest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn conversations(&self, since: Timestamp) -> Result<Vec<SlackConversation>, DbError> {
        let rows = slack_message::Entity::find()
            .select_only()
            .column(slack_message::Column::Workspace)
            .column(slack_message::Column::Who)
            .column_as(slack_message::Column::Id.count(), "n")
            .column_as(slack_message::Column::FromMe.sum(), "mine")
            .column_as(slack_message::Column::At.max(), "last")
            .filter(slack_message::Column::At.gt(millis(since)))
            .group_by(slack_message::Column::Workspace)
            .group_by(slack_message::Column::Who)
            .into_tuple::<(String, String, i64, Option<i64>, i64)>()
            .all(&self.conn)
            .await?;
        let mut conversations = rows
            .into_iter()
            .map(|(workspace, who, n, mine, last)| {
                Ok(SlackConversation {
                    workspace,
                    who,
                    messages: u64::try_from(n).unwrap_or(0),
                    mine: u64::try_from(mine.unwrap_or(0)).unwrap_or(0),
                    last_at: instant(last)?,
                })
            })
            .collect::<Result<Vec<_>, DbError>>()?;
        conversations.sort_by_key(|c| std::cmp::Reverse(c.last_at));
        Ok(conversations)
    }

    /// `who`'s newest `limit` messages in `workspace`, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn messages(
        &self,
        workspace: &str,
        who: &str,
        limit: u64,
    ) -> Result<Vec<SlackMessage>, DbError> {
        let mut rows = slack_message::Entity::find()
            .filter(slack_message::Column::Workspace.eq(workspace))
            .filter(slack_message::Column::Who.eq(who))
            .order_by_desc(slack_message::Column::At)
            .limit(limit)
            .all(&self.conn)
            .await?;
        rows.reverse();
        rows.into_iter().map(from_row).collect()
    }

    /// The newest `limit` messages sent after `since` whose text, conversation or sender holds
    /// `words` (in any case), newest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`].
    pub async fn search(
        &self,
        words: &str,
        since: Timestamp,
        limit: u64,
    ) -> Result<Vec<SlackMessage>, DbError> {
        slack_message::Entity::find()
            .filter(slack_message::Column::At.gt(millis(since)))
            .filter(
                Condition::any()
                    .add(slack_message::Column::Body.contains(words))
                    .add(slack_message::Column::Who.contains(words))
                    .add(slack_message::Column::Sender.contains(words)),
            )
            .order_by_desc(slack_message::Column::At)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(from_row)
            .collect()
    }

    /// Replaces `old` with `new` in message `id`'s body (a voice note's line, once it has
    /// text); whether the message is kept.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn replace_in_body(&self, id: &str, old: &str, new: &str) -> Result<bool, DbError> {
        let Some(found) = slack_message::Entity::find_by_id(id)
            .one(&self.conn)
            .await?
        else {
            return Ok(false);
        };
        let body = found.body.replace(old, new);
        if body != found.body {
            slack_message::Entity::update_many()
                .col_expr(slack_message::Column::Body, Expr::value(body))
                .filter(slack_message::Column::Id.eq(id))
                .exec(&self.conn)
                .await?;
        }
        Ok(true)
    }
}

fn row(message: &SlackMessage) -> slack_message::ActiveModel {
    slack_message::ActiveModel {
        id: Set(message.id.clone()),
        workspace: Set(message.workspace.clone()),
        who: Set(message.who.clone()),
        body: Set(message.body.clone()),
        at: Set(millis(message.at)),
        from_me: Set(message.from_me),
        sender: Set(message.sender.clone()),
        channel: Set(message.channel.clone()),
    }
}

fn from_row(row: slack_message::Model) -> Result<SlackMessage, DbError> {
    Ok(SlackMessage {
        id: row.id,
        workspace: row.workspace,
        who: row.who,
        body: row.body,
        at: instant(row.at)?,
        from_me: row.from_me,
        sender: row.sender,
        channel: row.channel,
    })
}
