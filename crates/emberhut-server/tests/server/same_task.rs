//! Same-task memory across rooms: a turn that names a task's key is shown what the other rooms
//! said about it in the last 48 hours, each line once.

use emberhut_core::RoomId;

use crate::common::TestApp;

#[tokio::test]
async fn a_turn_naming_a_key_hears_what_the_other_rooms_said_about_it() {
    let app = TestApp::new().await;
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let atlas = db.rooms().create("Atlas", app.clock_now()).await.unwrap();
    let talk = app.state.talk();
    app.assistant.answers(&[], Some("Noted, TASK-12 waits."));
    talk.talk(atlas.id, "Hold TASK-12 until Friday".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    talk.talk(atlas.id, "Something else".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;

    talk.talk(RoomId::MAIN, "Ship TASK-12 today".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let turns = app.assistant.turns();
    let prompt = &turns[2].prompt;
    assert!(
        prompt.starts_with("Ship TASK-12 today\n\n(Same task"),
        "{prompt}"
    );
    assert!(
        prompt.contains("TASK-12:\n- [")
            && prompt.contains(", Atlas] Owner: Hold TASK-12 until Friday"),
        "{prompt}"
    );
    assert!(
        prompt.contains(", Atlas] Assistant: Noted, TASK-12 waits."),
        "{prompt}"
    );
    assert!(!prompt.contains("Something else"), "{prompt}");
    assert!(
        !prompt.contains("Owner: Ship TASK-12 today"),
        "not its own message"
    );

    // Given once: the next turn about it in Main hears only what is new.
    talk.talk(RoomId::MAIN, "And TASK-12 again".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let again = &app.assistant.turns()[3].prompt;
    assert!(!again.contains("Hold TASK-12 until Friday"), "{again}");

    // A turn naming no key gets nothing.
    talk.talk(RoomId::MAIN, "Thanks".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(app.assistant.turns()[4].prompt, "Thanks");
}
