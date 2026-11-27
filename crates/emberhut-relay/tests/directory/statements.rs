//! The statements' freshness and replay rules (ADR 0007, section 5, "Replays").

use serde_json::json;

use crate::support::{
    ACCOUNT, Harness, VECTOR_MACHINE, VECTOR_TIME, link, machine_id, unlink, vector_link,
};

/// Five minutes, in milliseconds.
const FIVE_MINUTES: u64 = 5 * 60 * 1_000;

#[tokio::test]
async fn a_statement_is_fresh_for_five_minutes_either_side_of_the_clock() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    for (seed, time, fresh) in [
        (1, VECTOR_TIME - FIVE_MINUTES - 1, false),
        (2, VECTOR_TIME + FIVE_MINUTES + 1, false),
        (3, VECTOR_TIME - FIVE_MINUTES, true),
        (4, VECTOR_TIME + FIVE_MINUTES, true),
    ] {
        let answer = harness.link(&token, &link(seed, ACCOUNT, time)).await;
        if fresh {
            assert_eq!(answer.status, 200, "{answer:?}");
        } else {
            answer.refused(400, "stale");
        }
    }
    // The vectors' link, once the clock has moved on.
    harness.clock.set(VECTOR_TIME + FIVE_MINUTES + 1);
    let token = harness.session(ACCOUNT, &json!({}));
    harness
        .link(&token, &vector_link())
        .await
        .refused(400, "stale");
    // And a machine's own unlink.
    harness
        .unlink_itself(&machine_id(3), &unlink(3, VECTOR_TIME - 1))
        .await
        .refused(400, "stale");
}

#[tokio::test]
async fn a_statement_already_accepted_is_refused() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(harness.link(&token, &vector_link()).await.status, 200);
    harness
        .link(&token, &vector_link())
        .await
        .refused(400, "replayed");
    // Even after the machine was unlinked, and to an account that would take it.
    assert_eq!(harness.delete(&token, VECTOR_MACHINE).await.status, 204);
    harness
        .link(&token, &vector_link())
        .await
        .refused(400, "replayed");

    // An own unlink, twice.
    assert_eq!(
        harness
            .link(&token, &link(1, ACCOUNT, VECTOR_TIME))
            .await
            .status,
        200
    );
    let leave = unlink(1, VECTOR_TIME + 1);
    assert_eq!(
        harness.unlink_itself(&machine_id(1), &leave).await.status,
        204
    );
    harness
        .unlink_itself(&machine_id(1), &leave)
        .await
        .refused(400, "replayed");
}

#[tokio::test]
async fn an_unlink_captured_before_a_relink_cant_undo_it() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    // Signed first, posted last.
    let captured = unlink(1, VECTOR_TIME);
    assert_eq!(
        harness
            .link(&token, &link(1, ACCOUNT, VECTOR_TIME + 1_000))
            .await
            .status,
        200
    );
    harness
        .unlink_itself(&machine_id(1), &captured)
        .await
        .refused(400, "stale");
    assert_eq!(
        harness.list(&token).await.body["machines"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_link_older_than_the_machines_own_unlink_cant_undo_it() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    let captured = link(1, ACCOUNT, VECTOR_TIME + 1);
    assert_eq!(
        harness
            .link(&token, &link(1, ACCOUNT, VECTOR_TIME))
            .await
            .status,
        200
    );
    assert_eq!(
        harness
            .unlink_itself(&machine_id(1), &unlink(1, VECTOR_TIME + 2_000))
            .await
            .status,
        204
    );
    harness.link(&token, &captured).await.refused(400, "stale");
    // A newer link is taken.
    assert_eq!(
        harness
            .link(&token, &link(1, ACCOUNT, VECTOR_TIME + 3_000))
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn once_five_minutes_pass_the_replay_can_only_be_stale() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(harness.link(&token, &vector_link()).await.status, 200);
    harness.clock.set(VECTOR_TIME + FIVE_MINUTES + 1);
    let token = harness.session(ACCOUNT, &json!({}));
    // A newer statement prunes the old signature; the old one is then refused for its age.
    assert_eq!(
        harness
            .link(&token, &link(1, ACCOUNT, VECTOR_TIME + FIVE_MINUTES))
            .await
            .status,
        200
    );
    harness
        .link(&token, &vector_link())
        .await
        .refused(400, "stale");
}
