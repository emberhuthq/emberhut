//! The runners: their events and samples (with their own tokens), the sessions list and the
//! state's `sessions`, `mySessions` and `machines`, a session's transcript, live stream, reply,
//! stop, interrupt and settings, the person's own new sessions, the pickers, the Machines view,
//! the shared MCP list, and the accounts, the Usage card and the pin.

use emberhut_api::accounts::AccountsPage;
use emberhut_api::error::ErrorBody;
use emberhut_api::machines::{MachineState, MachinesPage};
use emberhut_api::runners::{NewSessionReply, RunnerReply, SessionsPage, SharedMcp};
use emberhut_api::state::State;
use emberhut_api::usage::{UsageCheckReply as UsageCheck, UseAccountReply};
use emberhut_db::Database;
use emberhut_server::services::runner_api::by;
use serde_json::{Value, json};

use crate::common::{Call, TestApp};
use crate::fake_runner::form_value;

/// The build runner's token.
const BUILD: &str = "token-of-build";
/// The laptop runner's token.
const LAPTOP: &str = "token-of-laptop";

async fn app() -> TestApp {
    let app = TestApp::new().await;
    app.runners
        .on(&[("build", "Build box", BUILD), ("laptop", "Laptop", LAPTOP)]);
    app
}

/// An event of `machine` about the session `id`.
fn event(machine: &str, id: &str, kind: &str, extra: &Value) -> Value {
    let mut event = json!({
        "machine": machine,
        "kind": kind,
        "at": "2026-11-25T10:00:00.000Z",
        "session": {
            "id": id, "title": format!("Session {id}"), "project": "", "status": "running",
            "account": "work", "cwd": "/work/app", "branch": "main",
        },
    });
    if let (Value::Object(event), Value::Object(extra)) = (&mut event, extra) {
        for (key, value) in extra {
            event.insert(key.clone(), value.clone());
        }
    }
    event
}

async fn report(app: &TestApp, token: &str, body: &Value) -> crate::common::Answer {
    app.call(
        Call::post("/api/runner/events")
            .header("authorization", &format!("Bearer {token}"))
            .json(body),
    )
    .await
}

async fn state(app: &TestApp) -> State {
    let session = app.sign_in().await;
    let answer = app.call(Call::get("/api/state").token(&session)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    answer.read_as()
}

/// The events the runner (`apps/runner`) sent in its own tests, recorded with stable times and ids
/// (`fixtures/runner/events.json`), reported again here as the build runner: they make the session
/// the runner had, with its ask, its question and its answer.
#[tokio::test]
pub(crate) async fn a_recorded_runners_events_make_its_session() {
    let app = app().await;
    let Value::Array(events) = crate::common::fixture("runner/events.json") else {
        panic!("a list of events");
    };
    assert_ne!(events.len(), 0, "the record has events");
    for mut event in events {
        event["machine"] = json!("build");
        let answer = report(&app, BUILD, &event).await;
        assert_eq!(answer.status, 200, "{event}: {}", answer.text);
    }
    let session = app.sign_in().await;
    let listed: SessionsPage = app
        .call(Call::get("/api/runner/sessions").token(&session))
        .await
        .read_as();
    let one = &listed.sessions[0];
    assert_eq!(one.id.as_str(), "r-1a2b3c4d");
    assert!(one.direct);
    assert_eq!(one.status, "waiting");
    assert_eq!(one.question.as_deref(), Some("Which one?"));
    assert_eq!(one.last.as_deref(), Some("Asked."));
    assert!(
        one.ask
            .as_deref()
            .is_some_and(|ask| ask.starts_with("Tidy the readme")),
        "the first ask: {:?}",
        one.ask
    );
    assert_eq!(one.account.as_deref(), Some("a-2"));
}

#[tokio::test]
pub(crate) async fn a_runners_events_make_its_sessions() {
    let app = app().await;
    let queued = event(
        "build",
        "r-1a2b",
        "turn_queued",
        &json!({ "turn": { "id": "r-1a2b:1", "by": by::PERSON, "prompt": "Fix the build\n\nEnd every reply with one last line: STATUS" } }),
    );
    let answer = report(&app, BUILD, &queued).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    report(
        &app,
        BUILD,
        &event(
            "build",
            "r-1a2b",
            "activity",
            &json!({ "doing": "Reading main.rs" }),
        ),
    )
    .await;
    let mut direct = event(
        "laptop",
        "r-cafe",
        "turn_finished",
        &json!({ "turn": { "id": "r-cafe:1", "outcome": "completed", "last": "All green.\nSTATUS: done" } }),
    );
    direct["session"]["status"] = json!("idle");
    direct["session"]["meta"] = json!({ "direct": true });
    direct["session"]["model"] = json!("model-a");
    direct["at"] = json!("2026-11-25T11:00:00.000Z");
    assert_eq!(report(&app, LAPTOP, &direct).await.status, 200);

    let session = app.sign_in().await;
    let listed: SessionsPage = app
        .call(Call::get("/api/runner/sessions").token(&session))
        .await
        .read_as();
    let ids: Vec<&str> = listed.sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["r-cafe", "r-1a2b"], "newest first");
    let build = &listed.sessions[1];
    assert_eq!(
        build.ask.as_deref(),
        Some("Fix the build"),
        "the status ask left out"
    );
    assert_eq!(build.doing.as_deref(), Some("Reading main.rs"));
    assert_eq!(build.place, "Build box");
    assert_eq!(build.cwd.as_deref(), Some("/work/app"));
    let laptop = &listed.sessions[0];
    assert!(laptop.direct);
    assert_eq!(laptop.last.as_deref(), Some("All green. STATUS: done"));
    assert_eq!(laptop.status_line.as_deref(), Some("STATUS: done"));
    assert_eq!(laptop.model.as_deref(), Some("model-a"));

    let mine: SessionsPage = app
        .call(
            Call::get("/api/runner/sessions")
                .query("direct=1")
                .token(&session),
        )
        .await
        .read_as();
    assert_eq!(mine.sessions.len(), 1);
    let linked: SessionsPage = app
        .call(
            Call::get("/api/runner/sessions")
                .query("task=7")
                .token(&session),
        )
        .await
        .read_as();
    assert!(
        linked.sessions.is_empty(),
        "no session is linked to a task yet"
    );

    let state = state(&app).await;
    let sessions: Vec<&str> = state.sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(sessions, ["r-1a2b", "r-cafe"], "the running one first");
    let my_sessions: Vec<&str> = state.my_sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(my_sessions, ["r-cafe"]);
}

#[tokio::test]
pub(crate) async fn only_a_runner_with_its_token_reports_and_only_for_itself() {
    let app = app().await;
    let body = event("build", "r-1", "activity", &json!({}));
    let answer = app.call(Call::post("/api/runner/events").json(&body)).await;
    assert_eq!(answer.status, 401);
    assert_eq!(
        answer.read_as::<ErrorBody>().error,
        "runners use their token"
    );
    let session = app.sign_in().await;
    let answer = app
        .call(Call::post("/api/runner/events").token(&session).json(&body))
        .await;
    assert_eq!(
        answer.status, 401,
        "a device's session is not a runner's token"
    );
    let answer = report(&app, LAPTOP, &body).await;
    assert_eq!(
        answer.status, 400,
        "the laptop can't report for the build box"
    );
    assert_eq!(
        answer.read_as::<ErrorBody>().error,
        "not a runner event (build)"
    );
    let answer = report(&app, BUILD, &json!({ "machine": "build", "session": {} })).await;
    assert_eq!(answer.status, 400);
    let answer = app
        .call(Call::post("/api/runner/machine").json(&json!({ "machine": "build" })))
        .await;
    assert_eq!(answer.status, 401);
    let answer = app.call(Call::get("/api/runner/mcp")).await;
    assert_eq!(answer.status, 401);
}

#[tokio::test]
pub(crate) async fn a_sessions_transcript_comes_from_its_runner_with_what_the_server_knows() {
    let app = app().await;
    report(
        &app,
        BUILD,
        &event(
            "build",
            "r-1a2b",
            "activity",
            &json!({ "doing": "Testing" }),
        ),
    )
    .await;
    app.runners.answer(
        "build",
        "/accounts",
        json!({ "accounts": [{ "id": "work", "name": "Work account", "loggedIn": true, "login": "never kept" }] }),
    );
    app.state.runners().tick().await;
    app.runners.answer(
        "build",
        "/sessions/r-1a2b/transcript",
        json!({ "session": { "id": "r-1a2b", "title": "Session", "status": "running", "account": "work" },
                "events": [{ "seq": 1, "kind": "text", "text": "Hello" }], "earlier": false }),
    );
    let session = app.sign_in().await;
    let answer = app
        .call(
            Call::get("/api/runner/sessions/r-1a2b/transcript")
                .query("after=4&limit=10")
                .token(&session),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let transcript = answer.json();
    assert_eq!(transcript["session"]["place"], "Build box");
    assert_eq!(transcript["session"]["accountName"], "Work account");
    assert_eq!(transcript["session"]["doing"], "Testing");
    assert_eq!(transcript["session"]["tasks"], json!([]));
    assert_eq!(transcript["events"][0]["text"], "Hello");
    let asked = &app.runners.requests_to("/sessions/r-1a2b/transcript")[0];
    assert_eq!(form_value(&asked.path, "after").as_deref(), Some("4"));
    assert_eq!(form_value(&asked.path, "limit").as_deref(), Some("10"));

    app.runners
        .refuse("build", "/sessions/r-1a2b/transcript", "no such session");
    let answer = app
        .call(Call::get("/api/runner/sessions/r-1a2b/transcript").token(&session))
        .await;
    assert_eq!(answer.status, 502);
    assert_eq!(answer.read_as::<ErrorBody>().error, "no such session");
    let answer = app
        .call(Call::get("/api/runner/sessions/r-xyz/transcript").token(&session))
        .await;
    assert_eq!(answer.status, 404, "not a session's id");
}

#[tokio::test]
pub(crate) async fn the_person_replies_stops_interrupts_and_changes_a_session() {
    let app = app().await;
    report(
        &app,
        BUILD,
        &event("build", "r-1a2b", "activity", &json!({})),
    )
    .await;
    app.runners.answer(
        "build",
        "/sessions/r-1a2b/send",
        json!({ "turn": "r-1a2b:2", "queued": true }),
    );
    let session = app.sign_in().await;
    let answer = app
        .call(
            Call::post("/api/runner/sessions/r-1a2b/send")
                .token(&session)
                .json(&json!({ "text": " Carry on " })),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let reply: RunnerReply = answer.read_as();
    assert_eq!(reply.rest["turn"], "r-1a2b:2");
    let sent = &app.runners.requests_to("/sessions/r-1a2b/send")[0];
    assert_eq!(sent.machine, "build");
    assert_eq!(
        sent.body,
        Some(json!({ "prompt": "Carry on", "by": by::PERSON }))
    );
    let answer = app
        .call(
            Call::post("/api/runner/sessions/r-1a2b/send")
                .token(&session)
                .json(&json!({ "text": "  " })),
        )
        .await;
    assert_eq!(answer.status, 400);
    assert_eq!(answer.read_as::<ErrorBody>().error, "nothing to send");

    app.runners.answer(
        "build",
        "/sessions/r-1a2b/stop",
        json!({ "ok": true, "running": true }),
    );
    let answer = app
        .call(Call::post("/api/runner/sessions/r-1a2b/stop").token(&session))
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.json(), json!({ "ok": true, "running": true }));
    let answer = app
        .call(Call::post("/api/runner/sessions/r-1a2b/interrupt").token(&session))
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(
        app.runners.requests_to("/sessions/r-1a2b/interrupt").len(),
        1
    );

    app.runners.answer(
        "build",
        "/sessions/r-1a2b/settings",
        json!({ "id": "r-1a2b", "model": "model-b" }),
    );
    let answer = app
        .call(
            Call::post("/api/runner/sessions/r-1a2b/settings")
                .token(&session)
                .json(&json!({ "model": "model-b" })),
        )
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.json()["session"]["model"], "model-b");
    assert_eq!(
        app.runners.requests_to("/sessions/r-1a2b/settings")[0].body,
        Some(json!({ "model": "model-b" }))
    );

    let db = Database::connect(&app.url).await.unwrap();
    let log = db.log().recent(10).await.unwrap();
    assert!(
        log.iter()
            .any(|line| line.text == "Stopped runner session r-1a2b"),
        "{log:?}"
    );
}

#[tokio::test]
pub(crate) async fn a_live_stream_is_the_runners_passed_through() {
    let app = app().await;
    report(
        &app,
        BUILD,
        &event("build", "r-1a2b", "activity", &json!({})),
    )
    .await;
    app.runners
        .live(&["data: {\"type\":\"step\"}\n\n", ": ping\n\n"]);
    let session = app.sign_in().await;
    let answer = app
        .call(
            Call::get("/api/runner/sessions/r-1a2b/live")
                .query("after=12")
                .token(&session),
        )
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.header("content-type"), "text/event-stream");
    assert_eq!(answer.text, "data: {\"type\":\"step\"}\n\n: ping\n\n");
    assert_eq!(
        app.runners.requests_to("/sessions/r-1a2b/live")[0].path,
        "/sessions/r-1a2b/live?after=12"
    );
}

#[tokio::test]
pub(crate) async fn the_person_opens_their_own_session() {
    let app = app().await;
    app.runners.answer(
        "laptop",
        "/sessions",
        json!({ "id": "r-beef", "turn": "r-beef:1", "title": "Tidy", "cwd": "/work/app" }),
    );
    let session = app.sign_in().await;
    let answer = app
        .call(
            Call::post("/api/runner/sessions")
                .token(&session)
                .json(&json!({
                    "machine": "laptop", "cwd": "/work/app", "text": "Tidy the readme",
                    "effort": "high", "worktree": { "branch": "tidy" }, "files": [],
                })),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let reply: NewSessionReply = answer.read_as();
    assert_eq!(reply.id.as_deref(), Some("r-beef"));
    assert_eq!(reply.turn.as_deref(), Some("r-beef:1"));
    let opened = app.runners.requests_to("/sessions")[0]
        .body
        .clone()
        .unwrap();
    assert_eq!(opened["prompt"], "Tidy the readme");
    assert_eq!(opened["meta"], json!({ "direct": true }));
    assert_eq!(opened["by"], by::PERSON);
    assert_eq!(opened["worktree"], json!({ "branch": "tidy" }));
    assert_eq!(opened["effort"], "high");
    assert!(opened.get("account").is_none(), "the runner picks");

    let answer = app
        .call(
            Call::post("/api/runner/sessions")
                .token(&session)
                .json(&json!({
                    "machine": "elsewhere", "cwd": "/", "text": "hi",
                })),
        )
        .await;
    assert_eq!(answer.status, 409);
    assert_eq!(
        answer.read_as::<ErrorBody>().error,
        "no runner called elsewhere"
    );
    let answer = app
        .call(
            Call::post("/api/runner/sessions")
                .token(&session)
                .json(&json!({
                    "machine": "laptop", "cwd": "/", "text": " ",
                })),
        )
        .await;
    assert_eq!(answer.status, 409);
    assert_eq!(
        answer.read_as::<ErrorBody>().error,
        "say something to start it with"
    );
}

#[tokio::test]
pub(crate) async fn the_pickers_list_a_runners_folders_and_branches() {
    let app = app().await;
    app.runners.answer(
        "build",
        "/folders",
        json!({ "machine": "build", "home": "/work", "recent": [], "documents": "/work/docs", "projects": ["dropped"] }),
    );
    app.runners.answer(
        "build",
        "/branches",
        json!({ "repo": "/work/app", "current": "main", "branches": ["main", "next"] }),
    );
    let session = app.sign_in().await;
    let answer = app
        .call(
            Call::get("/api/runner/folders")
                .query("machine=build")
                .token(&session),
        )
        .await;
    assert_eq!(answer.status, 200);
    let roots = answer.json();
    assert_eq!(roots["home"], "/work");
    assert_eq!(
        roots["projects"],
        json!([]),
        "no project has a runner folder yet"
    );
    let answer = app
        .call(
            Call::get("/api/runner/folders")
                .query("machine=build&path=/work/app")
                .token(&session),
        )
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(
        app.runners.requests_to("/folders").last().unwrap().path,
        "/folders?path=%2Fwork%2Fapp"
    );
    let answer = app
        .call(
            Call::get("/api/runner/branches")
                .query("machine=build&cwd=/work/app")
                .token(&session),
        )
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.json()["branches"], json!(["main", "next"]));
    let answer = app
        .call(
            Call::get("/api/runner/branches")
                .query("machine=nope")
                .token(&session),
        )
        .await;
    assert_eq!(answer.status, 400);
    assert_eq!(
        answer.json(),
        json!({ "ok": false, "error": "no such runner" })
    );
    app.runners.refuse("build", "/branches", "not a git folder");
    let answer = app
        .call(
            Call::get("/api/runner/branches")
                .query("machine=build&cwd=/")
                .token(&session),
        )
        .await;
    assert_eq!(answer.status, 502);
    assert_eq!(
        answer.json(),
        json!({ "ok": false, "error": "not a git folder" })
    );
}

/// A sample of a machine with `free` GB of disk.
fn sample(machine: &str, free: f64) -> Value {
    let gb = 1024.0 * 1024.0 * 1024.0;
    json!({
        "machine": machine, "host": "build.local", "platform": "linux",
        "cpu": { "cores": 8, "load1": 1.0, "load5": 1.2, "usage": 0.25 },
        "memory": { "total": 64.0 * gb, "available": 32.0 * gb, "used": 32.0 * gb, "pressure": "normal" },
        "disks": [{ "path": "/", "total": 500.0 * gb, "free": free * gb }],
        "gpus": [{ "name": "GPU 0", "util": 0.5, "memUsed": 2.0 * gb, "memTotal": 24.0 * gb }],
        "runner": { "sessions": 1, "queued": 0, "open": 2, "claude": 1, "cgroup": { "max": 48.0 * gb, "capUsed": 8.0 * gb, "maxBy": "the unit" } },
        "sessions": [{ "id": "r-1a2b", "running": true, "account": "work" }],
    })
}

/// A sample is kept as its machine's latest (`machine_latest`) and folded into its 5-minute
/// point (`machine_points`), and the Machines view shows both.
#[tokio::test]
pub(crate) async fn a_sample_is_kept_in_machine_latest_and_machine_points_and_shown() {
    let app = app().await;
    report(
        &app,
        BUILD,
        &event(
            "build",
            "r-1a2b",
            "activity",
            &json!({ "doing": "Building" }),
        ),
    )
    .await;
    for free in [100.0, 5.0] {
        let answer = app
            .call(
                Call::post("/api/runner/machine")
                    .header("authorization", &format!("Bearer {BUILD}"))
                    .json(&sample("build", free)),
            )
            .await;
        assert_eq!(answer.status, 200, "{}", answer.text);
    }
    let answer = app
        .call(
            Call::post("/api/runner/machine")
                .header("authorization", &format!("Bearer {BUILD}"))
                .json(&sample("laptop", 100.0)),
        )
        .await;
    assert_eq!(answer.status, 400, "a runner samples only its own machine");

    let session = app.sign_in().await;
    let answer = app.call(Call::get("/api/machines").token(&session)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let page: MachinesPage = answer.read_as();
    assert_eq!(page.machines.len(), 2, "every runner, sampled or not");
    let build = &page.machines[0];
    assert_eq!(build.name, "Build box");
    assert_eq!(build.state, MachineState::Low);
    assert_eq!(build.placeable, Some(false));
    assert_eq!(
        build.skip_why.as_deref(),
        Some("/ has only 5.0 GB free (99% full)")
    );
    assert_eq!(build.warnings[0].text, "/ has only 5.0 GB free (99% full)");
    assert_eq!(build.sessions[0].doing.as_deref(), Some("Building"));
    assert_eq!(
        build
            .runner
            .as_ref()
            .unwrap()
            .cap
            .as_ref()
            .unwrap()
            .by
            .as_deref(),
        Some("the unit")
    );
    let history = build.history.as_ref().unwrap();
    assert_eq!(history.len(), 1, "both samples in one 5-minute point");
    let gb = 1024.0 * 1024.0 * 1024.0;
    assert_eq!(
        history[0].disk_free,
        Some(5.0 * gb),
        "the least disk of the two"
    );
    let laptop = &page.machines[1];
    assert_eq!(laptop.state, MachineState::Unknown);
    assert_eq!(laptop.history.as_ref().unwrap().as_slice(), []);

    let answer = app
        .call(
            Call::get("/api/machines")
                .query("history=0")
                .token(&session),
        )
        .await;
    assert!(
        answer.read_as::<MachinesPage>().machines[0]
            .history
            .is_none()
    );

    app.advance_minutes(3);
    let page: MachinesPage = app
        .call(Call::get("/api/machines").token(&session))
        .await
        .read_as();
    assert_eq!(page.machines[0].state, MachineState::Offline);
    assert!(
        page.machines[0]
            .status
            .starts_with("Offline · last sample 3m ago")
    );
}

#[tokio::test]
pub(crate) async fn the_state_lists_each_machines_error() {
    let app = app().await;
    app.runners.take_down("laptop");
    app.state.runners().tick().await;
    let machines = state(&app).await.machines;
    assert_eq!(machines.get("build"), Some(&None));
    assert_eq!(
        machines.get("laptop"),
        Some(&Some("not reachable (could not connect)".to_owned()))
    );
}

#[tokio::test]
pub(crate) async fn the_runners_get_the_shared_mcp_list() {
    let app = app().await;
    let answer = app
        .call(Call::get("/api/runner/mcp").header("authorization", &format!("Bearer {LAPTOP}")))
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(
        answer.read_as::<SharedMcp>().mcp_servers.len(),
        0,
        "none listed yet"
    );
    std::fs::write(
        app.dir().join("runner-mcp.json"),
        r#"{ "mcpServers": { "docs": { "type": "http", "url": "https://docs.example/mcp" } } }"#,
    )
    .unwrap();
    let answer = app
        .call(Call::get("/api/runner/mcp").header("authorization", &format!("Bearer {LAPTOP}")))
        .await;
    assert_eq!(
        answer.json(),
        json!({ "mcpServers": { "docs": { "type": "http", "url": "https://docs.example/mcp" } } })
    );
}

/// The build runner's accounts.
fn accounts() -> Value {
    json!({ "accounts": [
        { "id": "work", "name": "Work (team)", "scope": "any", "order": 1, "enabled": true, "loggedIn": true,
          "email": "work@example.com", "used5h": 40.0, "used7d": 10.0, "checked": "2026-11-25T09:00:00.000Z",
          "login": "never kept" },
        { "id": "spare", "name": "Spare", "scope": "any", "order": 2, "enabled": true, "loggedIn": true,
          "used5h": 5.0, "used7d": 5.0, "checked": "2026-11-25T09:00:00.000Z" },
        { "id": "client", "name": "Client work", "scope": "client", "enabled": true, "loggedIn": true },
    ] })
}

#[tokio::test]
pub(crate) async fn the_accounts_in_usage_and_the_pin_in_prefs() {
    let app = app().await;
    app.runners.answer("build", "/accounts", accounts());
    app.state.runners().tick().await;
    let session = app.sign_in().await;
    let page: AccountsPage = app
        .call(Call::get("/api/accounts").token(&session))
        .await
        .read_as();
    let ids: Vec<&str> = page.accounts.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["work", "spare", "client"]);
    assert_eq!(page.accounts[0].place.as_deref(), Some("Build box"));
    assert_eq!(page.accounts[0].email.as_deref(), Some("work@example.com"));
    assert_eq!(page.assistant.mode, "auto");
    assert!(
        !app.runners.requests_to("/accounts")[0]
            .path
            .contains("login")
    );

    let seen = state(&app).await;
    assert_eq!(seen.accounts.len(), 3);
    let usage: Vec<(&str, &str, bool)> = seen
        .usage
        .iter()
        .map(|row| (row.id.as_str(), row.name.as_str(), row.best))
        .collect();
    assert_eq!(
        usage,
        [("work", "Work", false), ("spare", "Spare", true)],
        "names only, the roomiest best"
    );
    assert_eq!(seen.assistant_account.as_ref().unwrap().pin, None);
    assert_eq!(
        seen.account.as_ref().unwrap().id,
        None,
        "no turn has said yet"
    );
    // Signing in again (the state above did) replaces the device's session.
    let session = app.sign_in().await;

    let answer = app
        .call(
            Call::post("/api/accounts/use")
                .token(&session)
                .json(&json!({ "account": "work" })),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let reply: UseAccountReply = answer.read_as();
    assert_eq!(reply.account.pin.as_deref(), Some("work"));
    assert_eq!(reply.account.mode, "pinned");
    assert_eq!(reply.usage.len(), 2);
    let db = Database::connect(&app.url).await.unwrap();
    assert_eq!(
        db.prefs()
            .get(emberhut_db::PrefKey::Pin)
            .await
            .unwrap()
            .as_deref(),
        Some("work")
    );
    let answer = app
        .call(
            Call::post("/api/accounts/use")
                .token(&session)
                .json(&json!({ "account": "client" })),
        )
        .await;
    assert_eq!(answer.status, 400);
    assert_eq!(
        answer.json(),
        json!({ "ok": false, "error": "Client work isn't one the assistant runs on: it's off or only for some work." })
    );
    let answer = app
        .call(
            Call::post("/api/accounts/use")
                .token(&session)
                .json(&json!({ "account": "auto" })),
        )
        .await;
    assert_eq!(answer.read_as::<UseAccountReply>().account.pin, None);

    app.state.accounts().set_on(Some("work")).await.unwrap();
    let account = state(&app).await.account.unwrap();
    assert_eq!(account.id.as_deref(), Some("work"));
    assert_eq!(account.used5h, Some(0.4), "a share in the state's account");
    assert_eq!(account.email.as_deref(), Some("work@example.com"));
}

#[tokio::test]
pub(crate) async fn a_usage_check_asks_every_runner_at_most_once_in_two_minutes() {
    let app = app().await;
    let session = app.sign_in().await;
    let answer = app
        .call(Call::post("/api/usage/check").token(&session))
        .await;
    assert_eq!(answer.status, 200);
    assert_eq!(
        answer.read_as::<UsageCheck>(),
        UsageCheck {
            ok: true,
            asked: true
        }
    );
    let again: UsageCheck = app
        .call(Call::post("/api/usage/check").token(&session))
        .await
        .read_as();
    assert!(!again.asked);
    for _ in 0..50 {
        if app.runners.requests_to("/accounts/check").len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        app.runners.requests_to("/accounts/check").len(),
        2,
        "both runners"
    );
    app.advance_minutes(3);
    let later: UsageCheck = app
        .call(Call::post("/api/usage/check").token(&session))
        .await
        .read_as();
    assert!(later.asked);
}

#[tokio::test]
pub(crate) async fn the_runner_pages_need_a_session_a_session_id_and_a_runner_that_answers() {
    let app = app().await;
    for (method, path) in [
        ("GET", "/api/machines"),
        ("GET", "/api/runner/sessions"),
        ("POST", "/api/runner/sessions"),
        ("GET", "/api/runner/folders"),
        ("GET", "/api/runner/branches"),
        ("GET", "/api/runner/sessions/r-1a/transcript"),
        ("GET", "/api/runner/sessions/r-1a/live"),
        ("POST", "/api/runner/sessions/r-1a/send"),
        ("POST", "/api/runner/sessions/r-1a/stop"),
        ("POST", "/api/runner/sessions/r-1a/interrupt"),
        ("POST", "/api/runner/sessions/r-1a/settings"),
        ("GET", "/api/accounts"),
        ("POST", "/api/accounts/use"),
        ("POST", "/api/usage/check"),
    ] {
        let call = if method == "GET" {
            Call::get(path)
        } else {
            Call::post(path)
        };
        let answer = app.call(call).await;
        assert_eq!(answer.status, 401, "{method} {path}");
    }
    let session = app.sign_in().await;
    for (method, path) in [
        ("GET", "/api/runner/sessions/r-XYZ/live"),
        ("POST", "/api/runner/sessions/r-/send"),
        ("POST", "/api/runner/sessions/r-zz/stop"),
        ("POST", "/api/runner/sessions/r-zz/interrupt"),
        ("POST", "/api/runner/sessions/r-zz/settings"),
    ] {
        let call = if method == "GET" {
            Call::get(path)
        } else {
            Call::post(path)
        };
        let answer = app
            .call(call.token(&session).json(&json!({ "text": "hi" })))
            .await;
        assert_eq!(answer.status, 404, "{method} {path}");
        assert!(answer.text.is_empty(), "no body: {}", answer.text);
    }
    // A session no runner reported is looked for on the first runner, which knows nothing.
    for prefix in ["/sessions/r-1a/", "/folders"] {
        app.runners
            .refuse("build", prefix, "the runner is restarting");
    }
    for (method, path) in [
        ("GET", "/api/runner/sessions/r-1a/live"),
        ("POST", "/api/runner/sessions/r-1a/send"),
        ("POST", "/api/runner/sessions/r-1a/stop"),
        ("POST", "/api/runner/sessions/r-1a/interrupt"),
        ("POST", "/api/runner/sessions/r-1a/settings"),
        ("GET", "/api/runner/folders"),
    ] {
        let call = if method == "GET" {
            Call::get(path)
        } else {
            Call::post(path)
        };
        let call = if path.ends_with("folders") {
            call.query("machine=build")
        } else {
            call
        };
        let answer = app
            .call(call.token(&session).json(&json!({ "text": "hi" })))
            .await;
        assert_eq!(answer.status, 502, "{method} {path}: {}", answer.text);
        assert_eq!(answer.json()["ok"], false);
    }
    let answer = app
        .call(
            Call::get("/api/runner/folders")
                .query("machine=elsewhere")
                .token(&session),
        )
        .await;
    assert_eq!(answer.status, 400);
}

#[tokio::test]
async fn in_report_mode_a_request_without_a_session_reads_the_runners_but_never_acts() {
    let app = TestApp::with_mode(emberhut_server::config::AuthMode::Report).await;
    app.runners
        .on(&[("build", "Build box", BUILD), ("laptop", "Laptop", LAPTOP)]);
    let answer = report(&app, BUILD, &event("build", "r-1a", "activity", &json!({}))).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let asked = json!({ "text": "rm -rf ~", "machine": "build", "cwd": "/work/app",
                        "account": "work", "id": "session:r-1a", "kind": "mcp",
                        "name": "files", "on": false });
    for path in [
        "/api/runner/sessions",
        "/api/runner/sessions/r-1a/send",
        "/api/runner/sessions/r-1a/stop",
        "/api/runner/sessions/r-1a/interrupt",
        "/api/runner/sessions/r-1a/settings",
        "/api/runner/sessions/w-7/send",
        "/api/runner/sessions/w-7/stop",
        "/api/runner/sessions/w-7/interrupt",
        "/api/accounts/use",
        "/api/jobs/stop",
        "/api/switches",
    ] {
        let refused = app.call(Call::post(path).json(&asked)).await;
        assert_eq!(refused.status, 401, "POST {path}: {}", refused.text);
        assert_eq!(
            refused.read_as::<ErrorBody>(),
            ErrorBody::sign_in_required(),
            "POST {path}"
        );
    }
    assert_eq!(app.runners.requests(), [], "no runner was asked anything");

    // Reading still works without a session.
    for path in ["/api/runner/sessions", "/api/accounts", "/api/switches"] {
        let read = app.call(Call::get(path)).await;
        assert_eq!(read.status, 200, "GET {path}: {}", read.text);
    }

    // A signed-in device acts.
    let session = app.sign_in().await;
    app.runners.answer(
        "build",
        "/sessions/r-1a/send",
        json!({ "turn": "r-1a:2", "queued": false }),
    );
    let sent = app
        .call(
            Call::post("/api/runner/sessions/r-1a/send")
                .token(&session)
                .json(&json!({ "text": "Carry on." })),
        )
        .await;
    assert_eq!(sent.status, 200, "{}", sent.text);
}
