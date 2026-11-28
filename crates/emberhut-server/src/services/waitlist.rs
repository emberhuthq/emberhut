//! The waitlists (`/api/waitlist`): what the owner asked to hear about once it exists, kept in
//! the settings table of this server until a central list exists. Nothing is sent anywhere and
//! nothing is paid. Joining again keeps the first time.

use std::sync::Arc;

use emberhut_api::waitlist::{CLOUD_HOME, Waitlist, Waitlists};
use emberhut_core::Clock;
use emberhut_db::{DbError, SettingKey, SettingStore};
use serde::{Deserialize, Serialize};

use crate::convert::timestamp_to_api;

/// The lists there are: a home rented in the cloud.
const LISTS: [&str; 1] = [CLOUD_HOME];

/// When the owner joined a list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Joined {
    joined_at: String,
}

/// The waitlists on this server.
#[derive(Clone, Debug)]
pub struct WaitlistService {
    settings: SettingStore,
    clock: Arc<dyn Clock>,
}

impl WaitlistService {
    /// The waitlists kept in `settings`.
    #[must_use]
    pub fn new(settings: SettingStore, clock: Arc<dyn Clock>) -> Self {
        Self { settings, clock }
    }

    /// Every list, and whether the owner is on it.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the settings can't be read.
    pub async fn all(&self) -> Result<Waitlists, DbError> {
        let mut lists = Vec::with_capacity(LISTS.len());
        for id in LISTS {
            lists.push(self.list(id).await?);
        }
        Ok(Waitlists { lists })
    }

    /// Puts the owner on `id`'s list (the first time is kept); `None` when there is no such
    /// list.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the settings can't be read or written.
    pub async fn join(&self, id: &str) -> Result<Option<Waitlist>, DbError> {
        let Some(id) = LISTS.into_iter().find(|list| *list == id) else {
            return Ok(None);
        };
        if self.settings.get(&key(id)).await?.is_none() {
            let joined_at = timestamp_to_api(self.clock.now()).as_str().to_owned();
            self.settings.set(&key(id), &Joined { joined_at }).await?;
        }
        self.list(id).await.map(Some)
    }

    /// `id`'s list as it is now.
    async fn list(&self, id: &'static str) -> Result<Waitlist, DbError> {
        let joined = self.settings.get(&key(id)).await?;
        Ok(Waitlist {
            id: id.to_owned(),
            joined: joined.is_some(),
            joined_at: joined.map(|joined| emberhut_api::time::Timestamp::new(joined.joined_at)),
        })
    }
}

/// The setting `id`'s list is kept in (`waitlist:cloud-home`).
fn key(id: &str) -> SettingKey<Joined> {
    SettingKey::named(format!("waitlist:{id}"))
}
