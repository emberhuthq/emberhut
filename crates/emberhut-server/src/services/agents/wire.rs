//! Agents and what is configured around them, between the contract's types (`emberhut_api::agents`)
//! and the domain's (`emberhut_core::agents`, `emberhut_core::approvals`). Reading the contract's is
//! where a body that can't be one is refused, with the field it names.

use emberhut_api::agents as api;
use emberhut_api::ids::{ApproverId as ApiApproverId, ConfiguredAgentId};
use emberhut_core::agents::{
    Agent, AgentAccount, AgentId, AgentTool, Channel, ChannelKind, Currency, SpendCap,
    TrustedRequester,
};
use emberhut_core::approvals::{
    ActionKind, Approver, ApproverId, ApproverRecord, Policy, SlackIdentity, WebhookTarget,
};
use emberhut_core::secret_link::SecretName;
use emberhut_core::task::ProjectId;
use url::Url;

use crate::convert::{room_from_api, room_to_api};

/// A body the domain can't take, and why (the field first).
pub(crate) type Refusal = String;

/// The agent `id`, read from the contract.
pub(crate) fn agent_id(id: &str) -> Result<AgentId, Refusal> {
    AgentId::parse(id).map_err(|error| error.to_string())
}

/// The approver `id`, read from the contract.
pub(crate) fn approver_id(id: &str) -> Result<ApproverId, Refusal> {
    ApproverId::parse(id).map_err(|error| error.to_string())
}

/// `agent` as the contract shows it, with how many of its requests wait.
pub(crate) fn agent(agent: &Agent, pending: u32) -> api::Agent {
    api::Agent {
        id: ConfiguredAgentId::new(agent.id.as_str()),
        name: agent.name.clone(),
        persona: agent.persona.clone(),
        principal: agent.principal.clone(),
        room: room_to_api(agent.room),
        project: agent
            .project
            .map(|project| emberhut_api::ids::ProjectId::new(project.get())),
        spend_cap_cents: agent.spend_cap.as_ref().map(|cap| cap.cents),
        spend_currency: agent
            .spend_cap
            .as_ref()
            .map(|cap| cap.currency.as_str().to_owned()),
        enabled: agent.enabled,
        worker_shell: agent.worker_shell,
        pending_approvals: pending,
    }
}

/// `agent` as the state's row of it.
pub(crate) fn agent_row(agent: &Agent, pending: u32) -> api::AgentRow {
    api::AgentRow {
        id: ConfiguredAgentId::new(agent.id.as_str()),
        name: agent.name.clone(),
        principal: agent.principal.clone(),
        room: room_to_api(agent.room),
        enabled: agent.enabled,
        pending_approvals: pending,
    }
}

/// A new agent from the contract; its room is Main's placeholder when none is named (the store
/// makes one).
pub(crate) fn new_agent(new: &api::NewAgent) -> Result<Agent, Refusal> {
    Ok(Agent {
        id: agent_id(new.id.as_str())?,
        name: new.name.clone(),
        persona: new.persona.clone(),
        principal: new.principal.clone(),
        room: new.room.map_or(emberhut_core::RoomId::MAIN, room_from_api),
        project: new.project.map(|project| ProjectId::new(project.get())),
        spend_cap: new.spend_cap.as_ref().map(spend_cap).transpose()?,
        enabled: new.enabled,
        worker_shell: new.worker_shell,
    })
}

/// A spending cap from the contract.
pub(crate) fn spend_cap(cap: &api::SpendCap) -> Result<SpendCap, Refusal> {
    Ok(SpendCap {
        cents: cap.cents,
        currency: Currency::parse(&cap.currency)
            .map_err(|error| format!("spendCap.currency: {error}"))?,
    })
}

/// A secret's name from the contract, for `field`.
fn secret(name: &str, field: &str) -> Result<SecretName, Refusal> {
    SecretName::parse(name).map_err(|error| format!("{field}: {error}"))
}

/// A channel as the contract shows it.
pub(crate) fn channel_to_api(channel: &Channel) -> api::Channel {
    api::Channel {
        kind: api::ChannelKind::from(channel.kind.as_str()),
        workspace: channel.workspace.clone(),
        address: channel.address.clone(),
        token_secret: channel
            .token_secret
            .as_ref()
            .map(|name| name.as_str().to_owned()),
    }
}

/// The channels of a `PUT`.
pub(crate) fn channels(channels: &[api::Channel]) -> Result<Vec<Channel>, Refusal> {
    channels
        .iter()
        .enumerate()
        .map(|(index, channel)| {
            Ok(Channel {
                kind: ChannelKind::parse(channel.kind.as_str())
                    .ok_or_else(|| format!("channels[{index}].kind: slack or whatsapp"))?,
                workspace: channel.workspace.clone(),
                address: channel.address.clone(),
                token_secret: channel
                    .token_secret
                    .as_deref()
                    .map(|name| secret(name, &format!("channels[{index}].tokenSecret")))
                    .transpose()?,
            })
        })
        .collect()
}

/// A tool as the contract shows it.
pub(crate) fn tool_to_api(tool: &AgentTool) -> api::Tool {
    match tool {
        AgentTool::Server { name } => api::Tool {
            kind: api::ToolKind::Server,
            name: name.clone(),
            config: None,
        },
        AgentTool::Mcp { name, config } => api::Tool {
            kind: api::ToolKind::Mcp,
            name: name.clone(),
            config: Some(config.clone()),
        },
    }
}

/// The tools of a `PUT`.
pub(crate) fn tools(tools: &[api::Tool]) -> Result<Vec<AgentTool>, Refusal> {
    tools
        .iter()
        .enumerate()
        .map(|(index, tool)| match (&tool.kind, &tool.config) {
            (api::ToolKind::Server, None) => Ok(AgentTool::Server {
                name: tool.name.clone(),
            }),
            (api::ToolKind::Mcp, Some(config)) => Ok(AgentTool::Mcp {
                name: tool.name.clone(),
                config: config.clone(),
            }),
            (api::ToolKind::Server, Some(_)) => {
                Err(format!("tools[{index}].config: a server tool has none"))
            }
            (api::ToolKind::Mcp, None) => Err(format!(
                "tools[{index}].config: an MCP server's config file is required"
            )),
            _ => Err(format!("tools[{index}].kind: server or mcp")),
        })
        .collect()
}

/// An account as the contract shows it.
pub(crate) fn account_to_api(account: &AgentAccount) -> api::Account {
    api::Account {
        label: account.label.clone(),
        secret: account.secret.as_str().to_owned(),
    }
}

/// The accounts of a `PUT`.
pub(crate) fn accounts(accounts: &[api::Account]) -> Result<Vec<AgentAccount>, Refusal> {
    accounts
        .iter()
        .enumerate()
        .map(|(index, account)| {
            Ok(AgentAccount {
                label: account.label.clone(),
                secret: secret(&account.secret, &format!("accounts[{index}].secret"))?,
            })
        })
        .collect()
}

/// A policy as the contract shows it.
pub(crate) fn policy_to_api(policy: &Policy) -> api::Policy {
    api::Policy {
        kind: emberhut_api::approvals::ActionKind::from(policy.kind.as_str()),
        approver: policy.approver.as_str().into_owned(),
    }
}

/// The policies of a `PUT`.
pub(crate) fn policies(policies: &[api::Policy]) -> Result<Vec<Policy>, Refusal> {
    policies
        .iter()
        .enumerate()
        .map(|(index, policy)| {
            Ok(Policy {
                kind: ActionKind::parse(policy.kind.as_str())
                    .ok_or_else(|| format!("policies[{index}].kind: not a kind of action"))?,
                approver: Approver::parse(&policy.approver)
                    .map_err(|error| format!("policies[{index}].approver: {error}"))?,
            })
        })
        .collect()
}

/// An approver as the contract shows it.
pub(crate) fn approver_to_api(record: &ApproverRecord) -> api::Approver {
    api::Approver {
        id: ApiApproverId::new(record.id.as_str()),
        name: record.name.clone(),
        slack_workspace: record.slack.as_ref().map(|s| s.workspace.clone()),
        slack_user: record.slack.as_ref().map(|s| s.user.clone()),
        webhook_url: record.webhook.as_ref().map(|w| w.url.to_string()),
        webhook_secret: record
            .webhook
            .as_ref()
            .map(|w| w.secret.as_str().to_owned()),
        link: record.link,
    }
}

/// An approver from the contract.
pub(crate) fn approver(approver: &api::Approver) -> Result<ApproverRecord, Refusal> {
    let slack = match (&approver.slack_workspace, &approver.slack_user) {
        (Some(workspace), Some(user)) => Some(SlackIdentity {
            workspace: workspace.clone(),
            user: user.clone(),
        }),
        (None, None) => None,
        _ => return Err("slackWorkspace and slackUser go together".to_owned()),
    };
    let webhook = match (&approver.webhook_url, &approver.webhook_secret) {
        (Some(url), Some(name)) => Some(WebhookTarget {
            url: Url::parse(url).map_err(|error| format!("webhookUrl: {error}"))?,
            secret: secret(name, "webhookSecret")?,
        }),
        (None, None) => None,
        _ => return Err("webhookUrl and webhookSecret go together".to_owned()),
    };
    Ok(ApproverRecord {
        id: approver_id(approver.id.as_str())?,
        name: approver.name.clone(),
        slack,
        webhook,
        link: approver.link,
    })
}

/// A trusted requester as the contract shows it.
pub(crate) fn requester_to_api(requester: &TrustedRequester) -> api::TrustedRequester {
    api::TrustedRequester {
        requester: requester.requester.clone(),
        agent: requester
            .agent
            .as_ref()
            .map(|agent| ConfiguredAgentId::new(agent.as_str())),
    }
}

/// The trusted requesters of a `PUT`.
pub(crate) fn requesters(
    requesters: &[api::TrustedRequester],
) -> Result<Vec<TrustedRequester>, Refusal> {
    requesters
        .iter()
        .enumerate()
        .map(|(index, requester)| {
            Ok(TrustedRequester {
                requester: requester.requester.clone(),
                agent: requester
                    .agent
                    .as_ref()
                    .map(|agent| {
                        agent_id(agent.as_str())
                            .map_err(|error| format!("requesters[{index}].agent: {error}"))
                    })
                    .transpose()?,
            })
        })
        .collect()
}
