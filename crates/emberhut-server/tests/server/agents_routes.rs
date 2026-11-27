//! Agents, approvers and trusted requesters: written whole, checked by the rules, from a
//! signed-in device only, in either auth mode.

use emberhut_core::approvals::ActionKind;
use emberhut_server::config::AuthMode;
use emberhut_server::services::approvals::StepAsk;
use serde_json::json;

use crate::approvals_common::Desk;
use crate::common::{Call, TestApp};

/// An agent is made (in a room of its own), changed, given its lists and deleted; the default
/// agent keeps what makes it the default; an agent with approval history is kept.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "every route of an agent, with its refusals"
)]
pub(crate) async fn agents_are_made_changed_and_deleted() {
    let desk = Desk::new().await;
    let app = &desk.app;
    let session = &desk.session;
    let post =
        |body: serde_json::Value| app.call(Call::post("/api/agents").token(session).json(&body));

    let listed = app.call(Call::get("/api/agents").token(session)).await;
    assert_eq!(listed.status, 200);
    assert_eq!(listed.json()["agents"][0]["id"], "main");
    assert_eq!(listed.json()["agents"][1]["id"], "client-a");

    let blank = post(json!({ "id": "client-b", "name": " ", "principal": "Example" })).await;
    assert_eq!(blank.status, 400);
    let in_main =
        post(json!({ "id": "client-b", "name": "B", "principal": "Example", "room": 1 })).await;
    assert_eq!(in_main.status, 400, "Main is the default agent's room");
    let taken = post(json!({ "id": "client-a", "name": "B", "principal": "Example" })).await;
    assert_eq!(taken.status, 409);
    let main = post(json!({ "id": "main", "name": "B", "principal": "Example" })).await;
    assert_eq!(main.status, 409);
    let made =
        post(json!({ "id": "client-b", "name": "Example desk", "principal": "Example" })).await;
    assert_eq!(made.status, 200, "{}", made.text);
    assert_ne!(made.json()["agent"]["room"], 1);

    let page = app
        .call(Call::get("/api/agents/client-b").token(session))
        .await;
    assert_eq!(page.status, 200);
    assert_eq!(page.json()["agent"]["name"], "Example desk");
    assert_eq!(
        app.call(Call::get("/api/agents/nobody").token(session))
            .await
            .status,
        404
    );

    let patch = |path: &str, body: serde_json::Value| {
        app.call(Call::patch(path).token(session).json(&body))
    };
    let renamed = patch(
        "/api/agents/client-b",
        json!({ "name": "Renamed", "enabled": false }),
    )
    .await;
    assert_eq!(renamed.status, 200);
    assert_eq!(renamed.json()["agent"]["enabled"], false);
    assert_eq!(
        patch("/api/agents/main", json!({ "persona": "Be someone else" }))
            .await
            .status,
        400,
        "the default agent gets no persona"
    );
    assert_eq!(
        patch("/api/agents/nobody", json!({ "name": "X" }))
            .await
            .status,
        404
    );

    let put =
        |path: &str, body: serde_json::Value| app.call(Call::put(path).token(session).json(&body));
    let lists = [
        (
            "channels",
            json!({ "channels": [{ "kind": "whatsapp", "address": "120363000000000000@g.us" }] }),
            json!({ "channels": [{ "kind": "slack", "workspace": "T0EXAMPLE", "address": "C0EXAMPLE" }] }),
        ),
        (
            "tools",
            json!({ "tools": [{ "kind": "server", "name": "tasks" }, { "kind": "mcp", "name": "example-admin", "config": "/srv/example/mcp.json" }] }),
            json!({ "tools": [{ "kind": "server", "name": "slack_send" }] }),
        ),
        (
            "accounts",
            json!({ "accounts": [{ "label": "admin", "secret": "example-admin-token" }] }),
            json!({ "accounts": [{ "label": "Not A Label", "secret": "example-admin-token" }] }),
        ),
        (
            "policies",
            json!({ "policies": [{ "kind": "deploy", "approver": "example-lead" }] }),
            json!({ "policies": [{ "kind": "payment", "approver": "example-lead" }] }),
        ),
    ];
    for (list, good, refused) in lists {
        let list_path = format!("/api/agents/client-b/{list}");
        let written = put(&list_path, good.clone()).await;
        assert_eq!(written.status, 200, "{list}: {}", written.text);
        assert_eq!(written.json()[list], good[list], "{list}");
        assert_eq!(put(&list_path, refused).await.status, 400, "{list}");
        assert_eq!(
            put(&format!("/api/agents/nobody/{list}"), good)
                .await
                .status,
            404,
            "{list}"
        );
    }
    assert_eq!(
        put("/api/agents/main/tools", json!({ "tools": [] }))
            .await
            .status,
        400,
        "the default agent's tools are never narrowed"
    );

    let delete = |path: &str| app.call(Call::delete(path).token(session));
    assert_eq!(delete("/api/agents/main").await.status, 400);
    assert_eq!(delete("/api/agents/nobody").await.status, 404);
    desk.app
        .state
        .approvals()
        .request(
            &desk.agent,
            &StepAsk {
                kind: ActionKind::Post,
                target: "example blog".to_owned(),
                summary: "A post".to_owned(),
                text: Some("Hello".to_owned()),
                files: Vec::new(),
                amount_cents: None,
                task: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        delete("/api/agents/client-a").await.status,
        409,
        "approval history outlives nothing it names"
    );
    let deleted = delete("/api/agents/client-b").await;
    assert_eq!(deleted.status, 200);
    assert_eq!(deleted.json()["agents"].as_array().unwrap().len(), 2);
}

/// Approvers are added, changed and deleted (not while a policy names one); the trusted
/// requesters are replaced whole.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "every route of an approver and of the trusted requesters, with their refusals"
)]
pub(crate) async fn approvers_and_trusted_requesters_are_kept() {
    let desk = Desk::new().await;
    let app = &desk.app;
    let session = &desk.session;

    let listed = app.call(Call::get("/api/approvers").token(session)).await;
    assert_eq!(listed.status, 200);
    assert_eq!(listed.json()["approvers"][0]["id"], "example-lead");
    let post =
        |body: serde_json::Value| app.call(Call::post("/api/approvers").token(session).json(&body));
    assert_eq!(
        post(json!({ "id": "example-cto", "name": "Example CTO" }))
            .await
            .status,
        400,
        "an approver is reached somehow"
    );
    assert_eq!(
        post(json!({ "id": "example-lead", "name": "Again", "slackWorkspace": "T0EXAMPLE", "slackUser": "U0OTHER" }))
            .await
            .status,
        409
    );
    let made = post(json!({ "id": "example-cto", "name": "Example CTO", "slackWorkspace": "T0EXAMPLE", "slackUser": "U0CTO" })).await;
    assert_eq!(made.status, 200);

    let patch = |path: &str, body: serde_json::Value| {
        app.call(Call::patch(path).token(session).json(&body))
    };
    let changed = patch(
        "/api/approvers/example-cto",
        json!({ "name": "Example chief", "link": true }),
    )
    .await;
    assert_eq!(changed.status, 200);
    assert_eq!(changed.json()["name"], "Example chief");
    assert_eq!(changed.json()["slackUser"], "U0CTO");
    assert_eq!(
        patch(
            "/api/approvers/example-cto",
            json!({ "slackWorkspace": null, "slackUser": null })
        )
        .await
        .status,
        400,
        "a change that leaves no way to reach them"
    );
    assert_eq!(
        patch("/api/approvers/nobody", json!({ "name": "X" }))
            .await
            .status,
        404
    );

    let delete = |path: &str| app.call(Call::delete(path).token(session));
    assert_eq!(
        delete("/api/approvers/example-lead").await.status,
        409,
        "a policy names it"
    );
    desk.app
        .state
        .approvals()
        .request(
            &desk.agent,
            &StepAsk {
                kind: ActionKind::Post,
                target: "example blog".to_owned(),
                summary: "A post".to_owned(),
                text: Some("Hello".to_owned()),
                files: Vec::new(),
                amount_cents: None,
                task: None,
            },
        )
        .await
        .unwrap();
    desk.set_policies(&json!([])).await;
    assert_eq!(
        delete("/api/approvers/example-lead").await.status,
        409,
        "an open request still names it: a new approver with its id must not decide it"
    );
    assert_eq!(delete("/api/approvers/nobody").await.status, 404);
    let deleted = delete("/api/approvers/example-cto").await;
    assert_eq!(deleted.status, 200);
    assert_eq!(deleted.json()["approvers"].as_array().unwrap().len(), 1);

    let read = app
        .call(Call::get("/api/trusted-requesters").token(session))
        .await;
    assert_eq!(read.status, 200);
    assert_eq!(read.json()["requesters"], json!([]));
    let list = json!({ "requesters": [
        { "requester": "Example teammate", "agent": "client-a" },
        { "requester": "Example partner", "agent": null },
    ]});
    let written = app
        .call(
            Call::put("/api/trusted-requesters")
                .token(session)
                .json(&list),
        )
        .await;
    assert_eq!(written.status, 200, "{}", written.text);
    assert_eq!(written.json()["requesters"].as_array().unwrap().len(), 2);
    let unknown = app
        .call(
            Call::put("/api/trusted-requesters")
                .token(session)
                .json(&json!({ "requesters": [{ "requester": "Someone", "agent": "nobody" }] })),
        )
        .await;
    assert_eq!(unknown.status, 400);
}

/// Every owner route needs a signed-in device, in either auth mode: report mode's pass is for
/// reading the app, never for configuring agents or deciding approvals.
#[tokio::test]
pub(crate) async fn every_owner_route_needs_a_signed_in_device() {
    let routes = [
        Call::get("/api/agents"),
        Call::post("/api/agents").json(&json!({ "id": "client-b", "name": "B", "principal": "P" })),
        Call::get("/api/agents/main"),
        Call::patch("/api/agents/main").json(&json!({})),
        Call::delete("/api/agents/client-b"),
        Call::put("/api/agents/client-b/channels").json(&json!({ "channels": [] })),
        Call::put("/api/agents/client-b/tools").json(&json!({ "tools": [] })),
        Call::put("/api/agents/client-b/accounts").json(&json!({ "accounts": [] })),
        Call::put("/api/agents/client-b/policies").json(&json!({ "policies": [] })),
        Call::get("/api/approvers"),
        Call::post("/api/approvers")
            .json(&json!({ "id": "x", "name": "X", "slackWorkspace": "T0X", "slackUser": "U0X" })),
        Call::patch("/api/approvers/x").json(&json!({})),
        Call::delete("/api/approvers/x"),
        Call::get("/api/trusted-requesters"),
        Call::put("/api/trusted-requesters").json(&json!({ "requesters": [] })),
        Call::get("/api/approvals"),
        Call::get("/api/approvals/1"),
        Call::post("/api/approvals/1/decision")
            .json(&json!({ "decision": "approve", "payloadHash": "00".repeat(32) })),
        Call::post("/api/approvals/1/withdraw").json(&json!({})),
        Call::post("/api/approvals/1/use").json(&json!({})),
    ];
    for mode in [AuthMode::Enforce, AuthMode::Report] {
        let app = TestApp::with_mode(mode).await;
        for call in routes.clone() {
            let answer = app.call(call.clone()).await;
            assert_eq!(answer.status, 401, "{mode:?} {call:?}: {}", answer.text);
        }
    }
}
