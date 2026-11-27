//! Agents: configured assistants that work for someone on the owner's behalf, each with its own
//! persona, room, tools, channels, accounts and budget.
//!
//! The default agent ([`AgentId::MAIN`], [`Agent::main`]) is the assistant the server already
//! runs: Main and every room no other agent owns, the server's own rules, every tool, and every
//! approval the owner's. These are the rules on what an agent and its lists may be; where they
//! are kept is the database's, and what an agent may do without an OK is
//! [`crate::approvals`]'.

mod accounts;
mod agent;
mod channel;
mod error;
mod id;
pub mod quote;
mod tools;
mod trusted;

pub(crate) use channel::is_slack_id;
pub(crate) use id::is_slug;

pub use accounts::{ACCOUNT_ENV_PREFIX, AgentAccount, validate_accounts};
pub use agent::{
    Agent, Currency, DEFAULT_NAME, DEFAULT_PRINCIPAL, NAME_MAX, PERSONA_MAX, PRINCIPAL_MAX,
    SpendCap, check_delete,
};
pub use channel::{Channel, ChannelKind, validate_channels};
pub use error::AgentError;
pub use id::{AGENT_FACTS_PREFIX, AgentId, InvalidAgentId};
pub use tools::{
    AgentTool, OWNER_TOOL_PREFIXES, OWNER_TOOLS, ToolKind, acts_as_owner, validate_tools,
};
pub use trusted::{REQUESTER_MAX, TrustedRequester, validate_requesters};
