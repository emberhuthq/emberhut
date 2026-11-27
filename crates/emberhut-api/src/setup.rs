//! `GET /api/setup`: what the server has set up of what the assistant works with, item by item,
//! for the apps' onboarding (`docs/api.md`, "Setup").
//!
//! Before pairing, an app reads `GET /api/auth/status` (whether the server is in use at all);
//! this checklist is for a signed-in device, since it says what the owner has connected.

use serde::{Deserialize, Serialize};

use crate::open_enum::open_enum;

open_enum! {
    /// What a setup item is about.
    pub enum SetupItemId {
        /// The agent CLI the assistant and its workers run on (`EMBERHUT_AGENT_COMMAND`).
        Agent = "agent",
        /// What the agent thinks with: the provider saved from the apps
        /// (`PUT /api/agent/provider`), or the server's configured accounts.
        Provider = "provider",
        /// The accounts the runners report for the agent CLI, and whether they are logged in.
        Accounts = "accounts",
        /// The devices signed in.
        Devices = "devices",
        /// Phone pushes (APNs, FCM).
        Push = "push",
        /// The public address the server's links point at (`EMBERHUT_PUBLIC_URL`).
        PublicUrl = "public-url",
        /// The helper on the owner's Mac.
        Mac = "mac",
        /// The owner's Chrome profiles, paired through the extension.
        Chrome = "chrome",
        /// The owner's Gmail: the server's Google OAuth client and the accounts signed in.
        Gmail = "gmail",
        /// The owner's Slack: the workspaces a user token is saved for.
        Slack = "slack",
        /// The assistant's own WhatsApp number, through its daemon.
        WhatsApp = "whatsapp",
    }
}

open_enum! {
    /// How a setup item is.
    pub enum SetupState {
        /// Set up, and working as far as the server can tell.
        Ready = "ready",
        /// Not set up yet, and the owner can set it up from the apps (pair, sign in, save a
        /// token, log an account in).
        Missing = "missing",
        /// Not part of this server: turning it on takes the server's own configuration.
        Off = "off",
        /// Set up, but failing now.
        Error = "error",
    }
}

/// One item of the checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SetupItem {
    /// What it is about.
    pub id: SetupItemId,
    /// How it is.
    pub state: SetupState,
    /// A line for the owner: how many there are, or what to do; left out when there is
    /// nothing to add.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl SetupItem {
    /// The item `id` in `state`, with `detail`.
    #[must_use]
    pub fn new(id: SetupItemId, state: SetupState, detail: impl Into<String>) -> Self {
        Self {
            id,
            state,
            detail: Some(detail.into()),
        }
    }
}

/// `GET /api/setup`'s answer: the checklist, in the order the apps show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SetupPage {
    /// Every item, each once. A newer server may add items: a client shows or skips the ids
    /// it doesn't know.
    pub items: Vec<SetupItem>,
}

impl SetupPage {
    /// The item `id`, if the page has it.
    #[must_use]
    pub fn item(&self, id: &SetupItemId) -> Option<&SetupItem> {
        self.items.iter().find(|item| item.id == *id)
    }
}
