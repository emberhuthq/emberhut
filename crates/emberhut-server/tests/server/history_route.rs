//! `GET /api/history`: pages of a room's chat, older ones and what came after, read leniently,
//! with the state's `earlier` saying there is more.
#![expect(
    clippy::unwrap_used,
    reason = "a test helper stores messages: a failure there fails the test"
)]

use emberhut_api::chat::{ChatPage, HistoryQuery, join};
use emberhut_api::error::ErrorBody;
use emberhut_api::ids::MessageId;
use emberhut_api::state::State;
use emberhut_core::RoomId;
use emberhut_core::message::{Author, NewMessage};
use emberhut_db::Database;
use serde_json::json;

use crate::common::{Call, TestApp, assert_keys_within, fixture};

/// Stores `count` messages in Main, alternating who says them, through a second connection.
async fn say_many(app: &TestApp, count: usize) {
    let db = Database::connect(&app.url).await.unwrap();
    let messages = db.messages();
    for n in 1..=count {
        let author = if n % 2 == 1 {
            Author::User
        } else {
            Author::Assistant
        };
        messages
            .append(
                NewMessage {
                    room: RoomId::MAIN,
                    author,
                    text: format!("message {n}"),
                    reply_to: None,
                },
                app.clock_now(),
            )
            .await
            .unwrap();
    }
    db.close().await.unwrap();
}

fn ids(page: &ChatPage) -> Vec<i64> {
    page.chat.iter().map(|message| message.id.get()).collect()
}

#[tokio::test]
pub(crate) async fn pages_go_back_through_older_messages_and_catch_up_after_one() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    say_many(&app, 130).await;

    let state: State = app
        .call(Call::get("/api/state").token(&session))
        .await
        .read_as();
    assert_eq!(state.chat.len(), 60);
    assert!(state.earlier, "there is more before the state's chat");
    let oldest = state.chat[0].id;
    assert_eq!(oldest.get(), 71);

    let page = |query: HistoryQuery| {
        let call = Call::get("/api/history")
            .token(&session)
            .query(&query.to_query());
        async {
            let answer = app.call(call).await;
            assert_eq!(answer.status, 200);
            answer.read_as::<ChatPage>()
        }
    };
    let latest = page(HistoryQuery::latest(emberhut_api::ids::RoomId::MAIN)).await;
    assert_eq!(ids(&latest), (71..=130).collect::<Vec<_>>());
    let older = page(HistoryQuery::before(
        emberhut_api::ids::RoomId::MAIN,
        oldest,
    ))
    .await;
    assert_eq!(ids(&older), (11..=70).collect::<Vec<_>>());
    assert!(older.is_full());
    let oldest_page = page(HistoryQuery::before(
        emberhut_api::ids::RoomId::MAIN,
        MessageId::new(11),
    ))
    .await;
    assert_eq!(ids(&oldest_page), (1..=10).collect::<Vec<_>>());
    assert!(!oldest_page.is_full(), "the start of the room");
    assert!(oldest_page.chat[0].mine && !oldest_page.chat[1].mine);
    assert_eq!(oldest_page.chat[0].text, "message 1");

    let since = page(HistoryQuery::after(
        emberhut_api::ids::RoomId::MAIN,
        MessageId::new(125),
    ))
    .await;
    assert_eq!(ids(&since), [126, 127, 128, 129, 130]);
    let joined = join(&state.chat, &older.chat);
    assert_eq!(joined.len(), 120, "older pages join with what is held");
}

#[tokio::test]
async fn bounds_are_read_leniently_and_an_unknown_room_is_main() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    say_many(&app, 5).await;
    for (query, expected) in [
        ("before=abc", vec![1, 2, 3, 4, 5]),
        ("before=0", vec![1, 2, 3, 4, 5]),
        ("before=3.5", vec![1, 2, 3]),
        ("after=-2", vec![1, 2, 3, 4, 5]),
        ("after=2.5", vec![3, 4, 5]),
        ("room=99&before=3", vec![1, 2]),
        ("room=x&after=4", vec![5]),
        ("before=4&after=1", vec![2, 3]),
        // A repeated bound counts by its first value.
        ("before=4&before=2", vec![1, 2, 3]),
        ("after=1&after=3", vec![2, 3, 4, 5]),
        ("before=x&before=2", vec![1, 2, 3, 4, 5]),
    ] {
        let answer = app
            .call(Call::get("/api/history").token(&session).query(query))
            .await;
        assert_eq!(answer.status, 200, "{query}");
        assert_eq!(ids(&answer.read_as()), expected, "{query}");
    }
}

#[tokio::test]
async fn a_page_has_the_fixture_s_shape_and_its_replies_quote() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    say_many(&app, 2).await;
    let reply_to = 1;
    app.call(
        Call::post("/api/talk")
            .token(&session)
            .json(&json!({ "text": "An answer to the first", "replyTo": reply_to })),
    )
    .await;
    app.settle().await;
    let page = app
        .call(Call::get("/api/history").token(&session).query("before=99"))
        .await
        .json();
    assert_keys_within(&page, &fixture("history.json"), "history");
    let reply = &page["chat"][2];
    assert_eq!(reply["text"], "An answer to the first");
    assert_eq!(reply["reply"]["id"], reply_to);
    assert_eq!(reply["reply"]["text"], "message 1");
    assert_eq!(reply["reply"]["mine"], true);
    assert_eq!(reply["files"], json!([]));
}

#[tokio::test]
pub(crate) async fn history_needs_a_session() {
    let app = TestApp::new().await;
    let refused = app.call(Call::get("/api/history")).await;
    assert_eq!(refused.status, 401);
    assert_eq!(
        refused.read_as::<ErrorBody>(),
        ErrorBody::sign_in_required()
    );
}
