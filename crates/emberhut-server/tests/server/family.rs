//! `/family/...`: a family member's own rooms, through the household server on this machine.
//!
//! Family rooms are off for now: the first test checks that every way in refuses. The others
//! are the rooms' behaviour once they are on, and are ignored while they are off.
#![expect(clippy::unwrap_used, reason = "tests: a failure fails the test")]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::ConnectInfo;
use emberhut_api::family::{FamilyHistory, FamilyState, FamilyTalkReply};
use emberhut_api::theme::ThemePage;
use emberhut_core::RoomId;
use emberhut_core::push::{ApnsEnv, ApnsToken};
use emberhut_server::services::family::{FamilyDirectory, FamilyError};
use emberhut_server::services::talk::MAX_WAITING;
use futures_util::future::BoxFuture;
use http_body_util::BodyExt as _;
use serde_json::json;

use crate::common::{Call, TestApp};

/// One family member's device key.
pub(crate) const SAM: &str = "sam-device-key-0123456789abcdef0123456789";
/// Another's.
pub(crate) const LEE: &str = "lee-device-key-0123456789abcdef0123456789";

/// The household server of the tests: it knows two family members' keys.
#[derive(Debug)]
pub(crate) struct FakeFamily {
    keys: Mutex<HashMap<String, String>>,
}

impl Default for FakeFamily {
    fn default() -> Self {
        let keys = [(SAM, "sam"), (LEE, "lee")]
            .into_iter()
            .map(|(key, person)| (key.to_owned(), person.to_owned()))
            .collect();
        Self {
            keys: Mutex::new(keys),
        }
    }
}

impl FamilyDirectory for FakeFamily {
    fn who_is<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move { self.keys.lock().unwrap().get(key).cloned() })
    }
}

fn as_member(call: Call, key: &str) -> Call {
    call.header("authorization", &format!("Bearer {key}"))
}

async fn state(app: &TestApp, key: &str, query: &str) -> FamilyState {
    let got = app
        .call(as_member(Call::get("/family/state"), key).query(query))
        .await;
    assert_eq!(got.status, 200, "{}", got.text);
    got.read_as()
}

/// Every family route, as a family member's app calls it.
const ROUTES: [(&str, &str); 8] = [
    ("GET", "/family/state"),
    ("GET", "/family/history"),
    ("GET", "/family/live"),
    ("POST", "/family/talk"),
    ("POST", "/family/devices"),
    ("POST", "/family/seen"),
    ("GET", "/family/theme"),
    ("POST", "/family/theme"),
];

#[tokio::test]
pub(crate) async fn family_rooms_are_off_and_every_way_into_one_refuses() {
    let (apns, _fcm, senders) = crate::fake_push::FakePush::pair();
    let app = TestApp::with_push(emberhut_server::config::AuthMode::Enforce, senders).await;
    for (method, path) in ROUTES {
        let call = match method {
            "GET" => Call::get(path),
            _ => Call::post(path).json(&json!({ "text": "hi", "token": "ab".repeat(32) })),
        };
        let refused = app.call(as_member(call, SAM)).await;
        assert_eq!(refused.status, 501, "{method} {path}");
        assert_eq!(
            refused.json(),
            json!({ "error": "family rooms are not available yet" }),
            "{method} {path}"
        );
    }
    assert!(app.assistant.turns().is_empty(), "nothing was said");
    let made = app.state.family().rooms("sam", None).await;
    assert!(
        matches!(made, Err(FamilyError::NotAvailable)),
        "no room is made: {made:?}"
    );

    // A family room already in the database gets no tools and pushes to no one.
    let now = app.state.clock().now();
    let store = app.state.family().store();
    let home = store.home_room("sam", now).await.unwrap();
    let phone = ApnsToken::parse(&"ab".repeat(32)).unwrap();
    store
        .register_device(&phone, ApnsEnv::Production, "sam", now)
        .await
        .unwrap();
    let token = app.state.agent_tokens().for_room(home.id).unwrap();
    let listed = app
        .call(
            Call::post(emberhut_api::paths::MCP_ROOM)
                .header("authorization", &format!("Bearer {}", token.as_str()))
                .header("accept", "application/json, text/event-stream")
                .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
        )
        .await;
    assert_eq!(listed.status, 501, "{}", listed.text);
    assert_eq!(listed.json()["error"], "family rooms are not available yet");
    let pushed = app.state.push().reply(home.id, "Hello, Sam.").await;
    assert_eq!(pushed.delivered, 0);
    assert!(apns.sent().is_empty(), "{:?}", apns.sent());
}

#[tokio::test]
#[ignore = "family rooms are off (`FAMILY_ROOMS`) until tools scoped to a family member's own room exist"]
pub(crate) async fn a_family_member_talks_in_their_own_home_room() {
    let app = TestApp::new().await;
    let first = app.call(as_member(Call::get("/family/state"), SAM)).await;
    assert_eq!(first.status, 200);
    assert_eq!(first.header("cache-control"), "no-store");
    let tag = first.header("etag").to_owned();
    let mine: FamilyState = first.read_as();
    assert_eq!(mine.me, "sam");
    assert_eq!(mine.rooms.len(), 1);
    assert_eq!(mine.rooms[0].name, "Home");
    assert_ne!(i64::from(mine.room), 1, "never the owner's Main");
    assert_eq!(mine.chat, Vec::new());
    assert!(!mine.working);
    assert_eq!(mine.theme.preset, "paper");

    let unchanged = app
        .call(as_member(Call::get("/family/state"), SAM).header("if-none-match", &tag))
        .await;
    assert_eq!(unchanged.status, 304);

    app.assistant.answers(&["Thinking"], Some("Hello, Sam."));
    let said = app
        .call(as_member(Call::post("/family/talk"), SAM).json(&json!({ "text": "  hi  " })))
        .await;
    assert_eq!(said.status, 200);
    let said: FamilyTalkReply = said.read_as();
    assert!(said.ok);
    app.settle().await;
    let turns = app.assistant.turns();
    assert_eq!(
        turns.last().unwrap().room,
        RoomId::new(i64::from(mine.room))
    );

    let now = state(&app, SAM, "").await;
    let lines: Vec<(bool, &str)> = now.chat.iter().map(|l| (l.mine, l.text.as_str())).collect();
    assert_eq!(lines, vec![(true, "hi"), (false, "Hello, Sam.")]);
    let reply = now.chat[1].id;
    let older = app
        .call(as_member(Call::get("/family/history"), SAM).query(&format!(
            "before={}&room={}",
            i64::from(reply),
            i64::from(now.room)
        )))
        .await;
    assert_eq!(older.status, 200);
    let older: FamilyHistory = older.read_as();
    assert_eq!(older.chat.len(), 1);
    assert!(older.chat[0].mine);

    // Another family member has a room of their own, and can't pick Sam's.
    let lee = state(&app, LEE, &format!("room={}", i64::from(now.room))).await;
    assert_eq!(lee.me, "lee");
    assert_ne!(lee.room, now.room);
    assert_eq!(lee.chat, Vec::new());

    let seen = app.call(as_member(Call::post("/family/seen"), SAM)).await;
    assert_eq!(seen.status, 200);
    assert_eq!(seen.json()["ok"], true);
}

#[tokio::test]
#[ignore = "family rooms are off (`FAMILY_ROOMS`) until tools scoped to a family member's own room exist"]
pub(crate) async fn only_a_family_member_from_this_machine_gets_in() {
    let app = TestApp::new().await;
    for (method, path) in [
        ("GET", "/family/state"),
        ("GET", "/family/history"),
        ("GET", "/family/live"),
        ("POST", "/family/talk"),
        ("POST", "/family/devices"),
        ("POST", "/family/seen"),
        ("GET", "/family/theme"),
        ("POST", "/family/theme"),
    ] {
        let call = |key: Option<&str>| {
            let call = if method == "GET" {
                Call::get(path)
            } else {
                Call::post(path).json(&json!({}))
            };
            match key {
                Some(key) => as_member(call, key),
                None => call,
            }
        };
        let refused = app.call(call(None)).await;
        assert_eq!(refused.status, 401, "{method} {path}");
        assert_eq!(
            refused.json(),
            json!({ "error": "Only a family member's own devices." })
        );
        let session = app.sign_in().await;
        let owner = app.call(call(None).token(&session)).await;
        assert_eq!(owner.status, 401, "the owner's session isn't a family key");

        let mut from_afar = call(Some(SAM)).request();
        from_afar
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 7], 50_000))));
        let response = app.send(from_afar).await;
        assert_eq!(response.status(), 403, "{method} {path}");

        let mut local = call(Some(SAM)).request();
        local
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
        assert_ne!(app.send(local).await.status(), 403, "{method} {path}");

        // This machine over an IPv6 socket that takes IPv4 too: `::ffff:127.0.0.1`.
        let mut mapped = call(Some(SAM)).request();
        let v4_in_v6: std::net::Ipv6Addr = std::net::Ipv4Addr::LOCALHOST.to_ipv6_mapped();
        mapped
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((v4_in_v6, 50_000))));
        assert_ne!(app.send(mapped).await.status(), 403, "{method} {path}");
    }
}

#[tokio::test]
#[ignore = "family rooms are off (`FAMILY_ROOMS`) until tools scoped to a family member's own room exist"]
pub(crate) async fn what_a_family_member_sends_is_checked() {
    let app = TestApp::new().await;
    let big = format!("{{\"text\":\"{}\"}}", "x".repeat(3_000_000));
    for path in ["/family/talk", "/family/devices", "/family/theme"] {
        let not_json = app
            .call(as_member(Call::post(path), SAM).raw("not json"))
            .await;
        assert_eq!(not_json.status, 400, "{path}");
        let too_big = app.call(as_member(Call::post(path), SAM).raw(&big)).await;
        assert_eq!(too_big.status, 413, "{path}");
    }
    let blank = app
        .call(as_member(Call::post("/family/talk"), SAM).json(&json!({ "text": " " })))
        .await;
    assert_eq!(blank.status, 400);
    assert_eq!(blank.json(), json!({ "error": "say something" }));

    let bad = app
        .call(as_member(Call::post("/family/devices"), SAM).json(&json!({ "token": "nope" })))
        .await;
    assert_eq!(bad.status, 400);
    assert_eq!(bad.json(), json!({ "error": "not a device token" }));
    let token = "ab".repeat(32);
    let kept = app
        .call(
            as_member(Call::post("/family/devices"), SAM)
                .json(&json!({ "token": token, "env": "production" })),
        )
        .await;
    assert_eq!(kept.status, 200);
    assert_eq!(kept.json(), json!({ "ok": true }));
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    assert_eq!(
        db.family().devices_of("sam").await.unwrap(),
        vec![(token, "production".to_owned())]
    );
    assert_eq!(
        db.push().apns_devices().await.unwrap(),
        Vec::new(),
        "kept apart from the owner's phones"
    );
}

#[tokio::test]
#[ignore = "family rooms are off (`FAMILY_ROOMS`) until tools scoped to a family member's own room exist"]
pub(crate) async fn a_full_room_or_a_stopping_server_refuses_more() {
    let app = TestApp::new().await;
    app.assistant.answers(&["Thinking"], None);
    app.assistant.hold();
    for n in 0..MAX_WAITING {
        let said = app
            .call(
                as_member(Call::post("/family/talk"), SAM)
                    .json(&json!({ "text": format!("message {n}") })),
            )
            .await;
        assert_eq!(said.status, 200);
    }
    let full = app
        .call(as_member(Call::post("/family/talk"), SAM).json(&json!({ "text": "more" })))
        .await;
    assert_eq!(full.status, 429);
    app.assistant.release();
    app.settle().await;
    app.state.stopping().cancel();
    let late = app
        .call(as_member(Call::post("/family/talk"), SAM).json(&json!({ "text": "late" })))
        .await;
    assert_eq!(late.status, 503);
}

#[tokio::test]
#[ignore = "family rooms are off (`FAMILY_ROOMS`) until tools scoped to a family member's own room exist"]
pub(crate) async fn a_family_member_s_theme_is_their_own() {
    let app = TestApp::new().await;
    let page = app.call(as_member(Call::get("/family/theme"), SAM)).await;
    assert_eq!(page.status, 200);
    let page: ThemePage = page.read_as();
    assert_eq!(page.choice.preset, "paper");
    let set = app
        .call(as_member(Call::post("/family/theme"), SAM).json(&json!({ "preset": "teal" })))
        .await;
    assert_eq!(set.status, 200);
    assert_eq!(set.json()["choice"]["preset"], "teal");
    let refused = app
        .call(as_member(Call::post("/family/theme"), SAM).json(&json!({ "preset": "plaid" })))
        .await;
    assert_eq!(refused.status, 400);
    assert_eq!(state(&app, SAM, "").await.theme.preset, "teal");
    assert_eq!(state(&app, LEE, "").await.theme.preset, "paper");
    assert_eq!(
        app.state.theme().choice().await.unwrap().preset,
        "paper",
        "the owner's"
    );
}

#[tokio::test]
#[ignore = "family rooms are off (`FAMILY_ROOMS`) until tools scoped to a family member's own room exist"]
pub(crate) async fn the_live_stream_follows_their_room() {
    let app = TestApp::new().await;
    let home = state(&app, SAM, "").await.room;
    let response = app
        .send(as_member(Call::get("/family/live"), SAM).request())
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body();
    let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(frame.into_data().unwrap(), "data: null\n\n");
    app.state.live().set(
        RoomId::new(i64::from(home)),
        Some(emberhut_api::live::Live {
            text: String::new(),
            doing: Some("Looking".to_owned()),
        }),
    );
    let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let data = frame.into_data().unwrap();
    assert!(std::str::from_utf8(&data).unwrap().contains("Looking"));
}

#[tokio::test]
#[ignore = "family rooms are off (`FAMILY_ROOMS`) until tools scoped to a family member's own room exist"]
pub(crate) async fn a_family_room_has_its_own_tools_stays_off_the_owner_s_list_and_pushes_to_its_own_phones()
 {
    let (apns, _fcm, senders) = crate::fake_push::FakePush::pair();
    let app = TestApp::with_push(emberhut_server::config::AuthMode::Enforce, senders).await;
    let session = app.sign_in().await;
    let owner_phone = "a".repeat(64);
    let registered = app
        .call(
            Call::post(emberhut_api::paths::DEVICES)
                .token(&session)
                .json(&json!({ "token": owner_phone })),
        )
        .await;
    assert_eq!(registered.status, 200);
    let sam_phone = "ab".repeat(32);
    let kept = app
        .call(
            as_member(Call::post("/family/devices"), SAM)
                .json(&json!({ "token": sam_phone, "env": "production" })),
        )
        .await;
    assert_eq!(kept.status, 200);

    app.assistant.answers(&[], Some("Hello, Sam."));
    let said = app
        .call(as_member(Call::post("/family/talk"), SAM).json(&json!({ "text": "hi" })))
        .await;
    assert_eq!(said.status, 200);
    app.settle().await;
    let sent: Vec<String> = apns.sent().into_iter().map(|(token, _)| token).collect();
    assert_eq!(sent, vec![sam_phone], "their reply, on their phone only");

    let home = app.assistant.turns().last().unwrap().room;
    let owner = app
        .call(Call::get(emberhut_api::paths::STATE).token(&session))
        .await;
    assert_eq!(owner.status, 200);
    let rooms: Vec<i64> = owner.json()["rooms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_i64().unwrap())
        .collect();
    assert!(!rooms.contains(&i64::from(home)), "{rooms:?}");
    assert!(rooms.contains(&1));

    let token = app.state.agent_tokens().for_room(home).unwrap();
    let listed = app
        .call(
            Call::post(emberhut_api::paths::MCP_ROOM)
                .header("authorization", &format!("Bearer {}", token.as_str()))
                .header("accept", "application/json, text/event-stream")
                .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })),
        )
        .await;
    assert_eq!(listed.status, 200, "{}", listed.text);
    let mut names: Vec<String> = listed.json()["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "calendar",
            "cancel_checkin",
            "fact_drop",
            "fact_set",
            "facts",
            "list_checkins",
            "recall",
            "room_send",
            "tell_owner",
            "wake_me_at",
        ]
    );
    // Their alert reaches the owner as theirs, opening Main.
    let alerted = app
        .call(
            Call::post(emberhut_api::paths::MCP_ROOM)
                .header("authorization", &format!("Bearer {}", token.as_str()))
                .header("accept", "application/json, text/event-stream")
                .json(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": { "name": "tell_owner", "arguments": { "title": "The door is open" } } })),
        )
        .await;
    assert_eq!(alerted.status, 200, "{}", alerted.text);
    let (to, alert) = apns.sent().last().cloned().unwrap();
    assert_eq!(to, owner_phone);
    assert_eq!(alert.title(), "From Sam: The door is open");
    assert_eq!(alert.room(), Some(RoomId::MAIN));
    let main = app.rpc("tools/list", json!({})).await.json();
    assert!(
        main["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "task_create")
    );
}
