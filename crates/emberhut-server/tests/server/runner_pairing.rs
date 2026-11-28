//! Runners paired at run time: the code, the pairing and its refusals, the list, forgetting a
//! runner, and a paired runner reaching the server and being reached like a configured one.
#![expect(
    clippy::unwrap_used,
    clippy::too_many_lines,
    reason = "a stand-in runner in a helper, and a test that walks one pairing from code to calls"
)]

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Request, State};
use emberhut_api::error::{ErrorBody, OkReply};
use emberhut_api::runner::pairing::{PairedRunner, RunnerCode, RunnerSource, RunnersPage};
use emberhut_db::Database;
use emberhut_server::app::{AppState, Dependencies};
use emberhut_server::config::AuthMode;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::common::{Answer, Call, TestApp};

/// The configured runner's token.
const BUILD: &str = "token-of-build";

/// An app whose configuration names the runner `build`.
async fn app() -> TestApp {
    let app = TestApp::new().await;
    app.runners.on(&[("build", "Build box", BUILD)]);
    app
}

/// A new runner code, made on a signed-in device.
async fn code(app: &TestApp) -> String {
    let session = app.sign_in().await;
    let answer = app
        .call(Call::post("/api/runners/codes").token(&session))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    answer.read_as::<RunnerCode>().code
}

/// The runner `name` at `url` pairs with `code`.
async fn pair(app: &TestApp, code: &str, name: &str, url: &str) -> Answer {
    app.call(Call::post("/api/runner/pair").json(&json!({
        "code": code, "name": name, "url": url,
        "place": "Laptop", "platform": "linux", "version": "1.4.0",
    })))
    .await
}

/// The token a pairing answered.
fn token_of(answer: &Answer) -> String {
    assert_eq!(answer.status, 200, "{}", answer.text);
    answer.read_as::<PairedRunner>().token
}

/// An event of `machine`'s session `r-1a2b`, reported with `token`.
async fn report(app: &TestApp, token: &str, machine: &str) -> Answer {
    app.call(
        Call::post("/api/runner/events")
            .header("authorization", &format!("Bearer {token}"))
            .json(&json!({
                "machine": machine, "kind": "activity", "at": "2026-11-28T10:00:00.000Z",
                "session": { "id": "r-1a2b", "title": "Tidy", "status": "running" },
            })),
    )
    .await
}

/// The error a refusal says.
fn error_of(answer: &Answer) -> String {
    answer.read_as::<ErrorBody>().error
}

#[tokio::test]
pub(crate) async fn a_signed_in_device_makes_a_code_and_a_runner_pairs_with_it() {
    let app = app().await;
    let session = app.sign_in().await;
    let answer = app
        .call(Call::post("/api/runners/codes").token(&session))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let made: RunnerCode = answer.read_as();
    assert_eq!((made.code.len(), &made.code[4..5]), (9, "-"));
    assert!(
        made.expires_at
            .as_str()
            .parse::<jiff::Timestamp>()
            .is_ok_and(|at| at > app.clock_now()),
        "RFC 3339, ten minutes on: {}",
        made.expires_at
    );
    assert_eq!(
        (made.server.as_str(), made.reachable),
        ("http://127.0.0.1:18990", false),
        "no address another machine reaches: the server's own, said plainly"
    );
    assert_eq!(
        made.install.command,
        format!(
            "emberhut-runner install --server http://127.0.0.1:18990 --pair {}",
            made.code
        )
    );
    assert!(
        made.install
            .shell
            .starts_with("curl -fsSL https://github.com/emberhuthq/emberhut/releases/download/v")
            && made.install.shell.ends_with(&format!(
                "/install-runner.sh | sh -s -- --server http://127.0.0.1:18990 --pair {}",
                made.code
            )),
        "{}",
        made.install.shell
    );
    assert!(
        made.install.powershell.starts_with(&format!(
            "$env:EMBERHUT_SERVER=\"http://127.0.0.1:18990\"; $env:EMBERHUT_PAIR_CODE=\"{}\"; irm ",
            made.code
        )) && made
            .install
            .powershell
            .ends_with("/install-runner.ps1 | iex"),
        "{}",
        made.install.powershell
    );
    assert!(
        app.state
            .pairing()
            .redeem(&made.code, &crate::common::device("other"))
            .await
            .is_err(),
        "a runner's code doesn't sign a device in"
    );

    let answer = pair(
        &app,
        &made.code.to_lowercase().replace('-', " "),
        "laptop",
        "http://192.0.2.7:8800/",
    )
    .await;
    let paired: PairedRunner = answer.read_as();
    assert_eq!(paired.name, "laptop");
    assert_eq!(paired.token.len(), 43, "32 bytes in URL-safe base64");

    let listed: RunnersPage = app
        .call(Call::get("/api/runners").token(&session))
        .await
        .read_as();
    let names: Vec<&str> = listed.runners.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["build", "laptop"], "the configured runner first");
    let build = listed.runner("build").unwrap();
    assert_eq!(
        (
            build.source.clone(),
            build.place.as_str(),
            build.url.as_str()
        ),
        (RunnerSource::Flag, "Build box", "https://build.example.com")
    );
    assert!(build.created.is_none());
    let laptop = listed.runner("laptop").unwrap();
    assert_eq!(laptop.source, RunnerSource::Paired);
    assert_eq!(laptop.url, "http://192.0.2.7:8800", "the final / dropped");
    assert_eq!(laptop.place, "Laptop");
    assert_eq!(
        (laptop.platform.as_deref(), laptop.version.as_deref()),
        (Some("linux"), Some("1.4.0"))
    );
    assert!(laptop.created.is_some());
    let text = app
        .call(Call::get("/api/runners").token(&session))
        .await
        .text;
    assert!(
        !text.contains(&paired.token) && !text.contains(BUILD),
        "no token listed"
    );

    // The registry: the paired runner is a runner like the configured one.
    assert_eq!(app.state.runners().machines(), ["build", "laptop"]);
    assert_eq!(app.state.runners().place("laptop"), "Laptop");
    let answer = report(&app, &paired.token, "laptop").await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let answer = report(&app, &paired.token, "build").await;
    assert_eq!(
        answer.status, 400,
        "a paired runner reports only for itself"
    );
    assert_eq!(report(&app, BUILD, "build").await.status, 200);
    let sessions: Value = app
        .call(Call::get("/api/runner/sessions").token(&session))
        .await
        .json();
    let places: Vec<&str> = sessions["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|session| session["place"].as_str())
        .collect();
    assert!(
        places.contains(&"Laptop") && places.contains(&"Build box"),
        "{places:?}"
    );
}

#[tokio::test]
pub(crate) async fn making_a_code_and_forgetting_need_a_signed_in_device() {
    for mode in [AuthMode::Enforce, AuthMode::Report] {
        let app = TestApp::with_mode(mode).await;
        let answer = app.call(Call::post("/api/runners/codes")).await;
        assert_eq!(answer.status, 401, "{mode:?}: {}", answer.text);
        assert_eq!(answer.json()["signIn"], true);
        let answer = app.call(Call::delete("/api/runners/laptop")).await;
        assert_eq!(answer.status, 401, "{mode:?}: {}", answer.text);
    }
    let enforcing = TestApp::new().await;
    let answer = enforcing.call(Call::get("/api/runners")).await;
    assert_eq!(answer.status, 401, "reading passes the session gate");
    let reporting = TestApp::with_mode(AuthMode::Report).await;
    let answer = reporting.call(Call::get("/api/runners")).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(answer.read_as::<RunnersPage>().runners, []);
}

#[tokio::test]
pub(crate) async fn a_code_works_once_and_only_until_it_expires() {
    let app = app().await;
    let used = code(&app).await;
    token_of(&pair(&app, &used, "laptop", "https://laptop.example.com").await);
    let again = pair(&app, &used, "desk", "https://desk.example.com").await;
    assert_eq!(again.status, 401, "{}", again.text);
    assert_eq!(
        error_of(&again),
        "That code was already used. Make a new one."
    );

    let late = code(&app).await;
    app.advance_minutes(10);
    let answer = pair(&app, &late, "desk", "https://desk.example.com").await;
    assert_eq!(answer.status, 401);
    assert_eq!(error_of(&answer), "That code has expired. Make a new one.");

    let answer = pair(&app, "ZZZZ-ZZZZ", "desk", "https://desk.example.com").await;
    assert_eq!(answer.status, 401);
    let answer = pair(&app, " - ", "desk", "https://desk.example.com").await;
    assert_eq!(answer.status, 401);
    assert_eq!(error_of(&answer), "Type the pairing code.");
    let answer = app
        .call(Call::post("/api/runner/pair").json(&json!({ "name": "desk" })))
        .await;
    assert_eq!(answer.status, 401, "no code is no proof");
    let answer = pair(&app, &"A".repeat(40), "desk", "https://desk.example.com").await;
    assert_eq!(answer.status, 401, "longer than any code: a wrong code");
    let listed: Vec<String> = app.state.runners().machines();
    assert_eq!(listed, ["build", "laptop"]);
}

#[tokio::test]
pub(crate) async fn a_name_or_address_the_server_does_not_take_leaves_the_code_unused() {
    let app = app().await;
    let made = code(&app).await;
    for (name, url) in [
        ("build box", "https://laptop.example.com"),
        ("codes", "https://laptop.example.com"),
        ("laptop", "ftp://laptop.example.com"),
        ("laptop", "https://laptop.example.com/?a=1"),
    ] {
        let answer = pair(&app, &made, name, url).await;
        assert_eq!(answer.status, 400, "{name} {url}: {}", answer.text);
    }
    let answer = app
        .call(Call::post("/api/runner/pair").raw("not json"))
        .await;
    assert_eq!(answer.status, 400);

    let answer = app
        .call(Call::post("/api/runner/pair").json(&json!({
            "code": made, "name": "laptop", "url": "https://laptop.example.com",
        })))
        .await;
    assert_eq!(answer.status, 400, "the platform and version are asked for");

    token_of(&pair(&app, &made, "laptop", "https://laptop.example.com").await);
}

#[tokio::test]
pub(crate) async fn the_code_names_the_address_a_runner_reaches_the_server_at() {
    let app = TestApp::with_dependencies(AuthMode::Enforce, |deps, _| Dependencies {
        runner_routes: vec!["https://emberhut.example.com".to_owned()],
        ..deps
    })
    .await;
    let session = &app.sign_in().await;
    let ask = |call: Call| {
        let app = &app;
        async move {
            let answer = app.call(call.token(session)).await;
            assert_eq!(answer.status, 200, "{}", answer.text);
            answer.read_as::<RunnerCode>()
        }
    };

    let configured = ask(Call::post("/api/runners/codes").local("192.0.2.10:18990")).await;
    assert_eq!(
        (configured.server.as_str(), configured.reachable),
        ("https://emberhut.example.com", true),
        "the public URL before the address the request came in on"
    );
    assert!(
        configured
            .install
            .command
            .contains("--server https://emberhut.example.com --pair ")
    );

    let sent = ask(
        Call::post("/api/runners/codes").json(&json!({ "server": "http://192.0.2.20:18990/" }))
    )
    .await;
    assert_eq!(
        (sent.server.as_str(), sent.reachable),
        ("http://192.0.2.20:18990", true),
        "the app's address first"
    );

    let answer = app
        .call(
            Call::post("/api/runners/codes")
                .token(session)
                .json(&json!({ "server": "emberhut.example.com" })),
        )
        .await;
    assert_eq!(answer.status, 400, "{}", answer.text);

    let plain = TestApp::new().await;
    let session = plain.sign_in().await;
    let came_in_on: RunnerCode = plain
        .call(
            Call::post("/api/runners/codes")
                .token(&session)
                .local("192.0.2.10:18990"),
        )
        .await
        .read_as();
    assert_eq!(
        (came_in_on.server.as_str(), came_in_on.reachable),
        ("http://192.0.2.10:18990", true),
        "with nothing configured, the address the request came in on"
    );
    let looped: RunnerCode = plain
        .call(
            Call::post("/api/runners/codes")
                .token(&session)
                .local("127.0.0.1:18990"),
        )
        .await
        .read_as();
    assert_eq!(
        (looped.server.as_str(), looped.reachable),
        ("http://127.0.0.1:18990", false)
    );
}

#[tokio::test]
async fn a_taken_name_pairs_under_the_next_free_one() {
    let app = app().await;
    let configured = code(&app).await;
    let answer = pair(&app, &configured, "build", "https://elsewhere.example.com").await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let build_2: PairedRunner = answer.read_as();
    assert_eq!(
        build_2.name, "build-2",
        "the configured runner keeps its name"
    );
    assert_eq!(
        app.state.runners().place("build"),
        "Build box",
        "the configured runner stays"
    );

    let first = code(&app).await;
    let laptop = token_of(&pair(&app, &first, "laptop", "https://old.example.com").await);
    let second = code(&app).await;
    let answer = pair(&app, &second, "laptop", "https://new.example.com").await;
    let laptop_2: PairedRunner = answer.read_as();
    assert_eq!(laptop_2.name, "laptop-2");
    assert_ne!(laptop, laptop_2.token);

    assert_eq!(report(&app, &laptop, "laptop").await.status, 200);
    assert_eq!(report(&app, &laptop_2.token, "laptop-2").await.status, 200);
    assert_eq!(
        report(&app, &laptop_2.token, "laptop").await.status,
        400,
        "each token is its own runner's"
    );
    let session = app.sign_in().await;
    let listed: RunnersPage = app
        .call(Call::get("/api/runners").token(&session))
        .await
        .read_as();
    let names: Vec<&str> = listed.runners.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["build", "build-2", "laptop", "laptop-2"]);
    assert_eq!(
        listed.runner("laptop").unwrap().url,
        "https://old.example.com"
    );
    assert_eq!(
        listed.runner("laptop-2").unwrap().url,
        "https://new.example.com"
    );
}

#[tokio::test]
pub(crate) async fn with_every_name_near_it_taken_the_code_stays_unused() {
    let app = app().await;
    let db = Database::connect(&app.url).await.unwrap();
    let store = db.paired_runners();
    let names: Vec<String> = emberhut_api::runner::pairing::name_candidates("laptop").collect();
    for (at, name) in names.iter().enumerate() {
        let hash = emberhut_core::SecretHash::of(&format!("CODE{at:04}"));
        let now = app.clock_now();
        let id = store
            .insert_code(&hash, now, now + emberhut_core::pairing::CODE_LIFETIME)
            .await
            .unwrap();
        let runner = emberhut_db::RunnerPairing {
            name: name.clone(),
            url: "https://laptop.example.com".to_owned(),
            words: None,
            token: format!("token-{at}"),
            platform: None,
            version: None,
        };
        store
            .pair(id, std::slice::from_ref(name), &runner, now)
            .await
            .unwrap();
    }
    let made = code(&app).await;
    let answer = pair(&app, &made, "laptop", "https://laptop.example.com").await;
    assert_eq!(answer.status, 409, "{}", answer.text);
    assert!(error_of(&answer).contains("laptop"), "{}", answer.text);
    let desk = pair(&app, &made, "desk", "https://desk.example.com").await;
    assert_eq!(token_of_name(&desk), "desk", "the code was left unused");
}

/// The name a pairing answered.
fn token_of_name(answer: &Answer) -> String {
    assert_eq!(answer.status, 200, "{}", answer.text);
    answer.read_as::<PairedRunner>().name
}

#[tokio::test]
pub(crate) async fn too_many_wrong_codes_stop_every_code_for_a_while() {
    let app = app().await;
    let made = code(&app).await;
    for _ in 0..5 {
        let answer = pair(&app, "ZZZZ-ZZZZ", "laptop", "https://laptop.example.com").await;
        assert_eq!(answer.status, 401);
    }
    let answer = pair(&app, &made, "laptop", "https://laptop.example.com").await;
    assert_eq!(answer.status, 429, "{}", answer.text);
    assert!(answer.header("retry-after").parse::<u64>().unwrap() > 0);
    assert!(error_of(&answer).starts_with("Too many wrong codes."));
    let device = app
        .call(Call::post("/api/auth/pair").json(&json!({ "code": "ZZZZ-ZZZZ" })))
        .await;
    assert_eq!(
        device.status, 401,
        "devices' codes have a brake of their own"
    );
}

#[tokio::test]
pub(crate) async fn forgetting_a_runner_stops_its_token_at_once() {
    let app = app().await;
    let made = code(&app).await;
    let token = token_of(&pair(&app, &made, "laptop", "https://laptop.example.com").await);
    assert_eq!(report(&app, &token, "laptop").await.status, 200);
    let session = app.sign_in().await;

    let answer = app
        .call(Call::delete("/api/runners/laptop").token(&session))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert!(answer.read_as::<OkReply>().ok);
    let answer = report(&app, &token, "laptop").await;
    assert_eq!(answer.status, 401, "{}", answer.text);
    assert_eq!(app.state.runners().machines(), ["build"]);

    let answer = app
        .call(Call::delete("/api/runners/laptop").token(&session))
        .await;
    assert_eq!(answer.status, 404, "{}", answer.text);
    let answer = app
        .call(Call::delete("/api/runners/build").token(&session))
        .await;
    assert_eq!(answer.status, 409, "{}", answer.text);
    assert_eq!(report(&app, BUILD, "build").await.status, 200);
}

/// What a stand-in runner was asked: each request's path and `Authorization` header.
type Asked = Arc<Mutex<Vec<(String, String)>>>;

/// A stand-in runner on a free port that answers its health and one account: its address.
async fn stand_in(asked: Asked) -> String {
    async fn answer(State(asked): State<Asked>, request: Request) -> axum::Json<Value> {
        let authorization = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let path = request.uri().path().to_owned();
        asked.lock().unwrap().push((path.clone(), authorization));
        axum::Json(if path == "/accounts" {
            json!({ "accounts": [{ "id": "work", "name": "Work", "loggedIn": true, "used5h": 12.0 }] })
        } else {
            json!({ "ok": true })
        })
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().fallback(answer).with_state(asked);
    tokio::spawn(async move { axum::serve(listener, router).await });
    url
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tick_reaches_a_paired_runner_at_its_address_with_its_token() {
    let app = app().await;
    let asked = Asked::default();
    let url = stand_in(Arc::clone(&asked)).await;
    let made = code(&app).await;
    let token = token_of(&pair(&app, &made, "laptop", &url).await);

    app.state.runners().tick().await;
    let bearer = format!("Bearer {token}");
    let laptop: Vec<(String, String)> = asked.lock().unwrap().clone();
    assert_eq!(
        laptop,
        [
            ("/health".to_owned(), bearer.clone()),
            ("/accounts".to_owned(), bearer)
        ]
    );
    let look = app.state.runners().looks().remove("laptop").unwrap();
    assert_eq!(look.error, None, "it answered");
    let session = app.sign_in().await;
    let accounts = app
        .call(Call::get("/api/accounts").token(&session))
        .await
        .text;
    assert!(accounts.contains("laptop"), "{accounts}");

    // Forgotten, it is no longer looked at.
    app.state.runner_pairing().forget("laptop").await.unwrap();
    asked.lock().unwrap().clear();
    app.state.runners().tick().await;
    assert_eq!(*asked.lock().unwrap(), []);
}

#[tokio::test]
async fn the_paired_runners_are_read_again_when_the_server_starts() {
    let app = app().await;
    let made = code(&app).await;
    token_of(&pair(&app, &made, "laptop", "https://laptop.example.com").await);

    let db = Database::connect(&app.url).await.unwrap();
    let restarted = AppState::new(&db, Dependencies::default(), AuthMode::Enforce);
    assert_eq!(restarted.runners().machines(), Vec::<String>::new());
    restarted.runner_pairing().load().await.unwrap();
    assert_eq!(restarted.runners().machines(), ["laptop"]);
    assert_eq!(restarted.runners().place("laptop"), "Laptop");
}
