//! What the agents' and approvals' tests share: fakes of the approval channels, and an app with
//! one agent (`client-a`, in a room of its own, with a Slack channel and a monthly cap) whose
//! posts an external approver (`example-lead`, in Slack, with a webhook and links) approves.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers and the fakes' locks: a failure here fails the test"
)]

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use emberhut_core::RoomId;
use emberhut_core::agents::AgentId;
use emberhut_core::approvals::SlackPost;
use emberhut_core::session::Session;
use emberhut_server::app::Dependencies;
use emberhut_server::config::AuthMode;
use emberhut_server::services::approvals::channels::{
    ApprovalDeps, ChannelError, DmMessage, SignedCall, SlackDm, Webhook,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::common::{Call, TestApp};

/// The approver's Slack team, user, and the DM's channel.
pub(crate) const TEAM: &str = "T0EXAMPLE";
pub(crate) const USER: &str = "U0APPROVER";
pub(crate) const DM: &str = "D0EXAMPLE";
/// The workspace's signing secret, the bot token and the approver's webhook secret.
pub(crate) const SIGNING: &str = "example-signing-secret";
pub(crate) const BOT_TOKEN: &str = "xoxb-example";
pub(crate) const WEBHOOK_SECRET: &str = "example-webhook-secret";
/// Where the approver's app takes requests.
pub(crate) const WEBHOOK_URL: &str = "https://approver.example/hook";

/// Slack, as the approval DMs reach it: every post and update, in order.
#[derive(Debug, Default)]
pub(crate) struct FakeSlackDm {
    pub(crate) posts: Mutex<Vec<(String, DmMessage)>>,
    pub(crate) updates: Mutex<Vec<(SlackPost, String)>>,
}

impl SlackDm for FakeSlackDm {
    fn post<'a>(
        &'a self,
        token: &'a str,
        message: &'a DmMessage,
    ) -> BoxFuture<'a, Result<SlackPost, ChannelError>> {
        Box::pin(async move {
            let mut posts = self.posts.lock().unwrap();
            posts.push((token.to_owned(), message.clone()));
            Ok(SlackPost {
                channel: DM.to_owned(),
                ts: format!("1800000000.{:06}", posts.len()),
            })
        })
    }

    fn update<'a>(
        &'a self,
        _: &'a str,
        post: &'a SlackPost,
        outcome: &'a str,
    ) -> BoxFuture<'a, Result<(), ChannelError>> {
        Box::pin(async move {
            self.updates
                .lock()
                .unwrap()
                .push((post.clone(), outcome.to_owned()));
            Ok(())
        })
    }
}

/// Approvers' apps: every call they were sent.
#[derive(Debug, Default)]
pub(crate) struct FakeWebhook {
    pub(crate) calls: Mutex<Vec<SignedCall>>,
}

impl Webhook for FakeWebhook {
    fn post<'a>(&'a self, call: &'a SignedCall) -> BoxFuture<'a, Result<(), ChannelError>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(call.clone());
            Ok(())
        })
    }
}

/// An app with the agent, its approver and their secrets, and a signed-in device.
pub(crate) struct Desk {
    pub(crate) app: TestApp,
    pub(crate) slack: Arc<FakeSlackDm>,
    pub(crate) webhook: Arc<FakeWebhook>,
    pub(crate) session: Session,
    pub(crate) agent: AgentId,
    pub(crate) room: RoomId,
}

impl Desk {
    /// The app with a public address.
    pub(crate) async fn new() -> Self {
        Self::build(true, AuthMode::Enforce).await
    }

    /// The app without a public address: nothing is routed outside.
    pub(crate) async fn without_public_address() -> Self {
        Self::build(false, AuthMode::Enforce).await
    }

    /// The app with a public address, in report mode.
    pub(crate) async fn in_report_mode() -> Self {
        Self::build(true, AuthMode::Report).await
    }

    async fn build(public: bool, auth: AuthMode) -> Self {
        let slack = Arc::new(FakeSlackDm::default());
        let webhook = Arc::new(FakeWebhook::default());
        let deps = ApprovalDeps {
            slack: Arc::clone(&slack) as _,
            webhook: Arc::clone(&webhook) as _,
            public_url: public.then(|| "https://emberhut.example/".to_owned()),
        };
        let app = TestApp::with_dependencies(auth, move |dependencies, _| Dependencies {
            approvals: deps,
            ..dependencies
        })
        .await;
        let secrets = app.dir().join("secrets");
        std::fs::create_dir_all(&secrets).unwrap();
        for (name, value) in [
            ("example-bot-token", BOT_TOKEN),
            ("slack-t0example-signing", SIGNING),
            ("example-lead-webhook", WEBHOOK_SECRET),
            ("slack-t0other-signing", "other-signing-secret"),
        ] {
            std::fs::write(secrets.join(name), value).unwrap();
        }
        let session = app.sign_in().await;
        let made = app
            .call(Call::post("/api/agents").token(&session).json(&json!({
                "id": "client-a",
                "name": "Example helper",
                "principal": "Example Ltd",
                "spendCap": { "cents": 10_000, "currency": "EUR" },
            })))
            .await;
        assert_eq!(made.status, 200, "{}", made.text);
        let room = RoomId::new(made.json()["agent"]["room"].as_i64().unwrap());
        let desk = Self {
            app,
            slack,
            webhook,
            session,
            agent: AgentId::parse("client-a").unwrap(),
            room,
        };
        desk.put(
            "/api/agents/client-a/channels",
            &json!({ "channels": [{
                "kind": "slack", "workspace": TEAM, "address": "C0EXAMPLE",
                "tokenSecret": "example-bot-token",
            }]}),
        )
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

    /// A file of the agent's, written now in its own folder (where an agent's files come
    /// from); its absolute path.
    pub(crate) fn file(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let folder = self.app.dir().join("agents").join(self.agent.as_str());
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// The agent's room token, as its session (and its MCP servers) carry it.
    pub(crate) fn room_token(&self) -> String {
        self.app
            .state
            .agent_tokens()
            .for_room(self.room)
            .unwrap()
            .as_str()
            .to_owned()
    }
}

/// Slack's `v0` signature over `body` at `timestamp` with `secret`.
pub(crate) fn slack_signature(secret: &str, timestamp: i64, body: &str) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    let tag = ring::hmac::sign(&key, format!("v0:{timestamp}:{body}").as_bytes());
    let hex = tag.as_ref().iter().fold(String::new(), |mut hex, b| {
        write!(hex, "{b:02x}").unwrap();
        hex
    });
    format!("v0={hex}")
}

/// Slack's interactivity form for a click on `action` with `value`, by `user` of `team`.
pub(crate) fn slack_click(team: &str, user: &str, action: &str, value: &str) -> String {
    slack_click_on(FIRST_DM_TS, team, user, action, value)
}

/// The timestamp of the first DM the fake Slack posts.
pub(crate) const FIRST_DM_TS: &str = "1800000000.000001";

/// As [`slack_click`], on the DM message at `message_ts`.
pub(crate) fn slack_click_on(
    message_ts: &str,
    team: &str,
    user: &str,
    action: &str,
    value: &str,
) -> String {
    let payload = json!({
        "type": "block_actions",
        "team": { "id": team },
        "user": { "id": user, "team_id": team },
        "container": { "channel_id": DM, "message_ts": message_ts },
        "actions": [{ "action_id": action, "value": value }],
    });
    form_urlencoded::Serializer::new(String::new())
        .append_pair("payload", &payload.to_string())
        .finish()
}
