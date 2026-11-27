//! The browser the assistant drives, over a scripted browser: the tools (reading a page with its
//! secrets masked, final steps that wait for the owner's OK, the hand-off and take-back, keys
//! saved unread, files in and out), the live view's routes, and the state's `browser`.

use emberhut_api::paths;
use serde_json::{Value, json};

use crate::common::{Call, TestApp, text};
use crate::fake_browser::{FakeElement, FakePage};

const LOGIN: &str = "https://example.com/login";
const KEY: &str = "xoxb-123456789012-abcdef";

/// The login page: a heading, two fields, a key shown on the page, a final step.
fn login() -> FakePage {
    FakePage {
        secrets: vec!["hunter2-secret".to_owned()],
        keys: vec![KEY.to_owned()],
        ..FakePage::new(
            LOGIN,
            "Sign in",
            &format!(
                "- heading \"Sign in\" [ref=e1]\n- textbox \"Email\" [ref=e2]: a@b.c\n- textbox \
                 \"Password\" [ref=e3]: hunter2-secret\n- text: your token {KEY} [ref=e4]\n- \
                 button \"Submit order\" [ref=e5]\n- button \"Upload\" [ref=e6]\n- button \"Create \
                 app\" [ref=e7]"
            ),
        )
    }
    .with(
        "e1",
        FakeElement {
            label: "Sign in".into(),
            ..FakeElement::default()
        },
    )
    .with(
        "e2",
        FakeElement {
            label: "a@b.c".into(),
            field: "email email user".into(),
            file_kind: "other".into(),
            ..FakeElement::default()
        },
    )
    .with(
        "e3",
        FakeElement {
            field: "password current-password pw".into(),
            key_at: json!({ "why": "password" }),
            ..FakeElement::default()
        },
    )
    .with(
        "e4",
        FakeElement {
            key_at: json!({ "value": KEY }),
            ..FakeElement::default()
        },
    )
    .with(
        "e5",
        FakeElement {
            label: "Submit order".into(),
            ..FakeElement::default()
        },
    )
    .with(
        "e6",
        FakeElement {
            label: "Upload".into(),
            file_kind: "field".into(),
            ..FakeElement::default()
        },
    )
    .with(
        "e7",
        FakeElement {
            label: "Create app".into(),
            ..FakeElement::default()
        },
    )
}

async fn opened(app: &TestApp) -> String {
    app.browser.site(login());
    let result = app
        .call_tool("browser_open", json!({ "url": "example.com/login" }))
        .await;
    text(&result).to_owned()
}

#[tokio::test]
async fn browser_open_and_browser_snapshot_read_the_page_with_passwords_and_keys_masked() {
    let app = TestApp::new().await;
    let page = opened(&app).await;
    assert!(
        page.starts_with("Sign in · https://example.com/login"),
        "{page}"
    );
    assert!(
        page.contains("- textbox \"Password\" [ref=e3]: ••••••"),
        "{page}"
    );
    assert!(
        page.contains("[secret: xoxb-…, use browser_save_secret ref e4]"),
        "{page}"
    );
    assert!(!page.contains("hunter2") && !page.contains(KEY), "{page}");
    assert!(app.browser.steps().contains(&format!("goto {LOGIN}")));

    let snapshot = app
        .call_tool("browser_snapshot", json!({ "screenshot": true }))
        .await;
    assert_eq!(snapshot.content.len(), 2, "a picture and the tree");
    let tree = snapshot.content.iter().find_map(|c| c.as_text()).unwrap();
    assert!(tree.contains("[ref=e5]") && !tree.contains(KEY));
    let missing = app
        .call_tool("browser_snapshot", json!({ "scope": "e99" }))
        .await;
    assert!(
        text(&missing).contains("Nothing for scope \"e99\""),
        "{}",
        text(&missing)
    );
}

#[tokio::test]
async fn browser_click_on_a_final_step_waits_for_the_owner_s_go_ahead() {
    let app = TestApp::new().await;
    opened(&app).await;
    let unowned = app.call_tool("browser_click", json!({ "ref": "e5" })).await;
    assert!(
        text(&unowned).contains("looks like a final step"),
        "{}",
        text(&unowned)
    );

    let paused = app
        .call_tool("browser_click", json!({ "ref": "e5", "task_id": 7 }))
        .await;
    assert!(
        text(&paused).starts_with("Paused: the owner was asked"),
        "{}",
        text(&paused)
    );
    let asked = app.desk.asks();
    assert!(
        asked
            .iter()
            .any(|(t, q)| *t == 7 && q.contains("\"Submit order\"")),
        "{asked:?}"
    );
    assert!(!app.browser.steps().contains(&"click e5".to_owned()));
    let waiting = app
        .call_tool("browser_click", json!({ "ref": "e5", "task_id": 7 }))
        .await;
    assert!(
        text(&waiting).starts_with("Still waiting"),
        "{}",
        text(&waiting)
    );

    app.desk.choose(7, "Go ahead");
    let clicked = app
        .call_tool("browser_click", json!({ "ref": "e5", "task_id": 7 }))
        .await;
    assert!(
        text(&clicked).starts_with("Clicked \"Submit order\" (OK'd: their Go ahead)"),
        "{}",
        text(&clicked)
    );
    assert!(app.browser.steps().contains(&"click e5".to_owned()));
    let logged = std::fs::read_to_string(app.dir().join("browser/logs/task-7.md")).unwrap();
    assert!(
        logged.contains("Paused before \"Submit order\"")
            && logged.contains("Clicked \"Submit order\"")
    );
    assert!(
        app.desk
            .attached()
            .iter()
            .any(|(t, p)| *t == 7 && p.ends_with("task-7.md"))
    );

    let plain = app.call_tool("browser_click", json!({ "ref": "e1" })).await;
    assert!(
        text(&plain).starts_with("Clicked \"Sign in\". Sign in · "),
        "{}",
        text(&plain)
    );
    let gone = app
        .call_tool("browser_click", json!({ "ref": "e40" }))
        .await;
    assert!(
        text(&gone).contains("No element e40 on this page now"),
        "{}",
        text(&gone)
    );
}

#[tokio::test]
async fn browser_plan_asks_once_for_the_job_and_its_go_ahead_covers_the_steps() {
    let app = TestApp::new().await;
    opened(&app).await;
    let planned = app
        .call_tool(
            "browser_plan",
            json!({ "job": "Set up the test app", "steps": ["Create app", "Pay now"], "task_id": 8 }),
        )
        .await;
    let said = text(&planned);
    assert!(
        said.contains("Asked the owner once for the whole job"),
        "{said}"
    );
    assert!(said.contains("\"Pay now\" still asks"), "{said}");
    assert!(
        app.desk
            .asks()
            .iter()
            .any(|(t, q)| *t == 8 && q.contains("Set up the test app"))
    );
    let paused = app
        .call_tool("browser_click", json!({ "ref": "e7", "task_id": 8 }))
        .await;
    assert!(
        text(&paused).contains("waits for the owner's OK on the job you planned"),
        "{}",
        text(&paused)
    );
    app.desk.choose(8, "Go ahead");
    let clicked = app
        .call_tool("browser_click", json!({ "ref": "e7", "task_id": 8 }))
        .await;
    assert!(
        text(&clicked).contains("(OK'd: their Go ahead on the job"),
        "{}",
        text(&clicked)
    );
    let bad = app
        .call_tool(
            "browser_plan",
            json!({ "job": "x", "steps": [], "task_id": 8 }),
        )
        .await;
    assert_eq!(bad.is_error, Some(true));
}

#[tokio::test]
async fn browser_type_refuses_secrets_and_browser_press_waits_on_a_final_form() {
    let app = TestApp::new().await;
    opened(&app).await;
    let secret = app
        .call_tool("browser_type", json!({ "ref": "e3", "text": "x" }))
        .await;
    assert!(
        text(&secret).contains("Never type those"),
        "{}",
        text(&secret)
    );
    let typed = app
        .call_tool(
            "browser_type",
            json!({ "ref": "e2", "text": "hello", "submit": true }),
        )
        .await;
    assert_eq!(text(&typed), "Pressed Enter.");
    assert!(
        app.browser
            .steps()
            .ends_with(&["fill e2 hello".to_owned(), "press Enter".to_owned()])
    );

    app.browser.site(
        FakePage {
            form_submit: "Send".into(),
            ..FakePage::new("https://mail.example.com/", "Mail", "- textbox [ref=e1]")
        }
        .with("e1", FakeElement::default()),
    );
    app.call_tool(
        "browser_open",
        json!({ "url": "https://mail.example.com/" }),
    )
    .await;
    let paused = app
        .call_tool("browser_press", json!({ "key": "Enter", "task_id": 5 }))
        .await;
    assert!(
        text(&paused).ends_with("(Pressing Enter sends the form \"Send\": press it again then.)"),
        "{}",
        text(&paused)
    );
    let tab = app
        .call_tool("browser_press", json!({ "key": "Tab" }))
        .await;
    assert_eq!(text(&tab), "Pressed Tab.");
    // With no task, Enter on that form is refused as its button is: nothing is sent.
    let before = app.browser.steps().len();
    for (tool, args) in [
        ("browser_press", json!({ "key": "Enter" })),
        (
            "browser_type",
            json!({ "ref": "e1", "text": "hi", "submit": true }),
        ),
    ] {
        let refused = app.call_tool(tool, args).await;
        assert!(
            text(&refused).contains("looks like a final step. Browse for a task"),
            "{}",
            text(&refused)
        );
    }
    assert!(
        !app.browser.steps()[before..].contains(&"press Enter".to_owned()),
        "{:?}",
        app.browser.steps()
    );
}

#[tokio::test]
async fn a_blocked_site_is_never_opened_and_a_tab_that_reaches_one_is_let_go_of() {
    let app = TestApp::new().await;
    opened(&app).await;
    let before = app.browser.steps().len();
    let open = app
        .call_tool(
            "browser_open",
            json!({ "url": "https://my.1password.com/vault" }),
        )
        .await;
    assert!(
        text(&open).starts_with("my.1password.com is a blocked site"),
        "{}",
        text(&open)
    );
    let tab = app
        .call_tool(
            "browser_tabs",
            json!({ "action": "new", "url": "passwords.google.com" }),
        )
        .await;
    assert!(
        text(&tab).starts_with("passwords.google.com is a blocked site"),
        "{}",
        text(&tab)
    );
    assert!(
        app.browser.steps()[before..].is_empty(),
        "{:?}",
        app.browser.steps()
    );

    // A link on an allowed page that lands on one: the tab is closed, and the agent told.
    app.browser.site(
        FakePage::new(
            "https://example.com/links",
            "Links",
            "- link \"Vault\" [ref=e1]",
        )
        .with(
            "e1",
            FakeElement {
                label: "Vault".into(),
                goes_to: "https://my.1password.com/vault".into(),
                ..FakeElement::default()
            },
        ),
    );
    app.call_tool(
        "browser_open",
        json!({ "url": "https://example.com/links" }),
    )
    .await;
    let clicked = app.call_tool("browser_click", json!({ "ref": "e1" })).await;
    assert!(
        text(&clicked).starts_with("That tab went to my.1password.com, a blocked site"),
        "{}",
        text(&clicked)
    );
    assert!(
        app.browser.steps().last().unwrap().starts_with("close tab"),
        "{:?}",
        app.browser.steps()
    );
    let next = app.call_tool("browser_snapshot", json!({})).await;
    assert!(!text(&next).contains("1password"), "{}", text(&next));
}

#[tokio::test]
async fn enter_in_a_chat_composer_outside_any_form_waits_for_the_owner_s_ok() {
    let app = TestApp::new().await;
    app.browser.site(
        FakePage {
            focus: "composer".into(),
            ..FakePage::new(
                "https://chat.example.com/",
                "Chat",
                "- textbox \"Message\" [ref=e1]",
            )
        }
        .with("e1", FakeElement::default()),
    );
    app.call_tool(
        "browser_open",
        json!({ "url": "https://chat.example.com/" }),
    )
    .await;
    for key in ["Enter", "Control+Enter", "Meta+Enter", "Shift+Enter"] {
        let refused = app.call_tool("browser_press", json!({ "key": key })).await;
        assert!(
            text(&refused).starts_with(&format!(
                "Pressing {key} here can send what is written in this box"
            )) && text(&refused).ends_with("Browse for a task (task_id) so the owner can OK it."),
            "{}",
            text(&refused)
        );
    }
    let typed = app
        .call_tool(
            "browser_type",
            json!({ "ref": "e1", "text": "hi", "submit": true }),
        )
        .await;
    assert!(
        text(&typed).contains("Browse for a task"),
        "{}",
        text(&typed)
    );
    assert!(
        !app.browser
            .steps()
            .iter()
            .any(|s| s.starts_with("press") && s.contains("Enter")),
        "{:?}",
        app.browser.steps()
    );

    let paused = app
        .call_tool("browser_press", json!({ "key": "Enter", "task_id": 9 }))
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
            .any(|(t, q)| *t == 9 && q.contains("press Enter in a text box on chat.example.com")),
        "{:?}",
        app.desk.asks()
    );
    app.desk.choose(9, "Go ahead");
    let sent = app
        .call_tool("browser_press", json!({ "key": "Enter", "task_id": 9 }))
        .await;
    assert_eq!(text(&sent), "Pressed Enter.");

    // A search field sends nothing: Enter needs no OK there.
    app.browser.site(FakePage {
        focus: "search".into(),
        ..FakePage::new("https://example.com/find", "Find", "- searchbox [ref=e1]")
    });
    app.call_tool("browser_open", json!({ "url": "https://example.com/find" }))
        .await;
    let searched = app
        .call_tool("browser_press", json!({ "key": "Enter" }))
        .await;
    assert_eq!(text(&searched), "Pressed Enter.");
}

#[tokio::test]
async fn browser_tabs_browser_scroll_browser_wait_for_browser_autofill_and_browser_close() {
    let app = TestApp::new().await;
    opened(&app).await;
    // Its address goes through browser_open's: no scheme is https.
    let tabs = app
        .call_tool(
            "browser_tabs",
            json!({ "action": "new", "url": "example.com/login" }),
        )
        .await;
    assert!(
        text(&tabs)
            .starts_with("0: https://example.com/login\n1 (in front): https://example.com/login"),
        "{}",
        text(&tabs)
    );
    let selected = app
        .call_tool("browser_tabs", json!({ "action": "select", "index": 0 }))
        .await;
    assert!(
        text(&selected).starts_with("0 (in front): "),
        "{}",
        text(&selected)
    );
    assert_eq!(
        text(&app.call_tool("browser_scroll", json!({ "dy": -300 })).await),
        "Scrolled."
    );
    assert!(app.browser.steps().contains(&"wheel -300".to_owned()));
    let ready = app
        .call_tool(
            "browser_wait_for",
            json!({ "text": "Sign in", "seconds": 1 }),
        )
        .await;
    assert!(
        text(&ready).starts_with("Ready. Sign in · "),
        "{}",
        text(&ready)
    );
    let never = app
        .call_tool(
            "browser_wait_for",
            json!({ "text": "never there", "seconds": 1 }),
        )
        .await;
    assert!(
        text(&never).starts_with("Browser: \"never there\" didn't show"),
        "{}",
        text(&never)
    );
    let filled = app.call_tool("browser_autofill", json!({})).await;
    assert!(
        text(&filled).starts_with("Asked the password manager"),
        "{}",
        text(&filled)
    );
    assert!(
        app.browser
            .steps()
            .contains(&"press Control+Backslash".to_owned())
    );
    assert_eq!(
        text(&app.call_tool("browser_close", json!({})).await),
        "Closed the tab."
    );
}

#[tokio::test]
async fn browser_handoff_gives_the_owner_the_browser_and_browser_takeback_ends_their_turn() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    opened(&app).await;
    let handed = app
        .call_tool(
            "browser_handoff",
            json!({ "reason": "Sign in", "task_id": 9 }),
        )
        .await;
    assert!(
        text(&handed).starts_with("The owner has the browser now"),
        "{}",
        text(&handed)
    );
    assert_eq!(
        app.desk.action(9).as_deref(),
        Some("In the assistant's browser: Sign in")
    );
    assert!(app.desk.wakes().iter().any(|(kind, _)| kind == "ask"));
    assert!(app.browser.owner_driving());
    let busy = app.call_tool("browser_snapshot", json!({})).await;
    assert!(
        text(&busy).starts_with("The owner has the browser (Sign in)"),
        "{}",
        text(&busy)
    );
    let state = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .json();
    assert_eq!(state["browser"]["control"], "owner");
    assert_eq!(state["browser"]["handoff"], "Sign in");

    let back = app
        .call(
            Call::post(paths::BROWSER_INPUT)
                .token(&session)
                .json(&json!({ "kind": "handback" })),
        )
        .await;
    assert_eq!(back.status, 200);
    assert_eq!(back.json()["control"], "agent");
    assert!(
        app.desk
            .wakes()
            .iter()
            .any(|(kind, said)| kind == "browser"
                && said.contains("(you asked: Sign in) for task 9"))
    );
    assert_eq!(app.desk.action(9), None);
    assert!(!app.browser.owner_driving());

    app.call_tool(
        "browser_handoff",
        json!({ "reason": "Type the code", "task_id": 9 }),
    )
    .await;
    let took = app
        .call_tool(
            "browser_takeback",
            json!({ "reason": "you're signed in", "task_id": 9 }),
        )
        .await;
    assert!(
        text(&took)
            .starts_with("Took the browser back from the owner (they were told: you're signed in)"),
        "{}",
        text(&took)
    );
    let again = app
        .call_tool("browser_takeback", json!({ "reason": "x" }))
        .await;
    assert!(
        text(&again).starts_with("The owner isn't driving"),
        "{}",
        text(&again)
    );
}

#[tokio::test]
pub(crate) async fn the_live_view_s_input_takes_over_drives_and_hands_back() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    opened(&app).await;
    let refused = app
        .call(
            Call::post(paths::BROWSER_INPUT)
                .token(&session)
                .json(&json!({ "kind": "back" })),
        )
        .await;
    assert_eq!(refused.status, 409);
    assert_eq!(refused.json()["error"], "Take over first");
    let took = app
        .call(
            Call::post(paths::BROWSER_INPUT)
                .token(&session)
                .json(&json!({ "kind": "takeover", "device": "phone-1" })),
        )
        .await;
    assert_eq!(took.status, 200);
    assert_eq!(
        (
            took.json()["ok"].clone(),
            took.json()["control"].clone(),
            took.json()["device"].clone()
        ),
        (json!(true), json!("owner"), json!("phone-1"))
    );
    let went = app
        .call(
            Call::post(paths::BROWSER_INPUT)
                .token(&session)
                .json(&json!({ "kind": "go", "url": "example.org" })),
        )
        .await;
    assert_eq!(went.status, 200);
    assert!(
        app.browser
            .steps()
            .contains(&"goto https://example.org".to_owned())
    );
    let unknown = app
        .call(
            Call::post(paths::BROWSER_INPUT)
                .token(&session)
                .json(&json!({ "kind": "fly" })),
        )
        .await;
    assert_eq!(unknown.status, 409);
    let back = app
        .call(
            Call::post(paths::BROWSER_INPUT)
                .token(&session)
                .json(&json!({ "kind": "handback" })),
        )
        .await;
    assert_eq!(back.json()["control"], "agent");
}

#[tokio::test]
pub(crate) async fn the_live_view_s_session_is_view_or_control_and_503_until_set_up() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let missing = app
        .call(Call::get(paths::BROWSER_SESSION).token(&session))
        .await;
    assert_eq!(missing.status, 503);
    let secrets = app.dir().join("secrets");
    std::fs::create_dir_all(&secrets).unwrap();
    std::fs::write(secrets.join("browser-view"), "watch-pass\n").unwrap();
    std::fs::write(secrets.join("browser-control"), "drive-pass").unwrap();
    let view = app
        .call(
            Call::get(paths::BROWSER_SESSION)
                .query("device=phone-1")
                .token(&session),
        )
        .await;
    assert_eq!(view.status, 200);
    assert_eq!(view.json()["mode"], "view");
    assert_eq!(view.json()["password"], "watch-pass");
    assert_eq!(view.json()["url"], "https://view.example.test/");
    app.call(
        Call::post(paths::BROWSER_INPUT)
            .token(&session)
            .json(&json!({ "kind": "takeover", "device": "phone-1" })),
    )
    .await;
    let control = app
        .call(
            Call::get(paths::BROWSER_SESSION)
                .query("device=phone-1")
                .token(&session),
        )
        .await;
    assert_eq!(
        (
            control.json()["mode"].clone(),
            control.json()["password"].clone()
        ),
        (json!("control"), json!("drive-pass"))
    );
    let other = app
        .call(
            Call::get(paths::BROWSER_SESSION)
                .query("device=laptop")
                .token(&session),
        )
        .await;
    assert_eq!(other.json()["mode"], "view");
}

#[tokio::test]
pub(crate) async fn the_frame_is_the_tab_s_picture_and_none_while_the_owner_drives() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    opened(&app).await;
    let frame = app
        .call(Call::get(paths::BROWSER_FRAME).token(&session))
        .await;
    assert_eq!(frame.status, 200);
    assert_eq!(frame.header("content-type"), "image/jpeg");
    app.call(
        Call::post(paths::BROWSER_INPUT)
            .token(&session)
            .json(&json!({ "kind": "takeover" })),
    )
    .await;
    let none = app
        .call(Call::get(paths::BROWSER_FRAME).token(&session))
        .await;
    assert_eq!(none.status, 404);
}

#[tokio::test]
pub(crate) async fn the_app_s_file_goes_into_the_page_that_asked_for_one() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    opened(&app).await;
    let unasked = app
        .call(
            Call::post(paths::BROWSER_UPLOAD)
                .query("name=photo.jpg")
                .token(&session)
                .raw("jpeg bytes"),
        )
        .await;
    assert_eq!(unasked.status, 409);
    assert_eq!(unasked.json()["error"], "The page isn't asking for a file");
    app.browser.ask_for_file(false);
    let state = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .json();
    assert_eq!(state["browser"]["fileRequest"]["multiple"], false);
    let sent = app
        .call(
            Call::post(paths::BROWSER_UPLOAD)
                .query("name=../photo.jpg")
                .token(&session)
                .raw("jpeg bytes"),
        )
        .await;
    assert_eq!(sent.status, 200, "{}", sent.text);
    assert_eq!(sent.json()["fileRequest"], Value::Null);
    assert!(app.browser.files().contains(&("-photo.jpg".to_owned(), 10)));
    assert!(app.dir().join("browser/uploads/-photo.jpg").exists());
}

#[tokio::test]
pub(crate) async fn the_browser_routes_need_a_session() {
    let app = TestApp::new().await;
    for call in [
        Call::get(paths::BROWSER_SESSION),
        Call::get(paths::BROWSER_FRAME),
        Call::post(paths::BROWSER_INPUT).json(&json!({ "kind": "takeover" })),
        Call::post(paths::BROWSER_UPLOAD).raw("x"),
        Call::post(paths::CHROME).json(&json!({ "kind": "add", "label": "Work" })),
    ] {
        assert_eq!(app.call(call).await.status, 401);
    }
    // Report mode lets a request without a session read the frame, never get the live view's
    // password, drive the browser, put a file in it or mint a pairing link.
    let report = TestApp::with_mode(emberhut_server::config::AuthMode::Report).await;
    for call in [
        Call::get(paths::BROWSER_SESSION),
        Call::post(paths::BROWSER_INPUT).json(&json!({ "kind": "takeover" })),
        Call::post(paths::BROWSER_UPLOAD).raw("x"),
        Call::post(paths::CHROME).json(&json!({ "kind": "add", "label": "Work" })),
    ] {
        let refused = report.call(call).await;
        assert_eq!(refused.status, 401, "{}", refused.text);
        assert_eq!(refused.json()["signIn"], true);
    }
    assert_ne!(
        report.call(Call::get(paths::BROWSER_FRAME)).await.status,
        401
    );
}

#[tokio::test]
async fn browser_save_secret_moves_a_key_into_the_secrets_unread() {
    let app = TestApp::new().await;
    opened(&app).await;
    let unowned = app
        .call_tool(
            "browser_save_secret",
            json!({ "ref": "e4", "name": "slack-acme-bot" }),
        )
        .await;
    assert!(text(&unowned).contains("needs task_id"));
    let saved = app
        .call_tool("browser_save_secret", json!({ "ref": "e4", "name": "slack-acme-bot", "expect_prefix": "xoxb-", "task_id": 3 }))
        .await;
    assert_eq!(
        text(&saved),
        "Saved slack-acme-bot: 24 characters, starts with \"xoxb-\". Never read it back or repeat it; tell the owner in one line that it's saved."
    );
    assert_eq!(
        std::fs::read_to_string(app.dir().join("secrets/slack-acme-bot")).unwrap(),
        KEY
    );
    let again = app
        .call_tool(
            "browser_save_secret",
            json!({ "ref": "e4", "name": "slack-acme-bot", "task_id": 3 }),
        )
        .await;
    assert!(
        text(&again).starts_with("Already saved"),
        "{}",
        text(&again)
    );
    let password = app
        .call_tool(
            "browser_save_secret",
            json!({ "ref": "e3", "name": "pw-x", "task_id": 3 }),
        )
        .await;
    assert!(
        text(&password).starts_with("That's a password field"),
        "{}",
        text(&password)
    );
    let own = app
        .call_tool(
            "browser_save_secret",
            json!({ "ref": "e4", "name": "browser-control", "task_id": 3 }),
        )
        .await;
    assert!(
        text(&own).contains("not one of the server's own keys"),
        "{}",
        text(&own)
    );
    let wrong = app
        .call_tool(
            "browser_save_secret",
            json!({ "ref": "e4", "name": "gh-key", "expect_prefix": "ghp_", "task_id": 3 }),
        )
        .await;
    assert!(
        text(&wrong).starts_with("Not saved: it doesn't start with \"ghp_\""),
        "{}",
        text(&wrong)
    );
    let log = std::fs::read_to_string(app.dir().join("browser/logs/task-3.md")).unwrap();
    assert!(
        log.contains("Saved a key from example.com") && !log.contains(KEY),
        "{log}"
    );
    assert!(app.desk.notes().iter().all(|(_, n)| !n.contains(KEY)));
}

#[tokio::test]
async fn browser_upload_and_browser_download_move_files_by_the_rules() {
    let app = TestApp::new().await;
    opened(&app).await;
    let home = app.dir().join("home");
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join("Documents/logo.png"), b"\x89PNG picture").unwrap();
    std::fs::write(home.join(".ssh/key.png"), b"\x89PNG").unwrap();
    let put = app
        .call_tool(
            "browser_upload",
            json!({ "path": "~/Documents/logo.png", "ref": "e6", "task_id": 4 }),
        )
        .await;
    assert!(
        text(&put).starts_with(
            "Put logo.png (12 bytes) into the page's file field (\"Upload\") on example.com"
        ),
        "{}",
        text(&put)
    );
    assert!(app.browser.files().contains(&("logo.png".to_owned(), 12)));
    let hidden = app
        .call_tool(
            "browser_upload",
            json!({ "path": "~/.ssh/key.png", "ref": "e6", "task_id": 4 }),
        )
        .await;
    assert!(
        text(&hidden).contains("is never uploaded from there"),
        "{}",
        text(&hidden)
    );
    let text_field = app
        .call_tool(
            "browser_upload",
            json!({ "path": "~/Documents/logo.png", "ref": "e2", "task_id": 4 }),
        )
        .await;
    assert!(
        text(&text_field).starts_with("\"a@b.c\" is a text field, not a file field"),
        "{}",
        text(&text_field)
    );

    app.browser.file(
        "https://example.com/files/week1.pdf",
        "application/pdf",
        "",
        b"%PDF-1.7 notes",
    );
    let got = app
        .call_tool(
            "browser_download",
            json!({ "url": "/files/week1.pdf", "dest": "~/Downloads/", "task_id": 4 }),
        )
        .await;
    assert!(
        text(&got).starts_with("Saved week1.pdf (14 bytes) from example.com to "),
        "{}",
        text(&got)
    );
    assert_eq!(
        std::fs::read(home.join("Downloads/week1.pdf")).unwrap(),
        b"%PDF-1.7 notes"
    );
    let second = app
        .call_tool(
            "browser_download",
            json!({ "url": "/files/week1.pdf", "dest": "~/Downloads/", "task_id": 4 }),
        )
        .await;
    assert!(text(&second).contains("week1 (2).pdf"), "{}", text(&second));
    let secret = app
        .call_tool(
            "browser_download",
            json!({ "url": "/files/week1.pdf", "dest": "~/.ssh/", "task_id": 4 }),
        )
        .await;
    assert!(
        text(&secret).starts_with("Nothing is saved into"),
        "{}",
        text(&secret)
    );
    let missing = app
        .call_tool(
            "browser_download",
            json!({ "url": "/files/none.pdf", "dest": "/tmp/", "task_id": 4 }),
        )
        .await;
    assert!(
        text(&missing).contains("the site answered 404"),
        "{}",
        text(&missing)
    );
}

#[tokio::test]
async fn whatsapp_web_types_a_draft_reads_it_back_and_sends_it_on_its_own() {
    use emberhut_server::services::browser::whatsapp::{WaChat, WaError, WaTick};
    let app = TestApp::new().await;
    app.browser.site(FakePage::new(
        "https://web.whatsapp.com/",
        "WhatsApp",
        "- text: chats",
    ));
    let unlinked = app
        .state
        .browser()
        .whatsapp_draft(
            &WaChat {
                name: "Book club".into(),
                phone: None,
                me: false,
            },
            "hi",
            Some(5),
        )
        .await;
    assert!(
        matches!(unlinked, Err(WaError::NotLinked(ref said)) if said.starts_with("The owner has the browser now")),
        "{unlinked:?}"
    );
    app.state
        .browser()
        .input(&emberhut_server::services::browser::BrowserInput {
            kind: Some("handback".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    app.browser.whatsapp_linked(true);
    let chat = WaChat {
        name: "Book club".into(),
        phone: Some("15550001111".into()),
        me: false,
    };
    let to = app
        .state
        .browser()
        .whatsapp_draft(&chat, "hi\nthere", Some(5))
        .await;
    assert_eq!(to, Ok("Book club".to_owned()));
    let steps = app.browser.steps();
    assert!(
        steps.contains(&"goto https://web.whatsapp.com/send?phone=15550001111".to_owned()),
        "{steps:?}"
    );
    assert!(
        steps.ends_with(&[
            "insert hi".to_owned(),
            "press Shift+Enter".to_owned(),
            "insert there".to_owned()
        ]),
        "{steps:?}"
    );
    assert!(
        !steps.iter().any(|s| s == "press Enter"),
        "nothing sent while drafting"
    );
    assert_eq!(
        app.state.browser().whatsapp_send("hi\nthere").await,
        Ok(WaTick::Delivered)
    );
    let nobody = WaChat {
        name: "Nobody".into(),
        phone: None,
        me: false,
    };
    let none = app
        .state
        .browser()
        .whatsapp_draft(&nobody, "hi", Some(5))
        .await;
    assert!(
        matches!(none, Err(WaError::NotOne(ref said)) if said.starts_with("No chat named \"Nobody\"")),
        "{none:?}"
    );
}
