use serde_json::{Value, json};

use emberhut_api::task_page::TaskPage;

use super::*;
use crate::fixtures::task_page;

fn now() -> ZonedDateTime {
    "2026-11-28T09:00:00+00:00[UTC]".parse().unwrap()
}

fn run(id: i64, thread: &str, extra: Value) -> Value {
    let mut run = json!({"id": id, "kind": "thread", "machine": "pc", "thread": thread,
        "title": null, "asked": null, "answer": null, "status": null, "requested": null,
        "started": "2026-11-28T08:00:00Z", "ended": "2026-11-28T08:20:00Z",
        "outcome": "completed"});
    if let (Value::Object(run), Value::Object(extra)) = (&mut run, extra) {
        run.extend(extra);
    }
    run
}

fn session(id: &str) -> Value {
    json!({"id": id, "machine": "pc", "title": "Emberhut · Build", "project": "",
        "status": "running", "updatedAt": "2026-11-28T08:00:00Z", "place": "PC", "doing": null,
        "account": null, "accountName": null, "ask": null, "last": null, "direct": false,
        "model": null})
}

fn step(id: u32, state: &str, finished: Option<&str>) -> Value {
    json!({"id": id, "title": format!("Step {id}"), "state": state, "finished": finished,
        "updated": finished.unwrap_or("2026-11-28T08:50:00Z")})
}

fn page() -> TaskPage {
    task_page(
        1,
        json!({
            "runs": [
                run(2, "s-2", json!({"ended": null, "started": "2026-11-28T08:45:00Z",
                    "asked": "Carry on"})),
                run(1, "s-1", json!({
                    "asked": "## Build it\nthen test it",
                    "answer": "\n**Built** the app.\nThe long rest of the answer.",
                    "status": "STATUS: working: the tests next"})),
            ],
            "sessions": [session("s-1")],
            // The server's own words for the plan's changes.
            "history": [
                {"at": "2026-11-28T07:00:00Z", "who": "assistant", "text": "Created"},
                {"at": "2026-11-28T07:20:00Z", "who": "worker", "text": "Plan set: 4 steps"},
                {"at": "2026-11-28T07:40:00Z", "who": "worker", "text": "Skipped: Step 2"},
                {"at": "2026-11-28T08:05:00Z", "who": "worker", "text": "Started: Step 1"},
                {"at": "2026-11-28T08:10:00Z", "who": "worker", "text": "Ticked: Step 1"},
                {"at": "2026-11-28T08:20:00Z", "who": "worker", "text": "Built · STATUS: done"},
                {"at": "2026-11-28T08:50:00Z", "who": "worker",
                    "text": "Blocked: Step 3 — no certificate"},
                {"at": "2026-11-28T08:52:00Z", "who": "worker",
                    "text": "Renamed step: Step 4 → Step four"},
            ],
            "artifacts": [{"id": 4, "kind": "md", "caption": null, "at": "2026-11-28T07:30:00Z",
                "name": "notes.md", "url": "/api/artifacts/4", "thumb": null}],
            "fullPlan": {"steps": [
                step(1, "done", Some("2026-11-28T08:10:00Z")),
                step(2, "skipped", Some("2026-11-28T07:40:00Z")),
                step(3, "blocked", None),
                step(4, "todo", None),
            ], "summary": {"done": 1, "total": 3}},
        }),
    )
}

fn shape(items: &[Item]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            Item::Turn(turn) => format!(
                "turn {} [{}]",
                turn.run,
                turn.receipts
                    .iter()
                    .map(|event| event.text.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Item::Event(event) => format!("{:?} {}", event.kind, event.text),
        })
        .collect()
}

#[test]
fn each_turn_reads_as_what_it_was_asked_and_its_first_line() {
    let items = feed(Some(&page()), &now(), Filter::Turns);
    let Some(Item::Turn(live)) = items.first() else {
        panic!("a turn first")
    };
    assert!(live.live);
    assert_eq!(live.said.as_deref(), Some("Working on it…"));
    assert_eq!(live.session, None, "its session isn't on the page");
    let Some(Item::Turn(done)) = items.get(1) else {
        panic!("then the older turn")
    };
    assert_eq!(done.asked.as_deref(), Some("Build it"));
    assert_eq!(done.said.as_deref(), Some("Built the app."));
    assert_eq!(done.state, Some(RunState::Working));
    assert_eq!(
        done.session,
        Some((SessionId::new("s-1"), "Build on PC".to_owned()))
    );
    assert_eq!(feed(None, &now(), Filter::All), []);
}

#[test]
fn the_feed_shows_each_thing_once_and_its_filters_narrow_it() {
    let page = page();
    assert_eq!(
        shape(&feed(Some(&page), &now(), Filter::All)),
        [
            "Planned Renamed step: Step 4 → Step four",
            "turn 2 [Step 3 — no certificate]",
            "Planned Started: Step 1",
            "turn 1 [Step 1]",
            "Skipped Step 2",
            "File notes.md",
            "Planned Plan set: 4 steps",
            "Note Created",
        ],
        "each tick once, under its turn, though the step is done in the plan too; an older \
         server's turn lines left to the turns"
    );
    assert_eq!(
        shape(&feed(Some(&page), &now(), Filter::Plan)),
        [
            "Planned Renamed step: Step 4 → Step four",
            "Blocked Step 3 — no certificate",
            "Ticked Step 1",
            "Planned Started: Step 1",
            "Skipped Step 2",
            "Planned Plan set: 4 steps",
        ],
        "exactly the server's plan lines"
    );
    assert_eq!(
        shape(&feed(Some(&page), &now(), Filter::Files)),
        ["File notes.md"]
    );
    let items = feed(Some(&page), &now(), Filter::All);
    assert_eq!(sessions_note(&items).as_deref(), Some("1 session"));
}

#[test]
fn the_servers_plan_lines_are_known_by_their_openings() {
    let line = plan_line;
    assert_eq!(
        line("Ticked: Sign it"),
        Some((EventKind::Ticked, "Sign it".to_owned()))
    );
    assert_eq!(
        line("Blocked: Upload — the key expired"),
        Some((EventKind::Blocked, "Upload — the key expired".to_owned()))
    );
    for text in [
        "Plan set: 6 steps, 2 done",
        "Started: Sign it",
        "Reopened: Sign it",
        "Added step: Announce",
        "Renamed step: A → B",
        "Note on Sign it: waits on the key",
        "Plan estimate: about 2 hours",
    ] {
        assert_eq!(line(text), Some((EventKind::Planned, text.to_owned())));
    }
    assert_eq!(line("Started"), None, "a state change, not the plan's");
    assert_eq!(line("Created"), None);
}

#[test]
fn times_and_first_lines_read_short() {
    let at = |text: &str| text.parse::<Timestamp>().unwrap();
    assert_eq!(time_label(at("2026-11-28T08:41:00Z"), &now()), "8:41 AM");
    assert_eq!(time_label(at("2026-11-26T08:41:00Z"), &now()), "Thu");
    assert_eq!(time_label(at("2026-11-02T08:41:00Z"), &now()), "2 Nov");
    assert_eq!(first_line("\n---\n# Title\nmore").as_deref(), Some("Title"));
    assert_eq!(first_line("- `item`").as_deref(), Some("item"));
    assert_eq!(first_line("  \n "), None);
}
