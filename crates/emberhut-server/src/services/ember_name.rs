//! The ember's name: what the owner calls their assistant, one name for every device
//! (`GET`/`POST /api/ember/name`, and the state's `emberName`). The assistant's own prompt and
//! its reply alerts read it too, so it calls itself by it.

use std::sync::Arc;

use emberhut_api::ember_name::{EmberName, NameRefusal, clean_name};
use emberhut_api::time::Timestamp;
use emberhut_core::Clock;
use emberhut_db::{DbError, LogStore, SettingKey, SettingStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::convert::timestamp_to_api;

/// The setting the name is kept under (`{ name, at }`; `name` null: the default).
const NAME: SettingKey<Saved> = SettingKey::new("emberName");

/// The name as it is kept: the owner's own (none: "Your ember"), and when it changed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    at: Option<Timestamp>,
}

/// Why a name was not changed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EmberNameError {
    /// A name the server doesn't take; the text is what the app shows.
    #[error(transparent)]
    Refused(#[from] NameRefusal),
    /// The body is not `{ name }` with text or `null`.
    #[error("name: text, or null for the default")]
    NotAChange,
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// The owner's own name for the ember, kept in `settings`; none when they gave none. What the
/// assistant's prompt and alerts read.
///
/// # Errors
///
/// [`DbError`].
pub async fn custom_name(settings: &SettingStore) -> Result<Option<String>, DbError> {
    Ok(settings.get(&NAME).await?.and_then(|saved| saved.name))
}

/// What the assistant's prompts say after "You are the owner's personal assistant..." when the
/// owner named it: a sentence that starts with a space.
#[must_use]
pub fn told(name: &str) -> String {
    format!(
        " The owner named you {name}: that is your name. Call yourself {name}, and answer to it."
    )
}

/// Keeps the ember's name.
#[derive(Clone, Debug)]
pub struct EmberNameService {
    settings: SettingStore,
    log: LogStore,
    clock: Arc<dyn Clock>,
}

impl EmberNameService {
    /// A service keeping the name in `settings`, noting changes in `log`.
    #[must_use]
    pub fn new(settings: SettingStore, log: LogStore, clock: Arc<dyn Clock>) -> Self {
        Self {
            settings,
            log,
            clock,
        }
    }

    /// The name: the one last saved, or "Your ember".
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn name(&self) -> Result<EmberName, DbError> {
        let saved = self.settings.get(&NAME).await?.unwrap_or_default();
        Ok(EmberName::new(saved.name, saved.at))
    }

    /// Changes the name to `change`'s (`{ name }`): trimmed, its runs of spaces made one, one
    /// line of 1 to 24 characters (an emoji is one); `null`, blank or "Your ember" go back to the
    /// default.
    ///
    /// # Errors
    ///
    /// [`EmberNameError::NotAChange`] for a body without `name` as text or `null`,
    /// [`EmberNameError::Refused`] for a name not taken (nothing is changed either way);
    /// [`EmberNameError::Database`].
    pub async fn change(&self, change: &Value) -> Result<EmberName, EmberNameError> {
        let name = match change.get("name") {
            Some(Value::Null) => None,
            Some(Value::String(text)) => clean_name(text)?,
            _ => return Err(EmberNameError::NotAChange),
        };
        let now = self.clock.now();
        let saved = Saved {
            name,
            at: Some(timestamp_to_api(now)),
        };
        self.settings.set(&NAME, &saved).await?;
        let line = match &saved.name {
            Some(name) => format!("Your ember is now called {name}"),
            None => "Your ember's name is back to the default".to_owned(),
        };
        self.log.add(line, now).await?;
        Ok(EmberName::new(saved.name, saved.at))
    }
}
