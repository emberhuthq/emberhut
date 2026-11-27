//! What the tests of an agent's room share: an app with one configured agent (`client-a`, in
//! a room of its own, with a Slack channel, its bridge's secret and the tools it lists), its
//! approver, a fake of the agent's Slack bot, and calls to the MCP endpoint with any room's
//! token.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers and the fakes' locks: a failure here fails the test"
)]

use std::sync::{Arc, Mutex};

use emberhut_api::mcp::{CallToolResult, RpcResponse};
use emberhut_core::RoomId;
use emberhut_core::approvals::{ApprovalRequest, webhook};
use emberhut_core::session::Session;
use emberhut_server::app::{Dependencies, WorkerSetup};
use emberhut_server::config::AuthMode;
use emberhut_server::services::agents::slack::{AgentSlack, Place, ThreadMessage};
use emberhut_server::services::approvals::channels::{ApprovalDeps, ChannelError, DmFile};
use emberhut_server::services::workers::WorkerRuntime;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::approvals_common::{FakeSlackDm, FakeWebhook, TEAM, USER, WEBHOOK_SECRET, WEBHOOK_URL};
use crate::common::{Answer, Call, TestApp};
use crate::fake_cli::FakeCli;

/// The agent's Slack channel, and its bridge's token.
pub(crate) const CHANNEL: &str = "C0EXAMPLE";
pub(crate) const INBOUND: &str = "example-inbound-token";

/// One post of the agent's bot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Posted {
    pub(crate) token: String,
    pub(crate) channel: String,
    pub(crate) thread: Option<String>,
    pub(crate) text: String,
    pub(crate) files: Vec<String>,
}

/// The agent's Slack bot: every post, and the thread it reads back.
#[derive(Debug, Default)]
pub(crate) struct FakeAgentSlack {
    pub(crate) posts: Mutex<Vec<Posted>>,
    pub(crate) thread: Mutex<Vec<ThreadMessage>>,
}

impl FakeAgentSlack {
    fn record(&self, token: &str, place: Place<'_>, text: &str, files: &[DmFile]) -> usize {
        let mut posts = self.posts.lock().unwrap();
        posts.push(Posted {
            token: token.to_owned(),
            channel: place.channel.to_owned(),
            thread: place.thread.map(str::to_owned),
            text: text.to_owned(),
            files: files.iter().map(|file| file.name.clone()).collect(),
        });
        posts.len()
    }
}

impl AgentSlack for FakeAgentSlack {
    fn post<'a>(
        &'a self,
        token: &'a str,
        place: Place<'a>,
        text: &'a str,
    ) -> BoxFuture<'a, Result<String, ChannelError>> {
        Box::pin(async move {
            let n = self.record(token, place, text, &[]);
            Ok(format!("1800000100.{n:06}"))
        })
    }

    fn upload<'a>(
        &'a self,
        token: &'a str,
        place: Place<'a>,
        text: &'a str,
        files: &'a [DmFile],
    ) -> BoxFuture<'a, Result<(), ChannelError>> {
        Box::pin(async move {
            self.record(token, place, text, files);
            Ok(())
        })
    }

    fn replies<'a>(
        &'a self,
        _: &'a str,
        _: Place<'a>,
        _: u32,
    ) -> BoxFuture<'a, Result<Vec<ThreadMessage>, ChannelError>> {
        Box::pin(async move { Ok(self.thread.lock().unwrap().clone()) })
    }
}

/// The app, the agent and its approver.
pub(crate) struct AgentDesk {
    pub(crate) app: TestApp,
    pub(crate) slack: Arc<FakeAgentSlack>,
    pub(crate) session: Session,
    pub(crate) room: RoomId,
}

impl AgentDesk {
    /// The app with `client-a` listing `tools` (server tools by name).
    pub(crate) async fn new(tools: &[&str]) -> Self {
        Self::build(tools, None).await
    }

    /// [`Self::new`], its workers run on `cli`.
    pub(crate) async fn with_workers(tools: &[&str], cli: &Arc<FakeCli>) -> Self {
        let runtime = WorkerRuntime {
            spawner: Arc::clone(cli) as _,
            config: crate::workers::worker_config(),
        };
        Self::build(tools, Some(runtime)).await
    }

    async fn build(tools: &[&str], runtime: Option<WorkerRuntime>) -> Self {
        let slack = Arc::new(FakeAgentSlack::default());
        let deps = ApprovalDeps {
            slack: Arc::new(FakeSlackDm::default()),
            webhook: Arc::new(FakeWebhook::default()),
            public_url: Some("https://emberhut.example".to_owned()),
        };
        let agent_slack: Arc<dyn AgentSlack> = Arc::clone(&slack) as _;
        let app = TestApp::with_dependencies(AuthMode::Enforce, move |dependencies, _| {
            let worker_setup = match runtime {
                Some(runtime) => WorkerSetup {
                    runtime: Some(runtime),
                    ..WorkerSetup::default()
                },
                None => dependencies.worker_setup,
            };
            Dependencies {
                approvals: deps,
                agent_slack,
                worker_setup,
                ..dependencies
            }
        })
        .await;
        let secrets = app.dir().join("secrets");
        std::fs::create_dir_all(&secrets).unwrap();
        for (name, value) in [
            ("example-bot-token", "xoxb-example"),
            ("example-lead-webhook", WEBHOOK_SECRET),
            ("agent-client-a-inbound", INBOUND),
        ] {
            std::fs::write(secrets.join(name), value).unwrap();
        }
        let session = app.sign_in().await;
        let made = app
            .call(Call::post("/api/agents").token(&session).json(&json!({
                "id": "client-a",
                "name": "Example helper",
                "principal": "Example Ltd",
                "persona": "You help the example team ship.",
            })))
            .await;
        assert_eq!(made.status, 200, "{}", made.text);
        let room = RoomId::new(made.json()["agent"]["room"].as_i64().unwrap());
        let desk = Self {
            app,
            slack,
            session,
            room,
        };
        desk.put(
            "/api/agents/client-a/channels",
            &json!({ "channels": [{
                "kind": "slack", "workspace": TEAM, "address": CHANNEL,
                "tokenSecret": "example-bot-token",
            }]}),
        )
        .await;
        let listed: Vec<Value> = tools
            .iter()
            .map(|name| json!({ "kind": "server", "name": name }))
            .collect();
        desk.put("/api/agents/client-a/tools", &json!({ "tools": listed }))
            .await;
        let approver = desk
            .app
            .call(
                Call::post("/api/approvers")
                    .token(&desk.session)
                    .json(&json!({
                        "id": "example-lead",
                        "name": "Example lead",
                        "slackWorkspace": TEAM,
                        "slackUser": USER,
                        "webhookUrl": WEBHOOK_URL,
                        "webhookSecret": "example-lead-webhook",
                        "link": true,
                    })),
            )
            .await;
        assert_eq!(approver.status, 200, "{}", approver.text);
        desk.set_policies(&json!([
            { "kind": "post", "approver": "example-lead" },
            { "kind": "merge", "approver": "none" },
        ]))
        .await;
        desk
    }

    /// `PUT`s `body` at `path` from the signed-in device; it must answer 200.
    pub(crate) async fn put(&self, path: &str, body: &Value) -> Value {
        let answer = self
            .app
            .call(Call::put(path).token(&self.session).json(body))
            .await;
        assert_eq!(answer.status, 200, "{path}: {}", answer.text);
        answer.json()
    }

    /// Replaces the agent's policies.
    pub(crate) async fn set_policies(&self, policies: &Value) {
        self.put(
            "/api/agents/client-a/policies",
            &json!({ "policies": policies }),
        )
        .await;
    }

    /// Turns the agent off (or on again).
    pub(crate) async fn enable(&self, enabled: bool) {
        let answer = self
            .app
            .call(
                Call::patch("/api/agents/client-a")
                    .token(&self.session)
                    .json(&json!({ "enabled": enabled })),
            )
            .await;
        assert_eq!(answer.status, 200, "{}", answer.text);
    }

    /// The agent's own folder.
    pub(crate) fn folder(&self) -> std::path::PathBuf {
        let folder = self.app.dir().join("agents").join("client-a");
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A file of the agent's, written now in its own folder; its absolute path.
    pub(crate) fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.folder().join(name);
        std::fs::write(&path, bytes).unwrap();
        path.display().to_string()
    }

    /// A file of the owner's, outside the agent's folders; its absolute path.
    pub(crate) fn owners_file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.app.dir().join(name);
        std::fs::write(&path, bytes).unwrap();
        path.display().to_string()
    }

    /// The token of `room`'s session.
    pub(crate) fn token_of(&self, room: RoomId) -> String {
        self.app
            .state
            .agent_tokens()
            .for_room(room)
            .unwrap()
            .as_str()
            .to_owned()
    }

    /// `tools/list` with the agent's room token: the tools' names.
    pub(crate) async fn tool_names(&self, room: RoomId) -> Vec<String> {
        let listed = rpc_as(&self.app, &self.token_of(room), "tools/list", json!({})).await;
        assert_eq!(listed.status, 200, "{}", listed.text);
        listed.json()["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }

    /// `tools/call` of `name` with `arguments`, from `room`'s session.
    pub(crate) async fn call_in(
        &self,
        room: RoomId,
        name: &str,
        arguments: Value,
    ) -> CallToolResult {
        call_tool_as(&self.app, &self.token_of(room), name, arguments).await
    }

    /// `tools/call` of `name` with `arguments`, from the agent's room.
    pub(crate) async fn call(&self, name: &str, arguments: Value) -> CallToolResult {
        self.call_in(self.room, name, arguments).await
    }

    /// Approves `request` through its approver's app, as the approver.
    pub(crate) async fn approve(&self, request: &ApprovalRequest) {
        let path = format!("/api/approvals/{}/webhook", request.id.get());
        let body = json!({
            "decision": "approve", "payloadHash": request.payload_hash.to_hex(), "by": "lead@example"
        })
        .to_string();
        let at = self.app.clock_now().as_second();
        let signature = webhook::sign(
            WEBHOOK_SECRET.as_bytes(),
            at,
            "POST",
            &path,
            body.as_bytes(),
        );
        let answer = self
            .app
            .call(
                Call::post(&path)
                    .header(webhook::TIMESTAMP_HEADER, &at.to_string())
                    .header(webhook::SIGNATURE_HEADER, &signature)
                    .raw(&body),
            )
            .await;
        assert_eq!(answer.status, 200, "{}", answer.text);
    }
}

/// One MCP message with `token`.
pub(crate) async fn rpc_as(app: &TestApp, token: &str, method: &str, params: Value) -> Answer {
    let body = json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params });
    app.call(
        Call::post(emberhut_api::paths::MCP_ROOM)
            .header("authorization", &format!("Bearer {token}"))
            .header("accept", "application/json, text/event-stream")
            .json(&body),
    )
    .await
}

/// `tools/call` of `name` with `arguments`, with `token`: its result.
pub(crate) async fn call_tool_as(
    app: &TestApp,
    token: &str,
    name: &str,
    arguments: Value,
) -> CallToolResult {
    let answer = rpc_as(
        app,
        token,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let reply: RpcResponse = answer.read_as();
    serde_json::from_value(reply.result.unwrap()).unwrap()
}
