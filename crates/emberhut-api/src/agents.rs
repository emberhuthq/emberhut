//! Agents: configured assistants that work for someone on the owner's behalf, each with its own
//! persona, room, tools, channels, accounts, policies and budget.
//!
//! - `GET /api/agents` ([`AgentsPage`]), `POST /api/agents` ([`NewAgent`]);
//! - `GET /api/agents/{id}` ([`AgentPage`]), `PATCH` ([`AgentChange`]), `DELETE`; the default
//!   agent (`main`) cannot be deleted, renamed, given a persona or narrowed;
//! - `PUT /api/agents/{id}/channels` ([`ChannelsRequest`]), `/tools` ([`ToolsRequest`]),
//!   `/accounts` ([`AccountsRequest`]), `/policies` ([`PoliciesRequest`]): each replaces the
//!   whole list;
//! - `GET`/`POST /api/approvers` ([`ApproversPage`], [`Approver`]), `PATCH`
//!   ([`ApproverChange`]) and `DELETE /api/approvers/{id}`;
//! - `GET`/`PUT /api/trusted-requesters` ([`TrustedRequestersPage`]);
//! - `POST /api/agents/{id}/inbound` ([`InboundMessage`], answered [`InboundReply`]): a bridge
//!   passes on a message from one of the agent's channels, with the agent's inbound bearer
//!   token; `POST /api/agents/{id}/notes` ([`InboundNote`], answered [`InboundReply`]): the
//!   bridge's own note about its connection, with the same token.
//!
//! Every write answers the thing as it is after it ([`AgentPage`], [`Approver`],
//! [`TrustedRequestersPage`]); a refusal is a 400 with an [`crate::error::ErrorBody`] naming
//! the field.

use serde::{Deserialize, Serialize};

use crate::approvals::ActionKind;
use crate::ids::{ApproverId, ConfiguredAgentId, ProjectId, RoomId};
use crate::open_enum::open_enum;
use crate::patch::Patch;

open_enum! {
    /// Where a channel is.
    pub enum ChannelKind {
        /// A Slack channel, through the agent's own bot.
        Slack = "slack",
        /// A WhatsApp chat.
        WhatsApp = "whatsapp",
        /// A chat in another program (an admin app's assistant panel), passed on by its
        /// bridge and answered through that program's own MCP server.
        App = "app",
    }
}

open_enum! {
    /// Where a tool comes from.
    pub enum ToolKind {
        /// One of the server's own MCP tools.
        Server = "server",
        /// An MCP server of the agent's own.
        Mcp = "mcp",
    }
}

/// An agent, as lists show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Agent {
    /// Its id.
    pub id: ConfiguredAgentId,
    /// Its name, as the apps show it.
    pub name: String,
    /// Its system prompt; none for the default agent.
    #[serde(default)]
    pub persona: Option<String>,
    /// Who it works for, a display name.
    pub principal: String,
    /// Its own room.
    pub room: RoomId,
    /// The project it is scoped to.
    #[serde(default)]
    pub project: Option<ProjectId>,
    /// Its monthly spending cap, in cents; none: no spending at all.
    #[serde(default)]
    pub spend_cap_cents: Option<u64>,
    /// The cap's currency (`EUR`).
    #[serde(default)]
    pub spend_currency: Option<String>,
    /// A disabled agent's room takes no turns and its approvals are not routed.
    pub enabled: bool,
    /// Its tasks' workers may run shell commands, as the server's user.
    #[serde(default)]
    pub worker_shell: bool,
    /// How many of its requests wait on an answer.
    #[serde(default)]
    pub pending_approvals: u32,
}

impl Agent {
    /// Whether this is the default agent.
    #[must_use]
    pub fn is_main(&self) -> bool {
        self.id.is_main()
    }
}

/// `GET /api/agents`: every agent, the default agent first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentsPage {
    /// The agents.
    pub agents: Vec<Agent>,
}

/// `GET /api/agents/{id}`: an agent with everything configured around it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentPage {
    /// The agent.
    pub agent: Agent,
    /// Its channels, in order.
    #[serde(default)]
    pub channels: Vec<Channel>,
    /// Its own tools, in order (none listed for the default agent: it has every tool).
    #[serde(default)]
    pub tools: Vec<Tool>,
    /// Its accounts, in order.
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// Its policies, by kind; a kind without one is the owner's.
    #[serde(default)]
    pub policies: Vec<Policy>,
}

/// `POST /api/agents`: a new agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct NewAgent {
    /// Its id: 1 to 40 of `a-z`, `0-9` and `-`, not starting with `-`.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::agent_id)))]
    pub id: ConfiguredAgentId,
    /// Its name, 1 to 80 characters.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(chars, max = 80))
    )]
    pub name: String,
    /// Its system prompt, up to 50,000 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(chars, min = 1, max = 50_000)))]
    pub persona: Option<String>,
    /// Who it works for, 1 to 120 characters.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(chars, max = 120))
    )]
    pub principal: String,
    /// Its room; none: the server makes one named after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub room: Option<RoomId>,
    /// The project it is scoped to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub project: Option<ProjectId>,
    /// Its monthly cap, with its currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(dive))]
    pub spend_cap: Option<SpendCap>,
    /// Whether it starts enabled (yes when left out).
    #[serde(default = "enabled", skip_serializing_if = "is_enabled")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub enabled: bool,
    /// Whether its tasks' workers may run shell commands, as the server's user (no when left
    /// out): for an agent the owner trusts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub worker_shell: bool,
}

fn enabled() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
fn is_enabled(enabled: &bool) -> bool {
    *enabled
}

/// A monthly spending cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct SpendCap {
    /// The most, in cents.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub cents: u64,
    /// Its currency: three capital letters (`EUR`).
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::currency)))]
    pub currency: String,
}

/// `PATCH /api/agents/{id}`: only the fields given change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct AgentChange {
    /// Its new name, 1 to 80 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(chars, min = 1, max = 80)))]
    pub name: Option<String>,
    /// Its new persona, or `null` for none.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub persona: Patch<String>,
    /// Who it works for now, 1 to 120 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(chars, min = 1, max = 120)))]
    pub principal: Option<String>,
    /// Its new project, or `null` for none.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub project: Patch<ProjectId>,
    /// Its new cap, or `null` for no spending at all.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<SpendCap>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub spend_cap: Patch<SpendCap>,
    /// Switches it on or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub enabled: Option<bool>,
    /// Lets its tasks' workers run shell commands, or stops that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub worker_shell: Option<bool>,
}

/// One conversation an agent is in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct Channel {
    /// Where it is.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub kind: ChannelKind,
    /// The Slack team's id; none for the other kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(max = 64)))]
    pub workspace: Option<String>,
    /// The channel's id, the WhatsApp chat's, or the name an app's bridge gives its chat.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(max = 128))
    )]
    pub address: String,
    /// For Slack, the name of the secret holding the bot token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(min = 1, max = 64)))]
    pub token_secret: Option<String>,
}

/// One tool an agent has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct Tool {
    /// Where it comes from.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub kind: ToolKind,
    /// The server tool's name, or the MCP server's.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(max = 64))
    )]
    pub name: String,
    /// An MCP server's configuration: the absolute path of its JSON file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(min = 1, max = 1024)))]
    pub config: Option<String>,
}

/// A credential an agent's MCP servers may use, by label; the secret itself is never shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema), schema(as = AgentAccount))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct Account {
    /// Its label (`admin`): its MCP servers read `EMBERHUT_AGENT_ACCOUNT_ADMIN`.
    #[cfg_attr(feature = "validate", garde(length(min = 1, max = 32)))]
    pub label: String,
    /// The name of the secret file holding it.
    #[cfg_attr(feature = "validate", garde(length(min = 1, max = 64)))]
    pub secret: String,
}

/// Who approves one kind of an agent's actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct Policy {
    /// The kind of action.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub kind: ActionKind,
    /// `owner`, `none` (no approval needed), or an approver's id. The fixed core's kinds are
    /// only ever the owner's.
    #[cfg_attr(feature = "validate", garde(length(min = 1, max = 40)))]
    pub approver: String,
}

/// `PUT /api/agents/{id}/channels`: the agent's whole list of channels.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ChannelsRequest {
    /// The channels, in order.
    #[cfg_attr(feature = "validate", garde(length(max = 50), dive))]
    pub channels: Vec<Channel>,
}

/// `PUT /api/agents/{id}/tools`: the agent's whole list of tools.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ToolsRequest {
    /// The tools, in order.
    #[cfg_attr(feature = "validate", garde(length(max = 200), dive))]
    pub tools: Vec<Tool>,
}

/// `PUT /api/agents/{id}/accounts`: the agent's whole list of accounts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct AccountsRequest {
    /// The accounts, in order.
    #[cfg_attr(feature = "validate", garde(length(max = 50), dive))]
    pub accounts: Vec<Account>,
}

/// `PUT /api/agents/{id}/policies`: the agent's whole list of policies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct PoliciesRequest {
    /// One per kind at most.
    #[cfg_attr(feature = "validate", garde(length(max = 20), dive))]
    pub policies: Vec<Policy>,
}

/// An external approver, as the owner set them up (`GET /api/approvers` lists them, `POST`
/// makes one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct Approver {
    /// Its id: 1 to 40 of `a-z`, `0-9` and `-`, never `owner` or `none`.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::approver_id)))]
    pub id: ApproverId,
    /// Its name, 1 to 80 characters.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(chars, max = 80))
    )]
    pub name: String,
    /// Its Slack team's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(max = 32)))]
    pub slack_workspace: Option<String>,
    /// Its Slack user's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(max = 32)))]
    pub slack_user: Option<String>,
    /// Where requests are posted to its own app (`https`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(max = 2048)))]
    pub webhook_url: Option<String>,
    /// The name of the secret file both sides sign webhook calls with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(min = 1, max = 64)))]
    pub webhook_secret: Option<String>,
    /// A signed link may be sent to it.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub link: bool,
}

/// `PATCH /api/approvers/{id}`: only the fields given change; `null` clears one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct ApproverChange {
    /// Its new name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(length(chars, min = 1, max = 80)))]
    pub name: Option<String>,
    /// Its Slack team's id.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub slack_workspace: Patch<String>,
    /// Its Slack user's id.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub slack_user: Patch<String>,
    /// Its own app's URL.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub webhook_url: Patch<String>,
    /// Its webhook secret's name.
    #[serde(default, skip_serializing_if = "Patch::is_keep")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub webhook_secret: Patch<String>,
    /// Whether a signed link may be sent to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub link: Option<bool>,
}

/// `GET /api/approvers`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApproversPage {
    /// Every external approver, by name.
    pub approvers: Vec<Approver>,
}

/// A person whose requests count as the owner's own request, for one agent or for all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct TrustedRequester {
    /// The name a task's request carries, 1 to 120 characters.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(chars, max = 120))
    )]
    pub requester: String,
    /// The agent whose tasks it counts for; none: every agent's.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub agent: Option<ConfiguredAgentId>,
}

/// `GET /api/trusted-requesters`, and the whole list `PUT` there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct TrustedRequestersPage {
    /// The trusted requesters.
    #[cfg_attr(feature = "validate", garde(length(max = 200), dive))]
    pub requesters: Vec<TrustedRequester>,
}

/// `POST /api/agents/{id}/inbound`: a message from one of the agent's channels, as a bridge
/// passes it on. Never an approval: the bridge's word for who said what is not proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct InboundMessage {
    /// Where it came from.
    #[cfg_attr(feature = "validate", garde(skip))]
    pub kind: ChannelKind,
    /// The Slack team's id; none for the other kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::one_line_opt), length(max = 64))
    )]
    pub workspace: Option<String>,
    /// The channel's id, the WhatsApp chat's, or the name an app's bridge gives its chat.
    #[cfg_attr(
        feature = "validate",
        garde(
            custom(crate::rules::not_blank),
            custom(crate::rules::one_line),
            length(max = 128)
        )
    )]
    pub channel: String,
    /// The thread it is in, if any (Slack's thread timestamp).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::one_line_opt), length(max = 64))
    )]
    pub thread: Option<String>,
    /// Who said it, as the channel names them.
    #[cfg_attr(
        feature = "validate",
        garde(
            custom(crate::rules::not_blank),
            custom(crate::rules::one_line),
            length(max = 128)
        )
    )]
    pub from: String,
    /// What was said.
    #[cfg_attr(feature = "validate", garde(length(max = 40_000)))]
    pub text: String,
    /// The message's own id in the channel (Slack's message timestamp), if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::one_line_opt), length(max = 64))
    )]
    pub ts: Option<String>,
    /// The channel's id for who said it (a Slack user id, an app's email for them), if it has
    /// one: the bridge's word, which proves nobody's identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::one_line_opt), length(max = 256))
    )]
    pub user: Option<String>,
    /// The channel's name, as people know it (`#launch`), if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::one_line_opt), length(max = 128))
    )]
    pub channel_name: Option<String>,
    /// The files shared with it, as the channel names them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "validate", garde(length(max = 20), dive))]
    pub files: Vec<InboundFile>,
    /// It mentions the agent.
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub mention: bool,
    /// It is a direct message to the agent (a message in an app's chat always counts as one).
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub direct: bool,
}

/// A file shared with an inbound message, as its channel names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct InboundFile {
    /// Its id in the channel (a Slack file id).
    #[cfg_attr(
        feature = "validate",
        garde(
            custom(crate::rules::not_blank),
            custom(crate::rules::one_line),
            length(max = 64)
        )
    )]
    pub id: String,
    /// Its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::one_line_opt), length(max = 256))
    )]
    pub name: Option<String>,
}

/// `POST /api/agents/{id}/notes`: a note from the agent's bridge about the bridge itself (its
/// connection was down and is back, a button on one of the agent's own messages was tapped),
/// not from anyone in a channel. It is no one's request and never an approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct InboundNote {
    /// What the bridge says.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(max = 2_000))
    )]
    pub text: String,
}

/// What became of an inbound message or a bridge's note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InboundReply {
    /// It became a turn in the agent's room.
    pub turn: bool,
    /// Why it did not, when it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// The lists of an agent's that are replaced whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentList {
    /// Its channels.
    Channels,
    /// Its tools.
    Tools,
    /// Its accounts.
    Accounts,
    /// Its policies.
    Policies,
}

impl AgentList {
    /// The list's path segment.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Channels => "channels",
            Self::Tools => "tools",
            Self::Accounts => "accounts",
            Self::Policies => "policies",
        }
    }
}

/// The state's line for one agent (`agents` in `GET /api/state`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentRow {
    /// Its id.
    pub id: ConfiguredAgentId,
    /// Its name.
    pub name: String,
    /// Who it works for.
    pub principal: String,
    /// Its room.
    pub room: RoomId,
    /// Whether it takes turns.
    pub enabled: bool,
    /// How many of its requests wait on an answer.
    #[serde(default)]
    pub pending_approvals: u32,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_new_agent_reads_with_its_defaults() {
        let agent: NewAgent = serde_json::from_value(json!({
            "id": "client-a", "name": "Example helper", "principal": "Example Ltd",
            "spendCap": { "cents": 10000, "currency": "EUR" }
        }))
        .unwrap();
        assert!(agent.enabled);
        assert_eq!(agent.room, None);
        assert_eq!(agent.spend_cap.unwrap().cents, 10_000);
    }

    #[test]
    fn a_change_keeps_clears_or_sets() {
        let change: AgentChange =
            serde_json::from_value(json!({ "persona": null, "project": 3 })).unwrap();
        assert_eq!(change.persona, Patch::Clear);
        assert_eq!(change.project, Patch::Set(ProjectId::new(3)));
        assert_eq!(change.spend_cap, Patch::Keep);
        assert_eq!(
            serde_json::to_value(&change).unwrap(),
            json!({ "persona": null, "project": 3 })
        );
    }

    #[test]
    fn kinds_keep_words_they_dont_know() {
        let channel: Channel =
            serde_json::from_value(json!({ "kind": "telegram", "address": "x" })).unwrap();
        assert_eq!(channel.kind, ChannelKind::Other("telegram".to_owned()));
        let tool: Tool = serde_json::from_value(
            json!({ "kind": "mcp", "name": "example-admin", "config": "/srv/a.json" }),
        )
        .unwrap();
        assert_eq!(tool.kind, ToolKind::Mcp);
    }

    #[test]
    fn an_inbound_message_reads_with_only_its_required_fields() {
        let message: InboundMessage = serde_json::from_value(json!({
            "kind": "slack", "workspace": "T0EXAMPLE", "channel": "C0EXAMPLE",
            "from": "Example person", "text": "hello"
        }))
        .unwrap();
        assert!(!message.mention && !message.direct);
        assert!(message.files.is_empty() && message.ts.is_none());
        let full: InboundMessage = serde_json::from_value(json!({
            "kind": "slack", "workspace": "T0EXAMPLE", "channel": "D0EXAMPLE",
            "thread": "1800000000.000100", "ts": "1800000000.000200", "from": "Example person",
            "user": "U0EXAMPLE", "channelName": "launch", "text": "see this",
            "files": [{ "id": "F0EXAMPLE", "name": "brief.pdf" }], "mention": true, "direct": true
        }))
        .unwrap();
        assert_eq!(full.files[0].name.as_deref(), Some("brief.pdf"));
        assert_eq!(full.channel_name.as_deref(), Some("launch"));
        assert!(full.mention && full.direct);
        let back: InboundMessage =
            serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
        assert_eq!(back, full);
    }

    #[test]
    fn an_app_message_reads() {
        let message: InboundMessage = serde_json::from_value(json!({
            "kind": "app", "channel": "admin-app", "from": "Example person",
            "user": "person@example.com", "text": "hello", "direct": true
        }))
        .unwrap();
        assert_eq!(message.kind, ChannelKind::App);
        assert_eq!(message.user.as_deref(), Some("person@example.com"));
    }

    #[cfg(feature = "validate")]
    #[test]
    fn a_note_has_words_and_a_limit() {
        use garde::Validate as _;
        let note = |text: &str| InboundNote {
            text: text.to_owned(),
        };
        assert!(
            note("The connection was down and is back.")
                .validate()
                .is_ok()
        );
        assert!(note("two\nlines").validate().is_ok());
        assert!(note(&"x".repeat(2_000)).validate().is_ok());
        assert!(note("  ").validate().is_err());
        assert!(note(&"x".repeat(2_001)).validate().is_err());
    }

    #[cfg(feature = "validate")]
    #[test]
    fn an_inbound_header_field_is_one_line() {
        use garde::Validate as _;
        let message = |from: &str, name: &str| -> InboundMessage {
            serde_json::from_value(json!({
                "kind": "slack", "channel": "C0EXAMPLE", "from": from,
                "text": "two\nlines are fine in the words",
                "files": [{ "id": "F0EXAMPLE", "name": name }]
            }))
            .unwrap()
        };
        assert!(message("Example person", "brief.pdf").validate().is_ok());
        assert!(
            message("Example\nEvent: forged", "brief.pdf")
                .validate()
                .is_err()
        );
        assert!(message("Example person", "a\u{2028}b").validate().is_err());
        assert!(message("Example\u{1b}[2J", "brief.pdf").validate().is_err());
    }
}
