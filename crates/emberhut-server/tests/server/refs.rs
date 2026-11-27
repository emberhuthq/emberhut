//! The chips under the assistant's answers: the pull requests, tickets and reports an answer
//! links to, kept with it and shown on its line in the state's `chat` and in the history.

use emberhut_api::chat::ChatPage;
use emberhut_api::chat::RefKind;
use emberhut_api::paths;
use emberhut_core::RoomId;

use crate::common::{Call, TestApp};

#[tokio::test]
async fn an_answer_s_links_become_its_chips() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.assistant.answers(
        &[],
        Some(
            "Merged https://github.com/acme/site/pull/7 and filed \
             https://linear.app/acme/issue/OPS-3/flaky; the report is at \
             https://reports.example.com/weekly.html.",
        ),
    );
    app.state
        .talk()
        .talk(RoomId::MAIN, "Status?".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let state = app.state.state().state(RoomId::MAIN).await.unwrap();
    assert_eq!(state.chat[0].refs, [], "the person's message has none");
    let chips: Vec<(RefKind, &str)> = state.chat[1]
        .refs
        .iter()
        .map(|chip| (chip.kind.clone(), chip.label.as_str()))
        .collect();
    assert_eq!(
        chips,
        [
            (RefKind::Pr, "PR #7"),
            (RefKind::Ticket, "OPS-3"),
            (RefKind::Report, "Report")
        ]
    );
    assert_eq!(
        state.chat[1].refs[0].url.as_deref(),
        Some("https://github.com/acme/site/pull/7")
    );
    assert_eq!(state.chat[1].refs[0].title.as_deref(), Some("acme/site"));

    let page: ChatPage = app
        .call(Call::get(paths::HISTORY).token(&session))
        .await
        .read_as();
    assert_eq!(page.chat[1].refs, state.chat[1].refs);
}
