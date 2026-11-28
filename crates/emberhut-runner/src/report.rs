//! What the runner tells its server: every change of a session as an event, in order.
//!
//! An event waits in the outbox until the server has taken it, so a server restart (or no server
//! yet) loses nothing; a newer "what it's doing" line replaces an older one not yet sent. The
//! reporter also asks the server, every 10 minutes, for the MCP servers its sessions get.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::sync::Notify;

use crate::store::{Store, now};
use crate::wire;

/// Reads the runner's token when it is needed (pairing may write it later).
pub type TokenFn = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// How the runner reaches its server.
#[derive(Clone)]
pub struct Link {
    /// The server's address (`None`: not paired yet).
    pub server: Option<String>,
    /// The runner's token, read when needed.
    pub token: TokenFn,
    /// The runner's name.
    pub name: String,
    /// The HTTP client.
    pub http: reqwest::Client,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link")
            .field("server", &self.server)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Sends the outbox to the server, oldest first.
#[derive(Debug)]
pub struct Reporter {
    store: Store,
    link: Link,
    wake: Notify,
    mcp: Mutex<Map<String, Value>>,
}

const WAIT: Duration = Duration::from_secs(15);

impl Reporter {
    /// A reporter writing to this store's outbox and sending over this link.
    #[must_use]
    pub fn new(store: Store, link: Link) -> Arc<Self> {
        Arc::new(Self {
            store,
            link,
            wake: Notify::new(),
            mcp: Mutex::default(),
        })
    }

    /// The link to the server.
    #[must_use]
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// Queues one event: its kind, and what it carries (the session as it is now, and the rest).
    pub async fn tell(&self, session: &str, kind: &str, body: Map<String, Value>) {
        let mut event = Map::new();
        event.insert("machine".to_owned(), Value::String(self.link.name.clone()));
        event.insert("kind".to_owned(), Value::String(kind.to_owned()));
        event.insert("at".to_owned(), Value::String(now()));
        event.extend(body);
        let text = Value::Object(event).to_string();
        if let Err(e) = self
            .store
            .push_outbox(session, kind, &text, kind == wire::event::ACTIVITY)
            .await
        {
            tracing::warn!(session, kind, "cannot queue an event: {e}");
        }
        self.wake.notify_one();
    }

    /// How many events wait for the server.
    pub async fn pending(&self) -> u64 {
        self.store.outbox_len().await.unwrap_or(0)
    }

    /// Sends what waits, for as long as the runner runs; on a failure it tries again later,
    /// waiting twice as long each time (30 s at most).
    pub async fn run(self: Arc<Self>) {
        let mut backoff = Duration::ZERO;
        loop {
            if self.flush().await.is_ok() {
                backoff = Duration::ZERO;
                self.wake.notified().await;
            } else {
                backoff = if backoff.is_zero() {
                    Duration::from_secs(2)
                } else {
                    (backoff * 2).min(Duration::from_secs(30))
                };
                // A new event doesn't make the server answer sooner: wait the backoff out.
                tokio::time::sleep(backoff).await;
            }
        }
    }

    /// Sends every waiting event once, in order; `Err` when the server didn't take one.
    async fn flush(&self) -> Result<(), ()> {
        let (Some(server), Some(token)) = (self.link.server.as_deref(), (self.link.token)()) else {
            return Ok(());
        };
        loop {
            let Ok(Some(e)) = self.store.first_outbox().await else {
                return Ok(());
            };
            let sent = self
                .link
                .http
                .post(format!("{server}{}", wire::EVENTS))
                .bearer_auth(&token)
                .header("Content-Type", "application/json")
                .body(e.body.clone().unwrap_or_default())
                .timeout(WAIT)
                .send()
                .await;
            match sent {
                Ok(r) if r.status().is_success() => {}
                Ok(r) if r.status() == reqwest::StatusCode::BAD_REQUEST => {
                    // The server will never take this one (not this runner's, or not an event):
                    // kept in the log, not retried forever.
                    let why = r.text().await.unwrap_or_default();
                    tracing::warn!("the server refused an event ({why}); dropped");
                }
                Ok(r) => {
                    tracing::debug!("the server answered {} to an event", r.status());
                    return Err(());
                }
                Err(e) => {
                    tracing::debug!("the server didn't take an event: {e}");
                    return Err(());
                }
            }
            if self.store.drop_outbox(e.id).await.is_err() {
                return Err(());
            }
        }
    }

    /// The MCP servers sessions get, as last heard from the server.
    #[must_use]
    pub fn shared_mcp(&self) -> Map<String, Value> {
        self.mcp.lock().map_or_else(|_| Map::new(), |m| m.clone())
    }

    /// Asks the server for the MCP servers sessions get; keeps the last answer when it can't be reached.
    pub async fn load_mcp(&self) {
        let (Some(server), Some(token)) = (self.link.server.as_deref(), (self.link.token)()) else {
            return;
        };
        let Ok(r) = self
            .link
            .http
            .get(format!("{server}{}", wire::MCP))
            .bearer_auth(token)
            .timeout(WAIT)
            .send()
            .await
        else {
            return;
        };
        if !r.status().is_success() {
            return;
        }
        if let Ok(Value::Object(mut body)) = r.json::<Value>().await
            && let Some(Value::Object(servers)) = body.remove("mcpServers")
            && let Ok(mut m) = self.mcp.lock()
        {
            *m = servers;
        }
    }

    /// Posts a machine sample.
    ///
    /// # Errors
    /// The server didn't take it: its status, or why it wasn't reached.
    pub async fn sample(&self, body: &Value) -> Result<(), String> {
        let (Some(server), Some(token)) = (self.link.server.as_deref(), (self.link.token)()) else {
            return Err("not paired".to_owned());
        };
        match self
            .link
            .http
            .post(format!("{server}{}", wire::MACHINE))
            .bearer_auth(token)
            .json(body)
            .timeout(WAIT)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => Err(r.status().as_u16().to_string()),
            Err(e) => Err(if e.is_timeout() {
                "timed out".to_owned()
            } else {
                "not reachable".to_owned()
            }),
        }
    }
}
