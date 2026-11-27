//! Family members: the same assistant, as their own user, in their own rooms.
//!
//! A household server on this machine (the one that pairs the family's devices) passes a
//! family member's requests on under `/family/...`, the device's key as the bearer. Who the key
//! belongs to is the household server's answer ([`FamilyDirectory`]), kept a minute: only a
//! key it says is a family member's gets in, with that person's name. A family member sees
//! only their own rooms (a Home room is made the first time): the owner's API stays behind the
//! owner's sign-in, and the family routes reach only rooms with that person's name on them.
//! Those rooms are left out of the owner's room list, a reply in one pushes to that person's
//! phones (never the owner's), and their theme is their own. The assistant in their rooms has
//! a short list of tools: their reminders, what it knows about the home,
//! the owner's calendar (whether the owner is free), handing something to another room, and an
//! alert to the owner; nothing that acts as the owner (messages, tasks, the browser, the Mac).
//!
//! All of it is off for now ([`FAMILY_ROOMS`]): those tools still act on the whole
//! installation, so family rooms wait for tools scoped to the family member's own room.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use emberhut_core::push::{ApnsEnv, ApnsToken, PushTokenError};
use emberhut_core::room::Room;
use emberhut_core::{Clock, RoomId};
use emberhut_db::{DbError, FamilyStore};
use futures_util::future::BoxFuture;
use jiff::Timestamp;
use serde_json::Value;
use tokio::time::Instant;

use crate::http::read_body;
use crate::services::theme::ThemeService;

/// Whether family members' rooms are served. Off: a family member's tools would reach the
/// owner's whole installation (recall, facts, check-ins, the calendar, the owner's rooms) and
/// their sessions would run with the owner's settings, so family rooms are not enabled until
/// tools scoped to a family member's own room exist. While off, the `/family/...` routes, a
/// family room's tools and its pushes all refuse, and no family room is made.
pub const FAMILY_ROOMS: bool = false;

/// What is said while [`FAMILY_ROOMS`] is off.
pub const NOT_AVAILABLE: &str = "family rooms are not available yet";

/// How long the household server's answer about a key is kept.
const KEPT: Duration = Duration::from_secs(60);

/// How many answers are kept before the stale ones are dropped.
const KEEP_AT_MOST: usize = 500;

/// How long the household server gets to answer.
const ASK_WITHIN: Duration = Duration::from_secs(5);

/// The longest answer read from the household server: `{ role, person }`.
const ANSWER_LIMIT: usize = 64 << 10;

/// Who a device key belongs to: a family member's name, or no one.
pub trait FamilyDirectory: fmt::Debug + Send + Sync {
    /// The family member `key` belongs to; `None` for anyone else, or when the directory
    /// can't be asked.
    fn who_is<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Option<String>>;
}

/// No household server: no one is a family member.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFamily;

impl FamilyDirectory for NoFamily {
    fn who_is<'a>(&'a self, _key: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }
}

/// The household server's `GET /api/whoami`, asked with the key as the bearer: a key whose
/// answer is `{ role: "family", person }` (a lowercase name) is that person's.
pub struct HttpFamilyDirectory {
    client: reqwest::Client,
    url: reqwest::Url,
    known: Mutex<Known>,
}

/// The answers kept: by the key's SHA-256, the person (or no one) and when it was asked.
type Known = HashMap<Vec<u8>, (Option<String>, Instant)>;

impl fmt::Debug for HttpFamilyDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpFamilyDirectory")
            .field("url", &self.url.as_str())
            .finish_non_exhaustive()
    }
}

impl HttpFamilyDirectory {
    /// The household server at `url` (`http://127.0.0.1:8798`), asked with `client`.
    #[must_use]
    pub fn new(client: reqwest::Client, url: reqwest::Url) -> Self {
        Self {
            client,
            url,
            known: Mutex::default(),
        }
    }

    async fn ask(&self, key: &str) -> Result<Option<String>, reqwest::Error> {
        let url = format!("{}/api/whoami", self.url.as_str().trim_end_matches('/'));
        let response = self
            .client
            .get(url)
            .bearer_auth(key)
            .timeout(ASK_WITHIN)
            .send()
            .await?;
        let ok = response.status().is_success();
        let body = read_body(response, ANSWER_LIMIT).await?;
        let answer: Value = serde_json::from_slice(&body.bytes).unwrap_or(Value::Null);
        Ok(ok.then(|| family_member(&answer)).flatten())
    }
}

impl FamilyDirectory for HttpFamilyDirectory {
    fn who_is<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            if !is_key(key) {
                return None;
            }
            let digest = ring::digest::digest(&ring::digest::SHA256, key.as_bytes())
                .as_ref()
                .to_vec();
            let now = Instant::now();
            {
                let known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some((person, at)) = known.get(&digest)
                    && now.duration_since(*at) < KEPT
                {
                    return person.clone();
                }
            }
            // The household server down: refused, and not remembered.
            let person = match self.ask(key).await {
                Ok(person) => person,
                Err(error) => {
                    tracing::warn!(%error, "the household server could not be asked about a key");
                    return None;
                }
            };
            if person.is_none() {
                tracing::info!("a family sign-in was refused: not a family member's key");
            }
            let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
            if known.len() > KEEP_AT_MOST {
                known.retain(|_, (_, at)| now.duration_since(*at) < KEPT);
            }
            known.insert(digest, (person.clone(), now));
            person
        })
    }
}

/// Whether `key` can be a device key at all: 32 to 128 characters, no spaces.
fn is_key(key: &str) -> bool {
    (32..=128).contains(&key.chars().count()) && !key.chars().any(char::is_whitespace)
}

/// The family member a `whoami` answer names: role `family`, and a person's name of a
/// lowercase letter then 1 to 30 lowercase letters or dashes.
#[must_use]
pub fn family_member(answer: &Value) -> Option<String> {
    let role = answer.get("role").and_then(Value::as_str)?;
    let person = answer.get("person").and_then(Value::as_str)?;
    let mut chars = person.chars();
    let named = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && (2..=31).contains(&person.len())
        && chars.all(|c| c.is_ascii_lowercase() || c == '-');
    (role == "family" && named).then(|| person.to_owned())
}

/// A check-in a room asked for: when it is due, and what for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder {
    /// When it is due.
    pub due: Timestamp,
    /// What for.
    pub text: String,
}

/// The check-ins each room asked for: the assistant's wake-ups, wherever they are kept.
pub trait RoomReminders: fmt::Debug + Send + Sync {
    /// `room`'s check-ins, the soonest first.
    fn of(&self, room: RoomId) -> BoxFuture<'_, Vec<Reminder>>;
}

/// No check-ins anywhere: until the assistant's wake-ups are kept on this server.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoReminders;

impl RoomReminders for NoReminders {
    fn of(&self, _room: RoomId) -> BoxFuture<'_, Vec<Reminder>> {
        Box::pin(async { Vec::new() })
    }
}

/// Why a family request was not done.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FamilyError {
    /// Family rooms are off ([`FAMILY_ROOMS`]).
    #[error("{NOT_AVAILABLE}")]
    NotAvailable,
    /// Not a device token.
    #[error(transparent)]
    Token(#[from] PushTokenError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// Family members' rooms, theme and phones.
#[derive(Clone, Debug)]
pub struct FamilyService {
    store: FamilyStore,
    clock: Arc<dyn Clock>,
    directory: Arc<dyn FamilyDirectory>,
    reminders: Arc<dyn RoomReminders>,
    theme: ThemeService,
}

impl FamilyService {
    /// A service over `store`, asking `directory` who a key is.
    #[must_use]
    pub fn new(
        store: FamilyStore,
        clock: Arc<dyn Clock>,
        directory: Arc<dyn FamilyDirectory>,
        reminders: Arc<dyn RoomReminders>,
        theme: ThemeService,
    ) -> Self {
        Self {
            store,
            clock,
            directory,
            reminders,
            theme,
        }
    }

    /// The family member `key` belongs to, if any.
    pub async fn who_is(&self, key: &str) -> Option<String> {
        self.directory.who_is(key).await
    }

    /// `person`'s rooms and the one asked for (`asked`, when it is one of theirs; else their
    /// Home room, made the first time).
    ///
    /// # Errors
    ///
    /// [`FamilyError::NotAvailable`] while family rooms are off, [`FamilyError::Database`].
    pub async fn rooms(
        &self,
        person: &str,
        asked: Option<RoomId>,
    ) -> Result<(Room, Vec<Room>), FamilyError> {
        if !FAMILY_ROOMS {
            return Err(FamilyError::NotAvailable);
        }
        let home = self.store.home_room(person, self.clock.now()).await?;
        let mine = self.store.rooms_of(person).await?;
        let room = asked
            .and_then(|asked| mine.iter().find(|room| room.id == asked).cloned())
            .unwrap_or(home);
        Ok((room, mine))
    }

    /// The store, for the room's conversation.
    #[must_use]
    pub fn store(&self) -> &FamilyStore {
        &self.store
    }

    /// `room`'s check-ins.
    pub async fn reminders(&self, room: RoomId) -> Vec<Reminder> {
        self.reminders.of(room).await
    }

    /// `person`'s own theme.
    #[must_use]
    pub fn theme(&self, person: &str) -> ThemeService {
        self.theme.for_person(person)
    }

    /// Keeps `person`'s iPhone's push address.
    ///
    /// # Errors
    ///
    /// [`FamilyError::NotAvailable`] while family rooms are off, [`FamilyError::Token`] for a
    /// token that isn't one, [`FamilyError::Database`].
    pub async fn register_device(
        &self,
        person: &str,
        token: &str,
        env: Option<&str>,
    ) -> Result<(), FamilyError> {
        if !FAMILY_ROOMS {
            return Err(FamilyError::NotAvailable);
        }
        let token = ApnsToken::parse(token)?;
        self.store
            .register_device(&token, ApnsEnv::from_wire(env), person, self.clock.now())
            .await?;
        Ok(())
    }

    /// What time it is.
    #[must_use]
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn only_a_family_role_with_a_plain_name_is_a_family_member() {
        let member = |answer| family_member(&answer);
        assert_eq!(
            member(json!({ "role": "family", "person": "sam" })).as_deref(),
            Some("sam")
        );
        assert_eq!(member(json!({ "role": "owner", "person": "sam" })), None);
        assert_eq!(member(json!({ "role": "family", "person": "Sam" })), None);
        assert_eq!(member(json!({ "role": "family", "person": "s" })), None);
        assert_eq!(member(json!({ "role": "family" })), None);
        assert_eq!(member(json!(null)), None);
        assert!(is_key(&"k".repeat(32)));
        assert!(!is_key(&"k".repeat(31)));
        assert!(!is_key(&format!("{} x", "k".repeat(40))));
    }
}
