//! A key or token written into a task's history is masked there: the history reaches the
//! apps and the assistant.

use std::sync::Arc;

use emberhut_core::task::Who;
use emberhut_server::services::guard::{GitRemotes, GuardConfig, NoJudge};

use crate::guard_common::{guard_app, task};

#[tokio::test]
async fn a_token_in_a_history_line_is_masked() {
    let app = guard_app(
        Arc::new(NoJudge),
        Arc::new(GitRemotes),
        GuardConfig::default(),
    )
    .await;
    let id = task(&app, "the Slack app", None).await;
    let token = format!("xoxb-{}", "12345678-abcdefgh");
    app.state
        .tasks()
        .note(
            id,
            Who::Worker,
            &format!("Made the bot token {token} and saved it"),
            false,
        )
        .await
        .unwrap();
    let history = app.state.tasks().history_of(id).await.unwrap();
    let line = &history.last().unwrap().text;
    assert_eq!(line, "Made the bot token [secret: xoxb-…] and saved it");
    assert!(!line.contains(&token));
}
