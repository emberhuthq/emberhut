//! The Claude accounts the assistant and its runners use (the state's `accounts`), each with its
//! machine and how much of it is used: what the new-session sheet offers. No tokens.

use serde::{Deserialize, Serialize};

use crate::ids::MachineId;
use crate::time::Timestamp;

/// The runner on the Linux PC: an account without a machine is on it.
pub const LINUX_RUNNER: &str = "runner";

/// One Claude account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClaudeAccount {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// What it is for: [`SCOPE_ANY`](crate::usage::SCOPE_ANY), or the server's word for the work
    /// it is kept for (never offered for a new session); none when the server doesn't say.
    #[serde(default)]
    pub scope: Option<String>,
    /// It may be picked.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// It is logged in.
    #[serde(default)]
    pub logged_in: Option<bool>,
    /// Its 5-hour use, in percent.
    #[serde(default, rename = "used5h")]
    pub used_5h: Option<f64>,
    /// Its weekly use, in percent.
    #[serde(default, rename = "used7d")]
    pub used_7d: Option<f64>,
    /// It is out of use until then.
    #[serde(default)]
    pub limited_until: Option<Timestamp>,
    /// The runner it is on ([`LINUX_RUNNER`] when none).
    #[serde(default)]
    pub machine: Option<MachineId>,
    /// That runner, in words.
    #[serde(default)]
    pub place: Option<String>,
    /// Its place in the order accounts are tried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i64>,
    /// The address it is signed in with.
    #[serde(default)]
    pub email: Option<String>,
    /// Its plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// When its 5-hour use starts again.
    #[serde(default, rename = "resets5h")]
    pub resets_5h: Option<Timestamp>,
    /// When its weekly use starts again.
    #[serde(default, rename = "resets7d")]
    pub resets_7d: Option<Timestamp>,
    /// Why it is out of use, in words.
    #[serde(default)]
    pub limited_why: Option<String>,
    /// When its use was last read.
    #[serde(default)]
    pub checked: Option<Timestamp>,
    /// What went wrong the last time its use was read.
    #[serde(default)]
    pub error: Option<String>,
}

impl ClaudeAccount {
    /// Whether it is on the runner `machine`.
    #[must_use]
    pub fn is_on(&self, machine: &MachineId) -> bool {
        self.machine
            .as_ref()
            .map_or(LINUX_RUNNER, MachineId::as_str)
            == machine.as_str()
    }

    /// Whether a new session may be put on it: enabled, and for any work (one the server keeps
    /// for other work is never offered).
    #[must_use]
    pub fn can_be_picked(&self) -> bool {
        self.enabled != Some(false)
            && self
                .scope
                .as_deref()
                .is_none_or(|scope| scope.is_empty() || scope == crate::usage::SCOPE_ANY)
    }
}
