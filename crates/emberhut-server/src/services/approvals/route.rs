//! Routing a new request to its approver: the owner sees it in the apps; an external approver
//! gets a Slack DM (through the agent's bot token in the approver's workspace, with buttons
//! that prove the approver's Slack identity) and a signed call to their own app, carrying the
//! one-time link when the approver may have one. The link never goes into the DM: the agent's
//! bot is a member of it and could read it back, and a link needs no identity. Nothing is
//! routed outside without the server's public address, nor for a disabled agent: the request
//! then waits in the apps only. The agent is never shown where it went. Everything shown has
//! its invisible characters written out (`emberhut_core::approvals::shown`).

use emberhut_core::agents::{Agent, AgentId, ChannelKind};
use emberhut_core::approvals::link::{LinkNonce, link_token};
use emberhut_core::approvals::shown::visible;
use emberhut_core::approvals::{
    ApprovalRequest, Approver, ApproverRecord, Decision, RequestState, webhook,
};
use jiff::Timestamp;
use serde::Serialize;

use super::channels::{DmFile, DmMessage, SignedCall};
use super::files::{StepFile, bytes_if_unchanged};
use super::{ApprovalError, ApprovalService};
use crate::convert::timestamp_to_api;

/// How a request's approver hears of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// It waits on the owner, in the apps.
    Owner,
    /// It reached the external approver through at least one channel.
    Sent,
    /// It waits on the external approver in the apps only, for this reason.
    AppsOnly(&'static str),
    /// The same payload was asked for before and still waits: nothing was sent again.
    AskedBefore,
}

impl Delivery {
    /// The sentence a tool answers with.
    #[must_use]
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Owner => "It waits on the owner in the apps.",
            Self::Sent => "It was sent to the approver.",
            Self::AppsOnly(why) => why,
            Self::AskedBefore => "It was asked for already and still waits.",
        }
    }
}

/// What an approver's app is sent when a request is made, signed with their secret.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WebhookNotice<'a> {
    id: i64,
    agent: &'a str,
    agent_name: &'a str,
    kind: &'a str,
    target: &'a str,
    summary: &'a str,
    text: Option<&'a str>,
    files: Vec<NoticeFile<'a>>,
    amount_cents: Option<u64>,
    payload_hash: String,
    expires_at: Option<String>,
    link: Option<&'a str>,
    answer_url: String,
}

/// A file as the webhook's notice lists it.
#[derive(Clone, Debug, Serialize)]
struct NoticeFile<'a> {
    name: &'a str,
    size: Option<u64>,
    sha256: String,
}

impl ApprovalService {
    /// Sends the new `request` of `agent` to its approver; `files` are its files as read.
    pub(super) async fn route(
        &self,
        agent: &Agent,
        request: &ApprovalRequest,
        files: &[StepFile],
    ) -> Delivery {
        let Approver::External(approver) = &request.approver else {
            return Delivery::Owner;
        };
        let Some(public) = self.inner.public_url.clone() else {
            return Delivery::AppsOnly(
                "It waits in the apps only: this server has no public address to send it from.",
            );
        };
        if !agent.enabled {
            return Delivery::AppsOnly("It waits in the apps only: the agent is disabled.");
        }
        let record = match self.inner.agents.approver(approver).await {
            Ok(Some(record)) => record,
            Ok(None) => return Delivery::AppsOnly("It waits in the apps only."),
            Err(error) => {
                tracing::warn!(%error, request = %request.id, "approvals: the approver could not be read");
                return Delivery::AppsOnly("It waits in the apps only.");
            }
        };
        let link = if record.link && record.webhook.is_some() {
            self.make_link(request, &public).await
        } else {
            None
        };
        let mut sent = false;
        if record.slack.is_some() {
            sent |= self.send_dm(agent, request, &record, files).await;
        }
        if record.webhook.is_some() {
            sent |= self
                .call_webhook(agent, request, &record, &public, link.as_deref())
                .await;
        }
        if sent {
            Delivery::Sent
        } else {
            Delivery::AppsOnly("It waits in the apps only: the approver could not be reached.")
        }
    }

    /// A one-time link to `request`'s page: its nonce kept (hashed) as the request's, the
    /// token signed with the link key (made on first use). None when it could not be made.
    async fn make_link(&self, request: &ApprovalRequest, public: &str) -> Option<String> {
        let made = async {
            let nonce = LinkNonce::generate(self.inner.random.as_ref())
                .map_err(super::secrets::SecretError::from)?;
            let key = self
                .inner
                .secrets
                .link_key_or_new(self.inner.random.as_ref())
                .await?;
            if !self
                .inner
                .approvals
                .set_link_nonce(request.id, &nonce.hash())
                .await?
            {
                return Ok::<_, ApprovalError>(None);
            }
            let expires = request
                .expires_at()
                .unwrap_or_else(|| self.inner.clock.now());
            let token = link_token(&key, request.id, &request.payload_hash, expires, &nonce);
            Ok(Some(format!("{public}/approve/{token}")))
        };
        made.await.unwrap_or_else(|error| {
            tracing::warn!(%error, request = %request.id, "approvals: no link was made");
            None
        })
    }

    /// The bot token of `agent`'s Slack channel in `workspace`, read now.
    pub(super) async fn bot_token(
        &self,
        agent: &AgentId,
        workspace: &str,
    ) -> Result<Option<String>, ApprovalError> {
        let channels = self.inner.agents.channels(agent).await?;
        let secret = channels.into_iter().find_map(|channel| {
            (channel.kind == ChannelKind::Slack && channel.workspace.as_deref() == Some(workspace))
                .then_some(channel.token_secret)
                .flatten()
        });
        match secret {
            Some(secret) => Ok(self.inner.secrets.read(&secret).await?),
            None => Ok(None),
        }
    }

    /// Posts `request` to the approver's Slack DM; whether it went.
    async fn send_dm(
        &self,
        agent: &Agent,
        request: &ApprovalRequest,
        record: &ApproverRecord,
        files: &[StepFile],
    ) -> bool {
        let Some(slack) = &record.slack else {
            return false;
        };
        let token = match self.bot_token(&agent.id, &slack.workspace).await {
            Ok(Some(token)) => token,
            Ok(None) => {
                tracing::warn!(
                    request = %request.id,
                    "approvals: the agent has no Slack channel with a bot token in the approver's workspace"
                );
                return false;
            }
            Err(error) => {
                tracing::warn!(%error, request = %request.id, "approvals: the bot token could not be read");
                return false;
            }
        };
        let message = dm_message(agent, request, &slack.user, files).await;
        match self.inner.slack.post(&token, &message).await {
            Ok(post) => {
                if let Err(error) = self.inner.approvals.mark_shown(request.id, &post).await {
                    tracing::warn!(%error, request = %request.id, "approvals: the DM's place was not kept");
                }
                true
            }
            Err(error) => {
                tracing::warn!(%error, request = %request.id, "approvals: the Slack DM was not sent");
                false
            }
        }
    }

    /// Posts `request` to the approver's own app, signed with their secret; whether it took it.
    async fn call_webhook(
        &self,
        agent: &Agent,
        request: &ApprovalRequest,
        record: &ApproverRecord,
        public: &str,
        link: Option<&str>,
    ) -> bool {
        let Some(target) = &record.webhook else {
            return false;
        };
        let secret = match self.inner.secrets.read(&target.secret).await {
            Ok(Some(secret)) => secret,
            Ok(None) => {
                tracing::warn!(request = %request.id, "approvals: the approver's webhook secret is missing");
                return false;
            }
            Err(error) => {
                tracing::warn!(%error, request = %request.id, "approvals: the webhook secret could not be read");
                return false;
            }
        };
        let notice = WebhookNotice {
            id: request.id.get(),
            agent: agent.id.as_str(),
            agent_name: &agent.name,
            kind: request.kind.as_str(),
            target: &request.target,
            summary: &request.summary,
            text: request.text.as_deref(),
            files: request
                .files
                .iter()
                .map(|file| NoticeFile {
                    name: &file.name,
                    size: file.size,
                    sha256: file.sha256.to_hex(),
                })
                .collect(),
            amount_cents: request.amount_cents,
            payload_hash: request.payload_hash.to_hex(),
            expires_at: request
                .expires_at()
                .map(|at| timestamp_to_api(at).as_str().to_owned()),
            link,
            answer_url: format!("{public}/api/approvals/{}/webhook", request.id),
        };
        let body = match serde_json::to_vec(&notice) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(%error, request = %request.id, "approvals: the webhook's notice could not be written");
                return false;
            }
        };
        let timestamp = self.inner.clock.now().as_second();
        let call = SignedCall {
            signature: webhook::sign(
                secret.as_bytes(),
                timestamp,
                "POST",
                target.url.path(),
                &body,
            ),
            url: target.url.clone(),
            timestamp,
            body,
        };
        match self.inner.webhook.post(&call).await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, request = %request.id, "approvals: the approver's app did not take the request");
                false
            }
        }
    }

    /// Says in `request`'s Slack DM what became of it, when it was posted there.
    pub(super) async fn tell_slack(&self, request: &ApprovalRequest) {
        let (Some(post), Approver::External(approver)) = (&request.shown, &request.approver) else {
            return;
        };
        let told = async {
            let Some(slack) = self
                .inner
                .agents
                .approver(approver)
                .await?
                .and_then(|record| record.slack)
            else {
                return Ok(());
            };
            let Some(token) = self.bot_token(&request.agent, &slack.workspace).await? else {
                return Ok(());
            };
            let outcome = outcome(request, &self.inner.zone);
            self.inner
                .slack
                .update(&token, post, &outcome)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, request = %request.id, "approvals: the DM was not updated");
                })
                .ok();
            Ok::<_, ApprovalError>(())
        };
        if let Err(error) = told.await {
            tracing::warn!(%error, request = %request.id, "approvals: the DM could not be updated");
        }
    }
}

/// `request`'s DM for the Slack user `user`: the words as they are (invisible characters
/// written out), each file's name, size and hash (attached when the bytes read again from its
/// handle still have the hash), and the hash.
async fn dm_message(
    agent: &Agent,
    request: &ApprovalRequest,
    user: &str,
    files: &[StepFile],
) -> DmMessage {
    let mut file_lines = Vec::new();
    let mut attached = Vec::new();
    for file in files {
        let digest = &file.digest;
        let mut entry = format!(
            "{}, {} bytes, sha256 {}",
            visible(&digest.name),
            digest.size,
            digest.sha256
        );
        match bytes_if_unchanged(file).await {
            Some(bytes) => attached.push(DmFile {
                name: digest.name.clone(),
                bytes,
            }),
            None => entry.push_str(" (changed since it was read: not attached)"),
        }
        file_lines.push(entry);
    }
    let amount = request.amount_cents.map_or_else(String::new, |cents| {
        format!(" of {}.{:02}", cents / 100, cents % 100)
    });
    DmMessage {
        user: user.to_owned(),
        heading: format!(
            "{} asks for your approval: {}{amount} to {}",
            visible(&agent.name),
            request.kind,
            visible(&request.target)
        ),
        summary: visible(&request.summary).into_owned(),
        text: request
            .text
            .as_deref()
            .map(|text| visible(text).into_owned()),
        file_lines,
        files: attached,
        payload_hash: request.payload_hash.to_hex(),
        button_value: format!("{}.{}", request.id, request.payload_hash),
    }
}

/// What became of `request`, in one line for its DM.
fn outcome(request: &ApprovalRequest, zone: &jiff::tz::TimeZone) -> String {
    let at = |at: Timestamp| zone.to_datetime(at).strftime("%H:%M").to_string();
    let what = match (&request.decided, request.state) {
        (_, RequestState::Withdrawn) => "Withdrawn.".to_owned(),
        (_, RequestState::Expired) => format!(
            "Expired: {}.",
            request
                .state_reason
                .as_deref()
                .unwrap_or("not answered in time")
        ),
        (Some(decided), state) => format!(
            "{} by {} at {}{}",
            match decided.decision {
                Decision::Approve => "Approved",
                Decision::Reject => "Rejected",
            },
            decided.by,
            at(decided.at),
            if state == RequestState::Used {
                ", and used."
            } else {
                "."
            }
        ),
        (None, state) => format!("{state}."),
    };
    format!("{}: {what}", request.summary)
}
