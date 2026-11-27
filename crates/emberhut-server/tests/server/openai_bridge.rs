//! The `OpenAI` bridge, end to end: the provider saved from the apps, a launch of the scripted CLI
//! given the bridge's address and a token of its own, and a client that speaks as Claude Code
//! does (Anthropic's Messages API, streamed and not) posting to the bridge, answered by a
//! stand-in for an `OpenAI` Chat Completions endpoint on loopback. No test reaches a real
//! provider.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure fails the test that called them"
)]

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::agent_provider::AgentProvider;
use emberhut_api::setup::SetupState;
use emberhut_core::OsRandom;
use emberhut_core::session::Session;
use emberhut_server::app::{Dependencies, WorkerSetup};
use emberhut_server::services::agent_provider::HttpProviderApi;
use emberhut_server::services::agent_provider::bridge::{BridgeParts, OpenAiBridge};
use emberhut_server::services::claude::{ClaudeAssistant, Launch};
use emberhut_server::services::messaging::web::WebClient;
use emberhut_server::services::talk::Assistant;
use emberhut_server::services::workers::WorkerRuntime;
use serde_json::{Value, json};

use crate::agent_provider::{TOKEN, echoing, provider_item, put, store_of, talk};
use crate::common::{Call, TestApp, http_client};
use crate::fake_cli::{FakeCli, config};

/// What the stand-in answers a chat completion with.
enum Scripted {
    /// These pieces of an event stream, each sent as it is.
    Stream(Vec<String>),
    /// This whole answer.
    Whole(Value),
    /// This status and body, with `retry-after: 7`.
    Refuse(u16, Value),
}

/// What the stand-in saw of one request: its path, its bearer, its body.
#[derive(Clone, Debug)]
struct Asked {
    path: String,
    bearer: Option<String>,
    body: Value,
}

/// A stand-in for an `OpenAI` Chat Completions endpoint: a model list at `/v1/models`, and the
/// scripted answers, in order, at `/v1/chat/completions`.
#[derive(Clone, Default)]
struct FakeOpenAi {
    asked: Arc<Mutex<Vec<Asked>>>,
    answers: Arc<Mutex<VecDeque<Scripted>>>,
}

impl FakeOpenAi {
    /// Serves it on loopback; its base URL, with `/v1`.
    async fn serve(&self) -> String {
        let app = Router::new().fallback(openai).with_state(self.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}/v1")
    }

    fn answers(&self, answer: Scripted) {
        self.answers.lock().unwrap().push_back(answer);
    }

    /// The chat completions asked for so far.
    fn completions(&self) -> Vec<Asked> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|asked| asked.path == "/v1/chat/completions")
            .cloned()
            .collect()
    }
}

async fn openai(State(fake): State<FakeOpenAi>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    let bearer = parts
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned);
    fake.asked.lock().unwrap().push(Asked {
        path: parts.uri.path().to_owned(),
        bearer: bearer.clone(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    });
    if bearer.as_deref() != Some(TOKEN) {
        let refused = json!({ "error": { "message": "Incorrect API key provided",
            "type": "invalid_request_error", "code": "invalid_api_key" } });
        return (StatusCode::UNAUTHORIZED, axum::Json(refused)).into_response();
    }
    match parts.uri.path() {
        "/v1/models" => axum::Json(json!({ "object": "list", "data": [
            { "id": "local-big", "object": "model" }, { "id": "local-small", "object": "model" },
        ] }))
        .into_response(),
        "/v1/chat/completions" => match fake.answers.lock().unwrap().pop_front() {
            Some(Scripted::Stream(pieces)) => {
                let pieces = pieces
                    .into_iter()
                    .map(|piece| Ok::<_, Infallible>(Bytes::from(piece)));
                let mut response =
                    Response::new(Body::from_stream(futures_util::stream::iter(pieces)));
                response.headers_mut().insert(
                    "content-type",
                    axum::http::HeaderValue::from_static("text/event-stream"),
                );
                response
            }
            Some(Scripted::Whole(answer)) => axum::Json(answer).into_response(),
            Some(Scripted::Refuse(status, body)) => (
                StatusCode::from_u16(status).unwrap(),
                [("retry-after", "7")],
                axum::Json(body),
            )
                .into_response(),
            None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A chunk of a streamed chat completion.
fn chunk(delta: &Value, finish: Option<&str>) -> String {
    let chunk = json!({ "id": "chatcmpl-1", "object": "chat.completion.chunk",
        "model": "local-big", "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] });
    format!("data: {chunk}\n\n")
}

/// An app whose agent runtime runs on the scripted `cli` and the saved provider, through an
/// `OpenAI` bridge served on loopback; its checks go to `fake`.
async fn app_on(fake: &str, cli: &Arc<FakeCli>) -> TestApp {
    let cli = Arc::clone(cli);
    let fake = fake.to_owned();
    let access = Arc::new(Mutex::new(None));
    let given = Arc::clone(&access);
    TestApp::with_agent_and(
        move |db, clock, dir| {
            let bridge = OpenAiBridge::bind(BridgeParts {
                store: store_of(dir, db),
                client: http_client(),
                random: Arc::new(OsRandom),
            })
            .unwrap();
            let reached = bridge.access();
            *given.lock().unwrap() = Some(reached.clone());
            tokio::spawn(bridge.serve(std::future::pending()));
            Arc::new(
                ClaudeAssistant::new(config(&[]), cli, db.settings(), db.rooms(), clock)
                    .with_provider(store_of(dir, db).with_bridge(reached)),
            ) as Arc<dyn Assistant>
        },
        move |dependencies, _| Dependencies {
            provider_api: Arc::new(HttpProviderApi::new(WebClient::new(http_client()), &fake)),
            agent_bridge: access.lock().unwrap().take(),
            // An agent CLI is configured, with no accounts: the saved provider is its login.
            worker_setup: WorkerSetup {
                runtime: Some(WorkerRuntime {
                    spawner: FakeCli::new(|_, _| Vec::new()),
                    config: crate::workers::worker_config(),
                }),
                ..WorkerSetup::default()
            },
            ..dependencies
        },
    )
    .await
}

/// A request as Claude Code makes one: its system prompt in blocks with cache marks, its
/// tools, thinking on, and what it says.
fn claude_code(model: &str, stream: bool, messages: &Value) -> Value {
    json!({
        "model": model,
        "max_tokens": 32000,
        "stream": stream,
        "system": [
            { "type": "text", "text": "You are Claude Code." },
            { "type": "text", "text": "Work in /work.", "cache_control": { "type": "ephemeral" } },
        ],
        "messages": messages,
        "tools": [
            { "name": "Bash", "description": "Runs a command.", "input_schema": {
                "type": "object", "properties": { "command": { "type": "string" } },
                "required": ["command"] } },
            { "name": "mcp__emberhut__task", "description": "Reads a task.",
                "input_schema": { "type": "object" } },
        ],
        "tool_choice": { "type": "auto" },
        "thinking": { "type": "enabled", "budget_tokens": 4000 },
        "metadata": { "user_id": "user_x" },
    })
}

/// What the bridge answered: its status, its `retry-after`, and its body.
struct Answered {
    status: u16,
    retry_after: Option<String>,
    text: String,
}

impl Answered {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap()
    }

    /// Its events, as `(name, data)`.
    fn events(&self) -> Vec<(String, Value)> {
        self.text
            .split("\n\n")
            .filter(|event| !event.trim().is_empty())
            .map(|event| {
                let mut name = String::new();
                let mut data = Value::Null;
                for line in event.lines() {
                    if let Some(value) = line.strip_prefix("event: ") {
                        value.clone_into(&mut name);
                    } else if let Some(value) = line.strip_prefix("data: ") {
                        data = serde_json::from_str(value).unwrap();
                    }
                }
                assert_eq!(data["type"], name.as_str(), "{event}");
                (name, data)
            })
            .collect()
    }
}

/// Posts `body` to the bridge's `path` as the CLI does, with `token` as its bearer.
async fn post(launch: &Launch, path: &str, token: Option<&str>, body: &Value) -> Answered {
    let base = launch.var("ANTHROPIC_BASE_URL").unwrap();
    let mut request = http_client()
        .post(format!("{base}{path}"))
        .header("content-type", "application/json")
        .header("anthropic-version", "2023-06-01")
        .header(
            "anthropic-beta",
            "claude-code-20250219,interleaved-thinking-2025-05-14",
        )
        .body(body.to_string());
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let text = response.text().await.unwrap();
    Answered {
        status,
        retry_after,
        text,
    }
}

/// The app on a stand-in `OpenAI`-compatible endpoint, its provider saved (`local-big`, quick
/// `local-small`), and the launch of Main's session with its bridge token.
struct OnTheBridge {
    fake: FakeOpenAi,
    base: String,
    cli: Arc<FakeCli>,
    app: TestApp,
    session: Session,
    launch: Launch,
    token: String,
}

async fn on_the_bridge() -> OnTheBridge {
    let fake = FakeOpenAi::default();
    let base = fake.serve().await;
    let cli = echoing();
    let app = app_on(&base, &cli).await;
    let session = app.sign_in().await;
    let saved = put(
        &app,
        &session,
        &json!({ "provider": "openai-compatible", "apiKey": TOKEN, "baseUrl": base,
            "model": "local-big", "fastModel": "local-small" }),
    )
    .await;
    assert_eq!(saved.status, 200, "{}", saved.text);
    let now: AgentProvider = saved.read_as();
    assert!(now.usable && now.in_use, "{}", saved.text);
    talk(&app, &session, "Hello").await;
    let launch = cli.launches()[0].clone();
    let token = launch.var("ANTHROPIC_AUTH_TOKEN").unwrap().to_owned();
    OnTheBridge {
        fake,
        base,
        cli,
        app,
        session,
        launch,
        token,
    }
}

/// A conversation where the model called `Bash` and the CLI gives its result back.
fn answered_call() -> Value {
    json!([
        { "role": "user", "content": "What is in /work?" },
        { "role": "assistant", "content": [
            { "type": "thinking", "thinking": "Let me look.", "signature": "c2ln" },
            { "type": "text", "text": "I'll list the files." },
            { "type": "tool_use", "id": "call_1", "name": "Bash",
                "input": { "command": "ls -la /work" } }] },
        { "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "call_1",
                "content": [{ "type": "text", "text": "a.txt" }] }] },
    ])
}

#[tokio::test]
async fn the_cli_gets_the_bridge_and_a_token_of_its_own_never_the_key() {
    let OnTheBridge {
        base,
        cli,
        app,
        session,
        launch,
        token,
        ..
    } = on_the_bridge().await;
    assert_eq!(
        provider_item(&app, &session).await,
        (
            SetupState::Ready,
            "The agent runs on the OpenAI-compatible endpoint, on local-big, through the \
             server's OpenAI bridge."
                .to_owned()
        )
    );
    talk(&app, &session, "Again").await;
    assert_eq!(
        cli.launches().len(),
        1,
        "the same provider keeps its session"
    );
    let bridge = launch.var("ANTHROPIC_BASE_URL").unwrap();
    assert!(bridge.starts_with("http://127.0.0.1:"), "{bridge}");
    assert_ne!(bridge, base);
    assert_ne!(token, TOKEN);
    assert_eq!(launch.var("ANTHROPIC_API_KEY"), Some(""));
    assert_eq!(launch.var("ANTHROPIC_MODEL"), Some("local-big"));
    assert_eq!(
        launch.var("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
        Some("local-small")
    );
    assert_eq!(launch.flag("--model"), None);
    assert!(launch.strips("ANTHROPIC_BASE_URL"));
    assert!(
        launch.env.iter().all(|(_, value)| !value.contains(TOKEN))
            && launch.args.iter().all(|arg| !arg.contains(TOKEN)),
        "the endpoint's key never reaches the CLI"
    );
}

#[tokio::test]
async fn a_streamed_turn_with_a_tool_call_comes_back_as_anthropic_s_events() {
    // The app is kept: its folder holds the endpoint's key.
    let OnTheBridge {
        fake,
        app: _app,
        launch,
        token,
        ..
    } = on_the_bridge().await;
    let launch = &launch;
    // Turn one, streamed: some text, then a call whose arguments come in pieces.
    let arguments = r#"{"command":"ls -la /work"}"#;
    let mut pieces = vec![
        chunk(&json!({ "role": "assistant", "content": "" }), None),
        chunk(&json!({ "content": "I'll list " }), None),
        chunk(&json!({ "content": "the files." }), None),
        chunk(
            &json!({ "tool_calls": [{ "index": 0, "id": "call_1", "type": "function",
                "function": { "name": "Bash", "arguments": "" } }] }),
            None,
        ),
    ];
    for piece in arguments.as_bytes().chunks(4) {
        let piece = std::str::from_utf8(piece).unwrap();
        pieces.push(chunk(
            &json!({ "tool_calls": [{ "index": 0, "function": { "arguments": piece } }] }),
            None,
        ));
    }
    pieces.push(chunk(&json!({}), Some("tool_calls")));
    let usage = json!({ "choices": [], "usage": { "prompt_tokens": 900, "completion_tokens": 40,
        "prompt_tokens_details": { "cached_tokens": 800 } } });
    pieces.push(format!("data: {usage}\n\n"));
    pieces.push("data: [DONE]\n\n".to_owned());
    // Cut mid-line too, as a network may.
    let whole = pieces.concat();
    let (a, b) = whole.split_at(whole.len() / 2);
    fake.answers(Scripted::Stream(vec![a.to_owned(), b.to_owned()]));
    let asked = json!([{ "role": "user", "content": "What is in /work?" }]);
    let answer = post(
        launch,
        "/v1/messages?beta=true",
        Some(&token),
        &claude_code("local-big", true, &asked),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let events = answer.events();
    let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names[..2], ["message_start", "ping"]);
    assert_eq!(names[names.len() - 2..], ["message_delta", "message_stop"]);
    let mut text = String::new();
    let mut input = String::new();
    for (name, data) in &events {
        match (name.as_str(), data["delta"]["type"].as_str()) {
            ("content_block_delta", Some("text_delta")) => {
                text.push_str(data["delta"]["text"].as_str().unwrap());
            }
            ("content_block_delta", Some("input_json_delta")) => {
                assert_eq!(data["index"], 1);
                input.push_str(data["delta"]["partial_json"].as_str().unwrap());
            }
            ("content_block_start", _) if data["index"] == 1 => {
                assert_eq!(
                    data["content_block"],
                    json!({ "type": "tool_use", "id": "call_1", "name": "Bash", "input": {} })
                );
            }
            _ => {}
        }
    }
    assert_eq!(text, "I'll list the files.");
    assert_eq!(
        serde_json::from_str::<Value>(&input).unwrap(),
        json!({ "command": "ls -la /work" })
    );
    let (_, delta) = &events[events.len() - 2];
    assert_eq!(delta["delta"]["stop_reason"], "tool_use");
    assert_eq!(
        delta["usage"],
        json!({ "input_tokens": 100, "output_tokens": 40, "cache_read_input_tokens": 800,
            "cache_creation_input_tokens": 0 })
    );

    let sent = &fake.completions()[0];
    assert_eq!(
        sent.bearer.as_deref(),
        Some(TOKEN),
        "the bridge holds the key"
    );
    let body = &sent.body;
    assert_eq!(body["model"], "local-big");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({ "include_usage": true }));
    assert_eq!(body["max_tokens"], 32000);
    assert_eq!(
        body["messages"][0],
        json!({ "role": "system", "content": "You are Claude Code.\n\nWork in /work." })
    );
    assert_eq!(body["tools"][1]["function"]["name"], "mcp__emberhut__task");
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body.get("thinking"), None);
    assert_eq!(body.get("metadata"), None);
}

#[tokio::test]
async fn a_tool_s_result_goes_back_whole_on_the_quick_model() {
    // The app is kept: its folder holds the endpoint's key.
    let OnTheBridge {
        fake,
        app: _app,
        launch,
        token,
        ..
    } = on_the_bridge().await;
    let launch = &launch;
    // Turn two, whole, on the quick model by a Claude model's name: the tool's result back.
    fake.answers(Scripted::Whole(json!({
        "id": "chatcmpl-2", "object": "chat.completion",
        "choices": [{ "index": 0, "finish_reason": "stop",
            "message": { "role": "assistant", "content": "There is one file, a.txt." } }],
        "usage": { "prompt_tokens": 950, "completion_tokens": 9 },
    })));
    let asked = answered_call();
    let answer = post(
        launch,
        "/v1/messages",
        Some(&token),
        &claude_code("claude-haiku-4-5-20251001", false, &asked),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let message = answer.json();
    assert_eq!(message["type"], "message");
    assert_eq!(message["role"], "assistant");
    assert_eq!(
        message["content"],
        json!([{ "type": "text", "text": "There is one file, a.txt." }])
    );
    assert_eq!(message["stop_reason"], "end_turn");
    assert_eq!(message["usage"]["output_tokens"], 9);
    let body = &fake.completions()[0].body;
    assert_eq!(
        body["model"], "local-small",
        "a haiku model is the quick one"
    );
    assert_eq!(body.get("stream"), None);
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(
        messages[2],
        json!({ "role": "assistant", "content": "I'll list the files.", "tool_calls": [
            { "id": "call_1", "type": "function",
                "function": { "name": "Bash", "arguments": "{\"command\":\"ls -la /work\"}" } }] })
    );
    assert_eq!(
        messages[3],
        json!({ "role": "tool", "tool_call_id": "call_1", "content": "a.txt" })
    );

    // Its token counted, as the CLI may ask.
    let counted = post(
        launch,
        "/v1/messages/count_tokens?beta=true",
        Some(&token),
        &claude_code("local-big", false, &asked),
    )
    .await;
    assert_eq!(counted.status, 200, "{}", counted.text);
    assert!(counted.json()["input_tokens"].as_u64().unwrap() > 10);
}

#[tokio::test]
async fn refusals_are_the_messages_api_s_and_nothing_passes_without_the_token() {
    let OnTheBridge {
        fake,
        app,
        session,
        launch,
        token,
        ..
    } = on_the_bridge().await;
    let launch = &launch;
    let asked = answered_call();
    // The endpoint's rate limit is the Messages API's.
    fake.answers(Scripted::Refuse(
        429,
        json!({ "error": { "message": "Rate limit reached for requests", "type": "requests",
            "code": "rate_limit_exceeded" } }),
    ));
    let limited = post(
        launch,
        "/v1/messages",
        Some(&token),
        &claude_code("local-big", true, &asked),
    )
    .await;
    assert_eq!(limited.status, 429);
    assert_eq!(limited.retry_after.as_deref(), Some("7"));
    assert_eq!(
        limited.json(),
        json!({ "type": "error", "error": { "type": "rate_limit_error",
            "message": "The OpenAI-compatible endpoint answered 429: Rate limit reached for requests" } })
    );

    // Nothing gets past the bridge without the launch's token.
    let before = fake.completions().len();
    for wrong in [None, Some("not-the-token"), Some(TOKEN)] {
        let refused = post(
            launch,
            "/v1/messages",
            wrong,
            &claude_code("local-big", false, &asked),
        )
        .await;
        assert_eq!(refused.status, 401, "{wrong:?}");
        assert_eq!(refused.json()["error"]["type"], "authentication_error");
    }
    let unknown = post(launch, "/v1/models", Some(&token), &json!({})).await;
    assert_eq!(unknown.status, 404);
    assert_eq!(unknown.json()["error"]["type"], "not_found_error");
    assert_eq!(
        fake.completions().len(),
        before,
        "none reached the endpoint"
    );

    // Once the provider is removed, the bridge serves none.
    let removed = app
        .call(Call::delete(emberhut_api::paths::AGENT_PROVIDER).token(&session))
        .await;
    assert_eq!(removed.status, 200);
    let gone = post(
        launch,
        "/v1/messages",
        Some(&token),
        &claude_code("local-big", false, &asked),
    )
    .await;
    assert_eq!(gone.status, 403, "{}", gone.text);
    assert_eq!(gone.json()["error"]["type"], "permission_error");
    assert_eq!(fake.completions().len(), before);
}

#[tokio::test]
async fn openai_s_own_api_gets_max_completion_tokens_and_a_broken_stream_ends_in_an_error() {
    let fake = FakeOpenAi::default();
    let base = fake.serve().await;
    let cli = echoing();
    let app = app_on(&base, &cli).await;
    let session = app.sign_in().await;
    let saved = put(
        &app,
        &session,
        &json!({ "provider": "openai", "apiKey": TOKEN, "baseUrl": base, "model": "local-big" }),
    )
    .await;
    assert_eq!(saved.status, 200, "{}", saved.text);
    talk(&app, &session, "Hello").await;
    let launch = &cli.launches()[0];
    let token = launch.var("ANTHROPIC_AUTH_TOKEN").unwrap().to_owned();
    assert_eq!(
        launch.var("ANTHROPIC_SMALL_FAST_MODEL"),
        Some("local-big"),
        "no quick model: the model"
    );

    // The stream stops mid-answer, with no finish.
    fake.answers(Scripted::Stream(vec![chunk(
        &json!({ "content": "Half an ans" }),
        None,
    )]));
    let asked = json!([{ "role": "user", "content": "Hi" }]);
    let answer = post(
        launch,
        "/v1/messages",
        Some(&token),
        &claude_code("claude-opus-5-5", true, &asked),
    )
    .await;
    assert_eq!(answer.status, 200);
    let events = answer.events();
    let (name, data) = events.last().unwrap();
    assert_eq!(name, "error");
    assert_eq!(data["error"]["type"], "api_error");
    assert!(events.iter().all(|(name, _)| name != "message_stop"));
    let body = &fake.completions()[0].body;
    assert_eq!(
        body["model"], "local-big",
        "a Claude model's name is the saved model"
    );
    assert_eq!(body["max_completion_tokens"], 32000);
    assert_eq!(body.get("max_tokens"), None);

    // A context too long reads as Anthropic's, so the CLI compacts.
    fake.answers(Scripted::Refuse(
        400,
        json!({ "error": { "message": "This model's maximum context length is 128000 tokens.",
            "type": "invalid_request_error", "code": "context_length_exceeded" } }),
    ));
    let long = post(
        launch,
        "/v1/messages",
        Some(&token),
        &claude_code("local-big", false, &asked),
    )
    .await;
    assert_eq!(long.status, 400);
    assert_eq!(long.json()["error"]["type"], "invalid_request_error");
    assert!(
        long.json()["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("prompt is too long: OpenAI answered 400")
    );
}
