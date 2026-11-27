//! The approval store: a request made once per pending payload, decided once, used once, and
//! expired, withdrawn or imported as its life says.
#![expect(clippy::unwrap_used, reason = "tests")]

use emberhut_core::OsRandom;
use emberhut_core::agents::Agent;
use emberhut_core::approvals::link::LinkNonce;
use emberhut_core::approvals::{
    ActionKind, ApprovalFile, ApprovalRequest, Approver, DecidedVia, Decision, DecisionRecord,
    FileDigest, ImportedApproval, MonthKey, NewApproval, PayloadHash, Policy, RequestError,
    RequestState, Sha256Digest, SlackPost,
};
use emberhut_db::{
    ApprovalFilter, ApprovalStoreError, Database, DecisionMade, StepToUse, UNANSWERED, UNUSED,
};
use jiff::tz::TimeZone;

use crate::agents::{agent_in_own_room, approver};
use crate::common::{at, database};

/// An agent whose posts an external approver approves, and that approver.
async fn set_up(db: &Database) -> (Agent, Approver) {
    let agent = agent_in_own_room(db, "client-a").await;
    let lead = approver("example-lead");
    db.agents().create_approver(&lead, at(0)).await.unwrap();
    let external = Approver::External(lead.id);
    db.agents()
        .replace_policies(
            &agent.id,
            &[Policy {
                kind: ActionKind::Post,
                approver: external.clone(),
            }],
        )
        .await
        .unwrap();
    (agent, external)
}

fn post(agent: &Agent, approver: &Approver, text: &str) -> NewApproval {
    NewApproval {
        agent: agent.id.clone(),
        kind: ActionKind::Post,
        target: "example blog".to_owned(),
        summary: "The launch post".to_owned(),
        text: Some(text.to_owned()),
        files: vec![FileDigest::of("cover.png", b"png bytes")],
        approver: approver.clone(),
        amount_cents: None,
        task: None,
    }
}

fn decision<'a>(
    request: &'a ApprovalRequest,
    approver: &'a Approver,
    decision: Decision,
) -> DecisionMade<'a> {
    DecisionMade {
        id: request.id,
        payload_hash: &request.payload_hash,
        approver,
        decision,
        by: "U0APPROVER",
        via: DecidedVia::Slack,
        reason: None,
    }
}

fn step<'a>(
    request: &'a ApprovalRequest,
    hash: &'a PayloadHash,
    month: &'a MonthKey,
) -> StepToUse<'a> {
    StepToUse {
        agent: &request.agent,
        request: None,
        kind: request.kind,
        payload_hash: hash,
        by: "task 12",
        month,
    }
}

fn month() -> MonthKey {
    MonthKey::of(at(0), &TimeZone::UTC)
}

#[tokio::test]
async fn the_same_pending_payload_is_one_request() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let (first, made) = approvals
        .create_or_find_pending(&post(&agent, &lead, "We shipped it."), at(0))
        .await
        .unwrap();
    assert!(made);
    assert_eq!(first.state, RequestState::Pending);
    assert_eq!(first.files[0].name, "cover.png");
    assert_eq!(first.files[0].size, Some(9));
    assert_eq!(
        first.payload_hash,
        post(&agent, &lead, "We shipped it.").payload_hash()
    );
    let (again, made) = approvals
        .create_or_find_pending(&post(&agent, &lead, "We shipped it."), at(5))
        .await
        .unwrap();
    assert!(!made);
    assert_eq!(again.id, first.id);
    let (changed, made) = approvals
        .create_or_find_pending(&post(&agent, &lead, "We shipped it!"), at(5))
        .await
        .unwrap();
    assert!(made, "a changed word is a new request");
    assert_ne!(changed.id, first.id);
    assert_eq!(approvals.get(first.id).await.unwrap(), Some(first));
}

#[tokio::test]
async fn a_bad_request_is_refused() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let mut payment = post(&agent, &lead, "Buy the ad credits");
    payment.kind = ActionKind::Payment;
    payment.amount_cents = Some(1_000);
    assert!(matches!(
        db.approvals().create_or_find_pending(&payment, at(0)).await,
        Err(ApprovalStoreError::Invalid(RequestError::FixedCore(
            ActionKind::Payment
        )))
    ));
}

#[tokio::test]
async fn a_decision_is_made_once_on_the_payload_shown_by_its_approver() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let (request, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "We shipped it."), at(0))
        .await
        .unwrap();

    let other_hash = Sha256Digest::of(b"another payload");
    let wrong_hash = DecisionMade {
        payload_hash: &other_hash,
        ..decision(&request, &lead, Decision::Approve)
    };
    assert!(!approvals.decide(&wrong_hash, at(1)).await.unwrap());
    let not_theirs = decision(&request, &Approver::Owner, Decision::Approve);
    assert!(
        !approvals.decide(&not_theirs, at(1)).await.unwrap(),
        "the owner does not decide what waits on another approver"
    );
    assert_eq!(
        approvals.get(request.id).await.unwrap().unwrap().state,
        RequestState::Pending
    );

    assert!(
        approvals
            .decide(&decision(&request, &lead, Decision::Approve), at(1))
            .await
            .unwrap()
    );
    for again in [Decision::Approve, Decision::Reject] {
        assert!(
            !approvals
                .decide(&decision(&request, &lead, again), at(2))
                .await
                .unwrap(),
            "a decided request is not decided again"
        );
    }
    let decided = approvals.get(request.id).await.unwrap().unwrap();
    assert_eq!(decided.state, RequestState::Approved);
    assert_eq!(
        decided.decided,
        Some(DecisionRecord {
            decision: Decision::Approve,
            at: at(1),
            by: "U0APPROVER".to_owned(),
            via: DecidedVia::Slack,
        })
    );
}

#[tokio::test]
async fn an_expired_request_is_not_decided() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let (request, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "We shipped it."), at(0))
        .await
        .unwrap();
    let three_days = 3 * 24 * 60;
    assert!(
        !approvals
            .decide(
                &decision(&request, &lead, Decision::Approve),
                at(three_days)
            )
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn an_approval_is_used_once() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let (request, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "We shipped it."), at(0))
        .await
        .unwrap();
    let month = month();
    let hash = request.payload_hash;
    assert_eq!(
        approvals
            .use_approval(&step(&request, &hash, &month), at(1))
            .await
            .unwrap(),
        None,
        "a pending request is not an approval"
    );
    approvals
        .decide(&decision(&request, &lead, Decision::Approve), at(1))
        .await
        .unwrap();

    let changed = post(&agent, &lead, "We shipped it!").payload_hash();
    assert_eq!(
        approvals
            .use_approval(&step(&request, &changed, &month), at(2))
            .await
            .unwrap(),
        None,
        "a changed payload matches nothing"
    );
    let used = approvals
        .use_approval(&step(&request, &hash, &month), at(2))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(used.id, request.id);
    assert_eq!(used.state, RequestState::Used);
    assert_eq!(used.used.as_ref().unwrap().by, "task 12");

    assert_eq!(
        approvals
            .use_approval(&step(&request, &hash, &month), at(3))
            .await
            .unwrap(),
        None,
        "a replay changes nothing"
    );
    assert_eq!(approvals.get(request.id).await.unwrap(), Some(used));
}

#[tokio::test]
async fn moving_the_policy_voids_an_approval() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let (request, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "We shipped it."), at(0))
        .await
        .unwrap();
    approvals
        .decide(&decision(&request, &lead, Decision::Approve), at(1))
        .await
        .unwrap();
    db.agents().replace_policies(&agent.id, &[]).await.unwrap();
    let month = month();
    assert_eq!(
        approvals
            .use_approval(&step(&request, &request.payload_hash, &month), at(2))
            .await
            .unwrap(),
        None,
        "posts are the owner's now: the approver's yes no longer counts"
    );
    assert_eq!(
        approvals.get(request.id).await.unwrap().unwrap().state,
        RequestState::Approved
    );
}

#[tokio::test]
async fn due_requests_expire() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let (unanswered, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "one"), at(0))
        .await
        .unwrap();
    let (approved, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "two"), at(0))
        .await
        .unwrap();
    let (fresh, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "three"), at(60 * 60))
        .await
        .unwrap();
    approvals
        .decide(&decision(&approved, &lead, Decision::Approve), at(10))
        .await
        .unwrap();

    let day = 24 * 60;
    assert_eq!(approvals.expire_due(at(day)).await.unwrap(), []);
    assert_eq!(
        approvals.expire_due(at(day + 10)).await.unwrap(),
        [approved.id],
        "approved and unused for 24 hours"
    );
    assert_eq!(
        approvals.expire_due(at(3 * day)).await.unwrap(),
        [unanswered.id],
        "unanswered for 3 days"
    );
    let expired = approvals.get(approved.id).await.unwrap().unwrap();
    assert_eq!(expired.state, RequestState::Expired);
    assert_eq!(expired.state_reason.as_deref(), Some(UNUSED));
    assert_eq!(expired.closed_at, Some(at(day + 10)));
    assert_eq!(
        approvals
            .get(unanswered.id)
            .await
            .unwrap()
            .unwrap()
            .state_reason
            .as_deref(),
        Some(UNANSWERED)
    );
    assert_eq!(
        approvals.get(fresh.id).await.unwrap().unwrap().state,
        RequestState::Pending
    );
    let pending = approvals
        .list(&ApprovalFilter {
            state: Some(RequestState::Pending),
            ..ApprovalFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(pending.iter().map(|r| r.id).collect::<Vec<_>>(), [fresh.id]);
    assert_eq!(
        approvals.pending_counts().await.unwrap(),
        [(agent.id.clone(), 1)]
    );
}

#[tokio::test]
async fn withdrawing_ends_a_request_once() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let (request, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "We shipped it."), at(0))
        .await
        .unwrap();
    let other = emberhut_core::agents::AgentId::parse("client-b").unwrap();
    assert!(
        !approvals
            .withdraw(request.id, Some(&other), None, at(1))
            .await
            .unwrap(),
        "an agent withdraws only its own"
    );
    assert!(
        approvals
            .withdraw(request.id, Some(&agent.id), Some("plans changed"), at(1))
            .await
            .unwrap()
    );
    assert!(
        !approvals
            .withdraw(request.id, None, None, at(2))
            .await
            .unwrap()
    );
    let withdrawn = approvals.get(request.id).await.unwrap().unwrap();
    assert_eq!(withdrawn.state, RequestState::Withdrawn);
    assert_eq!(withdrawn.state_reason.as_deref(), Some("plans changed"));
    assert!(
        !approvals
            .decide(&decision(&request, &lead, Decision::Approve), at(2))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_link_works_until_the_first_decision() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let (request, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "We shipped it."), at(0))
        .await
        .unwrap();
    let nonce = LinkNonce::generate(&OsRandom).unwrap();
    assert!(
        approvals
            .set_link_nonce(request.id, &nonce.hash())
            .await
            .unwrap()
    );
    assert_eq!(
        approvals
            .by_link_nonce(&nonce.hash())
            .await
            .unwrap()
            .map(|r| r.id),
        Some(request.id)
    );
    approvals
        .mark_shown(
            request.id,
            &SlackPost {
                channel: "D0EXAMPLE".to_owned(),
                ts: "1800000000.000100".to_owned(),
            },
        )
        .await
        .unwrap();
    let link_decision = DecisionMade {
        via: DecidedVia::Link,
        by: "signed link",
        ..decision(&request, &lead, Decision::Reject)
    };
    assert!(approvals.decide(&link_decision, at(1)).await.unwrap());
    assert_eq!(approvals.by_link_nonce(&nonce.hash()).await.unwrap(), None);
    let rejected = approvals.get(request.id).await.unwrap().unwrap();
    assert_eq!(rejected.state, RequestState::Rejected);
    assert_eq!(rejected.shown.unwrap().channel, "D0EXAMPLE");
    assert!(
        !approvals
            .set_link_nonce(request.id, &nonce.hash())
            .await
            .unwrap(),
        "no new link for a decided request"
    );
}

#[tokio::test]
async fn a_payment_counts_against_the_monthly_cap() {
    let (_dir, db) = database().await;
    let (agent, _) = set_up(&db).await;
    let approvals = db.approvals();
    let pay = |text: &str, cents| NewApproval {
        kind: ActionKind::Payment,
        approver: Approver::Owner,
        amount_cents: Some(cents),
        ..post(&agent, &Approver::Owner, text)
    };
    let month = month();
    let mut used = Vec::new();
    for (text, cents) in [("credits", 6_000), ("more credits", 5_000)] {
        let (request, _) = approvals
            .create_or_find_pending(&pay(text, cents), at(0))
            .await
            .unwrap();
        approvals
            .decide(
                &DecisionMade {
                    via: DecidedVia::App,
                    by: "device 1",
                    ..decision(&request, &Approver::Owner, Decision::Approve)
                },
                at(1),
            )
            .await
            .unwrap();
        used.push(
            approvals
                .use_approval(&step(&request, &request.payload_hash, &month), at(2))
                .await,
        );
    }
    assert!(used[0].as_ref().unwrap().is_some());
    assert!(matches!(
        &used[1],
        Err(ApprovalStoreError::OverCap(over)) if over.spent_cents == 6_000 && over.cap_cents == 10_000
    ));
    assert_eq!(
        approvals.month_spend(&agent.id, &month).await.unwrap(),
        6_000
    );
    let unused = approvals
        .list(&ApprovalFilter {
            state: Some(RequestState::Approved),
            agent: Some(agent.id.clone()),
            limit: None,
        })
        .await
        .unwrap();
    assert_eq!(
        unused.len(),
        1,
        "the refused payment stays approved and unused"
    );
}

#[tokio::test]
async fn imported_history_is_never_usable() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let new = post(&agent, &lead, "We shipped it.");
    let imported = ImportedApproval {
        agent: agent.id.clone(),
        kind: ActionKind::Post,
        target: new.target.clone(),
        summary: new.summary.clone(),
        text: new.text.clone(),
        files: vec![ApprovalFile {
            name: "cover.png".to_owned(),
            size: None,
            sha256: Sha256Digest::of(b"png bytes"),
        }],
        approver: lead.clone(),
        amount_cents: None,
        task: None,
        state: RequestState::Used,
        state_reason: None,
        source_ref: "import:approvals#4".to_owned(),
        created_at: at(0),
        decided: Some(DecisionRecord {
            decision: Decision::Approve,
            at: at(1),
            by: "U0APPROVER".to_owned(),
            via: DecidedVia::Slack,
        }),
        used: Some(emberhut_core::approvals::UseRecord {
            at: at(2),
            by: "a publish".to_owned(),
        }),
        closed_at: None,
    };
    let id = approvals.import(&imported).await.unwrap();
    let stored = approvals.get(id).await.unwrap().unwrap();
    assert_eq!(stored.state, RequestState::Used);
    assert_eq!(stored.source_ref.as_deref(), Some("import:approvals#4"));
    assert_eq!(stored.files[0].size, None);
    assert_ne!(stored.payload_hash, new.payload_hash());

    let mut expired = imported.clone();
    expired.state = RequestState::Expired;
    expired.used = None;
    expired.state_reason = Some("imported: never answered".to_owned());
    let id = approvals.import(&expired).await.unwrap();
    let stored = approvals.get(id).await.unwrap().unwrap();
    assert!(
        !approvals
            .decide(&decision(&stored, &lead, Decision::Approve), at(3))
            .await
            .unwrap()
    );
    let mut open = imported;
    open.state = RequestState::Pending;
    assert!(matches!(
        approvals.import(&open).await,
        Err(ApprovalStoreError::Invalid(RequestError::ImportedState(_)))
    ));
}

#[tokio::test]
async fn an_agent_with_approval_history_is_kept() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let (request, _) = db
        .approvals()
        .create_or_find_pending(&post(&agent, &lead, "We shipped it."), at(0))
        .await
        .unwrap();
    assert!(matches!(
        db.agents().delete(&agent.id).await,
        Err(emberhut_db::AgentStoreError::HasHistory(id)) if id == agent.id
    ));
    assert!(db.agents().get(&agent.id).await.unwrap().is_some());
    assert_eq!(db.approvals().get(request.id).await.unwrap(), Some(request));
}

#[tokio::test]
async fn a_request_pending_on_another_approver_is_not_found_again() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let (first, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "Hello"), at(0))
        .await
        .unwrap();
    let (moved, made) = approvals
        .create_or_find_pending(&post(&agent, &Approver::Owner, "Hello"), at(1))
        .await
        .unwrap();
    assert!(made, "the approver named now is asked");
    assert_ne!(moved.id, first.id);
    assert_eq!(moved.approver, Approver::Owner);
}

#[tokio::test]
async fn a_disabled_agent_uses_no_approval() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let (request, _) = approvals
        .create_or_find_pending(&post(&agent, &lead, "Hello"), at(0))
        .await
        .unwrap();
    assert!(
        approvals
            .decide(&decision(&request, &lead, Decision::Approve), at(1))
            .await
            .unwrap()
    );
    let mut off = agent.clone();
    off.enabled = false;
    db.agents().update(&off, at(2)).await.unwrap();
    let month = month();
    assert!(matches!(
        approvals
            .use_approval(&step(&request, &request.payload_hash, &month), at(3))
            .await,
        Err(ApprovalStoreError::Disabled(id)) if id == agent.id
    ));
    assert_eq!(
        approvals.get(request.id).await.unwrap().unwrap().state,
        RequestState::Approved,
        "the approval stays unused"
    );
}

#[tokio::test]
async fn a_step_naming_a_request_uses_that_one() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let approvals = db.approvals();
    let mut approved = Vec::new();
    for minute in [0, 2] {
        let (request, made) = approvals
            .create_or_find_pending(&post(&agent, &lead, "Hello"), at(minute))
            .await
            .unwrap();
        assert!(made);
        assert!(
            approvals
                .decide(
                    &decision(&request, &lead, Decision::Approve),
                    at(minute + 1)
                )
                .await
                .unwrap()
        );
        approved.push(request);
    }
    let month = month();
    let newer = &approved[1];
    let used = approvals
        .use_approval(
            &StepToUse {
                request: Some(newer.id),
                ..step(newer, &newer.payload_hash, &month)
            },
            at(4),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(used.id, newer.id, "the request named, not the oldest");
    assert_eq!(
        approvals.get(approved[0].id).await.unwrap().unwrap().state,
        RequestState::Approved
    );
}

#[tokio::test]
async fn an_approver_open_requests_name_is_not_deleted() {
    let (_dir, db) = database().await;
    let (agent, lead) = set_up(&db).await;
    let (request, _) = db
        .approvals()
        .create_or_find_pending(&post(&agent, &lead, "Hello"), at(0))
        .await
        .unwrap();
    db.agents().replace_policies(&agent.id, &[]).await.unwrap();
    let id = &emberhut_core::approvals::ApproverId::parse("example-lead").unwrap();
    assert!(matches!(
        db.agents().delete_approver(id).await,
        Err(emberhut_db::AgentStoreError::ApproverHasRequests(_))
    ));
    assert!(
        db.approvals()
            .withdraw(request.id, None, None, at(1))
            .await
            .unwrap()
    );
    assert!(db.agents().delete_approver(id).await.unwrap());
}
