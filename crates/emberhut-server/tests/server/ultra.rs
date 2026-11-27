//! Ultra runs: the lead's plan (`ultra_plan`), the gate before each new sub-agent and shell
//! command (`POST /ultra/gate`), the sub-agents seen in the lead's stream and shown as the
//! state's `agentsNow`, a part as its own lane (`ultra_lane`) and waiting for it
//! (`ultra_wait`).
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::sync::Arc;

use emberhut_core::task::TaskId;
use serde_json::{Value, json};

use crate::common::{Answer, Call, TestApp, text};
use crate::fake_cli::{FakeCli, init, is_user, result};
use crate::workers::{answering_cli, assistant_task, until_ended, worker_app, worker_tool};

/// The gate's answer to `hook` from `task`'s lead.
async fn gate(app: &TestApp, task: TaskId, hook: &Value) -> Answer {
    let token = app.state.worker_tokens().for_task(task).unwrap();
    app.call(
        Call::post(emberhut_api::paths::ULTRA_GATE)
            .header("authorization", &format!("Bearer {}", token.as_str()))
            .json(hook),
    )
    .await
}

fn why(answer: &Answer) -> String {
    answer.json()["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn agent(id: &str) -> Value {
    json!({ "tool_name": "Agent", "tool_use_id": id, "tool_input": { "description": "Part 1: x" } })
}

#[tokio::test]
pub(crate) async fn a_lead_plans_first_and_the_gate_holds_its_plan_and_caps() {
    let app = worker_app(&answering_cli("done")).await;
    let task = assistant_task(&app, "a big audit", true).await;

    let tools = crate::workers::worker_rpc(&app, task, "tools/list", json!({})).await;
    let names = tools.json()["result"]["tools"].to_string();
    for name in ["ultra_plan", "ultra_lane", "ultra_wait"] {
        assert!(names.contains(&format!("\"{name}\"")), "{name}");
    }

    let held = gate(&app, task, &agent("tu-0")).await;
    assert_eq!(held.status, 200, "{}", held.text);
    assert!(
        why(&held).starts_with("Post your plan first"),
        "{}",
        held.text
    );
    assert_eq!(
        gate(&app, task, &json!({ "tool_name": "Read" }))
            .await
            .json(),
        json!({}),
        "other tools pass"
    );
    let refused = app
        .call(
            Call::post(emberhut_api::paths::ULTRA_GATE)
                .header("authorization", "Bearer x")
                .json(&json!({})),
        )
        .await;
    assert_eq!(refused.status, 401);

    let plan = worker_tool(
        &app,
        task,
        "ultra_plan",
        json!({ "parts": [{ "n": 1, "name": "parser", "size": "S" }, { "n": 2, "name": "docs", "size": "S", "verify": false }] }),
    )
    .await;
    assert!(
        text(&plan).starts_with("Cleared: 3 sub-agents, about 200k tokens, 4 at a time, cap 3M."),
        "{}",
        text(&plan)
    );

    for n in 0..4 {
        assert_eq!(
            gate(&app, task, &agent(&format!("tu-{n}"))).await.json(),
            json!({}),
            "{n}"
        );
    }
    let fifth = gate(&app, task, &agent("tu-4")).await;
    assert!(
        why(&fifth).starts_with("4 sub-agents are running and this run takes 4 at a time"),
        "{}",
        fifth.text
    );

    let push = gate(
        &app,
        task,
        &json!({ "tool_name": "Bash", "tool_input": { "command": "cd repo && git push origin main" }, "cwd": "/" }),
    )
    .await;
    assert!(
        why(&push).starts_with("Outward actions (push, merge, PR, release, deploy) wait"),
        "{}",
        push.text
    );
    let detail = app.state.tasks().detail(task).await.unwrap().unwrap();
    let decision = detail.task.decision.unwrap();
    assert!(
        decision
            .question
            .starts_with("The ultra run's lead wants to push, merge or deploy:")
    );
    assert_eq!(
        gate(&app, task, &json!({ "tool_name": "Bash", "tool_input": { "command": "cat <<'EOF'\ngit push\nEOF" } })).await.json(),
        json!({}),
        "a heredoc's text is not a push"
    );

    // The owner's Go ahead on the outward card covers the task's outward steps from then on.
    app.state.tasks().decide(task, "Go ahead").await.unwrap();
    assert_eq!(
        gate(
            &app,
            task,
            &json!({ "tool_name": "Bash", "tool_input": { "command": "git push" } })
        )
        .await
        .json(),
        json!({})
    );
}

#[tokio::test]
async fn a_big_plan_waits_for_the_owner_and_their_go_ahead_raises_the_cap() {
    let app = worker_app(&answering_cli("done")).await;
    let task = assistant_task(&app, "a huge migration", true).await;
    let parts: Vec<Value> = (1..=8)
        .map(|n| json!({ "n": n, "name": format!("part {n}"), "size": "L" }))
        .collect();
    let plan = worker_tool(&app, task, "ultra_plan", json!({ "parts": parts })).await;
    assert_eq!(
        text(&plan),
        "Waiting for the owner's OK (a big run). Stop now, without task_done; you're resumed with their answer."
    );
    assert!(why(&gate(&app, task, &agent("tu-1")).await).starts_with("The owner hasn't OKed"));
    let done = worker_tool(&app, task, "task_done", json!({ "summary": "x" })).await;
    assert!(
        text(&done).starts_with("Not done: this ultra run waits"),
        "{}",
        text(&done)
    );

    app.state.tasks().decide(task, "Go ahead").await.unwrap();
    let run = app.state.workers().run_of(task).await.unwrap().unwrap();
    assert_eq!(run.state, "running");
    assert!(
        run.budget >= 6_000_000.0 * 1.25,
        "the cap is raised to fit: {}",
        run.budget
    );
    assert_eq!(gate(&app, task, &agent("tu-1")).await.json(), json!({}));
}

/// A lead whose stream reports one sub-agent starting, and stays in its turn.
fn lead_cli() -> Arc<FakeCli> {
    FakeCli::new(|_, line| {
        if is_user(line) {
            vec![
                init("s-lead"),
                json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                    "tool_use_id": "tu-1", "description": "Part 1: the parser", "task_type": "local_agent" })
                .to_string(),
                json!({ "type": "system", "subtype": "task_progress", "task_id": "a1",
                    "description": "Reading the grammar", "usage": { "total_tokens": 1200, "tool_uses": 3 } })
                .to_string(),
            ]
        } else {
            Vec::new()
        }
    })
}

#[tokio::test]
async fn the_sub_agents_in_a_lead_s_stream_are_the_agents_working_now() {
    let cli = lead_cli();
    let app = worker_app(&cli).await;
    let task = assistant_task(&app, "a parallel job", true).await;
    app.state.tasks().start_worker(task).await.unwrap();
    for _ in 0..300 {
        if !app
            .state
            .workers()
            .store()
            .of_task(task)
            .await
            .unwrap()
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let agents = app.state.workers().store().of_task(task).await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name.as_deref(), Some("Part 1: the parser"));
    let session = app.sign_in().await;
    let state = app.call(Call::get("/api/state").token(&session)).await;
    let now = state.json()["agentsNow"].clone();
    let rows = now.as_array().unwrap();
    assert_eq!(rows[0]["lead"], true, "{now}");
    assert_eq!(rows[0]["kind"], "lead");
    assert_eq!(rows[0]["viewer"], format!("w-{}", task.get()));
    assert_eq!(rows[1]["name"], "Part 1: the parser");
    assert_eq!(rows[1]["line"], "Reading the grammar");
    assert_eq!(rows[1]["kind"], "helper");
    assert_eq!(rows[1]["parent"], "tu-1");

    // Stopping the worker stops its run's sub-agents too.
    app.state.tasks().stop_worker(task).await.unwrap();
    until_ended(&app, task).await;
    let agents = app.state.workers().store().of_task(task).await.unwrap();
    assert_eq!(agents[0].state, "stopped");
    let state = app.call(Call::get("/api/state").token(&session)).await;
    assert_eq!(state.json()["agentsNow"], json!([]));
}

#[tokio::test]
async fn a_lane_waits_for_the_plan_and_waiting_reads_each_lane_s_state() {
    let app = worker_app(&answering_cli("done")).await;
    let task = assistant_task(&app, "a split job", true).await;
    let lane = worker_tool(
        &app,
        task,
        "ultra_lane",
        json!({ "part": 1, "name": "docs", "brief": "Write the docs for the parser, report back.", "cwd": "/srv/app" }),
    )
    .await;
    assert_eq!(
        text(&lane),
        "Not started: post the plan first (ultra_plan) and wait until it's cleared."
    );
    let mut seen = emberhut_db::AgentSeen::new(task, "r-1", "r-1", "runner");
    seen.name = Some("Lane 1: docs".to_owned());
    seen.state = Some("done".to_owned());
    app.state
        .workers()
        .store()
        .upsert(seen, app.clock_now())
        .await
        .unwrap();
    let waited = worker_tool(
        &app,
        task,
        "ultra_wait",
        json!({ "sessions": ["r-1"], "minutes": 0.1 }),
    )
    .await;
    assert_eq!(text(&waited), "r-1: done");
    let none = worker_tool(
        &app,
        task,
        "ultra_wait",
        json!({ "sessions": [], "minutes": 1 }),
    )
    .await;
    assert_eq!(none.is_error, Some(true), "at least one session");
}

#[tokio::test]
async fn a_worker_of_an_ultra_task_gets_the_gate_as_its_hook() {
    let cli = FakeCli::new(|_, line| {
        if is_user(line) {
            vec![result("ok")]
        } else {
            Vec::new()
        }
    });
    let app = worker_app(&cli).await;
    let task = assistant_task(&app, "an ultra job", true).await;
    app.state.tasks().start_worker(task).await.unwrap();
    until_ended(&app, task).await;
    let settings = cli.launches()[0].flag("--settings").unwrap().to_owned();
    let settings: Value = serde_json::from_str(&settings).unwrap();
    let hook = &settings["hooks"]["PreToolUse"][0];
    assert_eq!(hook["matcher"], "Agent|Task|Workflow|Bash");
    let command = hook["hooks"][0]["command"].as_str().unwrap();
    assert!(
        command.contains(emberhut_api::paths::ULTRA_GATE),
        "{command}"
    );
    assert!(command.contains("Bearer $EMBERHUT_MCP_TOKEN"), "{command}");
    let brief = &cli.said()[0];
    assert!(
        brief.contains("ULTRA MODE: you are the lead of an ultra run."),
        "{brief}"
    );
}

#[tokio::test]
pub(crate) async fn the_gate_takes_a_hook_s_input_only() {
    let app = worker_app(&answering_cli("done")).await;
    let task = assistant_task(&app, "an ultra job", true).await;
    let token = app.state.worker_tokens().for_task(task).unwrap();
    let answer = app
        .call(
            Call::post(emberhut_api::paths::ULTRA_GATE)
                .header("authorization", &format!("Bearer {}", token.as_str()))
                .raw("[1, 2]"),
        )
        .await;
    assert_eq!(answer.status, 400, "{}", answer.text);
}

#[tokio::test]
async fn a_plain_worker_gets_the_gate_before_its_shell_commands_and_its_push_is_held() {
    let cli = FakeCli::new(|_, line| {
        if is_user(line) {
            vec![init("s-1")]
        } else {
            Vec::new()
        }
    });
    let app = worker_app(&cli).await;
    let task = assistant_task(&app, "a fix", false).await;
    app.state.tasks().start_worker(task).await.unwrap();
    let settings = cli.launches()[0].flag("--settings").unwrap().to_owned();
    let settings: Value = serde_json::from_str(&settings).unwrap();
    let hook = &settings["hooks"]["PreToolUse"][0];
    assert_eq!(hook["matcher"], "Bash");
    assert!(
        cli.launches()[0]
            .args
            .iter()
            .any(|a| a == "Bash(git push:*)"),
        "the deny rules stay as well"
    );

    for command in [
        "git -C . push",
        "git --no-pager push",
        "/usr/bin/git push",
        "env git push",
        "sh -c 'git push'",
        "npm publish",
        "gh workflow run deploy.yml",
        "curl -X PUT https://api.github.com/repos/o/n/pulls/1/merge",
    ] {
        let held = gate(
            &app,
            task,
            &json!({ "tool_name": "Bash", "tool_input": { "command": command }, "cwd": "/" }),
        )
        .await;
        assert!(
            why(&held).starts_with("Outward actions (push, merge, PR, release, deploy) wait"),
            "{command}: {}",
            held.text
        );
    }
    let detail = app.state.tasks().detail(task).await.unwrap().unwrap();
    assert!(
        detail
            .task
            .decision
            .unwrap()
            .question
            .starts_with("The worker wants to push, merge or deploy:")
    );
    assert_eq!(
        gate(
            &app,
            task,
            &json!({ "tool_name": "Bash", "tool_input": { "command": "cargo test && git status" } })
        )
        .await
        .json(),
        json!({}),
        "a local command passes"
    );
    assert_eq!(
        gate(&app, task, &agent("tu-1")).await.json(),
        json!({}),
        "a plain worker's sub-agents have no plan to wait for"
    );
}

#[tokio::test]
async fn a_lane_never_starts_while_the_task_s_outward_steps_are_not_covered() {
    let app = worker_app(&answering_cli("done")).await;
    let task = assistant_task(&app, "a split job", true).await;
    let plan = worker_tool(
        &app,
        task,
        "ultra_plan",
        json!({ "parts": [{ "n": 1, "name": "ship", "size": "S" }] }),
    )
    .await;
    assert!(text(&plan).starts_with("Cleared"), "{}", text(&plan));
    let lane = worker_tool(
        &app,
        task,
        "ultra_lane",
        json!({ "part": 1, "name": "ship", "brief": "Push the branch and merge it: git push origin feat && gh pr merge 5", "cwd": "/srv/app" }),
    )
    .await;
    assert!(
        text(&lane).starts_with("Not started: a lane runs on a runner without the gate"),
        "{}",
        text(&lane)
    );
    let agents = app.state.workers().agents_of(task).await.unwrap();
    assert!(agents.is_empty(), "no lane was opened: {agents:?}");
}

#[tokio::test]
async fn waiting_reads_only_this_run_s_own_lanes() {
    let app = worker_app(&answering_cli("done")).await;
    let task = assistant_task(&app, "a split job", true).await;
    let other = assistant_task(&app, "another job", true).await;
    let mut seen = emberhut_db::AgentSeen::new(other, "r-9", "r-9", "runner");
    seen.state = Some("done".to_owned());
    seen.last = Some("the other task's answer".to_owned());
    app.state
        .workers()
        .store()
        .upsert(seen, app.clock_now())
        .await
        .unwrap();
    let waited = worker_tool(
        &app,
        task,
        "ultra_wait",
        json!({ "sessions": ["r-9"], "minutes": 1 }),
    )
    .await;
    assert_eq!(text(&waited), "r-9: not one of this run's lanes");
}
