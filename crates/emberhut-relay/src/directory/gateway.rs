//! What a gateway asks the directory, and hears from it (ADR 0007, section 5, "The directory
//! and the gateways"). This is an internal interface, not part of the directory's API, and can
//! change with the relay's own version.
//!
//! - A gateway looks up the ids of machines it doesn't know, batched, and keeps known answers
//!   for 5 minutes when the directory can't be reached; an id it gets no answer for is told to
//!   retry soon (relay.md's 4502), not refused.
//! - Unlinks are pushed: the gateway holding an unlinked machine closes its uplink with 4403. A
//!   gateway that falls behind the stream misses unlinks, which its check of all connected ids
//!   every 30 s catches.
//! - The gateway reports which machines it holds, for `connected` and `lastSeen`, and says when
//!   it starts, so the machines a crashed box held don't show as connected.
//!
//! [`InProcessDirectory`] is the implementation for a gateway in the directory's process, over
//! a tokio broadcast channel. A gateway in another process will reach the directory over a
//! private address, behind the same trait.

use std::future::Future;
use std::sync::Arc;

use emberhut_identity::{AccountId, MachineId};
use tokio::sync::broadcast;

use super::Clock;
use super::store::{Store, StoreError};

/// How many unlinks wait for a slow gateway before it misses some.
pub(super) const UNLINK_BACKLOG: usize = 1_024;

/// Whether a machine may use the relay.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MachineStatus {
    /// Linked to this account.
    Linked {
        /// The account, whose machines the gateway counts against its limit.
        account: AccountId,
    },
    /// Not linked to any account: refused.
    NotLinked,
}

/// Whether a gateway holds a machine's uplink.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Presence {
    /// The uplink is up, on the box named.
    Up {
        /// The gateway box holding it.
        held_by: String,
    },
    /// The uplink went down.
    Down,
}

/// The directory couldn't answer a gateway: it keeps its known answers, and tells machines it
/// has none for to retry soon. Whatever the directory is behind (a database here, a private
/// address later), the gateway sees only that it didn't answer.
#[derive(Debug, thiserror::Error)]
#[error("the directory couldn't answer")]
pub struct LookupError(#[source] Box<dyn std::error::Error + Send + Sync>);

impl From<StoreError> for LookupError {
    fn from(error: StoreError) -> Self {
        Self(Box::new(error))
    }
}

/// What a gateway asks the directory, and hears from it.
pub trait GatewayDirectory: Send + Sync + 'static {
    /// Whether each of `ids` may use the relay, in the same order.
    ///
    /// # Errors
    ///
    /// [`LookupError`] when the directory can't answer now.
    fn lookup(
        &self,
        ids: &[MachineId],
    ) -> impl Future<Output = Result<Vec<MachineStatus>, LookupError>> + Send;

    /// The machines unlinked from now on, as they are unlinked.
    fn unlinked(&self) -> broadcast::Receiver<MachineId>;

    /// Records that a machine's uplink came up on this gateway, or went down.
    ///
    /// # Errors
    ///
    /// [`LookupError`] when the directory can't record it now.
    fn report(
        &self,
        id: &MachineId,
        presence: Presence,
    ) -> impl Future<Output = Result<(), LookupError>> + Send;

    /// Records that the gateway box `held_by` (re)started: none of the uplinks it held before is
    /// up any more. A box calls this before it accepts its first uplink.
    ///
    /// # Errors
    ///
    /// [`LookupError`] when the directory can't record it now.
    fn started(&self, held_by: &str) -> impl Future<Output = Result<(), LookupError>> + Send;
}

/// The directory, for a gateway in the same process.
#[derive(Clone, Debug)]
pub struct InProcessDirectory {
    store: Store,
    unlinks: broadcast::Sender<MachineId>,
    clock: Arc<dyn Clock>,
}

impl InProcessDirectory {
    pub(super) fn new(
        store: Store,
        unlinks: broadcast::Sender<MachineId>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            store,
            unlinks,
            clock,
        }
    }
}

impl GatewayDirectory for InProcessDirectory {
    async fn lookup(&self, ids: &[MachineId]) -> Result<Vec<MachineStatus>, LookupError> {
        Ok(self
            .store
            .accounts_of(ids)
            .await?
            .into_iter()
            .map(|account| match account {
                Some(account) => MachineStatus::Linked { account },
                None => MachineStatus::NotLinked,
            })
            .collect())
    }

    fn unlinked(&self) -> broadcast::Receiver<MachineId> {
        self.unlinks.subscribe()
    }

    async fn report(&self, id: &MachineId, presence: Presence) -> Result<(), LookupError> {
        let held_by = match &presence {
            Presence::Up { held_by } => Some(held_by.as_str()),
            Presence::Down => None,
        };
        self.store.presence(id, held_by, self.clock.now()).await?;
        Ok(())
    }

    async fn started(&self, held_by: &str) -> Result<(), LookupError> {
        let released = self.store.release(held_by, self.clock.now()).await?;
        if released > 0 {
            tracing::info!(
                machines = released,
                "a gateway box started; its old uplinks are down"
            );
        }
        Ok(())
    }
}
