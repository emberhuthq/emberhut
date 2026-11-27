//! Approvals, against the threat model: a decision counts only as its channel proves it, once,
//! on the exact payload; an approval is used once, by the step it was given for, while the
//! policy still names its approver; the fixed core is the owner's whatever a policy says.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::time::Duration;

use emberhut_core::approvals::webhook;
use emberhut_core::approvals::{ActionKind, ApprovalRequest, Approver, DecidedVia, RequestState};
use emberhut_server::services::approvals::{
    ApprovalError, Asked, Delivery, Requested, StepAsk, StepUse, UseOutcome,
};
use http_body_util::BodyExt as _;
use jiff::SignedDuration;
use serde_json::{Value, json};

use crate::approvals_common::{
    BOT_TOKEN, Desk, SIGNING, TEAM, USER, WEBHOOK_SECRET, WEBHOOK_URL, slack_click, slack_click_on,
    slack_signature,
};
use crate::common::{Call, TestApp};

/// A post of `text` with `files` to the example blog.
fn post(text: &str, files: Vec<std::path::PathBuf>) -> StepAsk {
    StepAsk {
        kind: ActionKind::Post,
        target: "example blog".to_owned(),
        summary: "The launch post".to_owned(),
        text: Some(text.to_owned()),
        files,
        amount_cents: None,
        task: None,
    }
}

/// Asks for `ask` as the agent: the request, which must wait.
async fn ask(desk: &Desk, ask: &StepAsk) -> Requested {
    match desk.app.state.approvals().request(&desk.agent, ask).await {
        Ok(Asked::Waiting(requested)) => *requested,
        other => panic!("not waiting: {other:?}"),
    }
}

/// The link the approver's app was last sent: its token.
fn last_link_token(desk: &Desk) -> String {
    let calls = desk.webhook.calls.lock().unwrap();
    let body: Value = serde_json::from_slice(&calls.last().unwrap().body).unwrap();
    body["link"]
        .as_str()
        .unwrap()
        .strip_prefix("https://emberhut.example/approve/")
        .unwrap()
        .to_owned()
}

/// The approver's app answers request `id` with `decision` on `hash`, signed with `secret` at
/// `timestamp`.
async fn webhook_answer(
    desk: &Desk,
    id: i64,
    body: &Value,
    secret: &str,
    timestamp: i64,
) -> crate::common::Answer {
    let path = format!("/api/approvals/{id}/webhook");
    let body = body.to_string();
    let signature = webhook::sign(secret.as_bytes(), timestamp, "POST", &path, body.as_bytes());
    desk.app
        .call(
            Call::post(&path)
                .header(webhook::TIMESTAMP_HEADER, &timestamp.to_string())
                .header(webhook::SIGNATURE_HEADER, &signature)
                .raw(&body),
        )
        .await
}

/// Approves `request` through its approver's app.
async fn approve_by_webhook(desk: &Desk, request: &ApprovalRequest) {
    let answer = webhook_answer(
        desk,
        request.id.get(),
        &json!({ "decision": "approve", "payloadHash": request.payload_hash.to_hex(), "by": "lead@example" }),
        WEBHOOK_SECRET,
        desk.app.clock_now().as_second(),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
}

/// Uses the approval `id` for a step with `text` and `files`.
async fn use_step(
    desk: &Desk,
    id: emberhut_core::ApprovalId,
    text: &str,
    files: Vec<std::path::PathBuf>,
) -> UseOutcome {
    desk.app
        .state
        .approvals()
        .use_for_step(
            &desk.agent,
            id,
            &StepUse {
                text: Some(text.to_owned()),
                files,
                by: "task 12".to_owned(),
            },
        )
        .await
        .unwrap()
}

/// A request to the approver goes out word for word: the DM through the agent's bot token, its
/// file uploaded and listed with its hash, the buttons carrying the request and its hash; the
/// approver's app gets it signed with their secret, with the one-time link.
#[tokio::test]
pub(crate) async fn a_request_reaches_its_approver_word_for_word() {
    let desk = Desk::new().await;
    let cover = desk.file("cover.png", b"png bytes");
    let words = "We shipped <it> & *more*.\nSecond line.";
    let requested = ask(&desk, &post(words, vec![cover])).await;
    assert!(requested.made);
    assert_eq!(requested.delivery, Delivery::Sent);
    let request = &requested.request;
    assert_eq!(request.approver.role(), "the approver");

    let posts = desk.slack.posts.lock().unwrap().clone();
    assert_eq!(posts.len(), 1);
    let (token, message) = &posts[0];
    assert_eq!(token, BOT_TOKEN);
    assert_eq!(message.user, USER);
    assert_eq!(message.text.as_deref(), Some(words));
    assert_eq!(message.files.len(), 1);
    assert_eq!(message.files[0].bytes, b"png bytes");
    assert!(message.file_lines[0].contains(&request.files[0].sha256.to_hex()));
    assert_eq!(
        message.button_value,
        format!("{}.{}", request.id, request.payload_hash)
    );
    assert!(
        !format!("{message:?}").contains("/approve/"),
        "the DM, which the agent's bot can read back, carries no link"
    );

    let calls = desk.webhook.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.url.as_str(), WEBHOOK_URL);
    webhook::verify(
        WEBHOOK_SECRET.as_bytes(),
        &call.timestamp.to_string(),
        &call.signature,
        "POST",
        "/hook",
        &call.body,
        desk.app.clock_now(),
    )
    .unwrap();
    let notice: Value = serde_json::from_slice(&call.body).unwrap();
    assert_eq!(notice["text"], words);
    assert_eq!(notice["payloadHash"], request.payload_hash.to_hex());
    assert_eq!(notice["files"][0]["name"], "cover.png");
    assert!(
        notice["link"]
            .as_str()
            .unwrap()
            .starts_with("https://emberhut.example/approve/"),
        "the link goes to the approver's own app"
    );

    // The same payload asked again is the same request, sent nowhere again.
    let again = ask(
        &desk,
        &post(words, vec![desk.file("cover.png", b"png bytes")]),
    )
    .await;
    assert_eq!(again.request.id, request.id);
    assert!(!again.made);
    assert_eq!(desk.slack.posts.lock().unwrap().len(), 1);
}

/// The owner decides from a signed-in device, once, on the hash they were shown, and only a
/// request that waits on the owner. An agent's room token is not a device.
#[tokio::test]
pub(crate) async fn the_owner_decides_once_on_the_hash_shown() {
    let desk = Desk::new().await;
    let app = &desk.app;
    let deploy = ask(
        &desk,
        &StepAsk {
            kind: ActionKind::Deploy,
            target: "example-app production".to_owned(),
            summary: "Deploy the release".to_owned(),
            text: None,
            files: Vec::new(),
            amount_cents: None,
            task: None,
        },
    )
    .await;
    assert_eq!(deploy.delivery, Delivery::Owner);
    assert!(desk.slack.posts.lock().unwrap().is_empty());
    let request = deploy.request;
    let path = format!("/api/approvals/{}/decision", request.id);
    let right = json!({ "decision": "approve", "payloadHash": request.payload_hash.to_hex() });

    let as_agent = app
        .call(
            Call::post(&path)
                .header("authorization", &format!("Bearer {}", desk.room_token()))
                .json(&right),
        )
        .await;
    assert_eq!(as_agent.status, 401, "an agent's token is not a device");
    let malformed = app
        .call(
            Call::post(&path)
                .token(&desk.session)
                .json(&json!({ "decision": "maybe" })),
        )
        .await;
    assert_eq!(malformed.status, 400);
    let swapped = app
        .call(Call::post(&path).token(&desk.session).json(&json!({
            "decision": "approve", "payloadHash": "00".repeat(32),
        })))
        .await;
    assert_eq!(
        swapped.status, 409,
        "a hash the owner wasn't shown decides nothing"
    );
    let unknown = app
        .call(
            Call::post("/api/approvals/999/decision")
                .token(&desk.session)
                .json(&right),
        )
        .await;
    assert_eq!(unknown.status, 404);

    let decided = app
        .call(Call::post(&path).token(&desk.session).json(&right))
        .await;
    assert_eq!(decided.status, 200, "{}", decided.text);
    assert_eq!(decided.json()["approval"]["state"], "approved");
    assert_eq!(decided.json()["approval"]["decidedVia"], "app");
    let replayed = app
        .call(Call::post(&path).token(&desk.session).json(&json!({
            "decision": "reject", "payloadHash": request.payload_hash.to_hex(),
        })))
        .await;
    assert_eq!(replayed.status, 409, "a decision is made once");

    let theirs = ask(&desk, &post("Hello", Vec::new())).await.request;
    let not_owners = app
        .call(
            Call::post(&format!("/api/approvals/{}/decision", theirs.id))
                .token(&desk.session)
                .json(
                    &json!({ "decision": "approve", "payloadHash": theirs.payload_hash.to_hex() }),
                ),
        )
        .await;
    assert_eq!(
        not_owners.status, 403,
        "a request waiting on the approver is theirs, not the owner's"
    );
}

/// A Slack button counts only under the workspace's signature, fresh, from the approver's own
/// team and user, on the request's hash, once; the DM then says what was decided.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "every way a click is refused, then the one that counts"
)]
pub(crate) async fn a_slack_click_counts_only_from_the_approver() {
    let desk = Desk::new().await;
    let app = &desk.app;
    let request = ask(&desk, &post("Hello world", Vec::new())).await.request;
    let value = format!("{}.{}", request.id, request.payload_hash);
    let now = app.clock_now().as_second();
    let send = |body: String, secret: &'static str, at: i64| {
        let signature = slack_signature(secret, at, &body);
        app.call(
            Call::post("/api/approvals/slack")
                .header("x-slack-request-timestamp", &at.to_string())
                .header("x-slack-signature", &signature)
                .raw(&body),
        )
    };

    let unsigned = app
        .call(Call::post("/api/approvals/slack").raw(&slack_click(TEAM, USER, "approve", &value)))
        .await;
    assert_eq!(unsigned.status, 401);
    let forged = send(
        slack_click(TEAM, USER, "approve", &value),
        "not-the-secret",
        now,
    )
    .await;
    assert_eq!(
        forged.status, 401,
        "a bridge's or anyone's word is not Slack's signature"
    );
    let stale = send(
        slack_click(TEAM, USER, "approve", &value),
        SIGNING,
        now - 600,
    )
    .await;
    assert_eq!(
        stale.status, 401,
        "a recorded click can't be replayed later"
    );
    let stranger = send(
        slack_click(TEAM, "U0STRANGER", "approve", &value),
        SIGNING,
        now,
    )
    .await;
    assert_eq!(
        stranger.status, 403,
        "another user of the team is not the approver"
    );
    let other_team = send(
        slack_click("T0OTHER", USER, "approve", &value),
        "other-signing-secret",
        now,
    )
    .await;
    assert_eq!(
        other_team.status, 403,
        "a genuine click from another workspace is not the approver's"
    );
    let unknown_team = send(
        slack_click("T0NOWHERE", USER, "approve", &value),
        "any-secret",
        now,
    )
    .await;
    assert_eq!(unknown_team.status, 401);
    assert_eq!(
        unknown_team.json()["error"],
        forged.json()["error"],
        "an outsider can't tell which workspaces are set up"
    );
    let other_message = send(
        slack_click_on("1800000000.000099", TEAM, USER, "approve", &value),
        SIGNING,
        now,
    )
    .await;
    assert_eq!(
        other_message.status, 403,
        "a button on any message but the request's own decides nothing"
    );
    let not_a_button = send(slack_click(TEAM, USER, "delete", &value), SIGNING, now).await;
    assert_eq!(not_a_button.status, 400);
    let unknown = send(
        slack_click(
            TEAM,
            USER,
            "approve",
            &format!("999.{}", request.payload_hash),
        ),
        SIGNING,
        now,
    )
    .await;
    assert_eq!(unknown.status, 404);
    let swapped = send(
        slack_click(
            TEAM,
            USER,
            "approve",
            &format!("{}.{}", request.id, "ab".repeat(32)),
        ),
        SIGNING,
        now,
    )
    .await;
    assert_eq!(
        swapped.status, 409,
        "a button for another payload decides nothing"
    );
    assert_eq!(
        app.state
            .approvals()
            .status(&desk.agent, request.id)
            .await
            .unwrap()
            .state,
        RequestState::Pending
    );

    let clicked = send(slack_click(TEAM, USER, "approve", &value), SIGNING, now).await;
    assert_eq!(clicked.status, 200, "{}", clicked.text);
    let decided = app
        .state
        .approvals()
        .status(&desk.agent, request.id)
        .await
        .unwrap();
    assert_eq!(decided.state, RequestState::Approved);
    let record = decided.decided.unwrap();
    assert_eq!(record.via, DecidedVia::Slack);
    assert!(record.by.contains(USER));
    app.state.approvals().settled().await;
    let updates = desk.slack.updates.lock().unwrap().clone();
    assert!(
        updates.last().unwrap().1.contains("Approved"),
        "{updates:?}"
    );

    let replayed = send(slack_click(TEAM, USER, "reject", &value), SIGNING, now).await;
    assert_eq!(replayed.status, 409, "a decision is made once");
}

/// The signed link's page shows exactly what is decided, escaped; its form decides only on the
/// hash it showed, and only once.
#[tokio::test]
pub(crate) async fn a_signed_link_decides_once_on_the_hash_it_showed() {
    let desk = Desk::new().await;
    let app = &desk.app;
    let words = "<script>alert(1)</script> & \"quotes\"";
    let request = ask(&desk, &post(words, vec![desk.file("terms.pdf", b"pdf")]))
        .await
        .request;
    let token = last_link_token(&desk);
    let path = format!("/approve/{token}");

    let page = app.call(Call::get(&path)).await;
    assert_eq!(page.status, 200);
    assert_eq!(page.header("content-type"), "text/html; charset=utf-8");
    assert!(
        page.text
            .contains("<pre>&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;quotes&quot;</pre>")
    );
    assert!(!page.text.contains("<script>"));
    assert!(page.text.contains("terms.pdf"));
    assert!(page.text.contains(&request.files[0].sha256.to_hex()));
    assert!(page.text.contains(&request.payload_hash.to_hex()));
    assert!(page.text.contains("prefers-color-scheme:dark"));

    let mut forged = token.clone();
    forged.replace_range(0..1, if forged.starts_with('A') { "B" } else { "A" });
    assert_eq!(
        app.call(Call::get(&format!("/approve/{forged}")))
            .await
            .status,
        410
    );
    assert_eq!(
        app.call(Call::get("/approve/not-a-token")).await.status,
        410
    );

    let form = |decision: &str, hash: &str| {
        form_urlencoded::Serializer::new(String::new())
            .append_pair("decision", decision)
            .append_pair("payload_hash", hash)
            .finish()
    };
    let no_decision = app
        .call(Call::post(&path).raw(&form("maybe", &request.payload_hash.to_hex())))
        .await;
    assert_eq!(no_decision.status, 400);
    let swapped = app
        .call(Call::post(&path).raw(&form("approve", &"cd".repeat(32))))
        .await;
    assert_eq!(
        swapped.status, 409,
        "a page shown for one payload decides no other"
    );
    let decided = app
        .call(Call::post(&path).raw(&form("approve", &request.payload_hash.to_hex())))
        .await;
    assert_eq!(decided.status, 200, "{}", decided.text);
    let stored = app
        .state
        .approvals()
        .status(&desk.agent, request.id)
        .await
        .unwrap();
    assert_eq!(stored.state, RequestState::Approved);
    assert_eq!(stored.decided.unwrap().via, DecidedVia::Link);

    let again = app
        .call(Call::post(&path).raw(&form("reject", &request.payload_hash.to_hex())))
        .await;
    assert_eq!(again.status, 410, "a link works once");
    assert_eq!(app.call(Call::get(&path)).await.status, 410);
}

/// The approver's app's answer counts only under the approver's secret, fresh, on the
/// request's hash, once; `by` is kept as the app said.
#[tokio::test]
pub(crate) async fn a_webhook_answer_counts_only_under_the_approvers_secret() {
    let desk = Desk::new().await;
    let app = &desk.app;
    let request = ask(&desk, &post("Hello", Vec::new())).await.request;
    let id = request.id.get();
    let now = app.clock_now().as_second();
    let good = json!({ "decision": "approve", "payloadHash": request.payload_hash.to_hex(), "by": "lead@example" });

    let unsigned = app
        .call(Call::post(&format!("/api/approvals/{id}/webhook")).json(&good))
        .await;
    assert_eq!(unsigned.status, 401);
    assert_eq!(
        webhook_answer(&desk, id, &good, "wrong-secret", now)
            .await
            .status,
        401
    );
    assert_eq!(
        webhook_answer(&desk, id, &good, WEBHOOK_SECRET, now - 600)
            .await
            .status,
        401,
        "a recorded answer can't be replayed later"
    );
    assert_eq!(
        webhook_answer(&desk, 999, &good, WEBHOOK_SECRET, now)
            .await
            .json(),
        webhook_answer(&desk, id, &good, "wrong-secret", now)
            .await
            .json(),
        "an unsigned caller can't tell a request that isn't there"
    );
    assert_eq!(
        webhook_answer(
            &desk,
            id,
            &json!({ "decision": "approve" }),
            WEBHOOK_SECRET,
            now
        )
        .await
        .status,
        400
    );
    let swapped =
        json!({ "decision": "approve", "payloadHash": "ef".repeat(32), "by": "lead@example" });
    assert_eq!(
        webhook_answer(&desk, id, &swapped, WEBHOOK_SECRET, now)
            .await
            .status,
        409
    );

    let decided = webhook_answer(&desk, id, &good, WEBHOOK_SECRET, now).await;
    assert_eq!(decided.status, 200, "{}", decided.text);
    assert_eq!(decided.json()["approval"]["decidedBy"], "lead@example");
    assert_eq!(decided.json()["approval"]["decidedVia"], "webhook");
    assert_eq!(
        webhook_answer(&desk, id, &good, WEBHOOK_SECRET, now)
            .await
            .status,
        409,
        "a decision is made once"
    );
}

/// The payload is the server's: a changed word is a new request, and a step whose file was
/// edited after the approval matches nothing. An approval is used once.
#[tokio::test]
pub(crate) async fn a_changed_word_or_file_matches_nothing_and_an_approval_is_used_once() {
    let desk = Desk::new().await;
    let cover = desk.file("cover.png", b"png bytes");
    let first = ask(&desk, &post("Hello", vec![cover.clone()]))
        .await
        .request;
    let changed = ask(&desk, &post("Hello!", vec![cover.clone()]))
        .await
        .request;
    assert_ne!(first.id, changed.id, "a changed word is a new request");
    approve_by_webhook(&desk, &first).await;

    match use_step(&desk, first.id, "Hello!", vec![cover.clone()]).await {
        UseOutcome::Refused(why) => assert!(why.contains("not what was approved"), "{why}"),
        UseOutcome::Used(_) => panic!("another word used the approval"),
    }
    desk.file("cover.png", b"png bytes, edited");
    assert!(
        matches!(
            use_step(&desk, first.id, "Hello", vec![cover.clone()]).await,
            UseOutcome::Refused(_)
        ),
        "a file edited after the approval matches nothing"
    );
    desk.file("cover.png", b"png bytes");
    match use_step(&desk, first.id, "Hello", vec![cover.clone()]).await {
        UseOutcome::Used(used) => {
            assert_eq!(used.state, RequestState::Used);
            assert_eq!(used.used.unwrap().by, "task 12");
        }
        UseOutcome::Refused(why) => panic!("refused: {why}"),
    }
    assert!(
        matches!(
            use_step(&desk, first.id, "Hello", vec![cover]).await,
            UseOutcome::Refused(_)
        ),
        "an approval is used once"
    );
}

/// An approval stays its approver's: moving the kind to the owner voids it.
#[tokio::test]
pub(crate) async fn moving_a_kind_to_the_owner_voids_the_approvers_approval() {
    let desk = Desk::new().await;
    let request = ask(&desk, &post("Hello", Vec::new())).await.request;
    approve_by_webhook(&desk, &request).await;
    desk.set_policies(&json!([])).await;
    match use_step(&desk, request.id, "Hello", Vec::new()).await {
        UseOutcome::Refused(why) => assert!(why.contains("another approver"), "{why}"),
        UseOutcome::Used(_) => panic!("an approval outlived its approver's policy"),
    }
}

/// The fixed core is the owner's: a policy giving it to anyone else is refused, and a post
/// about a password is raised to a credential and waits on the owner, never the approver.
#[tokio::test]
pub(crate) async fn the_fixed_core_is_the_owners_whatever_a_policy_says() {
    let desk = Desk::new().await;
    let app = &desk.app;
    for kind in [
        "payment",
        "credential",
        "secret",
        "account_security",
        "as_owner",
    ] {
        let refused = app
            .call(
                Call::put("/api/agents/client-a/policies")
                    .token(&desk.session)
                    .json(&json!({ "policies": [{ "kind": kind, "approver": "example-lead" }] })),
            )
            .await;
        assert_eq!(refused.status, 400, "{kind}: {}", refused.text);
        let nobody = app
            .call(
                Call::put("/api/agents/client-a/policies")
                    .token(&desk.session)
                    .json(&json!({ "policies": [{ "kind": kind, "approver": "none" }] })),
            )
            .await;
        assert_eq!(nobody.status, 400, "{kind}: nobody");
    }
    let raised = ask(
        &desk,
        &post("Reset your password here: example.test/reset", Vec::new()),
    )
    .await;
    assert_eq!(raised.request.kind, ActionKind::Credential);
    assert_eq!(raised.request.approver, Approver::Owner);
    assert_eq!(raised.delivery, Delivery::Owner);
    assert!(
        desk.slack.posts.lock().unwrap().is_empty(),
        "the approver never sees a fixed-core request"
    );
    let decision = app
        .call(
            Call::post(&format!("/api/approvals/{}/decision", raised.request.id))
                .token(&desk.session)
                .json(&json!({ "decision": "approve", "payloadHash": raised.request.payload_hash.to_hex() })),
        )
        .await;
    assert_eq!(decision.status, 200, "the owner decides it in the apps");
}

/// A payment needs its amount within the month's cap, which only ever refuses.
#[tokio::test]
pub(crate) async fn a_payment_over_the_cap_is_refused() {
    let desk = Desk::new().await;
    let pay = |cents| StepAsk {
        kind: ActionKind::Payment,
        target: "example hosting".to_owned(),
        summary: "Top up the hosting".to_owned(),
        text: None,
        files: Vec::new(),
        amount_cents: Some(cents),
        task: None,
    };
    let approvals = desk.app.state.approvals();
    assert!(matches!(
        approvals.request(&desk.agent, &pay(10_001)).await,
        Err(ApprovalError::OverCap(_))
    ));
    let mut requests = Vec::new();
    for cents in [6_000, 5_000] {
        let request = ask(&desk, &pay(cents)).await.request;
        assert_eq!(
            request.approver,
            Approver::Owner,
            "a payment is the owner's"
        );
        let decided = desk
            .app
            .call(
                Call::post(&format!("/api/approvals/{}/decision", request.id))
                    .token(&desk.session)
                    .json(&json!({ "decision": "approve", "payloadHash": request.payload_hash.to_hex() })),
            )
            .await;
        assert_eq!(decided.status, 200);
        assert_eq!(decided.json()["approval"]["amountCents"], cents);
        assert_eq!(decided.json()["approval"]["amountCurrency"], "EUR");
        requests.push(request);
    }
    let step = StepUse {
        text: None,
        files: Vec::new(),
        by: "task 3".to_owned(),
    };
    let pay_now = |id| approvals.use_for_step(&desk.agent, id, &step);
    assert!(matches!(
        pay_now(requests[0].id).await.unwrap(),
        UseOutcome::Used(_)
    ));
    match pay_now(requests[1].id).await.unwrap() {
        UseOutcome::Refused(why) => assert!(why.contains("over the monthly cap"), "{why}"),
        UseOutcome::Used(_) => panic!("a payment went over the cap"),
    }
    assert_eq!(
        approvals
            .status(&desk.agent, requests[1].id)
            .await
            .unwrap()
            .state,
        RequestState::Approved,
        "the cap refuses; the approval stays unused"
    );
}

/// Without the server's public address nothing is routed outside: the request waits in the
/// apps, and asking says so.
#[tokio::test]
pub(crate) async fn without_a_public_address_a_request_waits_in_the_apps() {
    let desk = Desk::without_public_address().await;
    let requested = ask(&desk, &post("Hello", Vec::new())).await;
    assert!(matches!(requested.delivery, Delivery::AppsOnly(_)));
    assert!(requested.delivery.describe().contains("public address"));
    assert!(desk.slack.posts.lock().unwrap().is_empty());
    assert!(desk.webhook.calls.lock().unwrap().is_empty());
}

/// A kind nobody approves needs no request; a merge's words are not raised.
#[tokio::test]
pub(crate) async fn a_kind_nobody_approves_needs_no_request() {
    let desk = Desk::new().await;
    let merge = StepAsk {
        kind: ActionKind::Merge,
        target: "example/app main".to_owned(),
        summary: "Fix the password reset form".to_owned(),
        text: None,
        files: Vec::new(),
        amount_cents: None,
        task: None,
    };
    assert_eq!(
        desk.app
            .state
            .approvals()
            .request(&desk.agent, &merge)
            .await
            .unwrap(),
        Asked::NotNeeded(ActionKind::Merge)
    );
}

/// The use route takes only an agent's room token, and an agent uses only its own requests.
#[tokio::test]
pub(crate) async fn the_use_route_takes_an_agents_own_requests_only() {
    let desk = Desk::new().await;
    let app = &desk.app;
    let request = ask(&desk, &post("Hello", Vec::new())).await.request;
    approve_by_webhook(&desk, &request).await;
    let path = format!("/api/approvals/{}/use", request.id);
    let bearer = |token: &str| format!("Bearer {token}");

    let device = app
        .call(
            Call::post(&path)
                .token(&desk.session)
                .json(&json!({ "text": "Hello" })),
        )
        .await;
    assert_eq!(device.status, 401, "a device is not an agent");
    let main_token = app
        .state
        .agent_tokens()
        .for_room(emberhut_core::RoomId::MAIN)
        .unwrap();
    let other_agent = app
        .call(
            Call::post(&path)
                .header("authorization", &bearer(main_token.as_str()))
                .json(&json!({ "text": "Hello" })),
        )
        .await;
    assert_eq!(
        other_agent.status, 404,
        "another agent's request is not there for it"
    );
    let own = desk.room_token();
    let bad = app
        .call(
            Call::post(&path)
                .header("authorization", &bearer(&own))
                .json(&json!({ "files": "not a list" })),
        )
        .await;
    assert_eq!(bad.status, 400);
    let used = app
        .call(
            Call::post(&path)
                .header("authorization", &bearer(&own))
                .json(&json!({ "text": "Hello" })),
        )
        .await;
    assert_eq!(used.status, 200);
    assert_eq!(used.json()["used"], true, "{}", used.text);
    let receipt = used.json()["message"].as_str().unwrap().to_owned();
    assert!(receipt.contains("the approver"), "{receipt}");
    for secret in [USER, TEAM, "lead@example", "Example lead", "example-lead"] {
        assert!(
            !receipt.contains(secret),
            "an agent never learns who approved: {receipt}"
        );
    }
    let again = app
        .call(
            Call::post(&path)
                .header("authorization", &bearer(&own))
                .json(&json!({ "text": "Hello" })),
        )
        .await;
    assert_eq!(again.json()["used"], false, "an approval is used once");
}

/// No MCP tool decides an approval: deciding is the owner's device's, or the approver's own
/// channel's, never an agent's.
#[tokio::test]
pub(crate) async fn no_tool_decides_an_approval() {
    let app = TestApp::new().await;
    let listed = app.rpc("tools/list", json!({})).await;
    let names: Vec<String> = listed.json()["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    for name in &names {
        assert!(
            !name.contains("decide")
                && !name.contains("decision")
                && !name.starts_with("approve")
                && !name.contains("polic"),
            "{name} could decide or configure approvals"
        );
    }
}

/// Requests expire unanswered, say so in their DM and on the live stream; the owner may
/// withdraw one, once; the list and one request are the owner's to read.
#[tokio::test]
pub(crate) async fn requests_expire_and_are_withdrawn_and_listed() {
    let desk = Desk::new().await;
    let app = &desk.app;
    let mut events = app.state.approvals().events().subscribe();
    let stale = ask(&desk, &post("Old news", Vec::new())).await.request;
    assert!(events.has_changed().unwrap(), "a new request is an event");
    events.mark_unchanged();

    let fresh_later = {
        app.clock.advance(SignedDuration::from_hours(71));
        ask(&desk, &post("New news", Vec::new())).await.request
    };
    app.clock.advance(SignedDuration::from_hours(2));
    let expired = app.state.approvals().expire_due().await.unwrap();
    assert_eq!(expired, [stale.id]);
    let event = events.borrow_and_update().clone().unwrap();
    assert_eq!(event.state.as_str(), "expired");
    assert_eq!(event.pending, 1);
    assert!(
        desk.slack
            .updates
            .lock()
            .unwrap()
            .iter()
            .any(|(_, outcome)| outcome.contains("Expired")),
    );

    // The device's session has long expired: it signs in again.
    let session = app.sign_in().await;
    let listed = app
        .call(
            Call::get("/api/approvals")
                .query("state=pending")
                .token(&session),
        )
        .await;
    assert_eq!(listed.status, 200);
    let pending = listed.json()["approvals"].as_array().unwrap().clone();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["id"], fresh_later.id.get());
    assert_eq!(pending[0]["approverName"], "Example lead");
    assert_eq!(pending[0]["waitsOnOwner"], false);
    let none = app
        .call(
            Call::get("/api/approvals")
                .query("state=sleeping")
                .token(&session),
        )
        .await;
    assert_eq!(none.json()["approvals"], json!([]));
    let one = app
        .call(Call::get(&format!("/api/approvals/{}", stale.id)).token(&session))
        .await;
    assert_eq!(one.status, 200);
    assert_eq!(one.json()["approval"]["state"], "expired");
    assert_eq!(
        app.call(Call::get("/api/approvals/999").token(&session))
            .await
            .status,
        404
    );

    let withdraw = |id: i64| {
        app.call(
            Call::post(&format!("/api/approvals/{id}/withdraw"))
                .token(&session)
                .json(&json!({ "reason": "plans changed" })),
        )
    };
    let withdrawn = withdraw(fresh_later.id.get()).await;
    assert_eq!(withdrawn.status, 200);
    assert_eq!(withdrawn.json()["approval"]["state"], "withdrawn");
    assert_eq!(withdraw(fresh_later.id.get()).await.status, 409);
    assert_eq!(withdraw(stale.id.get()).await.status, 409);
    assert_eq!(withdraw(999).await.status, 404);
}

/// The live stream carries an `approvals` event when a request changes.
#[tokio::test]
pub(crate) async fn the_live_stream_says_when_an_approval_changes() {
    let desk = Desk::new().await;
    let response = desk
        .app
        .send(Call::get("/api/live").token(&desk.session).request())
        .await;
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    // The room's own first event.
    let first = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(first.data_ref().unwrap()).starts_with("data:"));
    let request = ask(&desk, &post("Hello", Vec::new())).await.request;
    let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(frame.data_ref().unwrap()).into_owned();
    assert!(text.starts_with("event: approvals\n"), "{text}");
    assert!(
        text.contains(&format!("\"id\":{}", request.id)) && text.contains("\"state\":\"pending\""),
        "{text}"
    );
}

/// The state lists the agents (with their rooms among the rooms) and the pending requests;
/// without agents it shows neither.
#[tokio::test]
pub(crate) async fn the_state_lists_agents_and_pending_approvals() {
    let plain = TestApp::new().await;
    let session = plain.sign_in().await;
    let state = plain
        .call(Call::get("/api/state").token(&session))
        .await
        .json();
    assert!(state.get("agents").is_none() && state.get("approvals").is_none());

    let desk = Desk::new().await;
    let request = ask(&desk, &post("Hello", Vec::new())).await.request;
    let state = desk
        .app
        .call(Call::get("/api/state").token(&desk.session))
        .await
        .json();
    let agents = state["agents"].as_array().unwrap();
    assert_eq!(agents[0]["id"], "main");
    assert_eq!(agents[1]["id"], "client-a");
    assert_eq!(agents[1]["pendingApprovals"], 1);
    assert_eq!(agents[1]["room"], desk.room.get());
    assert!(
        state["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|room| room["id"] == desk.room.get()),
        "the agent's room is among the rooms: {}",
        state["rooms"]
    );
    assert_eq!(state["approvals"][0]["id"], request.id.get());
}

/// A disabled agent asks for and uses no approval: disabling is the owner's off switch.
#[tokio::test]
pub(crate) async fn a_disabled_agent_asks_for_and_uses_nothing() {
    let desk = Desk::new().await;
    let app = &desk.app;
    let request = ask(&desk, &post("Hello", Vec::new())).await.request;
    approve_by_webhook(&desk, &request).await;
    let off = app
        .call(
            Call::patch("/api/agents/client-a")
                .token(&desk.session)
                .json(&json!({ "enabled": false })),
        )
        .await;
    assert_eq!(off.status, 200);
    assert!(matches!(
        app.state
            .approvals()
            .request(&desk.agent, &post("Another", Vec::new()))
            .await,
        Err(ApprovalError::Disabled(_))
    ));
    let used = app
        .call(
            Call::post(&format!("/api/approvals/{}/use", request.id))
                .header("authorization", &format!("Bearer {}", desk.room_token()))
                .json(&json!({ "text": "Hello" })),
        )
        .await;
    assert_eq!(used.status, 200);
    assert_eq!(used.json()["used"], false, "{}", used.text);
    assert!(used.text.contains("disabled"), "{}", used.text);
}

/// Report mode's pass without a session reads the app, never the approvals: not in the state,
/// not on the live stream.
#[tokio::test]
pub(crate) async fn report_mode_s_pass_reads_no_approvals() {
    let desk = Desk::in_report_mode().await;
    let app = &desk.app;
    let response = app.send(Call::get("/api/live").request()).await;
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let first = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(first.data_ref().unwrap()).starts_with("data:"));
    ask(&desk, &post("A client's post, word for word", Vec::new())).await;

    let anonymous = app.call(Call::get("/api/state")).await;
    assert_eq!(anonymous.status, 200);
    assert!(
        anonymous.json().get("approvals").is_none(),
        "{}",
        anonymous.text
    );
    assert!(anonymous.json().get("agents").is_none());
    assert!(!anonymous.text.contains("word for word"));
    let signed_in = app
        .call(Call::get("/api/state").token(&desk.session))
        .await
        .json();
    assert_eq!(signed_in["approvals"].as_array().unwrap().len(), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(500), body.frame())
            .await
            .is_err(),
        "no approvals event reaches a stream without a session"
    );
}

/// Characters that change how a target reads without showing themselves are written out on
/// the link's page and in the DM.
#[tokio::test]
pub(crate) async fn invisible_characters_are_shown_to_the_approver() {
    let desk = Desk::new().await;
    let request = ask(
        &desk,
        &StepAsk {
            target: "example.com/blog\u{202E}gol/".to_owned(),
            ..post("Hello\u{200B} there", Vec::new())
        },
    )
    .await
    .request;
    let page = desk
        .app
        .call(Call::get(&format!("/approve/{}", last_link_token(&desk))))
        .await;
    assert!(
        page.text.contains("example.com/blog[U+202E]gol/"),
        "{}",
        page.text
    );
    assert!(page.text.contains("Hello[U+200B] there"));
    assert!(!page.text.contains('\u{202E}'));
    let posts = desk.slack.posts.lock().unwrap().clone();
    let (_, message) = &posts[0];
    assert!(message.heading.contains("[U+202E]"), "{}", message.heading);
    assert_eq!(message.text.as_deref(), Some("Hello[U+200B] there"));
    assert_eq!(
        request.target, "example.com/blog\u{202E}gol/",
        "what is approved is the target as given"
    );
}
