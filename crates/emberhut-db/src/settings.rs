//! Settings: one JSON value per key, read and written as the type its key names.
//!
//! An area declares its keys once, with their type, and every read and write goes through
//! them:
//!
//! ```
//! use emberhut_db::SettingKey;
//!
//! /// Whether the weekly report is on.
//! const REPORT_ON: SettingKey<bool> = SettingKey::new("reportOn");
//! # assert_eq!(REPORT_ON.name(), "reportOn");
//! ```

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;

use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveValue::Set, ConnectionTrait, DatabaseConnection, EntityTrait};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::entity::setting;
use crate::{DbError, begin_write};

/// A setting's key, and the type of its value.
pub struct SettingKey<T> {
    name: Cow<'static, str>,
    value: PhantomData<fn() -> T>,
}

impl<T> SettingKey<T> {
    /// The key `name`.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name: Cow::Borrowed(name),
            value: PhantomData,
        }
    }

    /// A key made at run time (one per person, per machine, ...).
    #[must_use]
    pub fn named(name: String) -> Self {
        Self {
            name: Cow::Owned(name),
            value: PhantomData,
        }
    }

    /// The key as it is stored.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl<T> Clone for SettingKey<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            value: PhantomData,
        }
    }
}

impl<T> fmt::Debug for SettingKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SettingKey").field(&self.name).finish()
    }
}

/// The id of the machine key the database was served with (emberhut-server's, ADR 0007): the
/// server refuses to serve the database with another key, or none, unless asked to. Re-imports
/// that keep devices carry it with them.
pub const MACHINE_ID: SettingKey<String> = SettingKey::new("machineId");

/// Reads and writes settings.
#[derive(Clone, Debug)]
pub struct SettingStore {
    conn: DatabaseConnection,
}

impl SettingStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// The value under `key`; none when it was never set, or was set to `null`.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::Setting`] when the stored value is not a `T`.
    pub async fn get<T: DeserializeOwned>(
        &self,
        key: &SettingKey<T>,
    ) -> Result<Option<T>, DbError> {
        read(&self.conn, key).await
    }

    /// The value under `key`, or `fallback` when there is none.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub async fn get_or<T: DeserializeOwned>(
        &self,
        key: &SettingKey<T>,
        fallback: T,
    ) -> Result<T, DbError> {
        Ok(self.get(key).await?.unwrap_or(fallback))
    }

    /// Sets `key` to `value`, in place of what it was.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`], or [`DbError::Setting`] when `value` cannot be written as JSON.
    pub async fn set<T: Serialize>(&self, key: &SettingKey<T>, value: &T) -> Result<(), DbError> {
        write(&self.conn, key, value).await
    }

    /// Changes the value under `key` (or `fallback`, when there is none) with `change`, and
    /// returns what `change` returns, all at once: no other write to the database comes
    /// between the read and the write, so two callers taking the same thing out of a value
    /// can't both get it.
    ///
    /// # Errors
    ///
    /// As [`Self::get`] and [`Self::set`]; then nothing is changed.
    pub async fn update<T, R>(
        &self,
        key: &SettingKey<T>,
        fallback: T,
        change: impl FnOnce(&mut T) -> R,
    ) -> Result<R, DbError>
    where
        T: Serialize + DeserializeOwned,
    {
        let txn = begin_write(&self.conn).await?;
        let mut value = read(&txn, key).await?.unwrap_or(fallback);
        let changed = change(&mut value);
        write(&txn, key, &value).await?;
        txn.commit().await?;
        Ok(changed)
    }

    /// Removes `key`; `false` when it was not set.
    ///
    /// # Errors
    ///
    /// [`DbError::Query`].
    pub async fn remove<T>(&self, key: &SettingKey<T>) -> Result<bool, DbError> {
        let removed = setting::Entity::delete_by_id(key.name())
            .exec(&self.conn)
            .await?;
        Ok(removed.rows_affected > 0)
    }
}

/// The value under `key`, read through `conn`.
async fn read<T: DeserializeOwned>(
    conn: &impl ConnectionTrait,
    key: &SettingKey<T>,
) -> Result<Option<T>, DbError> {
    let Some(row) = setting::Entity::find_by_id(key.name()).one(conn).await? else {
        return Ok(None);
    };
    let unreadable = |source| DbError::Setting {
        key: key.name().to_owned(),
        source,
    };
    let value: serde_json::Value = serde_json::from_str(&row.value).map_err(unreadable)?;
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value).map(Some).map_err(unreadable)
}

/// Sets `key` to `value` through `conn`.
async fn write<T: Serialize>(
    conn: &impl ConnectionTrait,
    key: &SettingKey<T>,
    value: &T,
) -> Result<(), DbError> {
    let value = serde_json::to_string(value).map_err(|source| DbError::Setting {
        key: key.name().to_owned(),
        source,
    })?;
    setting::Entity::insert(setting::ActiveModel {
        key: Set(key.name().to_owned()),
        value: Set(value),
    })
    .on_conflict(
        OnConflict::column(setting::Column::Key)
            .update_column(setting::Column::Value)
            .to_owned(),
    )
    .exec_without_returning(conn)
    .await?;
    Ok(())
}
