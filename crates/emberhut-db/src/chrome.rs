//! The owner's Chrome profiles paired through the browser extension (`chrome_devices`), and the
//! browser each task works in (`chrome_tasks`).

use jiff::Timestamp;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};

use crate::DbError;
use crate::entity::{chrome_device, chrome_task};
use crate::time::{instant, millis};

/// A paired profile, as the server reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChromeProfileRow {
    /// Its id.
    pub id: i64,
    /// Its short name for the tools (`chrome:<name>`).
    pub name: String,
    /// The name the owner gave it, when it differs from `name`.
    pub label: Option<String>,
    /// The Google account the profile is signed in to; empty when it is signed in to none.
    pub email: Option<String>,
    /// The machine it runs on.
    pub machine: Option<String>,
    /// Its operating system, as the extension says (`mac`, `win`, `linux`).
    pub os: Option<String>,
    /// The extension's version.
    pub version: Option<String>,
    /// Chrome's version.
    pub agent: Option<String>,
    /// The extension's id.
    pub ext: Option<String>,
    /// When it was paired.
    pub created: Timestamp,
    /// When it was last connected.
    pub seen: Option<Timestamp>,
}

impl ChromeProfileRow {
    /// The name it's shown by: its label, or its short name.
    #[must_use]
    pub fn label(&self) -> &str {
        self.label
            .as_deref()
            .filter(|l| !l.is_empty())
            .unwrap_or(&self.name)
    }
}

/// What the extension says about itself, kept on its row. A field it doesn't say keeps what was
/// there; `email: Some("")` says the profile is signed in to no account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChromeAbout {
    /// The extension's version.
    pub version: Option<String>,
    /// The profile's Google account.
    pub email: Option<String>,
    /// The operating system.
    pub os: Option<String>,
    /// Chrome's version.
    pub agent: Option<String>,
    /// The extension's id.
    pub ext: Option<String>,
}

/// Reads and writes the paired profiles.
#[derive(Clone, Debug)]
pub struct ChromeStore {
    conn: DatabaseConnection,
}

impl ChromeStore {
    pub(crate) fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Every paired profile (not unpaired), oldest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn profiles(&self) -> Result<Vec<ChromeProfileRow>, DbError> {
        chrome_device::Entity::find()
            .filter(chrome_device::Column::RevokedAt.is_null())
            .order_by_asc(chrome_device::Column::CreatedAt)
            .order_by_asc(chrome_device::Column::Id)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(row)
            .collect()
    }

    /// The paired profile `id`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn profile(&self, id: i64) -> Result<Option<ChromeProfileRow>, DbError> {
        Ok(self.profiles().await?.into_iter().find(|r| r.id == id))
    }

    /// The paired profile whose token hashes to `hash`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn by_token(&self, hash: &str) -> Result<Option<ChromeProfileRow>, DbError> {
        chrome_device::Entity::find()
            .filter(chrome_device::Column::Token.eq(hash))
            .filter(chrome_device::Column::RevokedAt.is_null())
            .one(&self.conn)
            .await?
            .map(row)
            .transpose()
    }

    /// Pairs a new profile `name` (`label`), with its token's `hash`, at `at`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn insert(
        &self,
        name: &str,
        label: &str,
        hash: &str,
        at: Timestamp,
    ) -> Result<ChromeProfileRow, DbError> {
        let model = chrome_device::Entity::insert(chrome_device::ActiveModel {
            name: Set(name.to_owned()),
            label: Set(Some(label.to_owned())),
            token: Set(hash.to_owned()),
            created_at: Set(millis(at)),
            ..Default::default()
        })
        .exec_with_returning(&self.conn)
        .await?;
        row(model)
    }

    /// Gives profile `id` a new token `hash` (a re-pair).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_token(&self, id: i64, hash: &str) -> Result<(), DbError> {
        chrome_device::Entity::update_many()
            .col_expr(chrome_device::Column::Token, Expr::value(hash))
            .filter(chrome_device::Column::Id.eq(id))
            .filter(chrome_device::Column::RevokedAt.is_null())
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Unpairs profile `id` at `at`; the tasks that worked in it have no browser of their own.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn revoke(&self, id: i64, at: Timestamp) -> Result<(), DbError> {
        chrome_device::Entity::update_many()
            .col_expr(chrome_device::Column::RevokedAt, Expr::value(millis(at)))
            .filter(chrome_device::Column::Id.eq(id))
            .exec(&self.conn)
            .await?;
        chrome_task::Entity::delete_many()
            .filter(chrome_task::Column::Device.eq(id))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Renames profile `id`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn rename(&self, id: i64, name: &str, label: &str) -> Result<(), DbError> {
        chrome_device::Entity::update_many()
            .col_expr(chrome_device::Column::Name, Expr::value(name))
            .col_expr(chrome_device::Column::Label, Expr::value(label))
            .filter(chrome_device::Column::Id.eq(id))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Keeps what the extension says about itself on profile `id`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn about(&self, id: i64, about: ChromeAbout) -> Result<(), DbError> {
        let mut update = chrome_device::Entity::update_many();
        let fields = [
            (chrome_device::Column::Version, about.version),
            (chrome_device::Column::Email, about.email),
            (chrome_device::Column::Os, about.os),
            (chrome_device::Column::Agent, about.agent),
            (chrome_device::Column::Ext, about.ext),
        ];
        let mut any = false;
        for (column, value) in fields {
            if let Some(value) = value {
                update = update.col_expr(column, Expr::value(value));
                any = true;
            }
        }
        if any {
            update
                .filter(chrome_device::Column::Id.eq(id))
                .exec(&self.conn)
                .await?;
        }
        Ok(())
    }

    /// Profile `id` runs on `machine`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_machine(&self, id: i64, machine: &str) -> Result<(), DbError> {
        chrome_device::Entity::update_many()
            .col_expr(chrome_device::Column::Machine, Expr::value(machine))
            .filter(chrome_device::Column::Id.eq(id))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// Profile `id` was there at `at`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn seen(&self, id: i64, at: Timestamp) -> Result<(), DbError> {
        chrome_device::Entity::update_many()
            .col_expr(chrome_device::Column::SeenAt, Expr::value(millis(at)))
            .filter(chrome_device::Column::Id.eq(id))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// The browser task `task` works in: a profile's id, 0 for the server's own browser, or
    /// none when it has none of its own.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn task_browser(&self, task: i64) -> Result<Option<i64>, DbError> {
        Ok(chrome_task::Entity::find_by_id(task)
            .one(&self.conn)
            .await?
            .map(|t| t.device))
    }

    /// Task `task` works in `device` (0: the server's own browser) from `at` on.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_task_browser(
        &self,
        task: i64,
        device: i64,
        at: Timestamp,
    ) -> Result<(), DbError> {
        chrome_task::Entity::insert(chrome_task::ActiveModel {
            task: Set(task),
            device: Set(device),
            at: Set(millis(at)),
        })
        .on_conflict(
            OnConflict::column(chrome_task::Column::Task)
                .update_columns([chrome_task::Column::Device, chrome_task::Column::At])
                .to_owned(),
        )
        .exec(&self.conn)
        .await?;
        Ok(())
    }

    /// Task `task` has no browser of its own.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn clear_task_browser(&self, task: i64) -> Result<(), DbError> {
        chrome_task::Entity::delete_by_id(task)
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    /// The tasks that work in profile `device`, by id.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn tasks_of(&self, device: i64) -> Result<Vec<i64>, DbError> {
        Ok(chrome_task::Entity::find()
            .filter(chrome_task::Column::Device.eq(device))
            .order_by_asc(chrome_task::Column::Task)
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|t| t.task)
            .collect())
    }

    /// The tasks of profile `from` work in `to` now (it was paired again under another name).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn move_tasks(&self, from: i64, to: i64) -> Result<(), DbError> {
        chrome_task::Entity::update_many()
            .col_expr(chrome_task::Column::Device, Expr::value(to))
            .filter(chrome_task::Column::Device.eq(from))
            .exec(&self.conn)
            .await?;
        Ok(())
    }
}

fn row(model: chrome_device::Model) -> Result<ChromeProfileRow, DbError> {
    Ok(ChromeProfileRow {
        id: model.id,
        name: model.name,
        label: model.label,
        email: model.email,
        machine: model.machine,
        os: model.os,
        version: model.version,
        agent: model.agent,
        ext: model.ext,
        created: instant(model.created_at)?,
        seen: model.seen_at.map(instant).transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    async fn store() -> (tempfile::TempDir, ChromeStore) {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("t.db").display())
            .parse()
            .unwrap();
        let db = Database::connect(&url).await.unwrap();
        (dir, db.chrome())
    }

    #[tokio::test]
    async fn chrome_devices_pair_rename_and_unpair_with_their_tasks() {
        let (_dir, chrome) = store().await;
        let at = Timestamp::from_second(1_700_000_000).unwrap_or_default();
        let work = chrome.insert("work", "Work", "hash-1", at).await.ok();
        let work = work.unwrap();
        assert_eq!(work.label(), "Work");
        assert_eq!(
            chrome.by_token("hash-1").await.ok().flatten().map(|r| r.id),
            Some(work.id)
        );
        chrome
            .about(
                work.id,
                ChromeAbout {
                    version: Some("0.6.5".into()),
                    email: Some(String::new()),
                    ..ChromeAbout::default()
                },
            )
            .await
            .ok();
        chrome.rename(work.id, "office", "Office").await.ok();
        let row = chrome.profile(work.id).await.ok().flatten();
        let row = row.unwrap();
        assert_eq!((row.name.as_str(), row.label()), ("office", "Office"));
        assert_eq!(row.version.as_deref(), Some("0.6.5"));
        assert_eq!(row.email.as_deref(), Some(""));

        chrome.set_task_browser(12, work.id, at).await.ok();
        chrome.set_task_browser(13, 0, at).await.ok();
        assert_eq!(chrome.task_browser(12).await.ok().flatten(), Some(work.id));
        assert_eq!(chrome.tasks_of(work.id).await.ok(), Some(vec![12]));
        chrome.revoke(work.id, at).await.ok();
        assert_eq!(chrome.profiles().await.ok().map(|p| p.len()), Some(0));
        assert_eq!(chrome.task_browser(12).await.ok().flatten(), None);
        assert_eq!(chrome.task_browser(13).await.ok().flatten(), Some(0));
        chrome.clear_task_browser(13).await.ok();
        assert_eq!(chrome.task_browser(13).await.ok().flatten(), None);
    }
}
