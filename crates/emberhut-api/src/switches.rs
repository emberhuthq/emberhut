//! `GET /api/switches`: the MCP servers and the skills the assistant's sessions have, each with
//! the person's switch; `POST /api/switches` turns one on or off, or reconnects a server, and
//! answers with the page as it is now.
//!
//! A switch is real: sessions start again without what was turned off once their turn is
//! over, and the runners' sessions at their next start. A request the server can't do (no
//! name, the assistant's own server, a server that is off) is refused with 400 and an
//! [`crate::error::ErrorBody`]; a server without these routes answers 404.

use serde::{Deserialize, Serialize};

use crate::open_enum::open_enum;
use crate::time::Timestamp;

open_enum! {
    /// Where an MCP server is.
    pub enum McpState {
        /// Connected and serving its tools.
        Connected = "connected",
        /// It failed to start or connect (`error` says why).
        Failed = "failed",
        /// It needs a sign-in first.
        NeedsAuth = "needs-auth",
        /// It is connecting.
        Pending = "pending",
        /// Turned off in the session's own settings, not here.
        Disabled = "disabled",
        /// Turned off here.
        Off = "off",
        /// Not loaded in the session right now.
        Unknown = "unknown",
    }
}

open_enum! {
    /// Where a skill comes from.
    pub enum SkillSource {
        /// The person's own.
        Yours = "yours",
        /// A plugin's.
        Plugin = "plugin",
        /// Built in.
        BuiltIn = "built in",
    }
}

/// An MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct McpServer {
    /// Its name.
    pub name: String,
    /// Where it is.
    pub state: McpState,
    /// Why it failed.
    #[serde(default)]
    pub error: Option<String>,
    /// Where it is configured (`user`: the person's own configuration).
    #[serde(default)]
    pub source: Option<String>,
    /// How it runs: `stdio`, `http`, `sse`, a hosted connector.
    #[serde(default)]
    pub kind: Option<String>,
    /// The person's switch.
    pub on: bool,
    /// The assistant's own tools: always on, no switch.
    #[serde(default)]
    pub fixed: bool,
    /// A line on what to do about it (how to sign in), or why it can't be switched.
    #[serde(default)]
    pub note: Option<String>,
}

impl McpServer {
    /// Whether it is on but not connected, so a reconnect may help.
    #[must_use]
    pub fn can_reconnect(&self) -> bool {
        self.on && !self.fixed && self.state != McpState::Connected
    }
}

/// A skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Skill {
    /// Its name.
    pub name: String,
    /// What it does (may be empty).
    #[serde(default)]
    pub description: String,
    /// Where it comes from.
    pub source: SkillSource,
    /// The person's switch.
    pub on: bool,
}

/// `GET /api/switches`'s answer, and `POST`'s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SwitchPage {
    /// When it was made.
    pub at: Timestamp,
    /// The account the assistant's main session runs on: its servers are the ones listed.
    #[serde(default)]
    pub account: Option<String>,
    /// Read from the open session now (else what it had last).
    pub live: bool,
    /// The servers: the assistant's own first, then by state and name.
    pub mcp: Vec<McpServer>,
    /// The skills: the person's own first, then by name.
    pub skills: Vec<Skill>,
    /// A line about the page (the session isn't open, so this is what it had last).
    #[serde(default)]
    pub note: Option<String>,
}

/// `POST /api/switches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub enum SwitchAction {
    /// Turn an MCP server on or off: `{ kind: "mcp", name, on }`.
    Mcp {
        /// The server. 1 to 120 characters, after trimming.
        #[cfg_attr(
            feature = "validate",
            garde(custom(crate::rules::not_blank), length(chars, max = 120))
        )]
        name: String,
        /// On or off.
        #[cfg_attr(feature = "validate", garde(skip))]
        on: bool,
    },
    /// Turn a skill on or off: `{ kind: "skill", name, on }`.
    Skill {
        /// The skill. 1 to 120 characters, after trimming.
        #[cfg_attr(
            feature = "validate",
            garde(custom(crate::rules::not_blank), length(chars, max = 120))
        )]
        name: String,
        /// On or off.
        #[cfg_attr(feature = "validate", garde(skip))]
        on: bool,
    },
    /// Reconnect an MCP server that is on: `{ kind: "reconnect", name }`.
    Reconnect {
        /// The server. 1 to 120 characters, after trimming.
        #[cfg_attr(
            feature = "validate",
            garde(custom(crate::rules::not_blank), length(chars, max = 120))
        )]
        name: String,
    },
}

impl SwitchAction {
    /// The server's or skill's name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Mcp { name, .. } | Self::Skill { name, .. } | Self::Reconnect { name } => name,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_tagged_by_kind() {
        let action = SwitchAction::Skill {
            name: "notes".into(),
            on: false,
        };
        assert_eq!(
            serde_json::to_string(&action).unwrap(),
            r#"{"kind":"skill","name":"notes","on":false}"#
        );
        let reconnect: SwitchAction =
            serde_json::from_str(r#"{"kind":"reconnect","name":"files"}"#).unwrap();
        assert_eq!(reconnect.name(), "files");
    }
}
