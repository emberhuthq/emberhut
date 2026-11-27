//! Memory in the agent runtime: the owner's new quick notes ride along with Main's next turn,
//! a message that sounds like a correction is logged with the answer before it, the owner's
//! rules are in each session's prompt, and recall indexes the tasks and the voice notes.

use emberhut_api::paths;
use emberhut_core::task::Who;
use serde_json::json;

use crate::common::{Call, TestApp};
use crate::fake_cli::{FakeCli, agent, config, init, is_request, is_user, result};

/// The owner says `text` in Main, and the turn ends.
async fn say(app: &TestApp, text: &str) {
    let session = app.sign_in().await;
    let said = app
        .call(
            Call::post(paths::TALK)
                .token(&session)
                .json(&json!({ "text": text })),
        )
        .await;
    assert_eq!(said.status, 200);
    app.settle().await;
}

#[tokio::test]
async fn the_owner_s_new_quick_notes_ride_along_with_main_s_next_turn_once() {
    let app = TestApp::new().await;
    app.assistant.answers(&[], Some("OK."));
    app.state
        .notes()
        .save("note-0001", "Buy milk on the way home", None)
        .await
        .unwrap();
    say(&app, "Hello").await;
    say(&app, "And again").await;
    let turns = app.assistant.turns();
    assert_eq!(turns.len(), 2);
    assert!(turns[0].prompt.starts_with("Hello"), "{}", turns[0].prompt);
    assert!(
        turns[0]
            .prompt
            .contains("New quick notes since you last looked")
    );
    assert!(turns[0].prompt.contains("Buy milk on the way home"));
    assert!(
        !turns[1].prompt.contains("Buy milk"),
        "taken once: {}",
        turns[1].prompt
    );
}

#[tokio::test]
async fn a_message_that_sounds_like_a_correction_is_logged_with_the_answer_before_it() {
    let app = TestApp::new().await;
    app.assistant.answers(&[], Some("The meeting is at five."));
    say(&app, "When is the meeting?").await;
    say(&app, "Thanks").await;
    assert!(
        app.state
            .learning()
            .corrections(7)
            .await
            .unwrap()
            .is_empty(),
        "nothing that sounds like a correction yet"
    );
    say(&app, "That's wrong, it moved to six").await;
    let logged = app.state.learning().corrections(7).await.unwrap();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].said, "That's wrong, it moved to six");
    assert_eq!(
        logged[0].wrong,
        "Your answer before it: The meeting is at five."
    );
}

#[tokio::test]
async fn the_owner_s_rules_are_in_each_session_s_prompt() {
    let cli = FakeCli::new(|_, line| {
        if is_user(line) {
            vec![init("s-1"), result("Done.")]
        } else {
            Vec::new()
        }
    });
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    app.state
        .learning()
        .set_lesson(None, "Say which file you changed.", None)
        .await
        .unwrap();
    say(&app, "Fix the build").await;
    let lines = cli.lines();
    let start = lines
        .iter()
        .find(|(_, line)| is_request(line, "initialize"))
        .map(|(_, line)| line.clone())
        .unwrap();
    let append = start["request"]["appendSystemPrompt"]
        .as_str()
        .unwrap_or_default();
    assert!(append.contains("Say which file you changed."), "{append}");
}

#[tokio::test]
async fn recall_indexes_the_tasks_their_history_and_the_voice_notes() {
    use emberhut_core::messaging::voice::{SeenNote, VoiceSource};

    let app = TestApp::new().await;
    let tasks = app.state.tasks();
    let task = tasks
        .create(
            emberhut_db::NewTask {
                title: "Renew the passport".to_owned(),
                body: Some("The form is at the consulate".to_owned()),
                ..emberhut_db::NewTask::default()
            },
            Who::Assistant,
        )
        .await
        .unwrap();
    tasks
        .note(
            task.id,
            Who::Assistant,
            "Booked the appointment for Tuesday",
            false,
        )
        .await
        .unwrap();
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let voice = db.voice_notes();
    let now = app.clock_now();
    voice
        .seen(
            &SeenNote {
                id: "wa:voice-1".to_owned(),
                source: VoiceSource::WhatsApp,
                reference: "msg-1".to_owned(),
                name: None,
                seconds: Some(12.0),
                chat: Some("wa:12".to_owned()),
                chat_name: Some("Book club".to_owned()),
                sender: Some("Ann".to_owned()),
                msg_at: Some(now),
                msg: None,
            },
            now,
        )
        .await
        .unwrap();
    voice
        .finish(
            "wa:voice-1",
            &emberhut_db::messaging::Outcome::Done {
                text: "Bring the lanterns on Friday".to_owned(),
                seconds: Some(12.0),
            },
            now,
        )
        .await
        .unwrap();

    app.state.recall().sync().await.unwrap();
    let recall = db.recall();
    assert!(
        recall.count_of("task").await.unwrap() >= 2,
        "the task and its line"
    );
    assert_eq!(recall.count_of("voice").await.unwrap(), 1);
    let found = recall.search_words("passport", 10).await.unwrap();
    assert!(
        found
            .iter()
            .any(|doc| doc.source == "task" && doc.title.contains("Renew the passport")),
        "{found:?}"
    );
    let appointment = recall.search_words("appointment", 10).await.unwrap();
    assert!(appointment.iter().any(|doc| doc.source == "task"));
    let lanterns = recall.search_words("lanterns", 10).await.unwrap();
    assert_eq!(lanterns.len(), 1);
    assert_eq!(
        lanterns[0].title,
        "Voice note from Ann in Book club (WhatsApp)"
    );

    // Indexed once: a second pass adds nothing.
    let before = recall.count().await.unwrap();
    app.state.recall().sync().await.unwrap();
    assert_eq!(recall.count().await.unwrap(), before);
}
