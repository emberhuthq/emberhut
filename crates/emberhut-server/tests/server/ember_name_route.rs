//! `GET /api/ember/name` and `POST /api/ember/name`: the default, renaming, refusals, the
//! state's `emberName`, and the name in the assistant's prompt and its reply alerts.

use emberhut_api::ember_name::{DEFAULT_NAME, EmberName, EmberNameReply};
use emberhut_api::error::ErrorBody;
use emberhut_api::paths;
use emberhut_api::state::State;
use emberhut_core::RoomId;
use emberhut_server::config::AuthMode;
use serde_json::{Value, json};

use crate::common::{Call, TestApp};
use crate::fake_cli::{FakeCli, agent, config, init, is_request, is_user, result};
use crate::fake_push::FakePush;

async fn state(app: &TestApp, session: &emberhut_core::session::Session) -> State {
    app.call(Call::get(paths::STATE).token(session))
        .await
        .read_as()
}

#[tokio::test]
pub(crate) async fn without_a_name_the_ember_is_your_ember() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let answer = app.call(Call::get(paths::EMBER_NAME).token(&session)).await;
    assert_eq!(answer.status, 200);
    assert_eq!(
        answer.json(),
        json!({ "name": "Your ember", "custom": false, "at": null })
    );
    assert_eq!(
        state(&app, &session).await.ember_name,
        Some(EmberName::default())
    );
}

#[tokio::test]
pub(crate) async fn a_name_is_kept_for_every_device_and_the_state_carries_it() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let answer = app
        .call(
            Call::post(paths::EMBER_NAME)
                .token(&session)
                .json(&json!({ "name": "  Pip " })),
        )
        .await;
    assert_eq!(answer.status, 200);
    let reply: EmberNameReply = answer.read_as();
    assert!(reply.ok);
    assert_eq!(reply.ember.name, "Pip", "trimmed");
    assert!(reply.ember.custom);
    assert!(reply.ember.at.is_some());

    // Another device reads the same name, from the route and in the state it polls.
    let other = app.sign_in().await;
    let read: EmberName = app
        .call(Call::get(paths::EMBER_NAME).token(&other))
        .await
        .read_as();
    assert_eq!(read, reply.ember);
    assert_eq!(state(&app, &other).await.ember_name, Some(reply.ember));
    let did = state(&app, &other).await.did;
    assert!(
        did.iter()
            .any(|line| line.text == "Your ember is now called Pip")
    );
}

#[tokio::test]
async fn null_blank_or_the_default_s_words_go_back_to_the_default() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    for back in [json!(null), json!(""), json!("   "), json!("your ember")] {
        let named = app
            .call(
                Call::post(paths::EMBER_NAME)
                    .token(&session)
                    .json(&json!({ "name": "Juniper" })),
            )
            .await;
        assert_eq!(named.status, 200);
        let reply: EmberNameReply = app
            .call(
                Call::post(paths::EMBER_NAME)
                    .token(&session)
                    .json(&json!({ "name": back })),
            )
            .await
            .read_as();
        assert_eq!(reply.ember.name, DEFAULT_NAME, "{back}");
        assert!(!reply.ember.custom);
        assert!(reply.ember.at.is_some(), "a reset is a change too");
    }
}

#[tokio::test]
pub(crate) async fn a_name_the_server_does_not_take_is_400_and_changes_nothing() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.call(
        Call::post(paths::EMBER_NAME)
            .token(&session)
            .json(&json!({ "name": "Pip" })),
    )
    .await;
    let refused = [
        (
            json!({ "name": "x".repeat(25) }),
            "name: 24 characters at most",
        ),
        (
            json!({ "name": "Ju\nniper" }),
            "name: one line of plain text",
        ),
        (
            json!({ "name": "Ju\u{202E}niper" }),
            "name: one line of plain text",
        ),
        (
            json!({ "name": "\u{1F525}".repeat(25) }),
            "name: 24 characters at most",
        ),
        (json!({ "name": 7 }), "name: text, or null for the default"),
        (json!({}), "name: text, or null for the default"),
        (json!(["Pip"]), "name: text, or null for the default"),
    ];
    for (body, error) in refused {
        let answer = app
            .call(Call::post(paths::EMBER_NAME).token(&session).json(&body))
            .await;
        assert_eq!(answer.status, 400, "{body}");
        let refusal: ErrorBody = answer.read_as();
        assert_eq!(refusal.ok, Some(false));
        assert_eq!(refusal.error, error, "{body}");
    }
    let still: EmberName = app
        .call(Call::get(paths::EMBER_NAME).token(&session))
        .await
        .read_as();
    assert_eq!(still.name, "Pip");
    // 24 characters, emoji and accents included, are taken.
    let longest = "Ém🔥".repeat(8);
    let reply: EmberNameReply = app
        .call(
            Call::post(paths::EMBER_NAME)
                .token(&session)
                .json(&json!({ "name": longest })),
        )
        .await
        .read_as();
    assert_eq!(reply.ember.name, longest);
    // An emoji joined of several is one character, and runs of spaces are made one.
    let astronauts = "\u{1F469}\u{200D}\u{1F680}".repeat(24);
    for (asked, kept) in [
        (astronauts.as_str(), astronauts.as_str()),
        ("  Juniper \u{a0}  the   Brave ", "Juniper the Brave"),
    ] {
        let reply: EmberNameReply = app
            .call(
                Call::post(paths::EMBER_NAME)
                    .token(&session)
                    .json(&json!({ "name": asked })),
            )
            .await
            .read_as();
        assert_eq!(reply.ember.name, kept);
    }
    let reset: EmberNameReply = app
        .call(
            Call::post(paths::EMBER_NAME)
                .token(&session)
                .json(&json!({ "name": "your   ember" })),
        )
        .await
        .read_as();
    assert!(!reset.ember.custom, "the default's words, however spaced");

    let huge = json!({ "name": "x".repeat(3 * 1024 * 1024) }).to_string();
    let too_large = app
        .call(Call::post(paths::EMBER_NAME).token(&session).raw(&huge))
        .await;
    assert_eq!(too_large.status, 413);
}

#[tokio::test]
pub(crate) async fn renaming_needs_a_session() {
    let app = TestApp::new().await;
    let answer = app
        .call(Call::post(paths::EMBER_NAME).json(&json!({ "name": "Pip" })))
        .await;
    assert_eq!(answer.status, 401);
    assert_eq!(app.call(Call::get(paths::EMBER_NAME)).await.status, 401);
}

/// A CLI that answers every message with "ok".
fn answering() -> std::sync::Arc<FakeCli> {
    FakeCli::new(|launch, line| {
        if !is_user(line) {
            return Vec::new();
        }
        let session = launch.flag("--resume").unwrap_or("s-1").to_owned();
        vec![init(&session), result("ok")]
    })
}

/// The system prompt each launch was started with, in order.
fn prompts(cli: &FakeCli) -> Vec<String> {
    cli.lines()
        .into_iter()
        .filter(|(_, line)| is_request(line, "initialize"))
        .map(|(_, line)| {
            line["request"]["appendSystemPrompt"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn the_assistant_is_told_its_name_and_a_rename_restarts_its_session() {
    let cli = answering();
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    let session = app.sign_in().await;
    let talk = app.state.talk();
    talk.talk(RoomId::MAIN, "Hello".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let first = prompts(&cli);
    assert_eq!(first.len(), 1);
    assert!(
        !first[0].contains("named you"),
        "no name: the prompt as before: {}",
        first[0]
    );

    app.call(
        Call::post(paths::EMBER_NAME)
            .token(&session)
            .json(&json!({ "name": "Juniper" })),
    )
    .await;
    talk.talk(RoomId::MAIN, "Again".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let launches = cli.launches();
    assert_eq!(launches.len(), 2, "renamed: the session started again");
    assert_eq!(
        launches[1].flag("--resume"),
        Some("s-1"),
        "on the same conversation"
    );
    let second = prompts(&cli);
    assert!(
        second[1].contains("The owner named you Juniper"),
        "{}",
        second[1]
    );
    assert!(second[1].contains("Call yourself Juniper"), "{}", second[1]);

    // The same name again: the session goes on.
    talk.talk(RoomId::MAIN, "And again".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(cli.launches().len(), 2);
}

#[tokio::test]
async fn reply_alerts_are_titled_with_the_name() {
    let (apns, _fcm, senders) = FakePush::pair();
    let app = TestApp::with_push(AuthMode::Enforce, senders).await;
    let session = app.sign_in().await;
    let token = "a".repeat(64);
    app.call(
        Call::post(paths::DEVICES)
            .token(&session)
            .json(&json!({ "token": token, "env": "production" })),
    )
    .await;
    let say = |text: &'static str| {
        let app = &app;
        let session = &session;
        async move {
            app.assistant.answers(&[], Some("Done"));
            app.call(
                Call::post(paths::TALK)
                    .token(session)
                    .json(&json!({ "text": text })),
            )
            .await;
            app.settle().await;
        }
    };
    say("Before").await;
    app.call(
        Call::post(paths::EMBER_NAME)
            .token(&session)
            .json(&json!({ "name": "Pip" })),
    )
    .await;
    say("After").await;
    let titles: Vec<String> = apns
        .sent()
        .iter()
        .map(|(_, alert)| alert.title().to_owned())
        .collect();
    assert_eq!(titles, ["Emberhut", "Pip"]);
}

#[tokio::test]
async fn the_document_lists_the_rename() {
    let document: Value = crate::common::document();
    let post = &document["paths"]["/api/ember/name"]["post"];
    assert_eq!(post["tags"][0], "ember");
    assert!(document["components"]["schemas"]["EmberNameChange"].is_object());
    assert!(document["components"]["schemas"]["EmberNameReply"].is_object());
}
