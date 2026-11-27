//! Account failover: when a turn's account runs out, its session moves to the next account
//! that can take work, on the same conversation, and the turn carries on; the `testLimit`
//! setting makes a room's next answer count as running out.

use emberhut_api::paths;
use emberhut_api::settings::SettingsReply;
use emberhut_core::RoomId;
use serde_json::json;

use crate::common::{Call, TestApp};
use crate::fake_cli::{
    FakeCli, agent, config, error_result, init, is_user, rejected, result, step,
};

/// The account (its config dir's last part) a launch runs on.
fn account(launch: &emberhut_server::services::claude::Launch) -> String {
    launch
        .var("CLAUDE_CONFIG_DIR")
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure fails the test"
)]
async fn texts(app: &TestApp) -> Vec<String> {
    app.state
        .state()
        .state(RoomId::MAIN)
        .await
        .unwrap()
        .chat
        .into_iter()
        .map(|line| line.text)
        .collect()
}

#[tokio::test]
async fn a_session_whose_account_runs_out_moves_to_the_next_and_carries_on() {
    // The first account is refused (a rejected notice, then the CLI's own words); the second
    // answers.
    let cli = FakeCli::new(|launch, line| {
        if !is_user(line) {
            return Vec::new();
        }
        let said = line["message"]["content"].as_str().unwrap_or_default();
        if account(launch) == "first" {
            vec![
                init("s-1"),
                rejected(1_900_000_000),
                error_result("You've hit your limit · resets 5pm"),
            ]
        } else {
            vec![init("s-1"), result(&format!("Done on second: {said}"))]
        }
    });
    let app = TestApp::with_agent(agent(
        &cli,
        config(&[("first", "/accounts/first"), ("second", "/accounts/second")]),
    ))
    .await;
    app.state
        .talk()
        .talk(RoomId::MAIN, "Book it".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(texts(&app).await, ["Book it", "Done on second: Book it"]);
    let launches = cli.launches();
    assert_eq!(launches.len(), 2);
    assert_eq!(account(&launches[0]), "first");
    assert_eq!(account(&launches[1]), "second");
    assert_eq!(
        launches[1].flag("--resume"),
        Some("s-1"),
        "the same conversation, resumed"
    );
    assert_eq!(cli.said(), ["Book it", "Book it"], "the same message again");

    // The next turn stays on the second account: the first is out until its reset.
    app.state
        .talk()
        .talk(RoomId::MAIN, "And the hotel".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(cli.launches().len(), 2);
    assert_eq!(
        texts(&app).await.last().map(String::as_str),
        Some("Done on second: And the hotel")
    );
}

#[tokio::test]
async fn a_turn_that_had_done_something_is_told_to_carry_on() {
    let cli = FakeCli::new(|launch, line| {
        if !is_user(line) {
            return Vec::new();
        }
        if account(launch) == "first" {
            vec![
                init("s-1"),
                step("Opening the file.", "Read"),
                error_result("Claude usage limit reached."),
            ]
        } else {
            vec![init("s-1"), result("Finished")]
        }
    });
    let app = TestApp::with_agent(agent(
        &cli,
        config(&[("first", "/a/first"), ("second", "/a/second")]),
    ))
    .await;
    app.state
        .talk()
        .talk(RoomId::MAIN, "Edit the notes".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let said = cli.said();
    assert_eq!(said.len(), 2);
    assert_eq!(said[1], emberhut_server::services::claude::CARRY_ON);
    assert_eq!(texts(&app).await, ["Edit the notes", "Finished"]);
}

#[tokio::test]
async fn with_no_account_left_the_answer_comes_as_it_was() {
    let cli = FakeCli::new(|_, line| {
        if is_user(line) {
            vec![
                init("s-1"),
                error_result("You've hit your limit · resets 5pm"),
            ]
        } else {
            Vec::new()
        }
    });
    let app = TestApp::with_agent(agent(&cli, config(&[("only", "/a/only")]))).await;
    app.state
        .talk()
        .talk(RoomId::MAIN, "Hi".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(cli.launches().len(), 1);
    assert_eq!(
        texts(&app).await,
        ["Hi", "You've hit your limit · resets 5pm"]
    );
}

#[tokio::test]
async fn test_limit_makes_the_next_answer_count_as_running_out() {
    let cli = FakeCli::new(|launch, line| {
        if is_user(line) {
            vec![init("s-1"), result(&format!("on {}", account(launch)))]
        } else {
            Vec::new()
        }
    });
    let app = TestApp::with_agent(agent(
        &cli,
        config(&[("first", "/a/first"), ("second", "/a/second")]),
    ))
    .await;
    let session = app.sign_in().await;
    let talk = app.state.talk();
    talk.talk(RoomId::MAIN, "one".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let reply: SettingsReply = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&session)
                .json(&json!({ "testLimit": true, "room": 1 })),
        )
        .await
        .read_as();
    assert!(reply.ok);
    talk.talk(RoomId::MAIN, "two".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(texts(&app).await, ["one", "on first", "two", "on second"]);
    assert_eq!(cli.launches().len(), 2);
}

#[tokio::test]
async fn a_pinned_account_takes_the_session_and_the_state_says_where_main_runs() {
    use emberhut_db::PrefKey;

    let cli = FakeCli::new(|launch, line| {
        if is_user(line) {
            vec![init("s-1"), result(&format!("on {}", account(launch)))]
        } else {
            Vec::new()
        }
    });
    let app = TestApp::with_agent(agent(
        &cli,
        config(&[("first", "/accounts/first"), ("second", "/accounts/second")]),
    ))
    .await;
    let session = app.sign_in().await;
    let talk = |text: &'static str| {
        let session = session.clone();
        let app = &app;
        async move {
            app.call(
                Call::post(paths::TALK)
                    .token(&session)
                    .json(&json!({ "text": text })),
            )
            .await;
            app.settle().await;
        }
    };
    talk("Hello").await;
    let state = app.state.state().state(RoomId::MAIN).await.unwrap();
    assert_eq!(
        state.account.unwrap().id.as_deref(),
        Some("first"),
        "the runtime says where Main runs"
    );
    assert_eq!(
        state.assistant_account.unwrap().on.as_deref(),
        Some("first")
    );

    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    db.prefs().set(PrefKey::Pin, Some("second")).await.unwrap();
    talk("Again").await;
    let launches = cli.launches();
    assert_eq!(launches.len(), 2, "the session moved between turns");
    assert_eq!(account(&launches[1]), "second");
    assert_eq!(
        launches[1].flag("--resume"),
        Some("s-1"),
        "on the same conversation"
    );
    assert_eq!(
        texts(&app).await.last().map(String::as_str),
        Some("on second")
    );
    let state = app.state.state().state(RoomId::MAIN).await.unwrap();
    assert_eq!(state.account.unwrap().id.as_deref(), Some("second"));

    // A pin the runtime has no such account for changes nothing.
    db.prefs()
        .set(PrefKey::Pin, Some("elsewhere"))
        .await
        .unwrap();
    talk("Once more").await;
    assert_eq!(cli.launches().len(), 2);
}
