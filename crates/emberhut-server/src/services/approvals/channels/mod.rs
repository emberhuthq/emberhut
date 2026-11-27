//! How a request reaches an external approver: a Slack DM with buttons ([`SlackDm`]) and a
//! signed call to the approver's own app ([`Webhook`]). The signed link is the server's own
//! page (`routes::approve`); it goes only in the signed call, never in the DM, which the
//! agent's bot is a member of and could read back.
//!
//! Each channel is a trait with a real implementation over the server's HTTP client
//! ([`HttpSlackDm`], [`HttpWebhook`]) and a stand-in for a server that has none
//! ([`NoSlackDm`], [`NoWebhook`]); tests swap in fakes. What a channel proves on the way back
//! (Slack's signature, the approver's HMAC, the link's MAC) is checked by the service, not here.

mod slack;
mod webhook;

use std::sync::Arc;

pub(crate) use slack::{APPROVE_ACTION, REJECT_ACTION};
pub use slack::{DmFile, DmMessage, HttpSlackDm, NoSlackDm, SlackDm};
pub use webhook::{HttpWebhook, NoWebhook, SignedCall, Webhook};

/// Why a channel did not deliver.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ChannelError {
    /// This server has no such channel.
    #[error("this channel isn't set up on this server")]
    NotSetUp,
    /// The other end didn't answer.
    #[error("unreachable: {0}")]
    Unreachable(String),
    /// The other end answered with a refusal.
    #[error("refused: {0}")]
    Refused(String),
}

/// The outside services approvals are routed through, and the address they reach the server
/// at: what a test swaps.
#[derive(Clone, Debug)]
pub struct ApprovalDeps {
    /// Slack, for an approver's DM.
    pub slack: Arc<dyn SlackDm>,
    /// An approver's own app.
    pub webhook: Arc<dyn Webhook>,
    /// The server's address from outside (`EMBERHUT_PUBLIC_URL`), without a final `/`. None: no
    /// request is routed outside; each waits in the apps only.
    pub public_url: Option<String>,
}

impl Default for ApprovalDeps {
    /// No Slack, no webhook, no public address: approvals are made in the apps only.
    fn default() -> Self {
        Self {
            slack: Arc::new(NoSlackDm),
            webhook: Arc::new(NoWebhook),
            public_url: None,
        }
    }
}
