use serde_json::{Value, json};

use super::*;
use crate::fixtures::issue;

fn now() -> Timestamp {
    "2026-11-28T09:00:00Z".parse().unwrap()
}

fn plan(value: Value) -> Plan {
    serde_json::from_value(value).unwrap()
}

fn step(id: u32, parent: Option<u32>, state: &str) -> Value {
    json!({"id": id, "parent": parent, "title": format!("Step {id}"), "state": state,
        "updated": "2026-11-28T08:00:00Z"})
}

fn task(status: &str, live: &str, plan: &Value) -> Issue {
    issue(
        1,
        status,
        json!({"owner": "assistant", "live": {"state": live}, "plan": plan, "action": "Pick a key"}),
    )
}

fn segments(bar: Option<Bar>) -> Vec<Segment> {
    match bar {
        Some(Bar::Segments(segments)) => segments,
        other => panic!("not segments: {other:?}"),
    }
}

use Segment::{Blocked as B, Doing as D, Done as F, Todo as T};

#[test]
fn a_running_task_leads_with_its_spinner_its_step_its_ticks_and_its_bar() {
    let row = row_plan(&task(
        "working",
        "running",
        &json!({"done": 2, "total": 5, "current": {"id": 5, "title": "Build the app", "state": "doing"}}),
    ))
    .unwrap();
    assert_eq!(row.lead, Lead::Running);
    assert_eq!(row.line, ("Build the app".to_owned(), Tone::Muted));
    assert_eq!(row.ticks.as_deref(), Some("2/5"));
    assert_eq!(segments(row.bar), [F, F, D, T, T]);
    assert_eq!(spoken_row(&row_plan_of_empty()), NO_PLAN);
}

fn row_plan_of_empty() -> RowPlan {
    row_plan(&task("working", "running", &json!({"done": 0, "total": 0}))).unwrap()
}

#[test]
fn a_blocked_task_says_on_what_in_red() {
    let running_stuck = row_plan(&task(
        "working",
        "running",
        &json!({"done": 1, "total": 3, "blocker": "No certificate",
            "current": {"id": 2, "title": "Sign it", "state": "blocked"}}),
    ))
    .unwrap();
    assert_eq!(running_stuck.lead, Lead::Blocked);
    assert_eq!(
        running_stuck.line,
        ("Blocked: Sign it · No certificate".to_owned(), Tone::Danger)
    );
    assert_eq!(segments(running_stuck.bar), [F, B, T]);
    let open = row_plan(&task(
        "todo",
        "stalled",
        &json!({"done": 1, "total": 4, "blocker": "No token",
            "current": {"id": 3, "title": "Next one", "state": "todo"}}),
    ))
    .unwrap();
    assert_eq!(open.line, ("Blocked: No token".to_owned(), Tone::Danger));
    assert_eq!(segments(open.bar), [F, B, T, T], "red after the done ones");
    let running = row_plan(&task(
        "working",
        "running",
        &json!({"done": 1, "total": 4, "blocker": "No token",
            "current": {"id": 3, "title": "Next one", "state": "doing"}}),
    ))
    .unwrap();
    assert_eq!(running.lead, Lead::Running, "it still runs");
    assert_eq!(segments(running.bar), [F, D, B, T]);
}

#[test]
fn waiting_queued_and_open_tasks_say_where_they_are() {
    let waiting = row_plan(&task("waiting", "waiting", &json!({"done": 3, "total": 6}))).unwrap();
    assert_eq!(waiting.lead, Lead::NeedsYou);
    assert_eq!(
        waiting.line,
        ("Waiting on you: Pick a key".to_owned(), Tone::Accent)
    );
    assert_eq!(segments(waiting.bar), [F, F, F, T, T, T]);
    let queued = row_plan(&task(
        "working",
        "queued",
        &json!({"done": 0, "total": 2, "current": {"id": 1, "title": "Look", "state": "todo"}}),
    ))
    .unwrap();
    assert_eq!(queued.line.0, "Queued · first: Look");
    let fresh =
        json!({"done": 0, "total": 2, "current": {"id": 1, "title": "Look", "state": "todo"}});
    assert_eq!(
        row_plan(&task("todo", "stalled", &fresh)).unwrap().line.0,
        "Not started · first: Look"
    );
    let mine = issue(
        2,
        "todo",
        json!({"live": {"state": "todo"}, "plan": {"done": 0, "total": 0}}),
    );
    let row = row_plan(&mine).unwrap();
    assert_eq!(row.line, (format!("To do · {NO_PLAN}"), Tone::Faint));
    assert_eq!((row.ticks, row.bar), (None, None));
    assert_eq!(
        row_plan(&task("done", "done", &json!({"done": 2, "total": 2}))),
        None
    );
    assert_eq!(
        row_plan(&issue(3, "todo", json!({}))),
        None,
        "an older server sends no plan"
    );
}

#[test]
fn a_long_plan_has_one_plain_bar() {
    let summary: PlanSummary = serde_json::from_value(json!({"done": 13, "total": 20})).unwrap();
    assert_eq!(bar_of_summary(&summary, true), Some(Bar::Plain(0.65)));
}

#[test]
fn a_running_task_goes_quiet_after_twenty_minutes_without_news() {
    let quiet_task = task(
        "working",
        "running",
        &json!({"done": 1, "total": 3, "updated": "2026-11-28T08:35:00Z"}),
    );
    assert_eq!(quiet(&quiet_task, &[], now()).as_deref(), Some("quiet 25m"));
    let session: RunnerSession = serde_json::from_value(json!({"id": "s-1", "machine": "pc",
        "title": "A session", "project": "", "status": "running",
        "updatedAt": "2026-11-28T08:55:00Z", "place": "PC", "doing": null, "account": null,
        "accountName": null, "ask": null, "last": null, "direct": false, "model": null,
        "tasks": [1]}))
    .unwrap();
    assert_eq!(
        quiet(&quiet_task, &[&session], now()),
        None,
        "its session moved"
    );
    let waiting = task("waiting", "waiting", &json!({"done": 0, "total": 0}));
    assert_eq!(quiet(&waiting, &[], now()), None, "only a running task");
}

#[test]
fn the_summary_line_counts_as_the_server_does() {
    let written = plan(json!({
        "steps": [step(1, None, "done"), step(2, None, "doing"), step(3, Some(2), "done"),
            step(4, Some(2), "skipped"), step(5, None, "blocked"), step(6, None, "skipped")],
        "summary": {"done": 2, "total": 3, "eta": "about 2 hours", "updated": "2026-11-28T08:57:00Z",
            "current": {"id": 2, "title": "Step 2", "state": "doing"}}
    }));
    assert_eq!(
        summary_line(&written),
        "2 of 3 done · 1 blocked · 2 skipped"
    );
    assert_eq!(
        eta_line(&written.summary, now()).as_deref(),
        Some("about 2 hours · updated 3 minutes ago")
    );
    assert_eq!(
        segments(bar_of_plan(&written)),
        [F, F, B],
        "the sub-steps in place of their step, skipped ones out"
    );
    assert_eq!(now_line(&written).as_deref(), Some("Step 2"));
    let empty: PlanSummary = serde_json::from_value(
        json!({"done": 0, "total": 0, "unplannedTurn": "2026-11-28T08:40:00Z"}),
    )
    .unwrap();
    assert_eq!(
        unplanned_line(&empty, now()).as_deref(),
        Some("A turn ended 20 minutes ago without a plan")
    );
    assert!(fraction(&empty).abs() < f64::EPSILON);
    let skipped = plan(
        json!({"steps": [step(1, None, "skipped"), step(2, None, "skipped")],
        "summary": {"done": 0, "total": 0}}),
    );
    assert_eq!(
        summary_line(&skipped),
        "0 of 0 done · 2 skipped",
        "every step skipped is a plan, not none"
    );
    let parent = plan(
        json!({"steps": [step(1, None, "blocked"), step(2, Some(1), "todo")],
        "summary": {"done": 0, "total": 1}}),
    );
    assert_eq!(
        summary_line(&parent),
        "0 of 1 done · 1 blocked",
        "a blocked step with sub-steps counts"
    );
}

fn shape(items: &[Item<'_>]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Step(row) => format!(
                "{}{}{}",
                if row.nested { "-" } else { "" },
                row.step.id,
                if row.current { "*" } else { "" }
            ),
            Item::Earlier { count, all_done } => format!("earlier {count} {all_done}"),
            Item::Later(count) => format!("later {count}"),
        })
        .collect()
}

#[test]
fn sub_steps_follow_their_step_and_the_current_one_is_marked() {
    let plan = plan(json!({
        "steps": [step(1, None, "done"), step(3, Some(2), "doing"), step(2, None, "doing"),
            step(4, Some(2), "todo"), step(5, Some(9), "todo"), step(6, None, "skipped")],
        "summary": {"done": 1, "total": 4, "current": {"id": 2, "title": "Step 2", "state": "doing"}}
    }));
    assert_eq!(
        shape(&checklist(&plan, false, false)),
        ["1", "2*", "-3*", "-4", "5", "6"],
        "a sub-step of a step not in the plan stands on its own"
    );
    assert_eq!(now_line(&plan).as_deref(), Some("Step 2 › Step 3"));
}

#[test]
fn a_long_checklist_folds_around_the_current_step() {
    let mut steps: Vec<Value> = (1..=6).map(|id| step(id, None, "done")).collect();
    steps.push(step(7, None, "doing"));
    steps.extend((8..=12).map(|id| step(id, None, "todo")));
    let plan = plan(json!({"steps": steps, "summary": {"done": 6, "total": 12,
        "current": {"id": 7, "title": "Step 7", "state": "doing"}}}));
    assert_eq!(
        shape(&checklist(&plan, false, false)),
        ["earlier 5 true", "6", "7*", "8", "9", "later 3"]
    );
    assert_eq!(checklist(&plan, true, true).len(), 12, "both folds open");
    let short = Plan {
        steps: plan.steps[..8].to_vec(),
        ..plan.clone()
    };
    assert_eq!(
        checklist(&short, false, false).len(),
        8,
        "short plans never fold"
    );
}

#[test]
fn steps_read_short_and_aloud() {
    let mut blocked: PlanStep = serde_json::from_value(step(2, None, "blocked")).unwrap();
    blocked.note = Some("waiting on the certificate".into());
    assert_eq!(
        spoken(&blocked),
        "Blocked: Step 2, waiting on the certificate"
    );
    blocked.started = Some(emberhut_api::time::Timestamp::new("2026-11-28T08:52:00Z"));
    assert_eq!(so_far(&blocked, now()).as_deref(), Some("8m so far"));
    blocked.started = Some(emberhut_api::time::Timestamp::new("2026-11-28T08:53:57Z"));
    assert_eq!(
        so_far(&blocked, now()).as_deref(),
        Some("6m so far"),
        "no seconds"
    );
    assert_eq!(step_key(TaskId::new(4), 2), "plan-4-2");
    assert_eq!(
        link_label("https://github.com/example/app/pull/16?x=1"),
        "github.com/example/app/pull/16"
    );
    assert_eq!(link_label("https://www.example.com/"), "example.com");
    assert_eq!(link_label("/files/a1/report.html"), "report.html");
    let long = format!("https://example.com/{}", "a".repeat(60));
    assert_eq!(link_label(&long).chars().count(), LINK_CHARS + 1);
}
