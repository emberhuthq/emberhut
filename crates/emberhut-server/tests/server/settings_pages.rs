//! Settings' own pages: the MCP servers and skills switches, and Environments.

use emberhut_api::environments::EnvPage as EnvironmentsPage;
use emberhut_api::switches::SwitchPage;
use serde_json::json;

use crate::common::{Call, TestApp};

const BUILD: &str = "token-of-build";

#[tokio::test]
pub(crate) async fn the_switches_list_what_the_session_has_and_turn_it_off() {
    let app = TestApp::new().await;
    app.runners.on(&[("build", "Build box", BUILD)]);
    app.state
        .switches()
        .observe(
            &json!({ "mcp_servers": [{ "name": "docs", "status": "connected", "source": "user" }],
                          "skills": ["writing", "tools:lint"] }),
        )
        .await
        .unwrap();
    let session = app.sign_in().await;
    let page: SwitchPage = app
        .call(Call::get("/api/switches").token(&session))
        .await
        .read_as();
    assert!(!page.live);
    assert!(
        page.note.is_some(),
        "the session isn't open: what it had last"
    );
    assert_eq!(page.mcp[0].name, "docs");
    assert_eq!(page.mcp[0].state.as_str(), "connected");
    let skills: Vec<(&str, &str)> = page
        .skills
        .iter()
        .map(|s| (s.name.as_str(), s.source.as_str()))
        .collect();
    assert_eq!(skills, [("tools:lint", "plugin"), ("writing", "built in")]);

    let off = |kind: &str, name: &str, on: bool| {
        Call::post("/api/switches")
            .token(&session)
            .json(&json!({ "kind": kind, "name": name, "on": on }))
    };
    let answer = app.call(off("mcp", "docs", false)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let page: SwitchPage = answer.read_as();
    assert_eq!((page.mcp[0].state.as_str(), page.mcp[0].on), ("off", false));
    assert_eq!(
        app.runners.requests_to("/off")[0].body,
        Some(json!({ "mcp": ["docs"], "skills": [] })),
        "the runners take the lists for the sessions they start"
    );
    let settings = app.state.switches().session_settings().await.unwrap();
    assert_eq!(
        settings["deniedMcpServers"],
        json!([{ "serverName": "docs" }])
    );
    let page: SwitchPage = app.call(off("skill", "writing", false)).await.read_as();
    assert!(!page.skills.iter().find(|s| s.name == "writing").unwrap().on);

    let answer = app
        .call(off("mcp", emberhut_api::mcp::SERVER_NAME, false))
        .await;
    assert_eq!(answer.status, 400);
    let answer = app
        .call(
            Call::post("/api/switches")
                .token(&session)
                .json(&json!({ "kind": "mcp", "name": "docs" })),
        )
        .await;
    assert_eq!(answer.json()["error"], "on: true or false");
    let reconnect = |name: &str| {
        Call::post("/api/switches")
            .token(&session)
            .json(&json!({ "kind": "reconnect", "name": name }))
    };
    assert_eq!(
        app.call(reconnect("docs")).await.json()["error"],
        "docs is off: turn it on first"
    );
    app.call(off("mcp", "docs", true)).await;
    let answer = app.call(reconnect("docs")).await;
    assert_eq!(
        answer.json()["error"],
        "the assistant's session isn't open right now"
    );
    app.workers.open_session(vec![json!({ "name": "docs", "status": "failed", "error": "timed out", "config": { "type": "http" } })]);
    let answer = app.call(reconnect("docs")).await;
    assert_eq!(answer.status, 200);
    assert_eq!(app.workers.reconnected(), ["docs"]);
    let page: SwitchPage = answer.read_as();
    assert!(page.live);
    assert_eq!(page.mcp[0].kind.as_deref(), Some("http"));
    assert_eq!(page.mcp[0].error.as_deref(), Some("timed out"));
    assert_eq!(app.call(Call::get("/api/switches")).await.status, 401);
    assert_eq!(app.call(Call::post("/api/switches")).await.status, 401);
}

#[tokio::test]
pub(crate) async fn environments_show_the_runners_the_coding_app_and_the_accounts() {
    let app = TestApp::new().await;
    app.runners.on(&[
        ("build", "Build box", BUILD),
        ("laptop", "Laptop", "token-of-laptop"),
    ]);
    app.runners.answer(
        "build",
        "/accounts",
        json!({ "accounts": [{ "id": "work", "name": "Work", "loggedIn": true, "used5h": 30.0, "email": "work@example.com" }] }),
    );
    app.runners.take_down("laptop");
    app.coding_app.on(&["studio"]);
    app.state.threads().poll().await.unwrap();
    app.state.runners().tick().await;
    let session = app.sign_in().await;
    let answer = app
        .call(Call::get("/api/environments").token(&session))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let page: EnvironmentsPage = answer.read_as();
    let groups: Vec<&str> = page.groups.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(groups, ["runners", "t3", "connections"]);
    let build = &page.groups[0].items[0];
    assert_eq!(
        (build.name.as_str(), build.state.as_str()),
        ("Runner on the Build box", "ok")
    );
    assert_eq!(build.lists[0].items[0].value.as_deref(), Some("Room 70%"));
    assert_eq!(
        build.lists[0].items[0].note.as_deref(),
        Some("work@example.com · any work · 5h 30%, week ?")
    );
    let laptop = &page.groups[0].items[1];
    assert_eq!(
        (laptop.state.as_str(), laptop.status.as_str()),
        ("down", "Offline")
    );
    assert_eq!(
        laptop.last_error.as_ref().unwrap().error,
        "not reachable (could not connect)"
    );
    assert_eq!(page.groups[1].items[0].id.as_str(), "studio");
    assert_eq!(page.groups[2].items[0].status, "1 of 1 with room");
    assert_eq!((page.summary.ok, page.summary.down), (3, 1));

    let health = app.runners.requests().len();
    let answer = app
        .call(
            Call::post("/api/environments/check")
                .token(&session)
                .json(&json!({ "id": "build" })),
        )
        .await;
    assert_eq!(answer.status, 200);
    assert!(
        app.runners.requests().len() > health,
        "the build box's accounts were read again"
    );
    let answer = app
        .call(
            Call::post("/api/environments/check")
                .token(&session)
                .json(&json!({ "id": "build", "kind": "unlink" })),
        )
        .await;
    assert_eq!(answer.status, 400);
    assert_eq!(answer.json()["error"], "unknown action unlink");
    assert_eq!(app.call(Call::get("/api/environments")).await.status, 401);
    assert_eq!(
        app.call(Call::post("/api/environments/check")).await.status,
        401
    );
}
