//! `GET`/`PUT`/`DELETE /api/agent/provider` and `POST /api/agent/provider/test`: the model
//! provider the agent runs on, checked against a stand-in on loopback that answers as each
//! provider's API does (Anthropic's and `OpenAI`-style model lists, `OpenRouter`'s key route,
//! Ollama's version and tags), then handed to every launch of the scripted CLI in its
//! environment; and the setup checklist's `provider` item. No test reaches a real provider.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure fails the test that called them"
)]

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use emberhut_api::agent_provider::{AgentProvider, ProviderKind, ProviderTest};
use emberhut_api::paths;
use emberhut_api::setup::{SetupItemId, SetupPage, SetupState};
use emberhut_core::RoomId;
use emberhut_core::session::Session;
use emberhut_db::Database;
use emberhut_server::app::{Dependencies, WorkerSetup};
use emberhut_server::services::agent_provider::{HttpProviderApi, OLLAMA_TOKEN, ProviderStore};
use emberhut_server::services::claude::judge::CliJudge;
use emberhut_server::services::claude::worker::{WorkerConfig, WorkerLaunch};
use emberhut_server::services::claude::{ClaudeAssistant, Launch};
use emberhut_server::services::guard::Judge as _;
use emberhut_server::services::messaging::secrets::Secrets;
use emberhut_server::services::messaging::web::WebClient;
use emberhut_server::services::talk::Assistant;
use emberhut_server::services::workers::WorkerRuntime;
use serde_json::{Value, json};

use crate::common::{Answer, Call, TestApp, http_client};
use crate::fake_cli::{FakeCli, config, error_result, init, is_user, result};

const PROVIDER: &str = paths::AGENT_PROVIDER;
const TEST: &str = paths::AGENT_PROVIDER_TEST;

/// An Anthropic key the stand-in accepts.
const ANTHROPIC_KEY: &str = "sk-ant-api03-good-key-0123456789abcdefWXYZ";
/// Another provider's key it accepts, as a bearer token.
pub(crate) const TOKEN: &str = "sk-or-v1-good-token-0123456789abcdefJKLM";
/// A key of another provider's shape, which Anthropic's form refuses.
const OPENAI_SHAPED: &str = "sk-proj-not-an-anthropic-one";
/// A key it refuses, as a provider refuses a revoked one.
const REVOKED: &str = "sk-ant-api03-revoked-0123456789abcdefQRST";

/// What the stand-in saw of one request: its path and query, `x-api-key`, the bearer and
/// `anthropic-version`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    path: String,
    api_key: Option<String>,
    bearer: Option<String>,
    version: Option<String>,
}

/// A stand-in for the providers' APIs. A model list for an accepted key, at `/v1/models` and
/// `/models`; `OpenRouter`'s `/v1/key`; Ollama's `/api/version` and `/api/tags`, open to all;
/// 401 for any other key, or the status it is told to answer with.
#[derive(Clone)]
struct Stand {
    seen: Arc<Mutex<Vec<Seen>>>,
    answer: Arc<Mutex<Option<u16>>>,
    ollama: Arc<Mutex<String>>,
}

impl Default for Stand {
    fn default() -> Self {
        Self {
            seen: Arc::default(),
            answer: Arc::default(),
            ollama: Arc::new(Mutex::new("0.14.1".to_owned())),
        }
    }
}

impl Stand {
    /// Serves it on loopback; its base URL.
    async fn serve(&self) -> String {
        let app = Router::new().fallback(stand_in).with_state(self.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}")
    }

    /// From now on, every request is answered with `status`.
    fn answers(&self, status: u16) {
        *self.answer.lock().unwrap() = Some(status);
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn paths(&self) -> Vec<String> {
        self.seen().into_iter().map(|seen| seen.path).collect()
    }
}

async fn stand_in(State(stand): State<Stand>, request: Request) -> Response {
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let seen = Seen {
        path: request
            .uri()
            .path_and_query()
            .map_or_else(String::new, ToString::to_string),
        api_key: header("x-api-key"),
        bearer: header("authorization")
            .and_then(|value| value.strip_prefix("Bearer ").map(str::to_owned)),
        version: header("anthropic-version"),
    };
    stand.seen.lock().unwrap().push(seen.clone());
    if let Some(status) = *stand.answer.lock().unwrap() {
        let status = StatusCode::from_u16(status).unwrap();
        return (status, axum::Json(json!({ "type": "error" }))).into_response();
    }
    match request.uri().path() {
        "/api/version" => {
            let version = stand.ollama.lock().unwrap().clone();
            return axum::Json(json!({ "version": version })).into_response();
        }
        "/api/tags" => {
            return axum::Json(json!({ "models": [
                { "name": "qwen3-coder:latest", "size": 1 },
                { "name": "llama3.2:latest", "size": 2 },
            ] }))
            .into_response();
        }
        _ => {}
    }
    let accepted = [seen.api_key, seen.bearer]
        .into_iter()
        .flatten()
        .any(|key| key == ANTHROPIC_KEY || key == TOKEN);
    if !accepted {
        let refused = json!({ "type": "error",
            "error": { "type": "authentication_error", "message": "invalid key" } });
        return (StatusCode::UNAUTHORIZED, axum::Json(refused)).into_response();
    }
    match request.uri().path() {
        "/v1/models" | "/models" => axum::Json(json!({ "data": [
            { "id": "model-a", "display_name": "Model A", "name": "Model A" },
            { "id": "model-b" },
        ], "has_more": false }))
        .into_response(),
        "/v1/key" => axum::Json(json!({ "data": { "label": "sk-or-v1-…" } })).into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A CLI that answers every message with `Echo: <text>`; its session is `s-1`, or the one it
/// resumes.
pub(crate) fn echoing() -> Arc<FakeCli> {
    FakeCli::new(|launch, line| {
        if !is_user(line) {
            return Vec::new();
        }
        let session = launch.flag("--resume").unwrap_or("s-1").to_owned();
        let said = line["message"]["content"].as_str().unwrap_or_default();
        vec![init(&session), result(&format!("Echo: {said}"))]
    })
}

/// Where `app` keeps the provider, as the runtime reads it.
pub(crate) fn store_of(dir: &std::path::Path, db: &Database) -> ProviderStore {
    ProviderStore::new(Secrets::new(dir.join("secrets")), db.settings())
}

/// An app whose agent runtime runs on `cli` with the accounts `accounts` (`ID=DIR`) and the
/// saved provider, its workers configured with the same accounts, and its checks sent to
/// `stand` (Anthropic's API there too).
async fn app_on(stand: &str, cli: &Arc<FakeCli>, accounts: &[(&str, &str)]) -> TestApp {
    let settings = config(accounts);
    let worker = WorkerConfig {
        accounts: settings.accounts.clone(),
        ..crate::workers::worker_config()
    };
    let cli = Arc::clone(cli);
    let stand = stand.to_owned();
    TestApp::with_agent_and(
        move |db, clock, dir| {
            Arc::new(
                ClaudeAssistant::new(settings, cli, db.settings(), db.rooms(), clock)
                    .with_provider(store_of(dir, db)),
            ) as Arc<dyn Assistant>
        },
        move |dependencies, _| Dependencies {
            provider_api: Arc::new(HttpProviderApi::new(WebClient::new(http_client()), &stand)),
            worker_setup: WorkerSetup {
                runtime: Some(WorkerRuntime {
                    spawner: FakeCli::new(|_, _| Vec::new()),
                    config: worker,
                }),
                ..WorkerSetup::default()
            },
            ..dependencies
        },
    )
    .await
}

/// `PUT`s `body` as the agent's provider.
pub(crate) async fn put(app: &TestApp, session: &Session, body: &Value) -> Answer {
    app.call(Call::put(PROVIDER).token(session).json(body))
        .await
}

/// `POST`s `body` to the test route.
pub(crate) async fn test(app: &TestApp, session: &Session, body: &Value) -> Answer {
    app.call(Call::post(TEST).token(session).json(body)).await
}

/// What is saved now.
pub(crate) async fn saved(app: &TestApp, session: &Session) -> AgentProvider {
    let answer = app.call(Call::get(PROVIDER).token(session)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    answer.read_as()
}

/// Says `text` in Main and waits for the answer.
pub(crate) async fn talk(app: &TestApp, session: &Session, text: &str) {
    let said = app
        .call(
            Call::post(paths::TALK)
                .token(session)
                .json(&json!({ "text": text })),
        )
        .await;
    assert_eq!(said.status, 200, "{}", said.text);
    app.settle().await;
}

/// The key file of `provider` in `app`'s secrets folder.
fn key_file(app: &TestApp, provider: &str) -> std::path::PathBuf {
    app.dir()
        .join("secrets")
        .join(format!("agent-{provider}-key"))
}

/// The setup checklist's `provider` item.
pub(crate) async fn provider_item(app: &TestApp, session: &Session) -> (SetupState, String) {
    let page: SetupPage = app
        .call(Call::get(paths::SETUP).token(session))
        .await
        .read_as();
    let item = page.item(&SetupItemId::Provider).unwrap();
    (item.state.clone(), item.detail.clone().unwrap_or_default())
}

/// Every variable the launch keeps from the server is under `ANTHROPIC_`, and no key is among
/// its arguments.
fn assert_stripped_and_off_the_command_line(launch: &Launch, key: &str) {
    for inherited in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_CUSTOM_HEADERS",
        "ANTHROPIC_MODEL",
        "CLAUDE_CODE_USE_BEDROCK",
    ] {
        assert!(launch.strips(inherited), "{inherited} is kept out");
    }
    assert!(launch.args.iter().all(|arg| !arg.contains(key)));
}

#[tokio::test]
pub(crate) async fn an_anthropic_key_is_checked_saved_masked_and_used_by_the_next_session() {
    let stand = Stand::default();
    let cli = echoing();
    let app = app_on(&stand.serve().await, &cli, &[]).await;
    let session = app.sign_in().await;

    assert_eq!(saved(&app, &session).await, AgentProvider::NONE);
    assert_eq!(
        provider_item(&app, &session).await.0,
        SetupState::Missing,
        "the owner can fix it from the apps"
    );
    talk(&app, &session, "Before").await;
    let before = &cli.launches()[0];
    assert_eq!(before.var("ANTHROPIC_API_KEY"), None);
    assert!(
        !before.strips("ANTHROPIC_API_KEY"),
        "nothing saved: the server's own environment is left as it is"
    );

    let answer = put(
        &app,
        &session,
        &json!({ "provider": "anthropic", "apiKey": format!("  {ANTHROPIC_KEY}\n") }),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let checked = app
        .clock_now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    assert_eq!(
        answer.json(),
        json!({ "provider": "anthropic", "keySet": true, "keyHint": "WXYZ",
            "checkedAt": checked, "usable": true, "inUse": true })
    );
    assert!(!answer.text.contains(ANTHROPIC_KEY));
    assert_eq!(
        stand.seen(),
        [Seen {
            path: "/v1/models?limit=1000".to_owned(),
            api_key: Some(ANTHROPIC_KEY.to_owned()),
            bearer: None,
            version: Some("2023-06-01".to_owned()),
        }],
        "one authenticated request, the key trimmed"
    );
    let file = key_file(&app, "anthropic");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), ANTHROPIC_KEY);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the server's user's only");
    }
    assert_eq!(saved(&app, &session).await, answer.read_as());
    let (state, detail) = provider_item(&app, &session).await;
    assert_eq!(state, SetupState::Ready);
    assert_eq!(detail, "The agent runs on the saved Anthropic API key.");

    // The session started without it is restarted with it, on the same conversation.
    talk(&app, &session, "After").await;
    let launches = cli.launches();
    assert_eq!(launches.len(), 2);
    let launch = &launches[1];
    assert_eq!(launch.var("ANTHROPIC_API_KEY"), Some(ANTHROPIC_KEY));
    assert_eq!(launch.flag("--resume"), Some("s-1"));
    assert_eq!(
        launch.flag("--model"),
        Some("a-model"),
        "the configured model"
    );
    assert_eq!(launch.var("ANTHROPIC_BASE_URL"), None);
    assert_stripped_and_off_the_command_line(launch, ANTHROPIC_KEY);
    // The same provider keeps the session.
    talk(&app, &session, "Again").await;
    assert_eq!(cli.launches().len(), 2);
}

#[tokio::test]
pub(crate) async fn each_anthropic_compatible_provider_reaches_the_cli_by_its_environment() {
    let stand = Stand::default();
    let base = stand.serve().await;
    let cli = echoing();
    let app = app_on(&base, &cli, &[]).await;
    let session = app.sign_in().await;

    // (what is sent, the base URL and token the CLI gets, its model and quick model)
    let cases = [
        (
            json!({ "provider": "deepseek", "apiKey": TOKEN, "baseUrl": base }),
            format!("{base}/anthropic"),
            TOKEN,
            "deepseek-v4-pro",
            "deepseek-v4-flash",
            "/models",
        ),
        (
            json!({ "provider": "openrouter", "apiKey": TOKEN, "baseUrl": base,
                "model": "z-ai/glm-5", "fastModel": "z-ai/glm-5-air" }),
            base.clone(),
            TOKEN,
            "z-ai/glm-5",
            "z-ai/glm-5-air",
            "/v1/models",
        ),
        (
            json!({ "provider": "ollama", "baseUrl": base, "model": "qwen3-coder:latest" }),
            base.clone(),
            OLLAMA_TOKEN,
            "qwen3-coder:latest",
            "qwen3-coder:latest",
            "/api/tags",
        ),
        (
            json!({ "provider": "anthropic-compatible", "apiKey": TOKEN,
                "baseUrl": format!("{base}/"), "model": "model-a" }),
            base.clone(),
            TOKEN,
            "model-a",
            "model-a",
            "/v1/models?limit=1000",
        ),
    ];
    for (round, (body, url, token, model, fast, listed)) in cases.into_iter().enumerate() {
        let answer = put(&app, &session, &body).await;
        assert_eq!(answer.status, 200, "{body}: {}", answer.text);
        let now: AgentProvider = answer.read_as();
        assert!(now.usable && now.in_use, "{body}");
        assert_eq!(stand.paths().last().map(String::as_str), Some(listed));
        assert_eq!(provider_item(&app, &session).await.0, SetupState::Ready);

        talk(&app, &session, &format!("Round {round}")).await;
        let launches = cli.launches();
        assert_eq!(
            launches.len(),
            round + 1,
            "a new provider restarts the session"
        );
        let launch = &launches[round];
        assert_eq!(
            launch.var("ANTHROPIC_BASE_URL"),
            Some(url.as_str()),
            "{body}"
        );
        assert_eq!(launch.var("ANTHROPIC_AUTH_TOKEN"), Some(token), "{body}");
        for name in [
            "ANTHROPIC_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
        ] {
            assert_eq!(launch.var(name), Some(model), "{name} for {body}");
        }
        for name in [
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL",
        ] {
            assert_eq!(launch.var(name), Some(fast), "{name} for {body}");
        }
        assert_eq!(
            launch.flag("--model"),
            None,
            "the configured model isn't sent elsewhere"
        );
        let openrouter = body["provider"] == "openrouter";
        assert_eq!(
            launch.var("ANTHROPIC_API_KEY"),
            openrouter.then_some(""),
            "{body}"
        );
        assert_stripped_and_off_the_command_line(launch, TOKEN);
        assert!(launch.strips("CLAUDE_CODE_OAUTH_TOKEN"));
    }
    // One provider is saved at a time: the earlier ones' keys are gone.
    assert!(!key_file(&app, "deepseek").exists());
    assert!(!key_file(&app, "openrouter").exists());
    assert!(key_file(&app, "anthropic-compatible").exists());
}

#[tokio::test]
async fn workers_and_the_guard_s_judge_run_on_the_saved_provider_too() {
    let stand = Stand::default();
    let base = stand.serve().await;
    let app = app_on(&base, &echoing(), &[]).await;
    let session = app.sign_in().await;
    let body = json!({ "provider": "openrouter", "apiKey": TOKEN, "baseUrl": base,
        "model": "z-ai/glm-5" });
    assert_eq!(put(&app, &session, &body).await.status, 200);
    let db = Database::connect(&app.url).await.unwrap();
    let store = store_of(app.dir(), &db);

    let worker = WorkerConfig {
        provider: Some(store.clone()),
        ..crate::workers::worker_config()
    };
    let provider = worker.session_provider().await.unwrap();
    let run = WorkerLaunch {
        brief: "Do it".to_owned(),
        resume: None,
        ultra: true,
        deny_outward: true,
        model: Some("claude-haiku-5-5".to_owned()),
        folder: app.dir().join("task-1"),
        account: None,
        agent: None,
    };
    let launch = worker.launch(&run, "worker-token", provider.as_ref());
    assert_eq!(launch.var("ANTHROPIC_BASE_URL"), Some(base.as_str()));
    assert_eq!(launch.var("ANTHROPIC_AUTH_TOKEN"), Some(TOKEN));
    assert_eq!(launch.var("ANTHROPIC_MODEL"), Some("z-ai/glm-5"));
    assert_eq!(
        launch.flag("--model"),
        None,
        "a task's Claude model isn't sent elsewhere"
    );
    assert_eq!(launch.var("EMBERHUT_MCP_TOKEN"), Some("worker-token"));
    assert_stripped_and_off_the_command_line(&launch, TOKEN);
    // A worker on a configured account runs on that account's login.
    let on_account = WorkerConfig {
        accounts: config(&[("work", "/cfg/work")]).accounts,
        ..worker
    };
    assert_eq!(on_account.session_provider().await.unwrap(), None);

    let judging = FakeCli::new(|_, line| {
        if is_user(line) {
            vec![result("fine")]
        } else {
            Vec::new()
        }
    });
    let judge = CliJudge::new(
        Arc::clone(&judging) as _,
        "agent-cli".into(),
        Some("a-model".to_owned()),
        None,
    )
    .with_provider(store);
    assert_eq!(
        judge.judge("Rules.", "Is it fine?").await.as_deref(),
        Some("fine")
    );
    let launch = &judging.launches()[0];
    assert_eq!(launch.var("ANTHROPIC_BASE_URL"), Some(base.as_str()));
    assert_eq!(
        launch.var("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
        Some("z-ai/glm-5")
    );
    assert_eq!(launch.flag("--model"), None);
    assert_stripped_and_off_the_command_line(launch, TOKEN);
}

#[tokio::test]
async fn another_endpoint_s_error_fails_the_turn_and_is_not_read_as_a_usage_limit() {
    let stand = Stand::default();
    let base = stand.serve().await;
    let cli = FakeCli::new(|_, line| {
        if !is_user(line) {
            return Vec::new();
        }
        vec![
            init("s-1"),
            error_result("API Error: 429 rate limit reached, try again in 5 minutes"),
        ]
    });
    let app = app_on(&base, &cli, &[]).await;
    let session = app.sign_in().await;
    let body = json!({ "provider": "deepseek", "apiKey": TOKEN, "baseUrl": base });
    assert_eq!(put(&app, &session, &body).await.status, 200);
    talk(&app, &session, "Hello").await;
    let chat: Vec<(bool, String)> = app
        .state
        .state()
        .state(RoomId::MAIN)
        .await
        .unwrap()
        .chat
        .into_iter()
        .map(|line| (line.event, line.text))
        .collect();
    assert_eq!(
        chat,
        [
            (false, "Hello".to_owned()),
            (
                true,
                "Something went wrong while answering. Try again.".to_owned()
            ),
        ],
        "a failed turn, not the error as the answer"
    );
}

#[tokio::test]
pub(crate) async fn the_test_route_lists_models_and_saves_nothing() {
    let stand = Stand::default();
    let base = stand.serve().await;
    let app = app_on(&base, &echoing(), &[]).await;
    let session = app.sign_in().await;

    let ollama = test(
        &app,
        &session,
        &json!({ "provider": "ollama", "baseUrl": base }),
    )
    .await;
    assert_eq!(ollama.status, 200, "{}", ollama.text);
    assert_eq!(
        ollama.json(),
        json!({ "checked": true, "models": [
            { "id": "qwen3-coder:latest" }, { "id": "llama3.2:latest" },
        ] })
    );
    assert_eq!(stand.paths(), ["/api/version", "/api/tags"]);

    let router: ProviderTest = test(
        &app,
        &session,
        &json!({ "provider": "openrouter", "apiKey": TOKEN, "baseUrl": base }),
    )
    .await
    .read_as();
    assert_eq!(router.models[0].name.as_deref(), Some("Model A"));
    assert_eq!(&stand.paths()[2..], ["/v1/key", "/v1/models"]);
    let key_check = &stand.seen()[2];
    assert_eq!(key_check.bearer.as_deref(), Some(TOKEN));

    let own: ProviderTest = test(&app, &session, &json!({ "provider": "claude" }))
        .await
        .read_as();
    assert_eq!(
        own,
        ProviderTest {
            checked: false,
            models: Vec::new()
        },
        "only the CLI can check its own login"
    );
    assert_eq!(stand.seen().len(), 4);
    assert_eq!(saved(&app, &session).await, AgentProvider::NONE);
    assert!(!key_file(&app, "openrouter").exists());
}

#[tokio::test]
pub(crate) async fn a_refused_key_or_an_old_ollama_is_not_saved_and_what_was_stays() {
    let stand = Stand::default();
    let base = stand.serve().await;
    let app = app_on(&base, &echoing(), &[]).await;
    let session = app.sign_in().await;

    let refused = put(
        &app,
        &session,
        &json!({ "provider": "anthropic", "apiKey": REVOKED }),
    )
    .await;
    assert_eq!(refused.status, 422, "{}", refused.text);
    assert!(
        refused.json()["error"]
            .as_str()
            .unwrap()
            .starts_with("Anthropic didn't accept that key."),
        "{}",
        refused.text
    );
    assert!(!refused.text.contains(REVOKED));
    assert_eq!(saved(&app, &session).await, AgentProvider::NONE);

    let good = json!({ "provider": "anthropic", "apiKey": ANTHROPIC_KEY });
    assert_eq!(put(&app, &session, &good).await.status, 200);
    let router = json!({ "provider": "openrouter", "apiKey": REVOKED,
        "baseUrl": base, "model": "z-ai/glm-5" });
    assert_eq!(put(&app, &session, &router).await.status, 422);
    assert_eq!(test(&app, &session, &router).await.status, 422);

    *stand.ollama.lock().unwrap() = "0.13.5".to_owned();
    let old = json!({ "provider": "ollama", "baseUrl": base, "model": "qwen3-coder:latest" });
    let too_old = put(&app, &session, &old).await;
    assert_eq!(too_old.status, 422);
    assert_eq!(
        too_old.json()["error"],
        json!(
            "Ollama 0.13.5 can't run the agent: it needs 0.14 or later. Update it, then try again."
        )
    );
    let kept = saved(&app, &session).await;
    assert_eq!(kept.provider, Some(ProviderKind::Anthropic));
    assert_eq!(kept.key_hint.as_deref(), Some("WXYZ"));
}

#[tokio::test]
pub(crate) async fn a_provider_that_does_not_answer_saves_nothing() {
    // Nothing listens at a port just let go of.
    let closed = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let app = app_on(&closed, &echoing(), &[]).await;
    let session = app.sign_in().await;
    let gone = put(
        &app,
        &session,
        &json!({ "provider": "ollama", "baseUrl": closed, "model": "qwen3-coder" }),
    )
    .await;
    assert_eq!(gone.status, 502, "{}", gone.text);
    assert_eq!(
        gone.json(),
        json!({ "error": "Ollama couldn't be reached to check it, so nothing was saved. Check \
            the address, or try again in a moment." })
    );
    let anthropic = json!({ "provider": "anthropic", "apiKey": ANTHROPIC_KEY });
    assert_eq!(test(&app, &session, &anthropic).await.status, 502);

    // Answering, but overloaded: not a refusal of the key.
    let stand = Stand::default();
    stand.answers(529);
    let app = app_on(&stand.serve().await, &echoing(), &[]).await;
    let session = app.sign_in().await;
    assert_eq!(put(&app, &session, &anthropic).await.status, 502);
    assert_eq!(saved(&app, &session).await, AgentProvider::NONE);
}

#[tokio::test]
pub(crate) async fn what_a_provider_does_not_take_is_refused_before_it_is_asked() {
    let stand = Stand::default();
    let app = app_on(&stand.serve().await, &echoing(), &[]).await;
    let session = app.sign_in().await;
    for (body, said) in [
        (
            json!({ "provider": "anthropic", "apiKey": OPENAI_SHAPED }),
            "That isn't an Anthropic API key.",
        ),
        (
            json!({ "provider": "anthropic" }),
            "Anthropic needs an API key.",
        ),
        (
            json!({ "provider": "claude", "apiKey": ANTHROPIC_KEY }),
            "Claude takes no API key.",
        ),
        (
            json!({ "provider": "ollama", "baseUrl": "file:///etc", "model": "m" }),
            "That isn't a base URL",
        ),
        (
            json!({ "provider": "gemini" }),
            "gemini isn't a provider this server knows.",
        ),
    ] {
        for answer in [
            put(&app, &session, &body).await,
            test(&app, &session, &body).await,
        ] {
            assert_eq!(answer.status, 400, "{body}");
            assert!(
                answer.json()["error"].as_str().unwrap().starts_with(said),
                "{body}: {}",
                answer.text
            );
        }
    }
    // A test lists the models to choose from; what is saved needs one chosen.
    let unchosen =
        json!({ "provider": "openrouter", "apiKey": TOKEN, "baseUrl": "http://x.invalid" });
    let answer = put(&app, &session, &unchosen).await;
    assert_eq!(answer.status, 400);
    assert_eq!(
        answer.json()["error"],
        json!("OpenRouter needs a model: choose one of its models.")
    );
    let shapeless = put(&app, &session, &json!({ "apiKey": ANTHROPIC_KEY })).await;
    assert_eq!(shapeless.status, 400);
    assert_eq!(stand.seen(), [], "no provider was asked");
    assert_eq!(saved(&app, &session).await, AgentProvider::NONE);
}

#[tokio::test]
pub(crate) async fn only_a_signed_in_device_reads_tests_or_changes_the_provider() {
    let stand = Stand::default();
    let app = app_on(&stand.serve().await, &echoing(), &[]).await;
    let body = json!({ "provider": "anthropic", "apiKey": ANTHROPIC_KEY });
    for call in [
        Call::get(PROVIDER),
        Call::put(PROVIDER).json(&body),
        Call::delete(PROVIDER),
        Call::post(TEST).json(&body),
    ] {
        let answer = app.call(call).await;
        assert_eq!(answer.status, 401);
        assert_eq!(answer.json()["signIn"], json!(true));
    }
    assert_eq!(stand.seen(), []);
}

#[tokio::test]
pub(crate) async fn a_removed_provider_is_gone_and_no_longer_used() {
    let stand = Stand::default();
    let base = stand.serve().await;
    let cli = echoing();
    let app = app_on(&base, &cli, &[]).await;
    let session = app.sign_in().await;
    let body = json!({ "provider": "deepseek", "apiKey": TOKEN, "baseUrl": base });
    assert_eq!(put(&app, &session, &body).await.status, 200);
    talk(&app, &session, "With it").await;
    assert_eq!(cli.launches()[0].var("ANTHROPIC_AUTH_TOKEN"), Some(TOKEN));

    let removed = app.call(Call::delete(PROVIDER).token(&session)).await;
    assert_eq!(removed.status, 200, "{}", removed.text);
    assert_eq!(removed.read_as::<AgentProvider>(), AgentProvider::NONE);
    assert!(!key_file(&app, "deepseek").exists());
    let again = app.call(Call::delete(PROVIDER).token(&session)).await;
    assert_eq!(again.status, 200, "removing nothing is no error");
    assert_eq!(
        provider_item(&app, &session).await.0,
        SetupState::Missing,
        "what the turns showed went with it"
    );

    talk(&app, &session, "Without it").await;
    let launches = cli.launches();
    assert_eq!(launches.len(), 2, "restarted without it");
    assert_eq!(launches[1].var("ANTHROPIC_AUTH_TOKEN"), None);
    assert_eq!(launches[1].flag("--model"), Some("a-model"));
    assert_eq!(
        provider_item(&app, &session).await,
        (
            SetupState::Ready,
            "The agent runs on the agent CLI's own login on the server.".to_owned()
        ),
        "the CLI's own login answered"
    );
}

#[tokio::test]
async fn without_the_bridge_an_openai_provider_is_saved_and_checked_but_not_run_on() {
    let stand = Stand::default();
    let base = stand.serve().await;
    let cli = echoing();
    let app = app_on(&base, &cli, &[]).await;
    let session = app.sign_in().await;
    let without_model = json!({ "provider": "openai", "apiKey": TOKEN,
        "baseUrl": format!("{base}/v1") });
    let listed: ProviderTest = test(&app, &session, &without_model).await.read_as();
    assert_eq!(
        listed.models.len(),
        2,
        "a test lists the models to choose from"
    );
    let refused = put(&app, &session, &without_model).await;
    assert_eq!(refused.status, 400);
    assert_eq!(
        refused.json()["error"],
        "OpenAI needs a model: choose one of its models."
    );
    for (body, name) in [
        (
            json!({ "provider": "openai", "apiKey": TOKEN, "baseUrl": format!("{base}/v1"),
                "model": "model-a" }),
            "OpenAI",
        ),
        (
            json!({ "provider": "openai-compatible", "baseUrl": base, "apiKey": TOKEN,
                "model": "model-a" }),
            "The OpenAI-compatible endpoint",
        ),
    ] {
        let now: AgentProvider = put(&app, &session, &body).await.read_as();
        assert!(now.key_set && now.checked_at.is_some());
        assert!(now.usable && !now.in_use, "{body}");
        assert_eq!(
            provider_item(&app, &session).await,
            (
                SetupState::Off,
                format!(
                    "{name} is saved, but the OpenAI bridge it runs through isn't running on \
                     this server."
                )
            )
        );
    }
    talk(&app, &session, "Hello").await;
    let launch = &cli.launches()[0];
    assert_eq!(
        launch.var("ANTHROPIC_AUTH_TOKEN"),
        None,
        "nothing of it reaches the CLI"
    );
    assert_eq!(launch.var("ANTHROPIC_BASE_URL"), None);
}

#[tokio::test]
async fn configured_accounts_keep_their_logins_over_a_saved_provider() {
    let stand = Stand::default();
    let cli = echoing();
    let app = app_on(&stand.serve().await, &cli, &[("work", "/cfg/work")]).await;
    let session = app.sign_in().await;
    let (state, detail) = provider_item(&app, &session).await;
    assert_eq!(state, SetupState::Ready);
    assert_eq!(
        detail,
        "The agent runs on 1 configured account (EMBERHUT_AGENT_ACCOUNTS)."
    );

    let body = json!({ "provider": "anthropic", "apiKey": ANTHROPIC_KEY });
    let now: AgentProvider = put(&app, &session, &body).await.read_as();
    assert!(now.key_set && now.usable);
    assert!(!now.in_use, "the accounts are the credentials");
    talk(&app, &session, "Hello").await;
    let launch = &cli.launches()[0];
    assert_eq!(launch.var("CLAUDE_CONFIG_DIR"), Some("/cfg/work"));
    assert_eq!(launch.var("ANTHROPIC_API_KEY"), None);
    let (_, detail) = provider_item(&app, &session).await;
    assert!(
        detail.ends_with("The provider saved from the apps (anthropic) isn't used while they are.")
    );
}

#[tokio::test]
async fn without_an_agent_cli_there_is_nothing_to_connect() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    assert_eq!(provider_item(&app, &session).await.0, SetupState::Off);
    let own = put(&app, &session, &json!({ "provider": "claude" })).await;
    assert_eq!(own.status, 200, "{}", own.text);
    let now: AgentProvider = own.read_as();
    assert_eq!(now.provider, Some(ProviderKind::Claude));
    assert!(now.usable && !now.in_use, "no agent CLI runs");
    assert_eq!(now.checked_at, None, "only the CLI can check its own login");
}

#[tokio::test]
async fn an_anthropic_key_put_in_the_secrets_folder_by_hand_is_used_but_not_reported_checked() {
    let stand = Stand::default();
    let cli = echoing();
    let app = app_on(&stand.serve().await, &cli, &[]).await;
    let session = app.sign_in().await;
    let body = json!({ "provider": "anthropic", "apiKey": ANTHROPIC_KEY });
    assert_eq!(put(&app, &session, &body).await.status, 200);
    let other = "sk-ant-api03-by-hand-0123456789abcdefMNOP";
    std::fs::write(key_file(&app, "anthropic"), other).unwrap();
    let now = saved(&app, &session).await;
    assert_eq!(now.key_hint.as_deref(), Some("MNOP"));
    assert_eq!(now.checked_at, None, "the check was for another key");
    talk(&app, &session, "Hello").await;
    assert_eq!(cli.launches()[0].var("ANTHROPIC_API_KEY"), Some(other));
}

/// Log lines written while a test runs, kept in memory.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn no_key_reaches_the_logs() {
    let logs = Captured::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    // The test runs on one thread, so the subscriber sees every request's lines.
    let _logging = tracing::subscriber::set_default(subscriber);

    let stand = Stand::default();
    let base = stand.serve().await;
    let cli = echoing();
    let app = app_on(&base, &cli, &[]).await;
    let session = app.sign_in().await;
    let refused = json!({ "provider": "anthropic", "apiKey": REVOKED });
    assert_eq!(put(&app, &session, &refused).await.status, 422);
    let router = json!({ "provider": "openrouter", "apiKey": TOKEN, "baseUrl": base,
        "model": "z-ai/glm-5" });
    assert_eq!(put(&app, &session, &router).await.status, 200);
    talk(&app, &session, "Hello").await;
    assert_eq!(cli.launches()[0].var("ANTHROPIC_AUTH_TOKEN"), Some(TOKEN));
    let anthropic = json!({ "provider": "anthropic", "apiKey": ANTHROPIC_KEY });
    assert_eq!(put(&app, &session, &anthropic).await.status, 200);
    talk(&app, &session, "Again").await;
    assert_eq!(
        app.call(Call::delete(PROVIDER).token(&session))
            .await
            .status,
        200
    );
    stand.answers(500);
    assert_eq!(put(&app, &session, &anthropic).await.status, 502);

    // Only what is missing is checked: whether a line is there at all depends on which other
    // tests' threads registered its callsite first.
    let written = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    for secret in [ANTHROPIC_KEY, TOKEN, REVOKED, "WXYZ", "JKLM", "QRST"] {
        assert!(!written.contains(secret), "{secret} in {written}");
    }
}
