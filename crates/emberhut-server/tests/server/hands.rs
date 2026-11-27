//! The Mac tools over the fake Mac helper: an app asked for once and remembered, a window read
//! with its picture, a final step waiting for the owner's Go ahead, no secrets typed, Terminal
//! only looked at, menus, and the whole screen started on an idle Mac.

use serde_json::json;

use crate::common::{TestApp, text};

#[tokio::test]
async fn mac_apps_lists_the_apps_and_says_when_the_helper_is_off() {
    let app = TestApp::new().await;
    let off = app.call_tool("mac_apps", json!({})).await;
    assert!(
        text(&off).starts_with("Mac: The Mac helper isn't answering"),
        "{}",
        text(&off)
    );
    app.mac.hands_answer(
        "/apps",
        json!({ "text": "Pages: 1 window\nFinder: 2 windows" }),
    );
    let apps = app.call_tool("mac_apps", json!({})).await;
    assert_eq!(text(&apps), "Pages: 1 window\nFinder: 2 windows");
}

#[tokio::test]
async fn mac_app_open_asks_once_for_an_app_and_remembers_it() {
    let app = TestApp::new().await;
    app.mac.hands_answer("/which", json!({ "name": "Pages" }));
    app.mac
        .hands_answer("/open", json!({ "text": "Opened Pages (hidden)." }));
    let unowned = app
        .call_tool("mac_app_open", json!({ "app": "pages" }))
        .await;
    assert!(
        text(&unowned).starts_with("Pages hasn't been allowed on the owner's Mac yet"),
        "{}",
        text(&unowned)
    );
    let asked = app
        .call_tool("mac_app_open", json!({ "app": "pages", "task_id": 30 }))
        .await;
    assert!(
        text(&asked).starts_with("Paused: the owner was asked first"),
        "{}",
        text(&asked)
    );
    assert!(
        app.desk
            .asks()
            .iter()
            .any(|(t, q)| *t == 30 && q.starts_with("Let the assistant use Pages on your Mac?"))
    );
    app.desk.choose(30, "Go ahead");
    let opened = app
        .call_tool("mac_app_open", json!({ "app": "pages", "task_id": 30 }))
        .await;
    assert_eq!(text(&opened), "Opened Pages (hidden).");
    let later = app
        .call_tool("mac_app_open", json!({ "app": "pages", "visible": true }))
        .await;
    assert_eq!(
        text(&later),
        "Opened Pages (hidden).",
        "remembered for every task"
    );
    assert_eq!(
        app.mac.hands_calls().last().unwrap().1,
        json!({ "app": "Pages", "hidden": false })
    );
    let chrome = app
        .call_tool("mac_app_open", json!({ "app": "Google Chrome" }))
        .await;
    assert!(
        text(&chrome).starts_with("Chrome goes through the browser extension"),
        "{}",
        text(&chrome)
    );
}

#[tokio::test]
async fn mac_app_snapshot_mac_app_click_mac_app_type_and_mac_app_menu() {
    let app = TestApp::new().await;
    app.mac.hands_answer("/which", json!({ "name": "Finder" }));
    app.mac.hands_answer(
        "/snapshot",
        json!({ "text": "[a1] button \"Delete\"", "image": "/9j/AA==" }),
    );
    let snapshot = app
        .call_tool(
            "mac_app_snapshot",
            json!({ "app": "Finder", "picture": true, "task_id": 31 }),
        )
        .await;
    assert_eq!(
        snapshot.content.len(),
        2,
        "the window's picture and its elements"
    );
    assert!(
        std::fs::read_to_string(app.dir().join("mac/logs/task-31.md"))
            .unwrap()
            .contains("Looked at Finder")
    );

    app.mac
        .hands_answer("/describe", json!({ "label": "Delete", "kind": "button" }));
    app.mac
        .hands_answer("/press", json!({ "text": "Pressed Delete." }));
    let paused = app
        .call_tool(
            "mac_app_click",
            json!({ "app": "Finder", "ref": "a1", "task_id": 31 }),
        )
        .await;
    assert!(
        text(&paused).starts_with("Paused: the owner was asked"),
        "{}",
        text(&paused)
    );
    assert!(
        app.desk
            .asks()
            .iter()
            .any(|(_, q)| q.contains("\"Delete\" in Finder"))
    );
    app.desk.choose(31, "Go ahead");
    let pressed = app
        .call_tool(
            "mac_app_click",
            json!({ "app": "Finder", "ref": "a1", "task_id": 31 }),
        )
        .await;
    assert_eq!(text(&pressed), "Pressed Delete.");

    app.mac
        .hands_answer("/describe", json!({ "label": "Password", "secret": true }));
    let secret = app
        .call_tool(
            "mac_app_type",
            json!({ "app": "Finder", "ref": "a2", "text": "x" }),
        )
        .await;
    assert!(
        text(&secret).starts_with("That field wants a password or a code"),
        "{}",
        text(&secret)
    );
    app.mac
        .hands_answer("/describe", json!({ "label": "Name" }));
    app.mac
        .hands_refuse("/set", "That field can't be set directly");
    app.mac.hands_answer("/keys", json!({ "text": "Typed." }));
    let typed = app
        .call_tool(
            "mac_app_type",
            json!({ "app": "Finder", "ref": "a3", "text": "Notes", "key": "Tab" }),
        )
        .await;
    assert_eq!(text(&typed), "Typed. Typed.");
    let nothing = app
        .call_tool("mac_app_type", json!({ "app": "Finder" }))
        .await;
    assert_eq!(text(&nothing), "Nothing to type: give text, key or both.");

    app.mac
        .hands_answer("/menu", json!({ "text": "File: New, Open…, Export…" }));
    let menu = app
        .call_tool("mac_app_menu", json!({ "app": "Finder", "path": "File" }))
        .await;
    assert_eq!(text(&menu), "File: New, Open…, Export…");

    app.mac
        .hands_answer("/which", json!({ "name": "Terminal" }));
    let look = app
        .call_tool("mac_app_click", json!({ "app": "Terminal", "ref": "a1" }))
        .await;
    assert!(
        text(&look).contains("is only looked at on the Mac"),
        "{}",
        text(&look)
    );
}

#[tokio::test]
async fn mac_screen_starts_on_an_idle_mac_and_asks_otherwise() {
    let app = TestApp::new().await;
    let unowned = app
        .call_tool(
            "mac_screen",
            json!({ "action": "start", "reason": "drag a file" }),
        )
        .await;
    assert!(
        text(&unowned).starts_with("Driving the whole Mac needs a task"),
        "{}",
        text(&unowned)
    );
    app.mac
        .hands_answer("/health", json!({ "idle": 30, "stopped": 9000 }));
    let asked = app
        .call_tool(
            "mac_screen",
            json!({ "action": "start", "reason": "drag a file", "task_id": 40 }),
        )
        .await;
    assert!(
        text(&asked).starts_with("Paused: the owner was asked first"),
        "{}",
        text(&asked)
    );
    app.mac.hands_answer("/health", json!({ "idle": 600 }));
    app.mac.hands_answer(
        "/screen/start",
        json!({ "text": "Driving for 15 minutes." }),
    );
    let idle_app = TestApp::new().await;
    idle_app.mac.hands_answer("/health", json!({ "idle": 600 }));
    idle_app.mac.hands_answer(
        "/screen/start",
        json!({ "text": "Driving for 15 minutes." }),
    );
    let started = idle_app
        .call_tool(
            "mac_screen",
            json!({ "action": "start", "reason": "drag a file", "task_id": 41 }),
        )
        .await;
    assert_eq!(text(&started), "Driving for 15 minutes.");
    assert!(
        std::fs::read_to_string(idle_app.dir().join("mac/logs/task-41.md"))
            .unwrap()
            .contains("the Mac idle for 10 minutes")
    );
    idle_app.mac.hands_answer(
        "/screen/shot",
        json!({ "text": "1440x900", "image": "/9j/AA==" }),
    );
    let shot = idle_app
        .call_tool("mac_screen", json!({ "action": "shot" }))
        .await;
    assert_eq!(shot.content.len(), 2);
    let secret = idle_app
        .call_tool(
            "mac_screen",
            json!({ "action": "type", "element": "password field", "text": "x" }),
        )
        .await;
    assert!(
        text(&secret).starts_with("Never a password or a code"),
        "{}",
        text(&secret)
    );
    idle_app
        .mac
        .hands_answer("/screen/act", json!({ "text": "Clicked." }));
    let clicked = idle_app
        .call_tool(
            "mac_screen",
            json!({ "action": "click", "x": 10, "y": 20, "element": "the Downloads folder", "task_id": 41 }),
        )
        .await;
    assert_eq!(text(&clicked), "Clicked.");
    idle_app
        .mac
        .hands_answer("/screen/stop", json!({ "text": "Stopped." }));
    assert_eq!(
        text(
            &idle_app
                .call_tool("mac_screen", json!({ "action": "stop", "task_id": 41 }))
                .await
        ),
        "Stopped."
    );
}

#[tokio::test]
async fn chrome_is_refused_by_the_app_the_helper_finds_whatever_it_was_called() {
    let app = TestApp::new().await;
    for (asked, bundle) in [
        ("4242", "com.google.Chrome"),
        ("Work", "com.google.Chrome.app.abcdef"),
        ("Brave", "com.brave.Browser"),
    ] {
        app.mac.hands_answer(
            "/which",
            json!({ "name": "Browser", "bundle": bundle, "running": true }),
        );
        app.mac
            .hands_answer("/open", json!({ "text": "Opened Browser (hidden)." }));
        let refused = app
            .call_tool("mac_app_open", json!({ "app": asked, "task_id": 32 }))
            .await;
        assert!(
            text(&refused).starts_with("Chrome goes through the browser extension"),
            "{asked}: {}",
            text(&refused)
        );
        assert_eq!(
            app.mac.hands_calls().last().unwrap().0,
            "/which",
            "{asked}: nothing done after"
        );
    }
    assert!(app.desk.asks().is_empty(), "{:?}", app.desk.asks());
}

#[tokio::test]
async fn a_new_line_typed_into_an_app_that_sends_is_return() {
    let app = TestApp::new().await;
    app.mac.hands_answer(
        "/which",
        json!({ "name": "Messages", "bundle": "com.apple.MobileSMS" }),
    );
    app.mac.hands_answer("/keys", json!({ "text": "Typed." }));
    let owned = app
        .call_tool("mac_app_open", json!({ "app": "Messages", "task_id": 33 }))
        .await;
    assert!(text(&owned).starts_with("Paused"), "{}", text(&owned));
    app.desk.choose(33, "Go ahead");
    app.mac
        .hands_answer("/open", json!({ "text": "Opened Messages (hidden)." }));
    app.call_tool("mac_app_open", json!({ "app": "Messages", "task_id": 33 }))
        .await;
    let unowned = app
        .call_tool(
            "mac_app_type",
            json!({ "app": "Messages", "text": "see you\nat six" }),
        )
        .await;
    assert!(
        text(&unowned).contains("looks like a final step: do it for a task"),
        "{}",
        text(&unowned)
    );
    let paused = app
        .call_tool(
            "mac_app_type",
            json!({ "app": "Messages", "text": "see you\nat six", "task_id": 33 }),
        )
        .await;
    assert!(
        text(&paused).starts_with("Paused: the owner was asked"),
        "{}",
        text(&paused)
    );
    assert!(
        !app.mac
            .hands_calls()
            .iter()
            .any(|(path, _)| path == "/keys"),
        "nothing typed before the OK"
    );
    let plain = app
        .call_tool(
            "mac_app_type",
            json!({ "app": "Messages", "text": "one line", "task_id": 33 }),
        )
        .await;
    assert_eq!(text(&plain), "Typed.");
}

#[tokio::test]
async fn on_the_whole_screen_a_click_that_says_nothing_and_return_go_by_the_step_gate() {
    let app = TestApp::new().await;
    app.mac.hands_answer("/health", json!({ "idle": 600 }));
    app.mac
        .hands_answer("/screen/start", json!({ "text": "Driving." }));
    app.mac
        .hands_answer("/screen/act", json!({ "text": "Done." }));
    let started = app
        .call_tool(
            "mac_screen",
            json!({ "action": "start", "reason": "drag a file", "task_id": 42 }),
        )
        .await;
    assert_eq!(text(&started), "Driving.");
    let acted = || {
        app.mac
            .hands_calls()
            .iter()
            .filter(|(path, _)| path == "/screen/act")
            .count()
    };

    let click = json!({ "action": "click", "x": 10, "y": 20, "task_id": 42 });
    let unowned = app
        .call_tool("mac_screen", json!({ "action": "click", "x": 10, "y": 20 }))
        .await;
    assert!(
        text(&unowned).contains("says nothing of what it presses: do it for a task"),
        "{}",
        text(&unowned)
    );
    let paused = app.call_tool("mac_screen", click.clone()).await;
    assert!(
        text(&paused).starts_with("Paused: the owner was asked"),
        "{}",
        text(&paused)
    );
    assert!(
        app.desk
            .asks()
            .iter()
            .any(|(t, q)| *t == 42 && q.contains("click at 10,20 on the screen")),
        "{:?}",
        app.desk.asks()
    );
    assert_eq!(acted(), 0);
    app.desk.choose(42, "Go ahead");
    assert_eq!(text(&app.call_tool("mac_screen", click).await), "Done.");
    assert_eq!(acted(), 1);

    // The job's OK doesn't cover Return: it may send in the app in front.
    for key in ["Return", "cmd+return"] {
        let held = app
            .call_tool(
                "mac_screen",
                json!({ "action": "key", "key": key, "task_id": 42 }),
            )
            .await;
        assert!(
            text(&held).starts_with("Paused: the owner was asked"),
            "{key}: {}",
            text(&held)
        );
    }
    let typed = app
        .call_tool(
            "mac_screen",
            json!({ "action": "type", "text": "hi\n", "task_id": 42 }),
        )
        .await;
    assert!(
        text(&typed).starts_with("Still waiting") || text(&typed).starts_with("Paused"),
        "{}",
        text(&typed)
    );
    assert_eq!(acted(), 1, "no key pressed and nothing typed");
    let tab = app
        .call_tool(
            "mac_screen",
            json!({ "action": "key", "key": "Tab", "task_id": 42 }),
        )
        .await;
    assert_eq!(text(&tab), "Done.");
}
