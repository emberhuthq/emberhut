//! The tools across rooms and to the owner: `room_send`, `rooms`, `tell_owner`, `send_file`
//! and `set_routines`.

use emberhut_core::RoomId;
use emberhut_core::push::Alert;
use emberhut_server::config::AuthMode;
use serde_json::json;

use crate::common::{TestApp, text};
use crate::fake_push::FakePush;

#[tokio::test]
async fn room_send_hands_work_to_another_room_and_rooms_says_what_was_said() {
    let app = TestApp::new().await;
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let atlas = db.rooms().create("Atlas", app.clock_now()).await.unwrap();
    app.assistant.answers(&[], Some("Hello from the assistant"));
    app.state
        .talk()
        .talk(RoomId::MAIN, "Morning".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;

    let sent = app
        .call_tool(
            "room_send",
            json!({ "room": "atlas", "text": "Ship the fix today" }),
        )
        .await;
    assert_ne!(sent.is_error, Some(true));
    assert_eq!(text(&sent), "Sent to the Atlas room; it wakes with it.");
    let waiting = db.events().waiting(atlas.id, 100).await.unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].kind, "room");
    assert_eq!(waiting[0].text, "From the Main room: Ship the fix today");

    for (room, why) in [
        ("Main", "Not sent: that's this room"),
        ("Nowhere", "Not sent: no room \"Nowhere\""),
    ] {
        let refused = app
            .call_tool("room_send", json!({ "room": room, "text": "something" }))
            .await;
        assert_eq!(refused.is_error, Some(true), "{room}");
        assert_eq!(text(&refused), why);
    }
    let short = app
        .call_tool("room_send", json!({ "room": "atlas", "text": "hi" }))
        .await;
    assert_eq!(short.is_error, Some(true), "too short a text is refused");

    let rooms = app.call_tool("rooms", json!({})).await;
    let said = text(&rooms);
    assert!(said.starts_with("## Main\n["), "{said}");
    assert!(said.contains("] Owner: Morning"), "{said}");
    assert!(
        said.contains("] Assistant: Hello from the assistant"),
        "{said}"
    );
    assert!(said.contains("\n\nNow:\n"), "{said}");
    assert!(said.contains("\"name\": \"Atlas\""), "{said}");
    let only = app.call_tool("rooms", json!({ "room": "atlas" })).await;
    assert!(text(&only).starts_with("Nothing said in any room since then."));
    let none = app.call_tool("rooms", json!({ "room": "nowhere" })).await;
    assert_eq!(text(&none), "No room \"nowhere\".");
    let later = app
        .call_tool("rooms", json!({ "since": "2999-01-01T00:00:00Z" }))
        .await;
    assert!(text(&later).starts_with("Nothing said in any room since then."));
    let did: Vec<String> = app
        .state
        .state()
        .state(RoomId::MAIN)
        .await
        .unwrap()
        .did
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert!(did.contains(&"Handed to the Atlas room: Ship the fix today".to_owned()));
}

#[tokio::test]
async fn tell_emberhut_alerts_the_phones() {
    let app = TestApp::new().await;
    let none = app
        .call_tool("tell_owner", json!({ "title": "The site is down" }))
        .await;
    assert_eq!(
        text(&none),
        "Push is not set up on the server; say it in the chat."
    );

    let (apns, _fcm, senders) = FakePush::pair();
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let phone = app.sign_in().await;
    app.state
        .push()
        .register(
            &serde_json::from_value(json!({ "token": "ab".repeat(32) })).unwrap(),
            phone.device,
        )
        .await
        .unwrap();
    let sent = app
        .call_tool(
            "tell_owner",
            json!({ "title": "The site is down", "detail": "502 since 10:04" }),
        )
        .await;
    assert_eq!(text(&sent), "Sent.");
    let alerts: Vec<Alert> = apns.sent().into_iter().map(|(_, alert)| alert).collect();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].title(), "The site is down");
    assert_eq!(alerts[0].body(), "502 since 10:04");
}

#[tokio::test]
async fn send_file_goes_out_with_the_reply() {
    let app = TestApp::new().await;
    let dir = shown_dir();
    let picture = dir.path().join("chart.png");
    std::fs::write(&picture, b"not really a png").unwrap();

    let relative = app
        .call_tool("send_file", json!({ "path": "chart.png" }))
        .await;
    assert_eq!(relative.is_error, Some(true));
    let missing = app
        .call_tool("send_file", json!({ "path": "/nowhere/chart.png" }))
        .await;
    assert_eq!(missing.is_error, Some(true));
    assert!(
        text(&missing).starts_with("Not sent: "),
        "{}",
        text(&missing)
    );

    let attached = app
        .call_tool(
            "send_file",
            json!({ "path": picture.display().to_string() }),
        )
        .await;
    assert_eq!(
        text(&attached),
        "Attached chart.png; it goes out with your reply."
    );
    app.assistant.answers(&[], Some("Here is the chart."));
    app.state
        .talk()
        .talk(RoomId::MAIN, "Show me".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let chat = app.state.state().state(RoomId::MAIN).await.unwrap().chat;
    let reply = chat.last().unwrap();
    assert_eq!(reply.text, "Here is the chart.");
    assert_eq!(reply.files.len(), 1);
    assert_eq!(reply.files[0].name, "chart.png");

    // A file sent on a turn that says nothing else still reaches the chat.
    app.call_tool(
        "send_file",
        json!({ "path": picture.display().to_string() }),
    )
    .await;
    app.assistant.answers(&[], Some("NO_REPLY"));
    app.state
        .talk()
        .talk(RoomId::MAIN, "Again".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let chat = app.state.state().state(RoomId::MAIN).await.unwrap().chat;
    let reply = chat.last().unwrap();
    assert_eq!(reply.text, "");
    assert_eq!(reply.files.len(), 1);
}

/// A folder for files to show, not hidden (a temporary folder's default name starts with a
/// dot, and `send_file` never sends a hidden file).
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure fails the test"
)]
fn shown_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("send-file-")
        .tempdir()
        .unwrap()
}

#[tokio::test]
async fn send_file_refuses_a_secret_directly_and_through_a_link() {
    let app = TestApp::new().await;
    let secrets = app.state.secret_links().secrets_dir().to_path_buf();
    std::fs::create_dir_all(&secrets).unwrap();
    let key = secrets.join("openai.key");
    std::fs::write(&key, b"sk-not-a-real-key").unwrap();
    let dir = shown_dir();
    let link = dir.path().join("chart.png");
    std::os::unix::fs::symlink(&key, &link).unwrap();
    let folder_link = dir.path().join("reports");
    std::os::unix::fs::symlink(&secrets, &folder_link).unwrap();

    for path in [key.clone(), link, folder_link.join("openai.key")] {
        let refused = app
            .call_tool("send_file", json!({ "path": path.display().to_string() }))
            .await;
        assert_eq!(refused.is_error, Some(true), "{}", path.display());
        assert_eq!(
            text(&refused),
            "Not sent: that file never leaves the server",
            "{}",
            path.display()
        );
    }
    let climbing = format!(
        "{}/../secrets/openai.key",
        app.state.uploads().dir().display()
    );
    let refused = app
        .call_tool("send_file", json!({ "path": climbing }))
        .await;
    assert_eq!(refused.is_error, Some(true), "{climbing}");

    let hidden = dir.path().join(".env");
    std::fs::write(&hidden, b"TOKEN=x").unwrap();
    let refused = app
        .call_tool("send_file", json!({ "path": hidden.display().to_string() }))
        .await;
    assert_eq!(
        text(&refused),
        "Not sent: that file is hidden, or in a hidden folder"
    );

    let picture = dir.path().join("photo.png");
    std::fs::write(&picture, b"a picture").unwrap();
    app.call_tool(
        "send_file",
        json!({ "path": picture.display().to_string() }),
    )
    .await;
    let kept = std::fs::read_dir(app.state.uploads().dir())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let again = app
        .call_tool("send_file", json!({ "path": kept.display().to_string() }))
        .await;
    assert_eq!(
        text(&again),
        "Not sent: that file is one of the chat's files already"
    );
}

#[tokio::test]
async fn send_file_refuses_what_a_runner_is_never_sent() {
    let app = TestApp::new().await;
    let token = app.dir().join(crate::common::PRIVATE_FILES[0]);
    std::fs::write(&token, b"runner-token").unwrap();
    let dir = shown_dir();
    for (name, target) in [
        ("db.png", app.dir().join("emberhut.db")),
        ("journal.png", app.dir().join("emberhut.db-wal")),
        ("token.png", token),
    ] {
        if !target.exists() {
            std::fs::write(&target, b"x").unwrap();
        }
        let link = dir.path().join(name);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let refused = app
            .call_tool("send_file", json!({ "path": link.display().to_string() }))
            .await;
        assert_eq!(
            text(&refused),
            "Not sent: that file never leaves the server",
            "{}",
            target.display()
        );
    }
}

#[tokio::test]
async fn set_routines_replaces_the_routines_and_the_quiet_hours() {
    let app = TestApp::new().await;
    let routines = app.state.routines();
    let defaults = routines.routines().await.unwrap();
    assert!(defaults[0].what.starts_with("Morning brief"));
    let saved = app
        .call_tool(
            "set_routines",
            json!({ "routines": [{ "at": "07:15", "what": "Morning brief: short." }],
                "quiet": { "from": "23:00", "to": "07:00" } }),
        )
        .await;
    assert_eq!(text(&saved), "Saved.");
    let now = routines.routines().await.unwrap();
    assert_eq!(now.len(), 1);
    assert_eq!(now[0].at, "07:15");
    let quiet = routines.quiet().await.unwrap().unwrap();
    assert_eq!((quiet.from.as_str(), quiet.to.as_str()), ("23:00", "07:00"));
    let bad = app
        .call_tool(
            "set_routines",
            json!({ "routines": [{ "at": "7:15", "what": "x" }] }),
        )
        .await;
    assert_eq!(
        bad.is_error,
        Some(true),
        "a time not written HH:MM is refused"
    );
    assert_eq!(routines.routines().await.unwrap()[0].at, "07:15");
}
