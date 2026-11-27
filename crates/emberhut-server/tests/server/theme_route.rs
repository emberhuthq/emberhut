//! `GET /api/theme` and `POST /api/theme`: the page, changing the choice, previews, refusals,
//! and the state's `theme`.

use emberhut_api::error::ErrorBody;
use emberhut_api::paths;
use emberhut_api::state::State;
use emberhut_api::theme::{Appearance, ThemeChoice, ThemePage, ThemeSetReply};
use emberhut_db::Database;
use serde_json::json;

use crate::common::{Call, TestApp, fixture};

#[tokio::test]
pub(crate) async fn the_theme_page_matches_its_fixture() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let answer = app.call(Call::get("/api/theme").token(&session)).await;
    assert_eq!(answer.status, 200);
    let page: ThemePage = answer.read_as();
    assert_eq!(
        page.choice,
        ThemeChoice::default(),
        "blue, as the system is, never set"
    );

    // The fixture's page for blue in dark mode, colour for colour.
    let reply: ThemeSetReply = app
        .call(
            Call::post(paths::THEME)
                .token(&session)
                .json(&json!({ "appearance": "dark" })),
        )
        .await
        .read_as();
    let expected: ThemePage = serde_json::from_value(fixture("theme_page.json")).unwrap();
    let ThemePage {
        version,
        choice,
        tokens,
        contrast,
        presets,
        names,
    } = reply.page;
    assert_eq!(version, expected.version);
    assert_eq!(tokens, expected.tokens);
    assert_eq!(contrast, expected.contrast);
    assert_eq!(presets.len(), 7);
    for preset in &expected.presets {
        assert_eq!(
            presets.iter().find(|ours| ours.id == preset.id),
            Some(preset),
            "the fixture's presets, as listed there"
        );
    }
    assert_eq!(names, expected.names);
    assert_eq!(
        (choice.preset, choice.appearance, choice.accent),
        (
            expected.choice.preset,
            expected.choice.appearance,
            expected.choice.accent
        )
    );
}

#[tokio::test]
pub(crate) async fn a_change_is_kept_and_the_state_carries_it() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let state: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert_eq!(state.theme, Some(ThemeChoice::default()));

    let answer = app
        .call(
            Call::post(paths::THEME)
                .token(&session)
                .json(&json!({ "preset": " Teal ", "appearance": "DARK", "accent": "#36f" })),
        )
        .await;
    assert_eq!(answer.status, 200);
    let reply: ThemeSetReply = answer.read_as();
    assert!(reply.ok);
    let choice = reply.page.choice.clone();
    assert_eq!(choice.preset, "teal");
    assert_eq!(choice.appearance, Appearance::Dark);
    assert_eq!(choice.accent.as_deref(), Some("#3366FF"));
    assert!(choice.at.is_some(), "a saved change has its time");
    assert_eq!(reply.page.contrast.dark.asked, "#3366FF");

    let page: ThemePage = app
        .call(Call::get(paths::THEME).token(&session))
        .await
        .read_as();
    assert_eq!(page.choice, choice);
    let state: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert_eq!(state.theme, Some(choice.clone()), "the state's theme");

    // Only the fields given change; `accent: null` (or blank) is the preset's own again.
    app.advance_minutes(1);
    let reply: ThemeSetReply = app
        .call(
            Call::post(paths::THEME)
                .token(&session)
                .json(&json!({ "accent": null })),
        )
        .await
        .read_as();
    assert_eq!(reply.page.choice.preset, "teal");
    assert_eq!(reply.page.choice.appearance, Appearance::Dark);
    assert_eq!(reply.page.choice.accent, None);
    assert_ne!(
        reply.page.choice.at, choice.at,
        "a newer at: other devices fetch"
    );

    let db = Database::connect(&app.url).await.unwrap();
    let log: Vec<String> = db
        .log()
        .recent(10)
        .await
        .unwrap()
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert!(
        log.iter()
            .any(|line| line == "The theme is now teal with #3366FF, dark"),
        "{log:?}"
    );
}

#[tokio::test]
pub(crate) async fn a_preview_saves_nothing() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    let reply: ThemeSetReply = app
        .call(
            Call::post(paths::THEME)
                .token(&session)
                .json(&json!({ "preset": "rose", "accent": "#FFFF00", "preview": true })),
        )
        .await
        .read_as();
    assert_eq!(reply.page.choice.preset, "rose");
    assert_eq!(reply.page.choice.at, None, "not saved: no time");
    assert!(
        reply.page.contrast.light.adjusted,
        "yellow is darkened to read on light"
    );
    assert_ne!(reply.page.tokens.light.accent, "#FFFF00");
    let page: ThemePage = app
        .call(Call::get(paths::THEME).token(&session))
        .await
        .read_as();
    assert_eq!(page.choice, ThemeChoice::default());
}

#[tokio::test]
pub(crate) async fn a_value_the_theme_does_not_take_is_400_and_changes_nothing() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    for (body, error) in [
        (
            json!({ "preset": "pink" }),
            "preset: one of graphite, blue, teal, green, orange, rose, purple",
        ),
        (
            json!({ "preset": 5 }),
            "preset: one of graphite, blue, teal, green, orange, rose, purple",
        ),
        (
            json!({ "appearance": "dim" }),
            "appearance: system, light or dark",
        ),
        (
            json!({ "preset": "green", "accent": "red" }),
            "accent: a hex colour like #3366FF (or null for the preset's own)",
        ),
        (
            json!({ "accent": 12 }),
            "accent: a hex colour like #3366FF (or null for the preset's own)",
        ),
        (json!([]), "theme: { preset?, appearance?, accent? }"),
    ] {
        let answer = app
            .call(Call::post(paths::THEME).token(&session).json(&body))
            .await;
        assert_eq!(answer.status, 400, "{body}");
        assert_eq!(
            answer.json(),
            json!({ "ok": false, "error": error }),
            "{body}"
        );
    }
    let page: ThemePage = app
        .call(Call::get(paths::THEME).token(&session))
        .await
        .read_as();
    assert_eq!(page.choice, ThemeChoice::default(), "nothing was saved");

    let huge = json!({ "preset": "x".repeat(3 * 1024 * 1024) }).to_string();
    let too_large = app
        .call(Call::post(paths::THEME).token(&session).raw(&huge))
        .await;
    assert_eq!(too_large.status, 413);
}

#[tokio::test]
pub(crate) async fn the_theme_needs_a_session() {
    let app = TestApp::new().await;
    for call in [
        Call::get(paths::THEME),
        Call::post(paths::THEME).json(&json!({ "preset": "teal" })),
    ] {
        let refused = app.call(call).await;
        assert_eq!(refused.status, 401);
        assert_eq!(
            refused.read_as::<ErrorBody>(),
            ErrorBody::sign_in_required()
        );
    }
}
