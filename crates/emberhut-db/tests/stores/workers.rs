//! The workers' store: the `agents` of each task's run, the `streams` read so far, and an
//! ultra task's run, against a real SQLite file.

use emberhut_core::task::{Owner, TaskId, Who};
use emberhut_core::ultra::AgentKind;
use emberhut_db::{AgentSeen, NewTask, StreamPlace};

use crate::common::{at, database};

fn seen(task: TaskId, run: &str, key: &str) -> AgentSeen {
    AgentSeen::new(task, run, key, "worker")
}

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure here fails the test that called it"
)]
async fn task(db: &emberhut_db::Database) -> TaskId {
    let new = NewTask {
        title: "A big job".to_owned(),
        owner: Some(Owner::Assistant),
        ..NewTask::default()
    };
    db.tasks()
        .create(new, Who::Assistant, at(0))
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn agents_are_kept_by_their_run_and_key_and_a_finished_one_stays_finished() {
    let (_dir, db) = database().await;
    let workers = db.workers();
    let id = task(&db).await;
    let start = AgentSeen {
        name: Some("Part 2: **the parser**".to_owned()),
        tool_use: Some("tu-1".to_owned()),
        state: Some("running".to_owned()),
        ..seen(id, "s-1", "a1")
    };
    workers.upsert(start, at(1)).await.unwrap();
    let progress = AgentSeen {
        last: Some("Reading the grammar".to_owned()),
        tokens: Some(1200),
        ..seen(id, "s-1", "a1")
    };
    workers.upsert(progress, at(2)).await.unwrap();
    let all = workers.of_task(id).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].kind, AgentKind::Part);
    assert_eq!(all[0].part, Some(2.0));
    assert_eq!(all[0].name.as_deref(), Some("Part 2: the parser"));
    assert_eq!(all[0].last.as_deref(), Some("Reading the grammar"));
    assert_eq!(workers.running(id, None).await.unwrap(), 1);
    assert!(workers.started_by(id, "tu-1").await.unwrap());
    assert!(workers.known("s-1", "a1").await.unwrap());
    let end = AgentSeen {
        state: Some("done".to_owned()),
        tokens: Some(5000),
        ..seen(id, "s-1", "a1")
    };
    workers.upsert(end, at(3)).await.unwrap();
    let late = AgentSeen {
        state: Some("running".to_owned()),
        ..seen(id, "s-1", "a1")
    };
    workers.upsert(late, at(4)).await.unwrap();
    let all = workers.of_task(id).await.unwrap();
    assert_eq!(all[0].state, "done", "a late line doesn't bring it back");
    assert_eq!(all[0].ended, Some(at(3)));
    assert!((workers.spent(id).await.unwrap() - 5000.0).abs() < 1e-9);
}

#[tokio::test]
async fn a_run_that_ends_stops_its_agents_but_not_its_lanes() {
    let (_dir, db) = database().await;
    let workers = db.workers();
    let id = task(&db).await;
    let part = AgentSeen {
        name: Some("Part 1: x".to_owned()),
        ..seen(id, "s-1", "a1")
    };
    workers.upsert(part, at(1)).await.unwrap();
    let lane = AgentSeen {
        name: Some("Lane 2: y".to_owned()),
        ..seen(id, "s-1", "r-9")
    };
    workers.upsert(lane, at(1)).await.unwrap();
    let flow = AgentSeen {
        name: Some("Part 3: z".to_owned()),
        tokens: Some(700),
        ..seen(id, "s-1", "w1/a3")
    };
    workers.upsert(flow, at(1)).await.unwrap();
    assert_eq!(
        workers.running(id, Some("s-1")).await.unwrap(),
        2,
        "a workflow's agent isn't counted"
    );
    assert!(
        workers.spent(id).await.unwrap().abs() < 1e-9,
        "a workflow's agent's tokens are the workflow's"
    );
    assert_eq!(workers.end_run(id, "s-1", at(5)).await.unwrap(), 2);
    let states: Vec<String> = workers
        .of_task(id)
        .await
        .unwrap()
        .into_iter()
        .map(|a| a.state)
        .collect();
    assert_eq!(states, ["stopped", "running", "stopped"]);
    assert_eq!(workers.tasks_running().await.unwrap(), [id]);
}

#[tokio::test]
async fn a_stream_s_place_and_an_ultra_run_are_kept() {
    let (_dir, db) = database().await;
    let workers = db.workers();
    let id = task(&db).await;
    let place = StreamPlace {
        file: "/work/1/turn-1.jsonl".to_owned(),
        offset: 812,
        task: Some(id),
        doing: Some("Reading main.rs".to_owned()),
        tokens: 3400.0,
    };
    workers.set_place(&place).await.unwrap();
    workers
        .set_place(&StreamPlace {
            offset: 900,
            ..place.clone()
        })
        .await
        .unwrap();
    assert_eq!(
        workers.place(&place.file).await.unwrap().unwrap().offset,
        900
    );
    workers.prune_places(&[]).await.unwrap();
    assert_eq!(workers.places().await.unwrap().len(), 0);
    assert_eq!(workers.run_of(id).await.unwrap(), None);
    workers
        .set_run(id, r#"{"state":"planning"}"#, at(2))
        .await
        .unwrap();
    assert_eq!(
        workers.run_of(id).await.unwrap().as_deref(),
        Some(r#"{"state":"planning"}"#)
    );
    workers.set_turn_file(id, Some(&place.file)).await.unwrap();
    assert_eq!(
        workers.turn_file(id).await.unwrap(),
        Some(place.file.clone())
    );
    db.tasks()
        .set_worker(id, Some("w-1".to_owned()), Some("s-1".to_owned()))
        .await
        .unwrap();
    let working = workers.working().await.unwrap();
    assert_eq!(working.len(), 1);
    assert_eq!(working[0].unit, "w-1");
    assert_eq!(working[0].turn_file.as_deref(), Some(place.file.as_str()));
}
