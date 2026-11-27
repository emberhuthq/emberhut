//! The assistant's running awareness, as the `context` tool and the state's `contextAt` read
//! it: its open loops (who waits on the owner, promises, deadlines), its recent notes, when it
//! last looked through the owner's chats, and its routines and quiet hours.
//!
//! They are kept under these setting keys and shapes (`context`:
//! `{ loops, notes, at }`, `routines`, `quiet`), by the areas that change them (the loops, the
//! scout, the routines); this service only reads them, and marks the loops that may be
//! answered or have gone quiet.

use std::sync::Arc;

use emberhut_core::Clock;
use emberhut_db::{DbError, SettingKey, SettingStore};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The awareness: `{ loops, notes, at }`.
pub const CONTEXT: SettingKey<Awareness> = SettingKey::new("context");

/// The daily routines.
const ROUTINES: SettingKey<Value> = SettingKey::new("routines");

/// The quiet hours, `{ from, to }`.
const QUIET: SettingKey<Value> = SettingKey::new("quiet");

/// A loop with no message in its chat and no update for this long has gone quiet.
const QUIET_LOOP: SignedDuration = SignedDuration::from_hours(3 * 24);

/// The awareness as stored. Each loop and note is kept whole, whatever fields it has.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Awareness {
    /// The open loops: `{ id, who, what, due, source, since, chat?, touched?, maybe?, room? }`.
    #[serde(default)]
    pub loops: Vec<Map<String, Value>>,
    /// The assistant's notes: `{ at, text }`.
    #[serde(default)]
    pub notes: Vec<Value>,
    /// When the owner's chats were last looked through.
    #[serde(default)]
    pub at: Option<String>,
}

/// Reads the awareness.
#[derive(Clone, Debug)]
pub struct ContextService {
    settings: SettingStore,
    clock: Arc<dyn Clock>,
}

impl ContextService {
    /// A service reading `settings`.
    #[must_use]
    pub fn new(settings: SettingStore, clock: Arc<dyn Clock>) -> Self {
        Self { settings, clock }
    }

    /// The awareness as stored (empty when none was ever kept).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn awareness(&self) -> Result<Awareness, DbError> {
        Ok(self.settings.get(&CONTEXT).await?.unwrap_or_default())
    }

    /// When the owner's chats were last looked through; none: never.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn context_at(&self) -> Result<Option<emberhut_api::time::Timestamp>, DbError> {
        Ok(self
            .awareness()
            .await?
            .at
            .map(emberhut_api::time::Timestamp::new))
    }

    /// The `context` tool's answer: the awareness, each loop marked `maybeAnswered` when the
    /// owner wrote in its chat since it opened and `stale` when it has gone quiet, with the
    /// routines and quiet hours.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn context(&self) -> Result<Value, DbError> {
        let awareness = self.awareness().await?;
        let now = self.clock.now();
        let loops: Vec<Value> = awareness
            .loops
            .into_iter()
            .map(|mut open| {
                if let Some(maybe) = open
                    .get("maybe")
                    .and_then(Value::as_str)
                    .filter(|maybe| !maybe.is_empty())
                {
                    let note = format!("The user wrote in its chat at {maybe}");
                    open.insert("maybeAnswered".to_owned(), Value::from(note));
                }
                if is_stale(&open, now) {
                    open.insert(
                        "stale".to_owned(),
                        Value::from(
                            "3+ days with no message in its chat and no update: close it or chase it",
                        ),
                    );
                }
                Value::Object(open)
            })
            .collect();
        Ok(json!({
            "loops": loops,
            "notes": awareness.notes,
            "at": awareness.at,
            "routines": self.settings.get(&ROUTINES).await?.unwrap_or_else(|| json!([])),
            "quiet": self.settings.get(&QUIET).await?,
        }))
    }
}

/// Whether `open` had no message in its chat and no update for [`QUIET_LOOP`].
fn is_stale(open: &Map<String, Value>, now: Timestamp) -> bool {
    let last = ["touched", "since"].iter().find_map(|key| {
        open.get(*key)
            .and_then(Value::as_str)
            .filter(|at| !at.is_empty())
    });
    last.and_then(|at| at.parse::<Timestamp>().ok())
        .is_some_and(|at| now.duration_since(at) > QUIET_LOOP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_goes_quiet_after_three_days() {
        let now: Timestamp = "2026-11-24T10:00:00Z".parse().unwrap();
        let open = |since: &str, touched: Option<&str>| {
            let mut open = Map::new();
            open.insert("since".to_owned(), Value::from(since));
            if let Some(touched) = touched {
                open.insert("touched".to_owned(), Value::from(touched));
            }
            open
        };
        assert!(is_stale(&open("2026-11-20T10:00:00Z", None), now));
        assert!(!is_stale(
            &open("2026-11-20T10:00:00Z", Some("2026-11-23T10:00:00Z")),
            now
        ));
        assert!(!is_stale(&open("2026-11-22T10:00:00Z", None), now));
        assert!(!is_stale(&open("not a time", None), now));
    }
}
