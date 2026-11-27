//! The state's `awareness`: each source the assistant reads (the coding-thread app on each
//! machine) and how fresh it is.

use emberhut_api::wake::SourceState;
use emberhut_core::RoomId;

use crate::common::TestApp;
use emberhut_server::services::coding_app::ListedThread;

#[tokio::test]
async fn each_machine_s_threads_are_a_source_down_while_its_app_cannot_be_reached() {
    let app = TestApp::new().await;
    let state = app.state.state().state(RoomId::MAIN).await.unwrap();
    assert!(state.awareness.is_empty(), "no machine looked at yet");

    app.coding_app.on(&["laptop", "desktop"]);
    let fresh = app
        .clock_now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let thread = ListedThread {
        id: "t-1".to_owned(),
        title: "Fix the build".to_owned(),
        project: "atlas".to_owned(),
        status: "running".to_owned(),
        updated_at: fresh.clone(),
        parent: None,
    };
    app.coding_app.thread("laptop", thread, Vec::new());
    app.coding_app.take_down("desktop", "connection refused");
    app.state.threads().poll().await.unwrap();

    let state = app.state.state().state(RoomId::MAIN).await.unwrap();
    let sources: Vec<(&str, SourceState, &str)> = state
        .awareness
        .iter()
        .map(|source| (source.id.as_str(), source.state, source.detail.as_str()))
        .collect();
    assert_eq!(
        sources,
        [
            ("desktop", SourceState::Down, "0 threads"),
            ("laptop", SourceState::Ok, "1 thread")
        ]
    );
    assert!(state.awareness[0].error.is_some());
    assert_eq!(state.awareness[1].name, "Coding threads on laptop");
    assert_eq!(
        state.awareness[1]
            .at
            .as_ref()
            .map(|at| at.as_str().to_owned()),
        Some(fresh),
        "the time of its last answered poll"
    );
}

#[tokio::test]
async fn a_machine_whose_app_has_not_answered_for_five_minutes_is_stale() {
    let app = TestApp::new().await;
    app.coding_app.on(&["laptop"]);
    // A quiet machine: its only thread changed long ago, which does not make it stale.
    let thread = ListedThread {
        id: "t-1".to_owned(),
        title: "Old work".to_owned(),
        project: "atlas".to_owned(),
        status: "done".to_owned(),
        updated_at: "2026-01-01T00:00:00.000Z".to_owned(),
        parent: None,
    };
    app.coding_app.thread("laptop", thread, Vec::new());
    app.state.threads().poll().await.unwrap();
    let states = |app: &TestApp| {
        let app = app.state.clone();
        async move {
            app.state()
                .state(RoomId::MAIN)
                .await
                .unwrap()
                .awareness
                .iter()
                .map(|source| source.state)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(states(&app).await, [SourceState::Ok]);

    app.advance_minutes(4);
    assert_eq!(states(&app).await, [SourceState::Ok]);
    app.advance_minutes(2);
    assert_eq!(
        states(&app).await,
        [SourceState::Stale],
        "no answer for 6 minutes"
    );
    app.state.threads().poll().await.unwrap();
    assert_eq!(states(&app).await, [SourceState::Ok], "polled again");
}
