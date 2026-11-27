//! The emberhut.com account directory (ADR 0007, section 5): it lists an account's machines and
//! tells the relay which machines may connect. Its API is `docs/directory.md`, version 1, at
//! `/v1` on the relay domain.
//!
//! - [`Directory`] serves the API ([`Directory::router`]) over its own SQLite file, checking
//!   account tokens with an [`AccountVerifier`] ([`ClerkVerifier`] for Clerk's).
//! - Link and unlink statements are the machine's (`emberhut link v1`, `emberhut unlink v1`): their
//!   signature, account and relay are checked by `emberhut-api`'s types, their freshness (5 minutes
//!   either side of the directory's [`Clock`]) and replays here.
//! - Gateways look machines up and hear of unlinks through [`GatewayDirectory`];
//!   [`Directory::gateway`] is the in-process implementation.

use std::fmt;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::connect::MachineIdentity;
use emberhut_api::directory::LinkedMachine;
use emberhut_identity::{MachineId, RelayDomain, UnixMillis};
use tokio::sync::broadcast;

mod account;
mod clock;
mod gateway;
mod limit;
mod refusal;
mod routes;
mod store;

pub use account::{
    AccountRefusal, AccountVerifier, ClerkSettings, ClerkSetupError, ClerkVerifier, JwksUrl,
    JwksUrlError, KeyTiming,
};
pub use clock::{Clock, SystemClock};
pub use gateway::{GatewayDirectory, InProcessDirectory, LookupError, MachineStatus, Presence};
pub use limit::{Rate, RequestLimits};
pub use store::StoreError;

use limit::Limiter;
use store::{Store, StoredMachine};

/// How far a statement's time may be from the directory's clock, either way.
const FRESH_FOR: Duration = Duration::from_mins(5);

/// The directory's settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryConfig {
    /// The relay domain: statements must name it, and machines' relay URLs are on it.
    pub relay_domain: RelayDomain,
    /// The directory's SQLite file.
    pub database: PathBuf,
    /// How many machines one account may link (ADR 0007's open question 5: 5).
    pub account_cap: NonZeroU32,
    /// The request limits.
    pub limits: RequestLimits,
}

/// The account directory: its settings, database and token checks. Cheap to clone.
pub struct Directory<V> {
    shared: Arc<Shared<V>>,
}

impl<V> Clone for Directory<V> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<V> fmt::Debug for Directory<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Directory")
            .field("config", &self.shared.config)
            .finish_non_exhaustive()
    }
}

/// What every request reaches.
struct Shared<V> {
    config: DirectoryConfig,
    verifier: V,
    store: Store,
    clock: Arc<dyn Clock>,
    unlinks: broadcast::Sender<MachineId>,
    per_account: Limiter,
    per_machine: Limiter,
}

impl<V: AccountVerifier> Directory<V> {
    /// The directory of `config`, its database opened (made and migrated if needed), checking
    /// tokens with `verifier` and telling the time by `clock`.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the database can't be opened or migrated.
    pub async fn open(
        config: DirectoryConfig,
        verifier: V,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, StoreError> {
        let store = Store::open(&config.database).await?;
        let (unlinks, _) = broadcast::channel(gateway::UNLINK_BACKLOG);
        Ok(Self {
            shared: Arc::new(Shared {
                per_account: Limiter::new(config.limits.per_account),
                per_machine: Limiter::new(config.limits.per_machine),
                config,
                verifier,
                store,
                clock,
                unlinks,
            }),
        })
    }

    /// The directory's API, with its CORS for the web app.
    pub fn router(&self) -> axum::Router {
        routes::router(Arc::clone(&self.shared))
    }
}

impl<V> Directory<V> {
    /// The directory, for a gateway in the same process.
    #[must_use]
    pub fn gateway(&self) -> InProcessDirectory {
        InProcessDirectory::new(
            self.shared.store.clone(),
            self.shared.unlinks.clone(),
            Arc::clone(&self.shared.clock),
        )
    }
}

impl<V> Shared<V> {
    /// A stored machine as the API lists it, at `now`.
    fn listed(&self, machine: StoredMachine, now: UnixMillis) -> LinkedMachine {
        LinkedMachine {
            relay_url: format!(
                "wss://{}/v1/sealed/{}",
                self.config.relay_domain, machine.id
            ),
            machine: MachineIdentity::of(machine.public_key),
            name: machine.name,
            connected: machine.connected,
            // A connected machine is up now.
            last_seen: if machine.connected {
                Some(now)
            } else {
                machine.last_seen
            },
            linked_at: machine.linked_at,
        }
    }

    /// Tells the gateways that `id` was unlinked, so the one holding it closes its uplink.
    fn announce_unlink(&self, id: MachineId) {
        if self.unlinks.send(id).is_err() {
            tracing::debug!("no gateway is listening for unlinks");
        }
    }
}

/// The first 8 characters of a machine's id, which is all the logs name.
fn id_prefix(id: &MachineId) -> String {
    id.to_string().chars().take(8).collect()
}

/// `value` as a JSON answer.
fn json<T: serde::Serialize>(value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => (
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            body,
        )
            .into_response(),
        Err(error) => {
            tracing::error!(%error, "an answer couldn't be written as JSON");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
