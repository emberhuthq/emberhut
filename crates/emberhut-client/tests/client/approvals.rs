//! Agents and approvals: what the client sends and how it reads the answers.

use emberhut_api::approvals::{
    ApprovalState, ApprovalsQuery, Decision, DecisionRequest, WithdrawRequest,
};
use emberhut_api::ids::{ApprovalId, ConfiguredAgentId};
use serde_json::json;

use crate::support::{Reply, TestServer, fresh};

fn approval(state: &str) -> serde_json::Value {
    json!({
        "id": 7, "agent": "client-a", "kind": "post", "target": "example blog",
        "summary": "The launch post", "text": "We shipped it.", "files": [],
        "payloadHash": "cd".repeat(32), "approver": "owner", "waitsOnOwner": true,
        "state": state, "createdAt": "2026-11-27T10:00:00.000Z"
    })
}

#[test]
fn reads_the_agents_and_the_approvals() {
    let server = TestServer::start(|request| match request.path.as_str() {
        "/api/agents" => Reply::Json(
            200,
            json!({ "agents": [{
                "id": "main", "name": "Main", "persona": null, "principal": "the owner",
                "room": 1, "project": null, "spendCapCents": null, "spendCurrency": null,
                "enabled": true, "pendingApprovals": 1
            }] }),
        ),
        _ => Reply::Json(200, json!({ "approvals": [approval("pending")] })),
    });
    let client = server.client(fresh("r1"));
    let agents = client.agents().unwrap().agents;
    assert!(agents[0].is_main());
    assert_eq!(agents[0].pending_approvals, 1);

    let query = ApprovalsQuery {
        state: Some(ApprovalState::Pending),
        agent: Some(ConfiguredAgentId::new("client-a")),
    };
    let approvals = client.approvals(&query).unwrap().approvals;
    assert_eq!(approvals[0].id, ApprovalId::new(7));
    assert!(approvals[0].waits_on_owner);
    let asked = &server.requests_to("/api/approvals")[0];
    assert_eq!(asked.query, "state=pending&agent=client-a");
}

#[test]
fn decides_and_withdraws_on_the_exact_payload() {
    let server = TestServer::start(|request| {
        let state = if request.path.ends_with("/decision") {
            "approved"
        } else {
            "withdrawn"
        };
        Reply::Json(200, json!({ "approval": approval(state) }))
    });
    let client = server.client(fresh("r1"));
    let decision = DecisionRequest {
        decision: Decision::Approve,
        payload_hash: "cd".repeat(32),
        reason: None,
    };
    let reply = client.decide(ApprovalId::new(7), &decision).unwrap();
    assert_eq!(reply.approval.state, ApprovalState::Approved);
    let sent = &server.requests_to("/api/approvals/7/decision")[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(
        sent.json(),
        json!({ "decision": "approve", "payloadHash": "cd".repeat(32) })
    );

    let reply = client
        .withdraw(ApprovalId::new(7), &WithdrawRequest::default())
        .unwrap();
    assert_eq!(reply.approval.state, ApprovalState::Withdrawn);
    assert_eq!(
        server.requests_to("/api/approvals/7/withdraw")[0].json(),
        json!({})
    );
}

#[test]
fn reads_one_request_and_a_changed_one_is_a_conflict() {
    let server = TestServer::start(|request| {
        if request.path.ends_with("/decision") {
            Reply::Json(409, json!({ "error": "This request changed." }))
        } else {
            Reply::Json(200, json!({ "approval": approval("pending") }))
        }
    });
    let client = server.client(fresh("r1"));
    let reply = client.approval(ApprovalId::new(7)).unwrap();
    assert_eq!(reply.approval.text.as_deref(), Some("We shipped it."));
    assert_eq!(server.requests_to("/api/approvals/7")[0].method, "GET");

    let decision = DecisionRequest {
        decision: Decision::Reject,
        payload_hash: "ab".repeat(32),
        reason: None,
    };
    let error = client.decide(ApprovalId::new(7), &decision).unwrap_err();
    assert!(
        matches!(error, emberhut_client::Error::Status { status: 409, .. }),
        "{error:?}"
    );
}
