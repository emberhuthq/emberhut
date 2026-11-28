//! `GET /api/agent/accounts`: the agent CLI accounts the sessions run on, read from the
//! failover pool; and the waitlists (`GET /api/waitlist`, `PUT /api/waitlist/{list}`), kept on
//! this server.

use emberhut_api::agent_accounts::AgentAccounts;
use emberhut_api::paths;
use emberhut_api::waitlist::{Waitlist, Waitlists};
use emberhut_core::RoomId;
use serde_json::json;

use crate::common::{Call, TestApp};
use crate::fake_cli::{FakeCli, agent, config, error_result, init, is_user, rejected, result};

#[tokio::test]
pub(crate) async fn the_accounts_say_which_can_take_work_and_which_the_main_room_runs_on() {
    // The first account is refused until 1_900_000_000 (2030); the second answers.
    let cli = FakeCli::new(|launch, line| {
        if !is_user(line) {
            return Vec::new();
        }
        if launch.var("CLAUDE_CONFIG_DIR") == Some("/accounts/first") {
            vec![
                init("s-1"),
                rejected(1_900_000_000),
                error_result("You've hit your limit · resets 5pm"),
            ]
        } else {
            vec![init("s-1"), result("Done")]
        }
    });
    let app = TestApp::with_agent(agent(
        &cli,
        config(&[("first", "/accounts/first"), ("second", "/accounts/second")]),
    ))
    .await;
    let session = app.sign_in().await;
    let before: AgentAccounts = app
        .call(Call::get(paths::AGENT_CLI_ACCOUNTS).token(&session))
        .await
        .read_as();
    assert!(before.moves);
    assert!(before.accounts.iter().all(|account| account.can_work));
    assert_eq!(
        before
            .accounts
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );

    app.state
        .talk()
        .talk(RoomId::MAIN, "Book it".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;

    let answer = app
        .call(Call::get(paths::AGENT_CLI_ACCOUNTS).token(&session))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    let body: serde_json::Value = serde_json::from_str(&answer.text).unwrap();
    assert_eq!(
        body,
        json!({ "moves": true, "accounts": [
            { "id": "first", "canWork": false, "restingUntil": "2030-03-17T17:46:40.000Z",
              "why": "hit its five-hour limit", "on": false },
            { "id": "second", "canWork": true, "restingUntil": null, "why": null, "on": true },
        ] })
    );
}

#[tokio::test]
pub(crate) async fn a_server_without_accounts_lists_none_and_only_to_a_session() {
    let app = TestApp::new().await;
    let unsigned = app.call(Call::get(paths::AGENT_CLI_ACCOUNTS)).await;
    assert_eq!(unsigned.status, 401);
    let session = app.sign_in().await;
    let page: AgentAccounts = app
        .call(Call::get(paths::AGENT_CLI_ACCOUNTS).token(&session))
        .await
        .read_as();
    assert_eq!(
        page,
        AgentAccounts {
            accounts: Vec::new(),
            moves: false
        }
    );
}

#[tokio::test]
pub(crate) async fn joining_the_cloud_home_waitlist_is_kept_once_and_only_from_a_signed_in_device()
{
    let app = TestApp::new().await;
    let path = paths::WAITLIST.replace("{list}", "cloud-home");
    assert_eq!(app.call(Call::put(&path)).await.status, 401);
    assert_eq!(app.call(Call::get(paths::WAITLISTS)).await.status, 401);
    let session = app.sign_in().await;

    let lists: Waitlists = app
        .call(Call::get(paths::WAITLISTS).token(&session))
        .await
        .read_as();
    assert_eq!(lists.lists.len(), 1);
    assert_eq!(lists.lists[0].id, "cloud-home");
    assert!(!lists.lists[0].joined);
    assert_eq!(lists.lists[0].joined_at, None);

    let first: Waitlist = app.call(Call::put(&path).token(&session)).await.read_as();
    assert!(first.joined);
    assert!(first.joined_at.is_some());
    app.clock.advance(jiff::SignedDuration::from_mins(5));
    let again: Waitlist = app.call(Call::put(&path).token(&session)).await.read_as();
    assert_eq!(again.joined_at, first.joined_at, "the first time is kept");

    let lists: Waitlists = app
        .call(Call::get(paths::WAITLISTS).token(&session))
        .await
        .read_as();
    assert_eq!(lists.lists, [first]);

    let unknown = app
        .call(Call::put(&paths::WAITLIST.replace("{list}", "moon-base")).token(&session))
        .await;
    assert_eq!(unknown.status, 400);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&unknown.text).unwrap()["error"],
        "There is no such waitlist."
    );
}
