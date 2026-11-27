//! Findings: a scan's or a review's results filed under it with `task_findings`, each landed
//! or not, and what its area shows of them.
#![expect(
    clippy::too_many_lines,
    reason = "each test walks one story through the routes from start to end"
)]

use emberhut_core::task::TaskId;
use emberhut_core::task::wire::KEY_PREFIX;
use serde_json::{Value, json};

use crate::common::{TestApp, text};

async fn say(app: &TestApp, name: &str, arguments: Value) -> String {
    let result = app.call_tool(name, arguments).await;
    assert!(result.is_error != Some(true), "{name}: {}", text(&result));
    text(&result).to_owned()
}

#[tokio::test]
async fn a_scans_findings_are_filed_updated_and_counted_until_they_land() {
    let app = TestApp::new().await;
    say(&app, "project_set", json!({ "name": "Harness" })).await;
    say(
        &app,
        "area_set",
        json!({ "project_id": 1, "name": "Audit" }),
    )
    .await;
    say(
        &app,
        "task_create",
        json!({ "title": "Audit the cache", "owner": "assistant", "area_id": 1 }),
    )
    .await;
    say(
        &app,
        "task_create",
        json!({ "title": "Fix the leak", "owner": "assistant", "area_id": 1 }),
    )
    .await;
    let filed = say(
        &app,
        "task_findings",
        json!({ "scan_id": 1, "items": [
            { "title": "Leak in the cache", "landed": "not_landed", "fixed_by": 2, "body": "src/cache.rs" },
            { "title": "Stale index" },
            { "title": "Old flag", "landed": "landed" }
        ] }),
    )
    .await;
    assert_eq!(
        filed,
        format!(
            "{KEY_PREFIX}3 filed (not landed); {KEY_PREFIX}4 filed (unchecked); {KEY_PREFIX}5 filed (landed). {KEY_PREFIX}1: 3 findings, 2 not landed."
        )
    );
    let again = say(
        &app,
        "task_findings",
        json!({ "scan_id": 1, "items": [
            { "title": "leak in  the CACHE", "landed": "partly" },
            { "id": 4, "duplicate_of": 3 },
            { "title": "Old flag", "landed": "landed" }
        ] }),
    )
    .await;
    assert_eq!(
        again,
        format!(
            "{KEY_PREFIX}3 partly landed; {KEY_PREFIX}4 put away (a duplicate of {KEY_PREFIX}3). {KEY_PREFIX}1: 3 findings, 1 not landed."
        )
    );
    let finding = app
        .state
        .tasks()
        .issue(TaskId::new(3))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            finding.kind.as_deref(),
            finding
                .landed
                .as_ref()
                .map(emberhut_api::tasks::Landed::as_str)
        ),
        (Some("finding"), Some("partly"))
    );
    assert_eq!(finding.fixed_by.unwrap().get(), 2);
    assert_eq!(finding.status.as_str(), "todo", "partly landed stays open");

    let state = app
        .state
        .state()
        .state(emberhut_core::RoomId::MAIN)
        .await
        .unwrap();
    let area = &state.areas[0];
    assert_eq!(
        area.findings.total, 2,
        "the duplicate is put away, out of the counts"
    );
    assert_eq!((area.findings.partly, area.findings.landed), (1, 1));
    let open = &area.findings.unlanded[0];
    assert_eq!(
        (open.scan.get(), open.scan_title.as_str()),
        (1, "Audit the cache")
    );
    assert_eq!(open.fixed_by_title.as_deref(), Some("Fix the leak"));
    assert_eq!(area.tree[0].findings.unwrap().open, 1);
    assert!(
        state.issues.iter().all(|t| t.kind.is_none()),
        "findings never go on the board"
    );
    let outline = say(&app, "tasks", json!({ "area": "1" })).await;
    assert!(
        outline.contains("Findings: 2 (1 landed, 1 partly landed)"),
        "{outline}"
    );
    assert!(
        outline.contains(&format!("partly landed: {KEY_PREFIX}3 Leak in the cache (from {KEY_PREFIX}1, fix {KEY_PREFIX}2 todo)")),
        "{outline}"
    );
    let refusal = app
        .call_tool(
            "task_findings",
            json!({ "scan_id": 3, "items": [{ "title": "x" }] }),
        )
        .await;
    assert_eq!(refusal.is_error, Some(true));
    assert_eq!(
        text(&refusal),
        format!("{KEY_PREFIX}3 is a finding: file findings under their scan task")
    );
}
