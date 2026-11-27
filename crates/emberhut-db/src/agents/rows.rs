//! Agents' rows to and from `emberhut-core`'s types. A stored value that isn't one its column
//! takes is [`DbError::CorruptValue`], never a guess.

use emberhut_core::RoomId;
use emberhut_core::agents::{
    Agent, AgentAccount, AgentId, AgentTool, Channel, ChannelKind, Currency, SpendCap, ToolKind,
};
use emberhut_core::approvals::{
    ActionKind, Approver, ApproverId, ApproverRecord, Policy, SlackIdentity, WebhookTarget,
};
use emberhut_core::secret_link::SecretName;
use emberhut_core::task::ProjectId;
use jiff::Timestamp;
use sea_orm::ActiveValue::{NotSet, Set};

use crate::DbError;
use crate::entity::{
    agent_account, agent_channel, agent_tool, approval_policy, approver, configured_agent,
};
use crate::time::millis;

/// `value` of `column`, refused.
pub(crate) fn corrupt(column: &'static str, value: &str) -> DbError {
    DbError::CorruptValue {
        column,
        value: value.to_owned(),
    }
}

pub(crate) fn agent_id(id: &str) -> Result<AgentId, DbError> {
    AgentId::parse(id).map_err(|_| corrupt("agent_id", id))
}

pub(crate) fn approver_id(id: &str) -> Result<ApproverId, DbError> {
    ApproverId::parse(id).map_err(|_| corrupt("approvers.id", id))
}

pub(crate) fn approver_of(text: &str, column: &'static str) -> Result<Approver, DbError> {
    Approver::parse(text).map_err(|_| corrupt(column, text))
}

pub(crate) fn kind_of(text: &str, column: &'static str) -> Result<ActionKind, DbError> {
    ActionKind::parse(text).ok_or_else(|| corrupt(column, text))
}

fn secret(name: &str, column: &'static str) -> Result<SecretName, DbError> {
    SecretName::parse(name).map_err(|_| corrupt(column, name))
}

fn cents(value: i64, column: &'static str) -> Result<u64, DbError> {
    u64::try_from(value).map_err(|_| corrupt(column, &value.to_string()))
}

/// A cents amount as a column stores it (amounts past `i64::MAX` cents are no real money).
pub(crate) fn cents_column(cents: u64) -> i64 {
    i64::try_from(cents).unwrap_or(i64::MAX)
}

pub(crate) fn agent(row: configured_agent::Model) -> Result<Agent, DbError> {
    let spend_cap = match (row.spend_cap_cents, row.spend_currency.as_deref()) {
        (Some(amount), Some(currency)) => Some(SpendCap {
            cents: cents(amount, "configured_agents.spend_cap_cents")?,
            currency: Currency::parse(currency)
                .map_err(|_| corrupt("configured_agents.spend_currency", currency))?,
        }),
        _ => None,
    };
    Ok(Agent {
        id: agent_id(&row.id)?,
        name: row.name,
        persona: row.persona,
        principal: row.principal,
        room: RoomId::new(row.room_id),
        project: row.project_id.map(ProjectId::new),
        spend_cap,
        enabled: row.enabled,
        worker_shell: row.worker_shell,
    })
}

/// `agent`'s row, made at `created_at` (ms) and changed at `now`.
pub(crate) fn agent_row(
    agent: &Agent,
    created_at: i64,
    now: Timestamp,
) -> configured_agent::ActiveModel {
    configured_agent::ActiveModel {
        id: Set(agent.id.as_str().to_owned()),
        name: Set(agent.name.clone()),
        persona: Set(agent.persona.clone()),
        principal: Set(agent.principal.clone()),
        room_id: Set(agent.room.get()),
        project_id: Set(agent.project.map(ProjectId::get)),
        spend_cap_cents: Set(agent.spend_cap.as_ref().map(|cap| cents_column(cap.cents))),
        spend_currency: Set(agent
            .spend_cap
            .as_ref()
            .map(|cap| cap.currency.as_str().to_owned())),
        enabled: Set(agent.enabled),
        worker_shell: Set(agent.worker_shell),
        // Whether its room was made with it is the store's to say, when it makes the room.
        made_room: NotSet,
        created_at: Set(created_at),
        updated_at: Set(millis(now)),
    }
}

pub(crate) fn channel(row: agent_channel::Model) -> Result<Channel, DbError> {
    Ok(Channel {
        kind: ChannelKind::parse(&row.kind)
            .ok_or_else(|| corrupt("agent_channels.kind", &row.kind))?,
        workspace: row.workspace,
        address: row.address,
        token_secret: row
            .token_secret
            .as_deref()
            .map(|name| secret(name, "agent_channels.token_secret"))
            .transpose()?,
    })
}

pub(crate) fn channel_row(
    agent: &AgentId,
    position: i32,
    channel: &Channel,
) -> agent_channel::ActiveModel {
    agent_channel::ActiveModel {
        agent_id: Set(agent.as_str().to_owned()),
        position: Set(position),
        kind: Set(channel.kind.as_str().to_owned()),
        workspace: Set(channel.workspace.clone()),
        address: Set(channel.address.clone()),
        token_secret: Set(channel
            .token_secret
            .as_ref()
            .map(|name| name.as_str().to_owned())),
        ..Default::default()
    }
}

pub(crate) fn tool(row: agent_tool::Model) -> Result<AgentTool, DbError> {
    match (ToolKind::parse(&row.kind), row.config) {
        (Some(ToolKind::Server), _) => Ok(AgentTool::Server { name: row.name }),
        (Some(ToolKind::Mcp), Some(config)) => Ok(AgentTool::Mcp {
            name: row.name,
            config,
        }),
        (Some(ToolKind::Mcp), None) => Err(corrupt("agent_tools.config", "")),
        (None, _) => Err(corrupt("agent_tools.kind", &row.kind)),
    }
}

pub(crate) fn tool_row(
    agent: &AgentId,
    position: i32,
    tool: &AgentTool,
) -> agent_tool::ActiveModel {
    let config = match tool {
        AgentTool::Server { .. } => None,
        AgentTool::Mcp { config, .. } => Some(config.clone()),
    };
    agent_tool::ActiveModel {
        agent_id: Set(agent.as_str().to_owned()),
        position: Set(position),
        kind: Set(tool.kind().as_str().to_owned()),
        name: Set(tool.name().to_owned()),
        config: Set(config),
        ..Default::default()
    }
}

pub(crate) fn account(row: agent_account::Model) -> Result<AgentAccount, DbError> {
    Ok(AgentAccount {
        secret: secret(&row.secret, "agent_accounts.secret")?,
        label: row.label,
    })
}

pub(crate) fn account_row(
    agent: &AgentId,
    position: i32,
    account: &AgentAccount,
) -> agent_account::ActiveModel {
    agent_account::ActiveModel {
        agent_id: Set(agent.as_str().to_owned()),
        position: Set(position),
        label: Set(account.label.clone()),
        secret: Set(account.secret.as_str().to_owned()),
        ..Default::default()
    }
}

pub(crate) fn policy(row: &approval_policy::Model) -> Result<Policy, DbError> {
    Ok(Policy {
        kind: kind_of(&row.kind, "approval_policies.kind")?,
        approver: approver_of(&row.approver, "approval_policies.approver")?,
    })
}

pub(crate) fn policy_row(agent: &AgentId, policy: &Policy) -> approval_policy::ActiveModel {
    approval_policy::ActiveModel {
        agent_id: Set(agent.as_str().to_owned()),
        kind: Set(policy.kind.as_str().to_owned()),
        approver: Set(policy.approver.as_str().into_owned()),
        ..Default::default()
    }
}

pub(crate) fn approver(row: approver::Model) -> Result<ApproverRecord, DbError> {
    let slack = match (row.slack_workspace, row.slack_user) {
        (Some(workspace), Some(user)) => Some(SlackIdentity { workspace, user }),
        _ => None,
    };
    let webhook = match (row.webhook_url.as_deref(), row.webhook_secret.as_deref()) {
        (Some(url), Some(name)) => Some(WebhookTarget {
            url: url
                .parse()
                .map_err(|_| corrupt("approvers.webhook_url", url))?,
            secret: secret(name, "approvers.webhook_secret")?,
        }),
        _ => None,
    };
    Ok(ApproverRecord {
        id: approver_id(&row.id)?,
        name: row.name,
        slack,
        webhook,
        link: row.link,
    })
}

/// `record`'s row, made at `created_at` (ms) and changed at `now`.
pub(crate) fn approver_row(
    record: &ApproverRecord,
    created_at: i64,
    now: Timestamp,
) -> approver::ActiveModel {
    approver::ActiveModel {
        id: Set(record.id.as_str().to_owned()),
        name: Set(record.name.clone()),
        slack_workspace: Set(record.slack.as_ref().map(|s| s.workspace.clone())),
        slack_user: Set(record.slack.as_ref().map(|s| s.user.clone())),
        webhook_url: Set(record.webhook.as_ref().map(|w| w.url.to_string())),
        webhook_secret: Set(record
            .webhook
            .as_ref()
            .map(|w| w.secret.as_str().to_owned())),
        link: Set(record.link),
        created_at: Set(created_at),
        updated_at: Set(millis(now)),
    }
}
