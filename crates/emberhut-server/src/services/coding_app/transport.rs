//! The MCP client the server reaches each machine's coding-thread app with: JSON-RPC over HTTP
//! (the streamable transport). It opens a session per machine, sends the protocol version on
//! every request, and opens a new session once when the app has forgotten its own.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};

use super::CodingAppError;
use crate::http::{ClientError, Protocol, client, read_body};

/// The MCP protocol version the server speaks to the app.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long the session's opening may take.
const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a tool call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// The longest answer read from the app: a page of a thread's items, with room to spare.
const ANSWER_LIMIT: usize = 8 * 1024 * 1024;

/// An answer: its status, the session it names, and its body.
struct Reply {
    status: u16,
    session: Option<String>,
    body: String,
}

/// The MCP client, with each machine's session once it is open.
#[derive(Debug)]
pub(super) struct Transport {
    client: Client,
    sessions: Mutex<HashMap<String, String>>,
}

impl Transport {
    /// A client with no session open.
    pub(super) fn new() -> Result<Self, ClientError> {
        Ok(Self {
            client: client(Protocol::Any)?,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    /// Calls the tool `name` with `arguments` on `machine`'s app at `url`, sending `token` as
    /// the bearer, and answers the reply's body (the JSON-RPC answer, plain or as an event
    /// stream).
    pub(super) async fn call(
        &self,
        machine: &str,
        url: &str,
        token: Option<&str>,
        name: &str,
        arguments: &Value,
    ) -> Result<String, CodingAppError> {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        let unreachable = |reason: String| CodingAppError::Unreachable {
            machine: machine.to_owned(),
            reason,
        };
        let mut retried = false;
        loop {
            let session = match self.session(machine) {
                Some(session) => session,
                None => self.open(machine, url, token).await?,
            };
            let reply = self
                .post(url, token, Some(&session), &message, CALL_TIMEOUT)
                .await
                .map_err(unreachable)?;
            if matches!(reply.status, 400 | 404) {
                // The app restarted and forgot the session: a new one, once.
                self.set_session(machine, None);
                if !retried {
                    retried = true;
                    continue;
                }
            }
            if !(200..300).contains(&reply.status) {
                return Err(unreachable(format!(
                    "the coding-thread app said {}",
                    reply.status
                )));
            }
            return Ok(reply.body);
        }
    }

    /// Opens an MCP session on `machine`.
    async fn open(
        &self,
        machine: &str,
        url: &str,
        token: Option<&str>,
    ) -> Result<String, CodingAppError> {
        let unreachable = |reason: String| CodingAppError::Unreachable {
            machine: machine.to_owned(),
            reason,
        };
        let hello = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "emberhut", "version": env!("CARGO_PKG_VERSION") },
            },
        });
        let reply = self
            .post(url, token, None, &hello, OPEN_TIMEOUT)
            .await
            .map_err(unreachable)?;
        let session = reply
            .session
            .filter(|_| (200..300).contains(&reply.status))
            .filter(|session| !session.is_empty())
            .ok_or_else(|| unreachable(format!("the coding-thread app said {}", reply.status)))?;
        let ready = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        self.post(url, token, Some(&session), &ready, OPEN_TIMEOUT)
            .await
            .map_err(unreachable)?;
        self.set_session(machine, Some(session.clone()));
        Ok(session)
    }

    async fn post(
        &self,
        url: &str,
        token: Option<&str>,
        session: Option<&str>,
        body: &Value,
        timeout: Duration,
    ) -> Result<Reply, String> {
        let mut request = self
            .client
            .post(url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION)
            .header("Content-Type", "application/json")
            .timeout(timeout)
            .body(body.to_string());
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        let response = request
            .send()
            .await
            .map_err(|error| crate::error::chain(&error))?;
        let status = response.status().as_u16();
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = read_body(response, ANSWER_LIMIT)
            .await
            .map_err(|error| crate::error::chain(&error))?;
        if body.cut {
            return Err(format!("an answer over {ANSWER_LIMIT} bytes"));
        }
        Ok(Reply {
            status,
            session,
            body: body.text(),
        })
    }

    fn session(&self, machine: &str) -> Option<String> {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(machine)
            .cloned()
    }

    fn set_session(&self, machine: &str, session: Option<String>) {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        match session {
            Some(session) => sessions.insert(machine.to_owned(), session),
            None => sessions.remove(machine),
        };
    }
}
