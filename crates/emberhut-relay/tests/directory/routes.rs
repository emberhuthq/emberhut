//! directory.md's routes and its errors table, row by row.

use std::time::Duration;

use emberhut_api::directory::{LinkedMachine, MachineList};
use emberhut_relay::directory::{GatewayDirectory as _, Rate};
use futures_util::future::join_all;
use serde_json::{Value, json};

use crate::support::{
    ACCOUNT, Harness, Setup, VECTOR_MACHINE, VECTOR_TIME, link, link_at, machine_id, unlink,
    unlink_at, vector_link, vectors,
};

#[tokio::test]
async fn the_vectors_link_lists_and_reads_as_the_contract_says() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    let linked = harness.link(&token, &vector_link()).await;
    assert_eq!(linked.status, 200, "{linked:?}");
    // The vectors' listed machine, as it is when just linked.
    let mut expected = vectors("directory.json")["messages"]["machines"]["machines"][0].clone();
    expected["connected"] = json!(false);
    expected["lastSeen"] = Value::Null;
    expected["linkedAt"] = json!(VECTOR_TIME);
    assert_eq!(expected["id"], VECTOR_MACHINE);
    assert_eq!(linked.body, expected);
    serde_json::from_value::<LinkedMachine>(linked.body.clone()).unwrap();

    let listed = harness.list(&token).await;
    assert_eq!(listed.status, 200);
    assert_eq!(listed.body, json!({ "machines": [linked.body] }));
    serde_json::from_value::<MachineList>(listed.body).unwrap();

    // Another account sees none of it.
    let other = harness
        .list(&harness.session("user_other", &json!({})))
        .await;
    assert_eq!(other.body, json!({ "machines": [] }));
}

#[tokio::test]
async fn readers_ignore_fields_they_dont_know() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    let link = vectors("connect.json")["withUnknownFields"]["signedLink"].clone();
    assert!(link.get("fromALaterVersion").is_some());
    assert_eq!(harness.link(&token, &link).await.status, 200);
    let unlink = vectors("directory.json")["withUnknownFields"]["unlink"].clone();
    assert_eq!(
        harness.unlink_itself(VECTOR_MACHINE, &unlink).await.status,
        204
    );
}

#[tokio::test]
async fn relinking_in_the_same_account_only_updates_the_name() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    let first = harness
        .link(
            &token,
            &link_at(1, "Old name", ACCOUNT, "relay.example", VECTOR_TIME),
        )
        .await;
    assert_eq!(first.status, 200);
    harness.clock.set(VECTOR_TIME + 1_000);
    let again = harness
        .link(
            &token,
            &link_at(1, "New name", ACCOUNT, "relay.example", VECTOR_TIME + 1_000),
        )
        .await;
    assert_eq!(again.status, 200, "{again:?}");
    assert_eq!(again.body["name"], "New name");
    assert_eq!(again.body["linkedAt"], VECTOR_TIME);
    assert_eq!(
        harness.list(&token).await.body["machines"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn bad_request_for_bodies_it_cant_read_and_statements_for_elsewhere() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    let post = |body: String, content_type: &'static str| {
        harness.post_raw("/v1/machines", &token, body, content_type)
    };
    post("not json".to_owned(), "application/json")
        .await
        .refused(400, "bad-request");
    post("{}".to_owned(), "application/json")
        .await
        .refused(400, "bad-request");
    post(vector_link().to_string(), "text/plain")
        .await
        .refused(400, "bad-request");
    post(" ".repeat(20 * 1024), "application/json")
        .await
        .refused(400, "bad-request");

    // A name that isn't trimmed doesn't read, whatever it signs.
    let mut untrimmed = vector_link();
    untrimmed["name"] = json!(" Main computer");
    harness
        .link(&token, &untrimmed)
        .await
        .refused(400, "bad-request");
    let mut long = vector_link();
    long["name"] = json!("x".repeat(61));
    harness
        .link(&token, &long)
        .await
        .refused(400, "bad-request");

    // A statement for another relay.
    harness
        .link(
            &token,
            &link_at(1, "Machine", ACCOUNT, "other.example", VECTOR_TIME),
        )
        .await
        .refused(400, "bad-request");

    // The machine's own unlink: for another relay, for another machine than the path's, or
    // with a path that isn't a machine id.
    harness
        .unlink_itself(&machine_id(1), &unlink_at(1, "other.example", VECTOR_TIME))
        .await
        .refused(400, "bad-request");
    harness
        .unlink_itself(&machine_id(2), &unlink(1, VECTOR_TIME))
        .await
        .refused(400, "bad-request");
    harness
        .unlink_itself("not-a-machine", &unlink(1, VECTOR_TIME))
        .await
        .refused(400, "bad-request");
    for check in vectors("directory.json")["unlinkChecks"]
        .as_array()
        .unwrap()
    {
        let mut body = vectors("directory.json")["messages"]["unlink"].clone();
        body["relayDomain"] = check["relayDomain"].clone();
        let path = check["path"].as_str().unwrap();
        let answer = harness.unlink_itself(path, &body).await;
        match check["expect"].as_str().unwrap() {
            // The statement checks pass; the machine isn't linked.
            "ok" => answer.refused(404, "not-found"),
            code => answer.refused(400, code),
        }
    }
}

#[tokio::test]
async fn sign_in_without_a_token_that_passes() {
    let harness = Harness::start().await;
    let no_token = crate::support::Answer::of(
        harness
            .http
            .get(harness.url("/v1/machines"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    no_token.refused(401, "sign-in");
    let basic = crate::support::Answer::of(
        harness
            .http
            .get(harness.url("/v1/machines"))
            .basic_auth("user", Some("password"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    basic.refused(401, "sign-in");
    harness.list("opaque-token").await.refused(401, "sign-in");
    harness
        .link("opaque-token", &vector_link())
        .await
        .refused(401, "sign-in");
    harness
        .delete("opaque-token", VECTOR_MACHINE)
        .await
        .refused(401, "sign-in");
    // The fixtures' refusal is this one.
    let answer = harness.list("opaque-token").await;
    assert_eq!(answer.body, vectors("directory.json")["messages"]["error"]);
}

#[tokio::test]
async fn bad_signature_and_not_yours() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    let mut renamed = vector_link();
    renamed["name"] = json!("Another name");
    harness
        .link(&token, &renamed)
        .await
        .refused(403, "bad-signature");
    let mut unlink = vectors("directory.json")["messages"]["unlink"].clone();
    unlink["time"] = json!(VECTOR_TIME + 1);
    harness
        .unlink_itself(VECTOR_MACHINE, &unlink)
        .await
        .refused(403, "bad-signature");

    let other = harness.session("user_other", &json!({}));
    harness
        .link(&other, &vector_link())
        .await
        .refused(403, "not-yours");
}

#[tokio::test]
async fn not_found_outside_the_account() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    harness
        .delete(&token, &machine_id(1))
        .await
        .refused(404, "not-found");
    harness
        .delete(&token, "not-a-machine")
        .await
        .refused(404, "not-found");
    // Another account's machine is not in this one.
    assert_eq!(harness.link(&token, &vector_link()).await.status, 200);
    let other = harness.session("user_other", &json!({}));
    harness
        .delete(&other, VECTOR_MACHINE)
        .await
        .refused(404, "not-found");
    // A machine that isn't linked can't unlink itself.
    harness
        .unlink_itself(&machine_id(1), &unlink(1, VECTOR_TIME))
        .await
        .refused(404, "not-found");
}

#[tokio::test]
async fn the_cap_refuses_the_sixth_machine_but_not_a_relink() {
    let harness = Harness::start().await;
    let token = harness.session(ACCOUNT, &json!({}));
    for seed in 1..=5 {
        let answer = harness
            .link(&token, &link(seed, ACCOUNT, VECTOR_TIME))
            .await;
        assert_eq!(answer.status, 200, "{answer:?}");
    }
    let full = harness.link(&token, &link(6, ACCOUNT, VECTOR_TIME)).await;
    full.refused(409, "account-full");
    assert_eq!(
        full.body["error"],
        "This account has all the machines it can hold. Unlink one first."
    );
    // Relinking one already there only renames it.
    let renamed = harness
        .link(
            &token,
            &link_at(3, "Renamed", ACCOUNT, "relay.example", VECTOR_TIME + 1),
        )
        .await;
    assert_eq!(renamed.status, 200);
    // Another account has its own cap.
    let other = harness.session("user_other", &json!({}));
    assert_eq!(
        harness
            .link(&other, &link(6, "user_other", VECTOR_TIME))
            .await
            .status,
        200
    );
    // Unlinking one makes room.
    assert_eq!(harness.delete(&token, &machine_id(1)).await.status, 204);
    assert_eq!(
        harness
            .link(&token, &link(7, ACCOUNT, VECTOR_TIME))
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn a_cap_set_lower_is_followed() {
    let harness = Harness::with(Setup {
        account_cap: 1,
        ..Setup::default()
    })
    .await;
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(
        harness
            .link(&token, &link(1, ACCOUNT, VECTOR_TIME))
            .await
            .status,
        200
    );
    harness
        .link(&token, &link(2, ACCOUNT, VECTOR_TIME))
        .await
        .refused(409, "account-full");
}

#[tokio::test]
async fn a_machine_linked_elsewhere_is_refused() {
    let harness = Harness::start().await;
    let first = harness.session(ACCOUNT, &json!({}));
    assert_eq!(
        harness
            .link(&first, &link(1, ACCOUNT, VECTOR_TIME))
            .await
            .status,
        200
    );
    let second = harness.session("user_other", &json!({}));
    let answer = harness
        .link(&second, &link(1, "user_other", VECTOR_TIME + 1))
        .await;
    answer.refused(409, "linked-elsewhere");
    assert_eq!(
        answer.body["error"],
        "This machine is linked to another account. Unlink it there first."
    );
    // Once unlinked there, it links here.
    assert_eq!(harness.delete(&first, &machine_id(1)).await.status, 204);
    assert_eq!(
        harness
            .link(&second, &link(1, "user_other", VECTOR_TIME + 2))
            .await
            .status,
        200
    );
}

#[tokio::test]
async fn limit_with_retry_after() {
    let harness = Harness::with(Setup {
        per_account: Rate {
            burst: 2,
            every: Duration::from_secs(30),
        },
        ..Setup::default()
    })
    .await;
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(harness.list(&token).await.status, 200);
    assert_eq!(harness.list(&token).await.status, 200);
    let limited = harness.list(&token).await;
    limited.refused(429, "limit");
    let seconds: u64 = limited.header("retry-after").unwrap().parse().unwrap();
    assert!((29..=30).contains(&seconds), "{seconds}");
    // Another account has its own budget.
    let other = harness.session("user_other", &json!({}));
    assert_eq!(harness.list(&other).await.status, 200);
}

#[tokio::test]
async fn a_machines_own_unlinks_are_limited_per_machine() {
    let harness = Harness::start().await;
    // Someone who knows only the id can't use the machine's budget up: unsigned or
    // wrongly signed unlinks are refused before they are counted.
    for time in 0..10 {
        let mut forged = unlink(1, VECTOR_TIME + time);
        forged["signature"] = unlink(2, VECTOR_TIME + time)["signature"].clone();
        harness
            .unlink_itself(&machine_id(1), &forged)
            .await
            .refused(403, "bad-signature");
    }
    // Five at once, then one every 12 s.
    for _ in 0..5 {
        harness
            .unlink_itself(&machine_id(1), &unlink(1, VECTOR_TIME))
            .await
            .refused(404, "not-found");
    }
    harness
        .unlink_itself(&machine_id(1), &unlink(1, VECTOR_TIME))
        .await
        .refused(429, "limit");
    harness
        .unlink_itself(&machine_id(2), &unlink(2, VECTOR_TIME))
        .await
        .refused(404, "not-found");
}

#[tokio::test]
async fn the_account_unlinks_and_the_gateways_hear_of_it() {
    let harness = Harness::start().await;
    let mut unlinked = harness.gateway.unlinked();
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(harness.link(&token, &vector_link()).await.status, 200);
    let answer = harness.delete(&token, VECTOR_MACHINE).await;
    assert_eq!((answer.status, answer.body.clone()), (204, Value::Null));
    assert_eq!(unlinked.recv().await.unwrap().to_string(), VECTOR_MACHINE);
    assert_eq!(harness.list(&token).await.body, json!({ "machines": [] }));
}

#[tokio::test]
async fn a_machine_leaves_its_account_by_itself() {
    let harness = Harness::start().await;
    let mut unlinked = harness.gateway.unlinked();
    let token = harness.session(ACCOUNT, &json!({}));
    assert_eq!(harness.link(&token, &vector_link()).await.status, 200);
    let unlink = vectors("directory.json")["messages"]["unlink"].clone();
    let answer = harness.unlink_itself(VECTOR_MACHINE, &unlink).await;
    assert_eq!((answer.status, answer.body.clone()), (204, Value::Null));
    assert_eq!(unlinked.recv().await.unwrap().to_string(), VECTOR_MACHINE);
    assert_eq!(harness.list(&token).await.body, json!({ "machines": [] }));
}

#[tokio::test]
async fn unknown_routes_and_methods_are_refused_in_the_same_form() {
    let harness = Harness::start().await;
    let missing = crate::support::Answer::of(
        harness
            .http
            .get(harness.url("/v2/machines"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    missing.refused(404, "not-found");
    let wrong_method = crate::support::Answer::of(
        harness
            .http
            .put(harness.url("/v1/machines"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(wrong_method.status, 405);
    assert_eq!(wrong_method.body["code"], "bad-request");
}

#[tokio::test]
async fn two_accounts_linking_one_machine_at_once_get_one_link() {
    let harness = Harness::with(Setup {
        account_cap: 10,
        ..Setup::default()
    })
    .await;
    let first = harness.session(ACCOUNT, &json!({}));
    let second = harness.session("user_other", &json!({}));
    for seed in 1..=10 {
        let (for_first, for_second) = (
            link(seed, ACCOUNT, VECTOR_TIME),
            link(seed, "user_other", VECTOR_TIME),
        );
        let (a, b) = tokio::join!(
            harness.link(&first, &for_first),
            harness.link(&second, &for_second),
        );
        let (won, lost) = if a.status == 200 { (a, b) } else { (b, a) };
        assert_eq!(won.status, 200, "{won:?}");
        lost.refused(409, "linked-elsewhere");
    }
    let held = |token| {
        let harness = &harness;
        async move {
            harness.list(token).await.body["machines"]
                .as_array()
                .unwrap()
                .len()
        }
    };
    assert_eq!(held(&first).await + held(&second).await, 10);
}

#[tokio::test]
async fn links_racing_the_cap_link_no_more_than_it() {
    let harness = Harness::with(Setup {
        account_cap: 3,
        ..Setup::default()
    })
    .await;
    let token = harness.session(ACCOUNT, &json!({}));
    let bodies: Vec<Value> = (1..=8)
        .map(|seed| link(seed, ACCOUNT, VECTOR_TIME))
        .collect();
    let answers = join_all(bodies.iter().map(|body| harness.link(&token, body))).await;
    let linked = answers.iter().filter(|answer| answer.status == 200).count();
    assert_eq!(linked, 3);
    for answer in answers.iter().filter(|answer| answer.status != 200) {
        answer.refused(409, "account-full");
    }
    assert_eq!(
        harness.list(&token).await.body["machines"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
