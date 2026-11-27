//! The assistant's accounts: each runner's accounts and their use ([`UsageStore`], the `usage`
//! table) and the person's small choices ([`PrefStore`], the `prefs` table): the pinned
//! account, and the MCP servers and skills turned off.

use emberhut_core::account::AccountUse;
use jiff::Timestamp;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};

use crate::entity::{pref, usage};
use crate::time::{instant, millis};
use crate::{DbError, begin_write};

/// Keeps what each runner last said of its accounts.
#[derive(Clone, Debug)]
pub struct UsageStore {
    conn: DatabaseConnection,
}

impl UsageStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Replaces `machine`'s accounts with `accounts` (their `machine` is taken as `machine`),
    /// received at `at`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn replace(
        &self,
        machine: &str,
        accounts: &[AccountUse],
        at: Timestamp,
    ) -> Result<(), DbError> {
        let txn = begin_write(&self.conn).await?;
        usage::Entity::delete_many()
            .filter(usage::Column::Machine.eq(machine))
            .exec(&txn)
            .await?;
        for account in accounts {
            usage::Entity::insert(to_row(machine, account, at))
                .on_conflict(
                    OnConflict::columns([usage::Column::Machine, usage::Column::Id])
                        .update_columns([usage::Column::Name, usage::Column::At])
                        .to_owned(),
                )
                .exec(&txn)
                .await?;
        }
        txn.commit().await?;
        Ok(())
    }

    /// Every account, by machine, each machine's in its runner's order (the ranked ones first).
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::InstantOutOfRange`] for a corrupt row.
    pub async fn all(&self) -> Result<Vec<AccountUse>, DbError> {
        let mut rows = usage::Entity::find()
            .order_by_asc(usage::Column::Machine)
            .order_by_asc(usage::Column::Id)
            .all(&self.conn)
            .await?;
        // Ranked first, lowest first; then the unranked, by id.
        rows.sort_by(|a, b| {
            a.machine
                .cmp(&b.machine)
                .then_with(|| a.rank.is_none().cmp(&b.rank.is_none()))
                .then_with(|| a.rank.cmp(&b.rank))
        });
        rows.into_iter().map(from_row).collect()
    }
}

fn to_row(machine: &str, account: &AccountUse, at: Timestamp) -> usage::ActiveModel {
    usage::ActiveModel {
        machine: Set(machine.to_owned()),
        id: Set(account.id.clone()),
        name: Set(account.name.clone()),
        scope: Set(account.scope.clone()),
        enabled: Set(account.enabled),
        logged_in: Set(account.logged_in),
        rank: Set(account.rank),
        email: Set(account.email.clone()),
        plan: Set(account.plan.clone()),
        used5h: Set(account.used5h),
        used7d: Set(account.used7d),
        resets5h: Set(account.resets5h.map(millis)),
        resets7d: Set(account.resets7d.map(millis)),
        limited_until: Set(account.limited_until.map(millis)),
        limited_why: Set(account.limited_why.clone()),
        checked: Set(account.checked.map(millis)),
        error: Set(account.error.clone()),
        at: Set(millis(at)),
    }
}

fn from_row(row: usage::Model) -> Result<AccountUse, DbError> {
    let when = |at: Option<i64>| at.map(instant).transpose();
    Ok(AccountUse {
        machine: row.machine,
        id: row.id,
        name: row.name,
        scope: row.scope,
        enabled: row.enabled,
        logged_in: row.logged_in,
        rank: row.rank,
        email: row.email,
        plan: row.plan,
        used5h: row.used5h,
        used7d: row.used7d,
        resets5h: when(row.resets5h)?,
        resets7d: when(row.resets7d)?,
        limited_until: when(row.limited_until)?,
        limited_why: row.limited_why,
        checked: when(row.checked)?,
        error: row.error,
    })
}

/// The keys of [`PrefStore`]'s values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrefKey {
    /// The account the person pinned the assistant to (none: automatic).
    Pin,
    /// The account the assistant's main room runs on now, as its runtime last said.
    On,
    /// The MCP servers the person turned off (a JSON list of names).
    McpOff,
    /// The skills the person turned off (a JSON list of names).
    SkillsOff,
    /// The MCP servers the assistant's session had (JSON, by name).
    McpSeen,
    /// The skills the assistant's session had (JSON, by name).
    SkillsSeen,
}

impl PrefKey {
    /// The key as stored.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pin => "assistant-pin",
            Self::On => "assistant-on",
            Self::McpOff => "mcpOff",
            Self::SkillsOff => "skillsOff",
            Self::McpSeen => "mcpSeen",
            Self::SkillsSeen => "skillsSeen",
        }
    }
}

/// Small text values by key.
#[derive(Clone, Debug)]
pub struct PrefStore {
    conn: DatabaseConnection,
}

impl PrefStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// `key`'s value, if it has one.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn get(&self, key: PrefKey) -> Result<Option<String>, DbError> {
        Ok(pref::Entity::find_by_id(key.as_str().to_owned())
            .one(&self.conn)
            .await?
            .map(|row| row.value))
    }

    /// Sets `key` to `value`, or clears it with `None`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn set(&self, key: PrefKey, value: Option<&str>) -> Result<(), DbError> {
        match value {
            None => {
                pref::Entity::delete_by_id(key.as_str().to_owned())
                    .exec(&self.conn)
                    .await?;
            }
            Some(value) => {
                pref::Entity::insert(pref::ActiveModel {
                    key: Set(key.as_str().to_owned()),
                    value: Set(value.to_owned()),
                })
                .on_conflict(
                    OnConflict::column(pref::Column::Key)
                        .update_column(pref::Column::Value)
                        .to_owned(),
                )
                .exec(&self.conn)
                .await?;
            }
        }
        Ok(())
    }
}
