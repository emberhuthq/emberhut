//! A new server whose agent has nothing to think with: no routine, check-in or other turn
//! nobody asked for starts, and none fails, until a provider is saved; then each runs at its
//! next due time, or catches up that day. A CLI that says it isn't logged in never reaches the
//! chat in its own words: the owner reads one plain note (and one for each message of theirs),
//! the setup checklist says what is missing, and what is unattended waits again.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure fails the test that called them"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use emberhut_api::paths;
use emberhut_api::setup::{SetupItemId, SetupPage, SetupState};
use emberhut_core::RoomId;
use emberhut_core::session::Session;
use emberhut_server::app::{Dependencies, WorkerSetup};
use emberhut_server::services::agent_provider::ProviderStore;
use emberhut_server::services::claude::{CANT_THINK, ClaudeAssistant};
use emberhut_server::services::messaging::secrets::Secrets;
use emberhut_server::services::routines::Routine;
use emberhut_server::services::talk::Assistant;
use emberhut_server::services::workers::WorkerRuntime;
use jiff::tz::TimeZone;
use serde_json::json;

use crate::common::{Call, TestApp};
use crate::fake_cli::{FakeCli, config, error_result, init, is_user, result};

/// The CLI's own words when it has no login.
const NOT_LOGGED_IN: &str = "Not logged in · Please run /login";

/// A CLI that answers `Echo: <text>` while `signed_in` holds, and otherwise says it isn't
/// logged in, as Claude Code does.
fn cli(signed_in: &Arc<AtomicBool>) -> Arc<FakeCli> {
    let signed_in = Arc::clone(signed_in);
    FakeCli::new(move |_, line| {
        if !is_user(line) {
            return Vec::new();
        }
        if !signed_in.load(Ordering::SeqCst) {
            return vec![init("s-1"), error_result(NOT_LOGGED_IN)];
        }
        let said = line["message"]["content"].as_str().unwrap_or_default();
        vec![init("s-1"), result(&format!("Echo: {said}"))]
    })
}

/// A fresh server with an agent CLI (`cli`), no account configured and nothing saved: what a
/// new owner starts with. Its workers run on a CLI that answers nothing.
async fn fresh(cli: &Arc<FakeCli>) -> TestApp {
    let settings = config(&[]);
    let cli = Arc::clone(cli);
    let app = TestApp::with_agent_and(
        move |db, clock, dir| {
            let store = ProviderStore::new(Secrets::new(dir.join("secrets")), db.settings());
            Arc::new(
                ClaudeAssistant::new(settings, cli, db.settings(), db.rooms(), clock)
                    .with_provider(store),
            ) as Arc<dyn Assistant>
        },
        |dependencies, _| Dependencies {
            worker_setup: WorkerSetup {
                runtime: Some(WorkerRuntime {
                    spawner: FakeCli::new(|_, _| Vec::new()),
                    config: crate::workers::worker_config(),
                }),
                ..WorkerSetup::default()
            },
            ..dependencies
        },
    )
    .await;
    // A day starts at midnight: keep the test's times inside one day.
    let hour = app.clock_now().to_zoned_datetime(TimeZone::system()).hour();
    if hour == 0 || hour >= 22 {
        app.advance_minutes(180);
    }
    app
}

/// `HH:MM` `minutes` from now, in the server's zone.
fn local(app: &TestApp, minutes: i64) -> String {
    let at = app
        .clock_now()
        .checked_add(jiff::SignedDuration::from_mins(minutes))
        .unwrap()
        .to_zoned_datetime(TimeZone::system());
    at.strftime("%H:%M").to_string()
}

/// The morning brief, due five minutes ago, and a later routine.
async fn brief_due(app: &TestApp) {
    let routines = [
        Routine {
            at: local(app, -5),
            what: "Morning brief: keep it short.".to_owned(),
            day: None,
        },
        Routine {
            at: local(app, 60),
            what: "Later: a second look.".to_owned(),
            day: None,
        },
    ];
    app.state
        .routines()
        .set(Some(&routines), None)
        .await
        .unwrap();
}

/// Main's chat: (the owner's, an event line, the text).
async fn chat(app: &TestApp) -> Vec<(bool, bool, String)> {
    app.state
        .state()
        .state(RoomId::MAIN)
        .await
        .unwrap()
        .chat
        .into_iter()
        .map(|line| (line.mine, line.event, line.text))
        .collect()
}

/// The wake rule's look, then the turns it started.
async fn tick(app: &TestApp) {
    app.state.wake().tick().await.unwrap();
    app.settle().await;
}

/// The setup checklist's `provider` item.
async fn provider_item(app: &TestApp, session: &Session) -> (SetupState, String) {
    let page: SetupPage = app
        .call(Call::get(paths::SETUP).token(session))
        .await
        .read_as();
    let item = page.item(&SetupItemId::Provider).unwrap();
    (item.state.clone(), item.detail.clone().unwrap_or_default())
}

/// Saves the agent CLI's own Claude login as the provider.
async fn save_claude(app: &TestApp, session: &Session) {
    let saved = app
        .call(
            Call::put(paths::AGENT_PROVIDER)
                .token(session)
                .json(&json!({ "provider": "claude" })),
        )
        .await;
    assert_eq!(saved.status, 200, "{}", saved.text);
}

/// Says `text` in Main and waits for the answer.
async fn talk(app: &TestApp, session: &Session, text: &str) {
    let said = app
        .call(
            Call::post(paths::TALK)
                .token(session)
                .json(&json!({ "text": text })),
        )
        .await;
    assert_eq!(said.status, 200, "{}", said.text);
    app.settle().await;
}

/// Whether any line of the chat carries the CLI's own words.
fn says_the_cli_s_words(chat: &[(bool, bool, String)]) -> bool {
    chat.iter()
        .any(|(_, _, text)| text.contains("/login") || text.contains("Not logged in"))
}

#[tokio::test]
async fn a_fresh_server_with_nothing_to_think_with_starts_no_brief_and_posts_nothing() {
    let signed_in = Arc::new(AtomicBool::new(false));
    let cli = cli(&signed_in);
    let app = fresh(&cli).await;
    let session = app.sign_in().await;
    brief_due(&app).await;
    app.state
        .wake()
        .wake_me_at(1, "look at the build", RoomId::MAIN)
        .await
        .unwrap();
    app.advance_minutes(2);

    tick(&app).await;
    tick(&app).await;
    assert!(cli.launches().is_empty(), "no CLI was started");
    assert!(chat(&app).await.is_empty(), "nothing was posted");
    // Nothing failed and nothing was lost: the check-in is still there, nothing ran today.
    assert_eq!(app.state.wake().checkins().await.unwrap().len(), 1);
    assert_eq!(app.state.wake().pending().await.unwrap(), 0);
    let (state, detail) = provider_item(&app, &session).await;
    assert_eq!(state, SetupState::Missing);
    assert!(detail.starts_with("No model provider is saved"), "{detail}");
}

#[tokio::test]
async fn once_a_provider_is_saved_what_waited_runs_and_the_brief_catches_up_that_day() {
    let signed_in = Arc::new(AtomicBool::new(true));
    let cli = cli(&signed_in);
    let app = fresh(&cli).await;
    let session = app.sign_in().await;
    brief_due(&app).await;
    tick(&app).await;
    assert!(cli.launches().is_empty(), "nothing to think with yet");

    save_claude(&app, &session).await;
    tick(&app).await;
    let said = cli.said();
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(
        said[0].starts_with("Event: - [routine] Morning brief: keep it short."),
        "{}",
        said[0]
    );
    let lines = chat(&app).await;
    assert_eq!(lines.len(), 2, "{lines:?}");
    let (_, _, answer) = &lines[1];
    assert!(answer.starts_with("Echo: Event: - [routine] Morning brief"));
    // The answer proved the CLI's login.
    let (state, _) = provider_item(&app, &session).await;
    assert_eq!(state, SetupState::Ready);

    // Once that day; the later routine at its own time.
    tick(&app).await;
    assert_eq!(cli.said().len(), 1);
    app.advance_minutes(61);
    tick(&app).await;
    let said = cli.said();
    assert_eq!(said.len(), 2);
    assert!(said[1].starts_with("Event: - [routine] Later: a second look."));
}

#[tokio::test]
async fn a_cli_s_own_login_that_answered_the_owner_lets_the_routines_run() {
    let signed_in = Arc::new(AtomicBool::new(true));
    let cli = cli(&signed_in);
    let app = fresh(&cli).await;
    let session = app.sign_in().await;
    brief_due(&app).await;
    tick(&app).await;
    assert!(cli.launches().is_empty(), "nothing to think with yet");

    // Nothing saved, but the CLI signed in on the server answers the owner.
    talk(&app, &session, "Hello").await;
    let (state, detail) = provider_item(&app, &session).await;
    assert_eq!(state, SetupState::Ready);
    assert_eq!(
        detail,
        "The agent runs on the agent CLI's own login on the server."
    );
    tick(&app).await;
    let said = cli.said();
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said[1].starts_with("Event: - [routine] Morning brief"));
}

#[tokio::test]
async fn a_cli_that_isn_t_logged_in_gives_one_plain_note_and_unattended_turns_wait() {
    let signed_in = Arc::new(AtomicBool::new(false));
    let cli = cli(&signed_in);
    let app = fresh(&cli).await;
    let session = app.sign_in().await;
    // `claude` saved: only a turn can tell whether the CLI is signed in.
    save_claude(&app, &session).await;
    brief_due(&app).await;

    tick(&app).await;
    assert_eq!(cli.said().len(), 1, "the brief ran");
    let chat_now = chat(&app).await;
    assert_eq!(chat_now.len(), 2, "{chat_now:?}");
    assert!(chat_now[0].1, "the routine's event line");
    assert_eq!(chat_now[1], (false, false, CANT_THINK.to_owned()));
    assert!(!says_the_cli_s_words(&chat_now));
    let (state, detail) = provider_item(&app, &session).await;
    assert_eq!(state, SetupState::Missing);
    assert_eq!(
        detail,
        "The agent CLI answered that it isn't logged in: choose a model provider, or sign the \
         CLI in on the server."
    );
    assert_eq!(cli.stopped(), [0], "its session was closed");

    // Nothing unattended starts now: a check-in, the later routine.
    app.state
        .wake()
        .wake_me_at(1, "look at the build", RoomId::MAIN)
        .await
        .unwrap();
    app.advance_minutes(61);
    let session = app.sign_in().await;
    tick(&app).await;
    tick(&app).await;
    assert_eq!(cli.said().len(), 1, "nothing else ran");
    assert_eq!(chat(&app).await.len(), 2, "and nothing else was posted");

    // A provider saved: what waited runs, on a CLI that is signed in now.
    signed_in.store(true, Ordering::SeqCst);
    save_claude(&app, &session).await;
    tick(&app).await;
    let said = cli.said();
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said[1].contains("[checkin] Your check-in: look at the build"));
    assert!(said[1].contains("[routine] Later: a second look."));
    let chat_now = chat(&app).await;
    assert!(chat_now.last().unwrap().2.starts_with("Echo: Event:"));
    assert_eq!(
        chat_now
            .iter()
            .filter(|(_, _, text)| text == CANT_THINK)
            .count(),
        1,
        "the note once"
    );
    assert!(!says_the_cli_s_words(&chat_now));
}

#[tokio::test]
async fn the_owner_s_own_message_gets_the_plain_note_every_time() {
    let signed_in = Arc::new(AtomicBool::new(false));
    let cli = cli(&signed_in);
    let app = fresh(&cli).await;
    let session = app.sign_in().await;

    talk(&app, &session, "Hello").await;
    talk(&app, &session, "Are you there?").await;
    assert_eq!(
        chat(&app).await,
        [
            (true, false, "Hello".to_owned()),
            (false, false, CANT_THINK.to_owned()),
            (true, false, "Are you there?".to_owned()),
            (false, false, CANT_THINK.to_owned()),
        ]
    );
    assert_eq!(provider_item(&app, &session).await.0, SetupState::Missing);

    // Signed in on the server since: the owner's next message is answered, and that clears it.
    signed_in.store(true, Ordering::SeqCst);
    talk(&app, &session, "And now?").await;
    let chat_now = chat(&app).await;
    assert_eq!(
        chat_now.last().unwrap(),
        &(false, false, "Echo: And now?".to_owned())
    );
    assert_eq!(provider_item(&app, &session).await.0, SetupState::Ready);
}
