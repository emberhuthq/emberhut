//! The person's changes and the pages behind Settings.

use emberhut_api::chrome::ChromeAction;
use emberhut_api::facts::FactCategory;
use emberhut_api::ids::{ChromeProfileId, TrialId};
use emberhut_api::settings::SettingsAction;
use emberhut_api::switches::SwitchAction;
use emberhut_client::Error;
use serde_json::json;

use crate::support::{Reply, TestServer, fixture, fresh};

#[test]
fn a_change_posts_its_own_keys() {
    let server = TestServer::start(|request| {
        if request.json().get("trialDrop").is_some() {
            Reply::Json(400, json!({ "ok": false, "error": "no such rule" }))
        } else {
            Reply::Json(200, json!({ "ok": true, "room": 1, "on": false }))
        }
    });
    let client = server.client(fresh("r1"));
    client.settings(SettingsAction::SetOn(false)).unwrap();
    let error = client
        .settings(SettingsAction::TrialDrop(TrialId::new(9)))
        .unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 400, message: Some(m) } if m == "no such rule"),
        "{error:?}"
    );
    let sent = server.requests_to("/api/settings");
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].json(), json!({ "on": false }));
    assert_eq!(sent[1].json(), json!({ "trialDrop": 9 }));
}

#[test]
fn the_switches_are_read_and_switched() {
    let server = TestServer::start(|request| match request.method.as_str() {
        "GET" => Reply::Json(200, fixture("switches.json")),
        _ if request.json()["name"] == "assistant" => Reply::Json(
            400,
            json!({ "error": "The assistant's own tools are always on" }),
        ),
        _ => Reply::Json(200, fixture("switches.json")),
    });
    let client = server.client(fresh("r1"));
    assert_eq!(client.switches().unwrap().mcp.len(), 4);
    let off = SwitchAction::Skill {
        name: "simplify".into(),
        on: false,
    };
    assert_eq!(client.switch(&off).unwrap().skills.len(), 3);
    let fixed = SwitchAction::Mcp {
        name: "assistant".into(),
        on: false,
    };
    let error = client.switch(&fixed).unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 400, message: Some(m) } if m.contains("always on")),
        "{error:?}"
    );
    let sent = server.requests_to("/api/switches");
    assert_eq!(
        sent[1].json(),
        json!({ "kind": "skill", "name": "simplify", "on": false })
    );
}

#[test]
fn a_server_without_switches_answers_404() {
    let server = TestServer::start(|_| Reply::Text(404, "Not found"));
    let error = server.client(fresh("r1")).switches().unwrap_err();
    assert!(
        matches!(error, Error::Status { status: 404, .. }),
        "{error:?}"
    );
}

#[test]
fn the_facts_are_read() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("facts.json")));
    let page = server.client(fresh("r1")).facts().unwrap();
    assert_eq!(page.facts[0].category, FactCategory::People);
    assert_eq!(page.trials.len(), 1);
    assert_eq!(server.requests_to("/api/facts")[0].method, "GET");
}

#[test]
fn a_chrome_change_answers_with_the_profiles() {
    let server = TestServer::start(|request| {
        if request.json()["kind"] == "pause" {
            Reply::Json(
                409,
                json!({ "ok": false, "error": "That Chrome isn't connected" }),
            )
        } else {
            Reply::Json(200, fixture("chrome_reply.json"))
        }
    });
    let client = server.client(fresh("r1"));
    let add = ChromeAction::Add {
        label: "Side project".into(),
    };
    let reply = client.chrome(&add).unwrap();
    assert_eq!(reply.pairing.unwrap().label, "Side project");
    let pause = ChromeAction::Pause {
        id: ChromeProfileId::new(3),
        minutes: None,
    };
    let error = client.chrome(&pause).unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 409, message: Some(m) } if m.contains("connected")),
        "{error:?}"
    );
    assert_eq!(
        server.requests_to("/api/chrome")[0].json(),
        json!({ "kind": "add", "label": "Side project" })
    );
}
