//! Parties and what was said in them.

use emberhut_core::party::{
    MessageKind, NewPartyMessage, Party, PartyMessage, PartyPlan, PartyStatus, Persona,
};
use emberhut_core::{PartyId, PartyMessageId};
use jiff::Timestamp;
use sea_orm::sea_query::{Expr, ExprTrait as _};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde_json::{Value, json};

use crate::entity::{party, party_message};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// Reads and writes parties and their talk.
#[derive(Clone, Debug)]
pub struct PartyStore {
    conn: DatabaseConnection,
}

impl PartyStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Stores a new party from `plan`, running or paused, with the person's `opening` line as
    /// its first message, in one transaction.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn create(
        &self,
        plan: &PartyPlan,
        status: PartyStatus,
        opening: &NewPartyMessage,
        now: Timestamp,
    ) -> Result<Party, DbError> {
        let txn = begin_write(&self.conn).await?;
        let row = party::ActiveModel {
            title: Set(plan.title.clone()),
            topic: Set(plan.topic.clone()),
            status: Set(status.as_str().to_owned()),
            why: Set(None),
            personas: Set(personas_json(&plan.personas)),
            turns: Set(0),
            max_turns: Set(i32::try_from(plan.max_turns).unwrap_or(i32::MAX)),
            tokens: Set(0),
            max_tokens: Set(i64::try_from(plan.max_tokens).unwrap_or(i64::MAX)),
            cost: Set(0.0),
            created: Set(millis(now)),
            updated: Set(millis(now)),
            ..Default::default()
        };
        let id = party::Entity::insert(row).exec(&txn).await?.last_insert_id;
        add(&txn, PartyId::new(id), opening, now).await?;
        let made = party::Entity::find_by_id(id).one(&txn).await?;
        txn.commit().await?;
        made.map(from_row)
            .transpose()?
            .ok_or_else(|| DbError::Query(sea_orm::DbErr::RecordNotInserted))
    }

    /// The party `id`, if there is one.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn get(&self, id: PartyId) -> Result<Option<Party>, DbError> {
        party::Entity::find_by_id(id.get())
            .one(&self.conn)
            .await?
            .map(from_row)
            .transpose()
    }

    /// The newest `limit` parties, the most recently changed first, each with its preview:
    /// the last line said (a direction left out), as `Name: text` on one line, cut to 140
    /// characters.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn list(&self, limit: u64) -> Result<Vec<(Party, Option<String>)>, DbError> {
        let rows = party::Entity::find()
            .order_by_desc(party::Column::Updated)
            .order_by_desc(party::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?;
        let mut parties = Vec::with_capacity(rows.len());
        for row in rows {
            let last = party_message::Entity::find()
                .filter(party_message::Column::Party.eq(row.id))
                .filter(party_message::Column::Kind.ne(MessageKind::Steer.as_str()))
                .order_by_desc(party_message::Column::Id)
                .one(&self.conn)
                .await?
                .map(|m| {
                    let text: String = emberhut_core::party::one_line(&m.text);
                    format!("{}: {}", m.name, preview(&text, 140))
                });
            parties.push((from_row(row)?, last));
        }
        Ok(parties)
    }

    /// The casts of the newest `limit` parties, newest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn recent_casts(&self, limit: u64) -> Result<Vec<Vec<Persona>>, DbError> {
        Ok(party::Entity::find()
            .order_by_desc(party::Column::Id)
            .limit(limit)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|row| personas_from_json(&row.personas))
            .collect())
    }

    /// Everything said in party `id`, oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn messages(&self, id: PartyId) -> Result<Vec<PartyMessage>, DbError> {
        party_message::Entity::find()
            .filter(party_message::Column::Party.eq(id.get()))
            .order_by_asc(party_message::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(message_from_row)
            .collect()
    }

    /// Adds `message` to party `id` at `now`, and notes the party changed then.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`] (one for a party that isn't there).
    pub async fn add_message(
        &self,
        id: PartyId,
        message: &NewPartyMessage,
        now: Timestamp,
    ) -> Result<PartyMessage, DbError> {
        let txn = begin_write(&self.conn).await?;
        let added = add(&txn, id, message, now).await?;
        txn.commit().await?;
        Ok(added)
    }

    /// Sets party `id`'s status and why at `now`; `false` when there is no such party.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn set_status(
        &self,
        id: PartyId,
        status: PartyStatus,
        why: Option<&str>,
        now: Timestamp,
    ) -> Result<bool, DbError> {
        let updated = party::Entity::update_many()
            .col_expr(party::Column::Status, Expr::value(status.as_str()))
            .col_expr(party::Column::Why, Expr::value(why.map(str::to_owned)))
            .col_expr(party::Column::Updated, Expr::value(millis(now)))
            .filter(party::Column::Id.eq(id.get()))
            .exec(&self.conn)
            .await?;
        Ok(updated.rows_affected > 0)
    }

    /// Sets the caps given on party `id`; `false` when there is no such party.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn set_caps(
        &self,
        id: PartyId,
        max_turns: Option<u32>,
        max_tokens: Option<u64>,
    ) -> Result<bool, DbError> {
        if max_turns.is_none() && max_tokens.is_none() {
            return Ok(party::Entity::find_by_id(id.get())
                .one(&self.conn)
                .await?
                .is_some());
        }
        let mut update = party::Entity::update_many();
        if let Some(turns) = max_turns {
            update = update.col_expr(
                party::Column::MaxTurns,
                Expr::value(i32::try_from(turns).unwrap_or(i32::MAX)),
            );
        }
        if let Some(tokens) = max_tokens {
            update = update.col_expr(
                party::Column::MaxTokens,
                Expr::value(i64::try_from(tokens).unwrap_or(i64::MAX)),
            );
        }
        let updated = update
            .filter(party::Column::Id.eq(id.get()))
            .exec(&self.conn)
            .await?;
        Ok(updated.rows_affected > 0)
    }

    /// Counts a model call on party `id`: its tokens and cost, and a turn when `turn` is set.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn add_use(
        &self,
        id: PartyId,
        turn: bool,
        tokens: u64,
        cost: f64,
    ) -> Result<(), DbError> {
        let tokens = i64::try_from(tokens).unwrap_or(i64::MAX);
        let mut update = party::Entity::update_many()
            .col_expr(
                party::Column::Tokens,
                Expr::col(party::Column::Tokens).add(tokens),
            )
            .col_expr(
                party::Column::Cost,
                Expr::col(party::Column::Cost).add(cost),
            );
        if turn {
            update = update.col_expr(party::Column::Turns, Expr::col(party::Column::Turns).add(1));
        }
        update
            .filter(party::Column::Id.eq(id.get()))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Deletes party `id` and its talk; `false` when there was no such party.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn delete(&self, id: PartyId) -> Result<bool, DbError> {
        let txn = begin_write(&self.conn).await?;
        party_message::Entity::delete_many()
            .filter(party_message::Column::Party.eq(id.get()))
            .exec(&txn)
            .await?;
        let deleted = party::Entity::delete_by_id(id.get()).exec(&txn).await?;
        txn.commit().await?;
        Ok(deleted.rows_affected > 0)
    }

    /// Pauses every running party, saying `why`: at start-up, nothing runs a bill up on its own.
    /// Answers how many were paused.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn pause_running(&self, why: &str, now: Timestamp) -> Result<u64, DbError> {
        let updated = party::Entity::update_many()
            .col_expr(
                party::Column::Status,
                Expr::value(PartyStatus::Paused.as_str()),
            )
            .col_expr(party::Column::Why, Expr::value(why))
            .col_expr(party::Column::Updated, Expr::value(millis(now)))
            .filter(party::Column::Status.eq(PartyStatus::Running.as_str()))
            .exec(&self.conn)
            .await?;
        Ok(updated.rows_affected)
    }
}

/// Adds `message` to party `id` on `conn`, and notes the party changed at `now`.
async fn add(
    conn: &impl ConnectionTrait,
    id: PartyId,
    message: &NewPartyMessage,
    now: Timestamp,
) -> Result<PartyMessage, DbError> {
    let row = party_message::ActiveModel {
        party: Set(id.get()),
        who: Set(message.who.clone()),
        name: Set(message.name.clone()),
        kind: Set(message.kind.as_str().to_owned()),
        text: Set(message.text.clone()),
        model: Set(message.model.clone()),
        tokens: Set(i64::try_from(message.tokens).unwrap_or(i64::MAX)),
        cost: Set(message.cost),
        cut: Set(message.cut),
        at: Set(millis(now)),
        ..Default::default()
    };
    let added = party_message::Entity::insert(row).exec(conn).await?;
    party::Entity::update_many()
        .col_expr(party::Column::Updated, Expr::value(millis(now)))
        .filter(party::Column::Id.eq(id.get()))
        .exec(conn)
        .await?;
    Ok(PartyMessage {
        id: PartyMessageId::new(added.last_insert_id),
        party: id,
        who: message.who.clone(),
        name: message.name.clone(),
        kind: message.kind,
        text: message.text.clone(),
        model: message.model.clone(),
        tokens: message.tokens,
        cost: message.cost,
        cut: message.cut,
        at: now,
    })
}

/// The first `max` characters of `text`, with an ellipsis when it was longer.
fn preview(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    }
}

/// A cast as the `personas` column keeps it.
fn personas_json(personas: &[Persona]) -> String {
    Value::Array(
        personas
            .iter()
            .map(|p| {
                json!({
                    "id": p.id, "name": p.name, "role": p.role, "model": p.model, "color": p.color,
                })
            })
            .collect(),
    )
    .to_string()
}

/// The cast a `personas` column holds; anything unreadable is no one.
fn personas_from_json(text: &str) -> Vec<Persona> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let field = |item: &Value, key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    items
        .iter()
        .filter(|item| item.is_object())
        .map(|item| Persona {
            id: field(item, "id"),
            name: field(item, "name"),
            role: field(item, "role"),
            model: field(item, "model"),
            color: field(item, "color"),
        })
        .collect()
}

fn from_row(row: party::Model) -> Result<Party, DbError> {
    Ok(Party {
        id: PartyId::new(row.id),
        personas: personas_from_json(&row.personas),
        title: row.title,
        topic: row.topic,
        status: PartyStatus::parse(&row.status),
        why: row.why,
        turns: u32::try_from(row.turns).unwrap_or_default(),
        max_turns: u32::try_from(row.max_turns).unwrap_or_default(),
        tokens: u64::try_from(row.tokens).unwrap_or_default(),
        max_tokens: u64::try_from(row.max_tokens).unwrap_or_default(),
        cost: row.cost,
        created: instant(row.created)?,
        updated: instant(row.updated)?,
    })
}

fn message_from_row(row: party_message::Model) -> Result<PartyMessage, DbError> {
    Ok(PartyMessage {
        id: PartyMessageId::new(row.id),
        party: PartyId::new(row.party),
        who: row.who,
        name: row.name,
        kind: MessageKind::parse(&row.kind),
        text: row.text,
        model: row.model,
        tokens: u64::try_from(row.tokens).unwrap_or_default(),
        cost: row.cost,
        cut: row.cut,
        at: instant(row.at)?,
    })
}
