//! `POST /api/settings`' "About you" actions: `factSet`, `personSet`, `factDrop`, `trialDrop`
//! and `trialKeep`, and the `fact` events that tell the assistant.
#![expect(
    clippy::unwrap_used,
    reason = "the test helpers here: a failure fails the test that called them"
)]

use emberhut_api::facts::FactsPage;
use emberhut_api::paths;
use emberhut_api::settings::SettingsReply;
use emberhut_core::RoomId;
use emberhut_db::Database;
use serde_json::{Value, json};

use crate::common::{Call, TestApp, text};

/// The `fact` events waiting for Main's assistant, oldest first.
async fn told(app: &TestApp) -> Vec<String> {
    let db = Database::connect(&app.url).await.unwrap();
    db.events()
        .waiting(RoomId::MAIN, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "fact")
        .map(|event| event.text)
        .collect()
}

#[tokio::test]
#[expect(clippy::too_many_lines, reason = "one walk through the page's changes")]
async fn the_owner_corrects_marks_and_forgets_facts() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let settings = |body: Value| Call::post(paths::SETTINGS).token(&session).json(&body);

    let answer = app
        .call(settings(json!({
            "factSet": { "category": "home", "title": "Keys", "text": "In the drawer", "project": "Garden" }
        })))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let reply: SettingsReply = answer.read_as();
    assert!(reply.ok);
    let page: FactsPage = app
        .call(Call::get(paths::FACTS).token(&session))
        .await
        .read_as();
    let fact = &page.facts[0];
    assert_eq!(fact.id.as_str(), "home/keys");
    assert!(
        fact.source.starts_with("you corrected it, 20"),
        "{}",
        fact.source
    );
    assert_eq!(fact.project.as_deref(), Some("Garden"));

    // Corrected again with its id, the project left out stays; null makes it shared.
    app.call(settings(json!({
        "factSet": { "id": "home/keys", "category": "home", "title": "Keys", "text": "On the hook" }
    })))
    .await;
    app.call(settings(json!({
        "factSet": { "id": "home/keys", "category": "home", "title": "Keys", "text": "On the hook", "project": null }
    })))
    .await;
    let page: FactsPage = app
        .call(Call::get(paths::FACTS).token(&session))
        .await
        .read_as();
    assert_eq!(page.facts[0].text, "On the hook");
    assert_eq!(page.facts[0].project, None);

    let refused = app
        .call(settings(
            json!({ "factSet": { "category": "pets", "title": "Cat", "text": "Grey" } }),
        ))
        .await;
    assert_eq!(refused.status, 400);
    assert_eq!(refused.json()["ok"], false);
    assert!(
        refused.json()["error"]
            .as_str()
            .unwrap()
            .starts_with("category must be one of")
    );
    let ignored = app
        .call(settings(json!({ "factSet": "not an object" })))
        .await;
    assert_eq!(ignored.status, 200);

    // personSet: who wakes the assistant.
    app.call_tool(
        "person_set",
        json!({ "who": "Ada Lovelace", "chat": "wa:662" }),
    )
    .await;
    let marked = app
        .call(settings(
            json!({ "personSet": { "id": "people/ada-lovelace", "important": true } }),
        ))
        .await;
    assert_eq!(marked.status, 200);
    let card = app
        .state
        .facts()
        .person_for("wa:662")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(card.important, Some(true));
    app.call(settings(
        json!({ "personSet": { "id": "people/ada-lovelace", "important": false } }),
    ))
    .await;
    let card = app
        .state
        .facts()
        .person_for("wa:662")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(card.important, None);
    let nobody = app
        .call(settings(
            json!({ "personSet": { "id": "people/nobody", "important": true } }),
        ))
        .await;
    assert_eq!(nobody.status, 400);
    assert_eq!(nobody.json()["error"], "no person people/nobody");

    // factDrop: forgotten.
    let dropped = app.call(settings(json!({ "factDrop": "home/keys" }))).await;
    assert_eq!(dropped.status, 200);
    assert_eq!(
        app.state.facts().facts(Some("home")).await.unwrap(),
        [] as [emberhut_core::memory::facts::Fact; 0]
    );
    let missing = app.call(settings(json!({ "factDrop": "home/keys" }))).await;
    assert_eq!(missing.status, 400);
    assert_eq!(missing.json()["error"], "no fact home/keys");
    assert_eq!(
        app.call(settings(json!({ "factDrop": 7 }))).await.status,
        200
    );

    assert_eq!(
        told(&app).await,
        [
            "The user corrected what you know: home/keys: In the drawer",
            "The user corrected what you know: home/keys: On the hook",
            "The user corrected what you know: home/keys: On the hook",
            "The user marked people/ada-lovelace as important: their messages wake you at once.",
            "The user unmarked people/ada-lovelace as important.",
            "The user told you to forget home/keys everywhere: it and 0 pieces of what was said are gone from your memory. Don't keep it again, and don't bring it up.",
        ]
    );
}

#[tokio::test]
async fn the_owner_drops_a_rule_on_trial() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let logged = app
        .call_tool(
            "correction_log",
            json!({ "said": "you never check", "wrong": "I guessed", "lesson": "Check first." }),
        )
        .await;
    assert!(text(&logged).contains("T1"));
    let page: FactsPage = app
        .call(Call::get(paths::FACTS).token(&session))
        .await
        .read_as();
    assert_eq!(page.trials.len(), 1);

    for ignored in [json!({ "trialDrop": "1" }), json!({ "trialDrop": 99 })] {
        let answer = app
            .call(Call::post(paths::SETTINGS).token(&session).json(&ignored))
            .await;
        assert_eq!(answer.status, 200);
    }
    assert_eq!(app.state.learning().trials().await.unwrap().len(), 1);

    let answer = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&session)
                .json(&json!({ "trialDrop": 1 })),
        )
        .await;
    assert_eq!(answer.status, 200);
    let page: FactsPage = app
        .call(Call::get(paths::FACTS).token(&session))
        .await
        .read_as();
    assert_eq!(page.trials, [] as [emberhut_api::facts::Trial; 0]);
    let review = app.call_tool("trial_drop", json!({ "id": 1 })).await;
    assert!(text(&review).starts_with("No trial rule T1 is running"));
}

#[tokio::test]
async fn the_owner_keeps_a_rule_on_trial_before_the_review() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.call_tool(
        "correction_log",
        json!({
            "said": "the reel is too short",
            "wrong": "I cut it to 20 seconds",
            "lesson": "Story reels run at least 60 seconds.",
            "lesson_to_user": "Story reels run at least 60 seconds; I check each cut's length before showing you."
        }),
    )
    .await;
    let page: FactsPage = app
        .call(Call::get(paths::FACTS).token(&session))
        .await
        .read_as();
    assert_eq!(
        page.trials[0].lesson.as_deref(),
        Some("Story reels run at least 60 seconds; I check each cut's length before showing you.")
    );

    for ignored in [json!({ "trialKeep": "1" }), json!({ "trialKeep": 99 })] {
        let answer = app
            .call(Call::post(paths::SETTINGS).token(&session).json(&ignored))
            .await;
        assert_eq!(answer.status, 200, "{}", answer.text);
    }
    assert_eq!(app.state.learning().trials().await.unwrap().len(), 1);
    assert_eq!(
        app.state.learning().lessons().await.unwrap(),
        [] as [emberhut_core::memory::learning::Lesson; 0]
    );

    let answer = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&session)
                .json(&json!({ "trialKeep": 1 })),
        )
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let page: FactsPage = app
        .call(Call::get(paths::FACTS).token(&session))
        .await
        .read_as();
    assert_eq!(page.trials, [] as [emberhut_api::facts::Trial; 0]);
    assert_eq!(page.lessons.len(), 1);
    assert_eq!(page.lessons[0].text, "Story reels run at least 60 seconds.");
    let db = Database::connect(&app.url).await.unwrap();
    assert_eq!(
        db.corrections().trial_end(1).await.unwrap(),
        Some(emberhut_db::TrialEnd::Kept(Some(page.lessons[0].id.get())))
    );

    let again = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&session)
                .json(&json!({ "trialKeep": 1 })),
        )
        .await;
    assert_eq!(again.status, 200);
    assert_eq!(
        app.state.learning().lessons().await.unwrap().len(),
        1,
        "kept once"
    );
    let review = app.call_tool("trial_drop", json!({ "id": 1 })).await;
    assert!(text(&review).starts_with("No trial rule T1 is running"));
}
