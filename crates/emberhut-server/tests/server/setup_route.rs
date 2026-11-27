//! `GET /api/setup`: the checklist the apps' onboarding walks, on a server with nothing set up
//! and on one with every part set up, a failing Mac helper and an unlinked WhatsApp number.

use std::sync::Arc;

use emberhut_api::setup::{SetupItemId, SetupPage, SetupState};
use emberhut_db::Database;
use emberhut_server::app::{Dependencies, WorkerSetup};
use emberhut_server::config::AuthMode;
use emberhut_server::services::mac::{MacHealth, NoMacHelper};
use emberhut_server::services::messaging::MessagingDeps;
use emberhut_server::services::workers::WorkerRuntime;
use jiff::Timestamp;
use serde_json::json;

use crate::common::{Call, TestApp};
use crate::fake_cli::FakeCli;
use crate::fake_push::FakePush;
use crate::messaging_fakes::FakeWaDaemon;

const SETUP: &str = emberhut_api::paths::SETUP;

/// Each item's id and state, in the page's order.
fn states(page: &SetupPage) -> Vec<(&str, &str)> {
    page.items
        .iter()
        .map(|item| (item.id.as_str(), item.state.as_str()))
        .collect()
}

/// The line of item `id`.
fn detail<'a>(page: &'a SetupPage, id: &SetupItemId) -> &'a str {
    page.item(id)
        .and_then(|item| item.detail.as_deref())
        .unwrap_or_default()
}

#[tokio::test]
pub(crate) async fn a_bare_server_says_what_it_lacks_and_only_to_a_signed_in_device() {
    let app = TestApp::with_dependencies(AuthMode::Enforce, |dependencies, _| Dependencies {
        mac: Arc::new(NoMacHelper),
        ..dependencies
    })
    .await;
    let unsigned = app.call(Call::get(SETUP)).await;
    assert_eq!(unsigned.status, 401);

    let session = app.sign_in().await;
    let answer = app.call(Call::get(SETUP).token(&session)).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let page: SetupPage = answer.read_as();
    assert_eq!(
        states(&page),
        [
            ("agent", "off"),
            ("accounts", "off"),
            ("devices", "ready"),
            ("push", "off"),
            ("public-url", "off"),
            ("mac", "off"),
            ("chrome", "missing"),
            ("gmail", "missing"),
            ("slack", "missing"),
            ("whatsapp", "off"),
        ]
    );
    assert!(detail(&page, &SetupItemId::Agent).contains("EMBERHUT_AGENT_COMMAND"));
    assert_eq!(detail(&page, &SetupItemId::Devices), "1 device signed in.");
    assert!(detail(&page, &SetupItemId::Gmail).contains("gmail-client-id"));
    // Every item says something, and no item comes twice.
    assert!(page.items.iter().all(|item| item.detail.is_some()));
    let mut ids: Vec<&str> = page.items.iter().map(|item| item.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), page.items.len());
}

/// A server with every part set up: the agent CLI, push, a public URL, two devices, a runner
/// with two accounts (one logged in), a paired Chrome profile, the Google client and an
/// account, a Slack token and a WhatsApp daemon. Its Mac helper doesn't answer until told to.
#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure here fails the test that called it"
)]
async fn set_up_app() -> TestApp {
    let (apns, _fcm, push) = FakePush::pair();
    apns.set_ready(true);
    let spawner = FakeCli::new(|_, _| Vec::new());
    let app = TestApp::with_dependencies(AuthMode::Enforce, move |dependencies, _| {
        let mut approvals = dependencies.approvals.clone();
        approvals.public_url = Some("https://box.example".to_owned());
        Dependencies {
            push,
            approvals,
            worker_setup: WorkerSetup {
                runtime: Some(WorkerRuntime {
                    spawner,
                    config: crate::workers::worker_config(),
                }),
                ..WorkerSetup::default()
            },
            messaging: MessagingDeps {
                wa_daemon: Arc::new(FakeWaDaemon::default()),
                ..MessagingDeps::default()
            },
            ..dependencies
        }
    })
    .await;
    app.state
        .auth()
        .open_session(&crate::common::device("phone-1"))
        .await
        .unwrap();

    // A runner with two accounts, one logged in.
    app.runners.on(&[("build", "Build box", "build-token")]);
    app.runners.answer(
        "build",
        "/accounts",
        json!({ "accounts": [
            { "id": "work", "name": "Work", "scope": "any", "enabled": true, "loggedIn": true },
            { "id": "spare", "name": "Spare", "scope": "any", "enabled": true, "loggedIn": false },
        ] }),
    );
    app.state.runners().tick().await;
    // A paired Chrome profile, the Google client and an account, a Slack token, and the
    // daemon's token.
    let db = Database::connect(&app.url).await.unwrap();
    db.chrome()
        .insert("work", "Work laptop", "token-hash", Timestamp::now())
        .await
        .unwrap();
    let secrets = app.dir().join("secrets");
    std::fs::create_dir_all(&secrets).unwrap();
    for (name, value) in [
        ("gmail-client-id", "client-id"),
        ("gmail-client-secret", "client-secret"),
        ("gmail-home-refresh", "refresh"),
        ("slack-acme-user", "xoxp-example"),
        ("whatsapp-token", "daemon-token"),
    ] {
        std::fs::write(secrets.join(name), value).unwrap();
    }
    app
}

/// The WhatsApp daemon tells the server how the number is.
async fn tell_whatsapp(app: &TestApp, status: serde_json::Value) {
    let call = Call::post("/api/whatsapp/in")
        .header("authorization", "Bearer daemon-token")
        .json(&status);
    assert_eq!(app.call(call).await.status, 200);
}

#[tokio::test]
pub(crate) async fn a_set_up_server_reads_ready_and_says_what_fails() {
    let app = set_up_app().await;
    let session = app.sign_in().await;
    tell_whatsapp(&app, json!({ "kind": "status", "state": "unlinked" })).await;

    let page: SetupPage = app.call(Call::get(SETUP).token(&session)).await.read_as();
    assert_eq!(
        states(&page),
        [
            ("agent", "ready"),
            ("accounts", "ready"),
            ("devices", "ready"),
            ("push", "ready"),
            ("public-url", "ready"),
            // The test's Mac helper doesn't answer until it is told to.
            ("mac", "error"),
            ("chrome", "ready"),
            ("gmail", "ready"),
            ("slack", "ready"),
            ("whatsapp", "missing"),
        ]
    );
    assert_eq!(
        detail(&page, &SetupItemId::Accounts),
        "1 of 2 accounts logged in."
    );
    assert_eq!(detail(&page, &SetupItemId::Devices), "2 devices signed in.");
    assert_eq!(
        detail(&page, &SetupItemId::PublicUrl),
        "https://box.example"
    );
    assert!(detail(&page, &SetupItemId::Mac).contains("isn't answering"));
    assert_eq!(
        detail(&page, &SetupItemId::Chrome),
        "1 profile paired, 0 online."
    );
    assert_eq!(
        detail(&page, &SetupItemId::Gmail),
        "1 account signed in: home."
    );
    assert_eq!(
        detail(&page, &SetupItemId::Slack),
        "A token for 1 workspace: acme."
    );

    // The Mac answers and the number is linked.
    app.mac.answers(MacHealth {
        locked: true,
        idle: None,
    });
    tell_whatsapp(
        &app,
        json!({ "kind": "status", "state": "open", "number": "15550100999" }),
    )
    .await;
    let page: SetupPage = app.call(Call::get(SETUP).token(&session)).await.read_as();
    let mac = page.item(&SetupItemId::Mac).unwrap();
    assert_eq!(mac.state, SetupState::Ready);
    assert_eq!(mac.detail.as_deref(), Some("Online; the screen is locked."));
    let whatsapp = page.item(&SetupItemId::WhatsApp).unwrap();
    assert_eq!(whatsapp.state, SetupState::Ready);
    assert_eq!(whatsapp.detail.as_deref(), Some("Linked as 15550100999."));

    // A daemon that reports a failure while unlinked.
    tell_whatsapp(
        &app,
        json!({ "kind": "status", "state": "logged_out", "error": "Logged out on the phone" }),
    )
    .await;
    let page: SetupPage = app.call(Call::get(SETUP).token(&session)).await.read_as();
    let whatsapp = page.item(&SetupItemId::WhatsApp).unwrap();
    assert_eq!(whatsapp.state, SetupState::Error);
    assert_eq!(whatsapp.detail.as_deref(), Some("Logged out on the phone"));
}
