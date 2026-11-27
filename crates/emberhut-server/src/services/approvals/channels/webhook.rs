//! An approver's own app: the server posts each new request to its `webhook_url`, signed with
//! the approver's secret (`emberhut_core::approvals::webhook`), and the app answers on the
//! request's webhook route.

use std::fmt;
use std::time::Duration;

use emberhut_core::approvals::webhook::{SIGNATURE_HEADER, TIMESTAMP_HEADER};
use futures_util::future::BoxFuture;

use super::ChannelError;
use crate::services::messaging::web::{WebClient, WebRequest};

/// How long the approver's app may take to take a request.
const CALL_WAIT: Duration = Duration::from_secs(15);

/// A call ready to go: its body, and the headers that sign it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedCall {
    /// Where it goes.
    pub url: url::Url,
    /// When it was signed, in unix seconds (`x-emberhut-timestamp`).
    pub timestamp: i64,
    /// `v1=<hex>` (`x-emberhut-signature`).
    pub signature: String,
    /// The JSON body, as signed.
    pub body: Vec<u8>,
}

/// Posts signed calls to approvers' apps.
pub trait Webhook: fmt::Debug + Send + Sync {
    /// Posts `call`; an answer other than 2xx is [`ChannelError::Refused`].
    fn post<'a>(&'a self, call: &'a SignedCall) -> BoxFuture<'a, Result<(), ChannelError>>;
}

/// No webhooks on this server.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoWebhook;

impl Webhook for NoWebhook {
    fn post<'a>(&'a self, _: &'a SignedCall) -> BoxFuture<'a, Result<(), ChannelError>> {
        Box::pin(async { Err(ChannelError::NotSetUp) })
    }
}

/// Webhooks over the server's HTTP client.
#[derive(Clone, Debug)]
pub struct HttpWebhook {
    web: WebClient,
}

impl HttpWebhook {
    /// Calls made with `web`.
    #[must_use]
    pub fn new(web: WebClient) -> Self {
        Self { web }
    }
}

impl Webhook for HttpWebhook {
    fn post<'a>(&'a self, call: &'a SignedCall) -> BoxFuture<'a, Result<(), ChannelError>> {
        Box::pin(async move {
            let request = WebRequest::post(
                call.url.as_str(),
                "application/json",
                call.body.clone(),
                CALL_WAIT,
            )
            .header(TIMESTAMP_HEADER, &call.timestamp.to_string())
            .header(SIGNATURE_HEADER, &call.signature)
            .limit(64 << 10);
            let answer = self
                .web
                .send(request)
                .await
                .map_err(|error| ChannelError::Unreachable(error.to_string()))?;
            if answer.ok() {
                Ok(())
            } else {
                Err(ChannelError::Refused(format!("HTTP {}", answer.status)))
            }
        })
    }
}
