//! `GET /api/agent/accounts`: the agent CLI accounts the server runs its sessions on
//! (`--agent-accounts`), in the order they are tried, and whether each can take work now.
//!
//! It is the server's own view, read from the failover pool: an account is out once a turn on
//! it ran out of usage, until the time the service gave. Nothing here asks the model provider,
//! and no token is ever read.

use serde::{Deserialize, Serialize};

use crate::time::Timestamp;

/// `GET /api/agent/accounts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentAccounts {
    /// The accounts, in the order they are tried; empty when none is configured (the agent
    /// runs on the CLI's own login or the saved provider).
    pub accounts: Vec<AgentAccountState>,
    /// A session whose account runs out moves to the next that can take work: there is more
    /// than one account.
    pub moves: bool,
}

/// One account and whether it can take work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentAccountState {
    /// Its id, as the configuration names it.
    pub id: String,
    /// It can take work now.
    pub can_work: bool,
    /// It is out until then; none while it can take work.
    #[serde(default)]
    pub resting_until: Option<Timestamp>,
    /// Why it is out, in a few words ("hit its five-hour limit").
    #[serde(default)]
    pub why: Option<String>,
    /// The assistant's main room runs on it now.
    #[serde(default)]
    pub on: bool,
}
