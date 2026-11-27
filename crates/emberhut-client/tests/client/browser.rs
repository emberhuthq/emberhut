//! The assistant's browser: the live view's session, the toolbar, and a file for the page.

use emberhut_api::browser::{BrowserInput, BrowserSessionQuery, StreamMode};
use emberhut_api::ids::BrowserDeviceId;
use emberhut_api::uploads::UploadQuery;
use emberhut_client::Error;
use serde_json::json;

use crate::support::{Reply, TestServer, fixture, fresh};

#[test]
fn a_session_names_the_device_and_a_machine_without_one_says_why() {
    let server = TestServer::start(|request| {
        if request.query.contains("other") {
            Reply::Json(
                503,
                json!({ "ok": false, "error": "The live view isn't set up on this machine" }),
            )
        } else {
            Reply::Json(200, fixture("browser_live_view.json"))
        }
    });
    let client = server.client(fresh("r1"));
    let here = BrowserSessionQuery {
        device: BrowserDeviceId::new("device 1"),
    };
    let reply = client.browser_session(&here).unwrap();
    assert_eq!(reply.session.mode, StreamMode::Control);
    let other = BrowserSessionQuery {
        device: BrowserDeviceId::new("other"),
    };
    let error = client.browser_session(&other).unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 503, message: Some(m) } if m.contains("live view")),
        "{error:?}"
    );
    let sent = server.requests_to("/api/browser/session");
    assert_eq!(sent[0].method, "GET");
    assert_eq!(sent[0].query, "device=device%201");
}

#[test]
fn input_posts_its_kind_and_a_refusal_keeps_the_servers_words() {
    let server = TestServer::start(|request| {
        if request.json()["kind"] == "back" {
            Reply::Json(409, json!({ "ok": false, "error": "Take over first" }))
        } else {
            Reply::Json(200, fixture("browser_reply.json"))
        }
    });
    let client = server.client(fresh("r1"));
    let take = BrowserInput::Takeover {
        device: BrowserDeviceId::new("device-1"),
    };
    let reply = client.browser_input(&take).unwrap();
    assert!(reply.browser.connected);
    let error = client.browser_input(&BrowserInput::Back).unwrap_err();
    assert!(
        matches!(&error, Error::Status { status: 409, message: Some(m) } if m == "Take over first"),
        "{error:?}"
    );
    assert_eq!(
        server.requests_to("/api/browser/input")[0].json(),
        json!({ "kind": "takeover", "device": "device-1" })
    );
}

#[test]
fn an_upload_sends_the_raw_file_under_its_name() {
    let server = TestServer::start(|_| Reply::Json(200, fixture("browser_reply.json")));
    let client = server.client(fresh("r1"));
    client
        .browser_upload(&UploadQuery::new("Scan 1.pdf"), b"%PDF-1.7")
        .unwrap();
    let sent = server.requests_to("/api/browser/upload");
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].query, "name=Scan%201.pdf");
    assert_eq!(sent[0].body, b"%PDF-1.7");
    assert_eq!(
        sent[0].header("content-type"),
        Some("application/octet-stream")
    );
}
