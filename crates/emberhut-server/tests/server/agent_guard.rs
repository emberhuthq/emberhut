//! The guard rails for a task made in an agent's room: its final steps and its outward steps
//! are its policies' to decide (nobody: through; the owner: the owner's cards; an external
//! approver: a request, and once approved the step goes through once), a hard step is always
//! the owner's card, a disabled agent's steps wait, and a trusted requester's request is the
//! job's grant. A default agent's task is the owner's rules', as before.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use emberhut_core::agents::AgentId;
use emberhut_core::task::{TaskId, Who};
use emberhut_server::services::guard::agents::AgentCheck;
use emberhut_server::services::guard::go_ahead::{GateCard, GateResult};
use serde_json::json;

use crate::agent_desk::AgentDesk;
use crate::common::text;

const PAGE: &str = "Example blog · https://blog.example/new";

fn card(question: &str) -> GateCard {
    GateCard {
        question: question.to_owned(),
        action: "OK the click".to_owned(),
        brief: Vec::new(),
    }
}

/// A task of the agent's, made in its room for `who`.
async fn agents_task(desk: &AgentDesk, who: &str) -> TaskId {
    let made = desk
        .call(
            "task_create",
            json!({ "title": "Publish the launch post", "requested_by": who,
                "brief": "Publish the launch post on the example blog, word for word." }),
        )
        .await;
    assert!(text(&made).contains("created"), "{made:?}");
    desk.app.state.tasks().in_room(desk.room).await.unwrap()[0].id
}

async fn gate(desk: &AgentDesk, task: TaskId, what: &str, composed: &[&str]) -> GateResult {
    let composed: Vec<String> = composed.iter().map(|said| (*said).to_owned()).collect();
    desk.app
        .state
        .guard()
        .step_gate(task, what, PAGE, &card(&format!("{what}?")), &composed)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_agents_post_waits_on_its_approver_and_goes_through_once_approved() {
    let desk = AgentDesk::new(&["task_create"]).await;
    let task = agents_task(&desk, "Example person").await;
    let held = gate(&desk, task, "Post", &["We shipped."]).await;
    let GateResult::Text(held) = held else {
        panic!("not held: {held:?}");
    };
    assert!(
        held.starts_with("Held: it waits on the approver's approval (request 1)."),
        "{held}"
    );
    assert!(!held.contains("example-lead") && !held.contains("U0APPROVER"));
    // No owner's card was asked.
    assert_ne!(
        desk.app.state.tasks().status_of(task).await.unwrap(),
        Some(emberhut_core::task::Status::Waiting)
    );
    let agent = AgentId::parse("client-a").unwrap();
    let request = desk
        .app
        .state
        .approvals()
        .status(&agent, emberhut_core::ApprovalId::new(1))
        .await
        .unwrap();
    assert_eq!(request.task, Some(task));
    assert_eq!(request.text.as_deref(), Some("We shipped."));
    assert_eq!(request.target, PAGE);

    // Other words are another request; the approved words go through once.
    let other = gate(&desk, task, "Post", &["We shipped!"]).await;
    assert!(
        matches!(&other, GateResult::Text(t) if t.contains("(request 2)")),
        "{other:?}"
    );
    desk.approve(&request).await;
    let through = gate(&desk, task, "Post", &["We shipped."]).await;
    assert!(
        matches!(&through, GateResult::By(by) if by.starts_with("Went through.") && by.contains("request 1 on the approver's approval at ")),
        "{through:?}"
    );
    let again = gate(&desk, task, "Post", &["We shipped."]).await;
    assert!(
        matches!(again, GateResult::Text(_)),
        "an approval goes through once"
    );
}

#[tokio::test]
async fn the_agents_policy_decides_a_final_step_and_a_hard_one_stays_the_owners() {
    let desk = AgentDesk::new(&["task_create"]).await;
    let task = agents_task(&desk, "Example person").await;
    // No policy for `other`: an approval request to the owner (in the apps), never the job's
    // grant or a card; the owner's decision lets it through once.
    let owners = gate(&desk, task, "Save", &[]).await;
    assert!(
        matches!(&owners, GateResult::Text(t) if t.starts_with("Held: it waits on the owner's approval (request 1).")),
        "{owners:?}"
    );
    let request = desk
        .app
        .state
        .approvals()
        .status(
            &AgentId::parse("client-a").unwrap(),
            emberhut_core::ApprovalId::new(1),
        )
        .await
        .unwrap();
    let decided = desk
        .app
        .call(
            crate::common::Call::post("/api/approvals/1/decision")
                .token(&desk.session)
                .json(
                    &json!({ "decision": "approve", "payloadHash": request.payload_hash.to_hex() }),
                ),
        )
        .await;
    assert_eq!(decided.status, 200, "{}", decided.text);
    let through = gate(&desk, task, "Save", &[]).await;
    assert!(
        matches!(&through, GateResult::By(by) if by.contains("the owner's approval")),
        "{through:?}"
    );
    desk.set_policies(&json!([
        { "kind": "post", "approver": "example-lead" },
        { "kind": "other", "approver": "none" },
    ]))
    .await;
    let through = gate(&desk, task, "Save", &[]).await;
    assert_eq!(
        through,
        GateResult::By("No approval is needed for a other.".to_owned())
    );
    // Money is the owner's card every time.
    let hard = gate(&desk, task, "Pay now", &[]).await;
    assert!(
        matches!(&hard, GateResult::Text(t) if t.contains("money or account security")),
        "{hard:?}"
    );
    // A disabled agent's steps wait.
    desk.enable(false).await;
    let off = gate(&desk, task, "Save", &[]).await;
    assert!(
        matches!(&off, GateResult::Text(t) if t.contains("the agent is disabled")),
        "{off:?}"
    );
}

#[tokio::test]
async fn an_agents_merge_and_deploy_are_its_policies_to_decide() {
    let desk = AgentDesk::new(&["task_create"]).await;
    let task = agents_task(&desk, "Example person").await;
    let workers = desk.app.state.workers();
    let bash = |command: &str| json!({ "tool_name": "Bash", "tool_input": { "command": command }, "cwd": "/tmp" });
    // `merge` needs nobody's approval for this agent.
    assert_eq!(workers.gate(task, &bash("git push")).await.unwrap(), None);
    // `deploy` has no policy: an approval request to the owner, never their outward card.
    let owners = workers
        .gate(task, &bash("npm publish"))
        .await
        .unwrap()
        .unwrap();
    assert!(owners.contains("waits on the owner's approval"), "{owners}");
    desk.app
        .state
        .approvals()
        .withdraw(emberhut_core::ApprovalId::new(1), None, None)
        .await
        .unwrap();
    // Given to the approver: a request, the command line its words.
    desk.set_policies(&json!([
        { "kind": "merge", "approver": "none" },
        { "kind": "deploy", "approver": "example-lead" },
    ]))
    .await;
    let held = workers
        .gate(task, &bash("npm publish"))
        .await
        .unwrap()
        .unwrap();
    assert!(held.contains("waits on the approver's approval"), "{held}");
    let agent = AgentId::parse("client-a").unwrap();
    let request = desk
        .app
        .state
        .approvals()
        .list(Some("pending"), Some("client-a"))
        .await
        .unwrap();
    assert_eq!(request[0].kind.as_str(), "deploy");
    assert_eq!(request[0].text.as_deref(), Some("npm publish"));
    let stored = desk
        .app
        .state
        .approvals()
        .status(&agent, emberhut_core::ApprovalId::new(request[0].id.get()))
        .await
        .unwrap();
    desk.approve(&stored).await;
    assert_eq!(
        workers.gate(task, &bash("npm publish")).await.unwrap(),
        None
    );
    // A command with no outward step is never held.
    assert_eq!(workers.gate(task, &bash("git status")).await.unwrap(), None);
}

#[tokio::test]
async fn no_grant_of_the_owners_ever_opens_an_agents_task_and_a_default_task_is_unchanged() {
    let desk = AgentDesk::new(&["task_create"]).await;
    let task = agents_task(&desk, "Example person").await;
    let guard = desk.app.state.guard();
    // A trusted requester the agent itself names grants nothing on its task.
    desk.put(
        "/api/trusted-requesters",
        &json!({ "requesters": [{ "requester": "Example person", "agent": "client-a" }] }),
    )
    .await;
    assert_eq!(guard.job_grant(task).await.unwrap(), None);
    // Nor do the owner's OKs: the job's grant and their OK for its outward steps.
    guard
        .grant_job(task, "their Go ahead at 10:00", None)
        .await
        .unwrap();
    guard
        .outward_ok(task, "their OK relayed at 10:00")
        .await
        .unwrap();
    assert_eq!(guard.job_grant(task).await.unwrap(), None);
    assert_eq!(guard.covered_by(task).await.unwrap(), None);
    let held = gate(&desk, task, "Save", &[]).await;
    assert!(
        matches!(&held, GateResult::Text(t) if t.contains("waits on the owner's approval")),
        "{held:?}"
    );
    // A hard step is the owner's own card, never a grant.
    let hard = gate(&desk, task, "Pay now", &[]).await;
    assert!(matches!(&hard, GateResult::Text(t) if t.contains("money or account security")));

    // The default agent's task: the owner's rules, untouched.
    let owners = desk
        .app
        .state
        .tasks()
        .create(
            emberhut_db::NewTask {
                title: "The owner's own job".to_owned(),
                ..emberhut_db::NewTask::default()
            },
            Who::Person,
        )
        .await
        .unwrap();
    assert_eq!(
        guard
            .agent_step(owners.id, "Post", PAGE, &["hi".to_owned()])
            .await
            .unwrap(),
        AgentCheck::Owner
    );
    assert_eq!(
        guard
            .agent_outward(owners.id, "npm publish", "/tmp")
            .await
            .unwrap(),
        AgentCheck::Owner
    );
    assert!(
        guard
            .job_grant(owners.id)
            .await
            .unwrap()
            .unwrap()
            .starts_with("approved by your request at ")
    );
}
