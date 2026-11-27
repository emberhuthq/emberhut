//! Coding threads and their follows against a real SQLite database.

use emberhut_core::thread::{CodingThread, Ending, Followed, ThreadStatus};

use crate::common::{at, database};

fn thread(id: &str, status: ThreadStatus, minutes: i64) -> CodingThread {
    CodingThread::new("runner", id, format!("Thread {id}"), status, at(minutes))
}

fn follow(id: &str, minutes: i64) -> Followed {
    Followed {
        thread: id.to_owned(),
        machine: "runner".to_owned(),
        why: "Fix the build".to_owned(),
        at: at(minutes),
    }
}

#[tokio::test]
async fn a_thread_is_kept_whole_and_replaced_by_its_next_look() {
    let (_dir, db) = database().await;
    let threads = db.threads();
    let mut first = thread("r-1", ThreadStatus::Running, 1);
    first.project = "emberhut".to_owned();
    first.ask = Some("Fix the build".to_owned());
    first.since = Some(at(1));
    first.direct = true;
    first.cwd = Some("/work/emberhut".to_owned());
    let recorded = threads.record(&first).await.unwrap();
    assert_eq!(recorded.previous, None);
    assert_eq!(recorded.followed, None);
    assert_eq!(
        threads.get("runner", "r-1").await.unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        threads.get("other", "r-1").await.unwrap(),
        None,
        "by machine too"
    );

    let mut next = first.clone();
    next.status = ThreadStatus::Other("parked".to_owned());
    next.since = None;
    next.updated_at = at(2);
    let recorded = threads.record(&next).await.unwrap();
    assert_eq!(recorded.previous, Some(first));
    assert_eq!(recorded.ending, None, "not followed");
    assert_eq!(threads.get("runner", "r-1").await.unwrap(), Some(next));

    assert!(threads.remove("runner", "r-1").await.unwrap());
    assert!(!threads.remove("runner", "r-1").await.unwrap());
    assert_eq!(threads.get("runner", "r-1").await.unwrap(), None);
}

#[tokio::test]
async fn recent_threads_are_the_top_level_ones_newest_first() {
    let (_dir, db) = database().await;
    let threads = db.threads();
    for (id, minutes) in [("a", 1), ("b", 3), ("c", 2)] {
        threads
            .record(&thread(id, ThreadStatus::Running, minutes))
            .await
            .unwrap();
    }
    let mut helper = thread("b-helper", ThreadStatus::Running, 9);
    helper.parent = Some("b".to_owned());
    threads.record(&helper).await.unwrap();

    let ids = |list: Vec<CodingThread>| list.into_iter().map(|t| t.id).collect::<Vec<_>>();
    assert_eq!(ids(threads.recent(40).await.unwrap()), ["b", "c", "a"]);
    assert_eq!(ids(threads.recent(2).await.unwrap()), ["b", "c"]);
}

#[tokio::test]
async fn a_followed_thread_s_end_is_news_once_and_ends_its_follow() {
    let (_dir, db) = database().await;
    let threads = db.threads();
    threads.follow(&follow("r-1", 0)).await.unwrap();
    assert_eq!(
        threads.followed().await.unwrap(),
        [(follow("r-1", 0), None)]
    );

    let running = thread("r-1", ThreadStatus::Running, 1);
    let recorded = threads.record(&running).await.unwrap();
    assert_eq!(recorded.followed, Some(follow("r-1", 0)));
    assert_eq!(recorded.ending, None);
    assert_eq!(
        threads.followed().await.unwrap(),
        [(follow("r-1", 0), Some(running))]
    );

    let waiting = thread("r-1", ThreadStatus::Waiting, 2);
    assert_eq!(
        threads.record(&waiting).await.unwrap().ending,
        Some(Ending::Waiting)
    );
    assert_eq!(threads.followed().await.unwrap().len(), 1, "still followed");
    assert_eq!(threads.record(&waiting).await.unwrap().ending, None, "once");

    let done = thread("r-1", ThreadStatus::Completed, 3);
    let recorded = threads.record(&done).await.unwrap();
    assert_eq!(recorded.ending, Some(Ending::Ended));
    assert_eq!(recorded.followed, Some(follow("r-1", 0)));
    assert_eq!(threads.followed().await.unwrap(), []);
    assert_eq!(threads.get("runner", "r-1").await.unwrap(), Some(done));
}

#[tokio::test]
async fn following_again_replaces_the_follow_and_unfollowing_ends_it() {
    let (_dir, db) = database().await;
    let threads = db.threads();
    threads.follow(&follow("r-2", 5)).await.unwrap();
    threads.follow(&follow("r-1", 1)).await.unwrap();
    let ids = |list: Vec<(Followed, Option<CodingThread>)>| {
        list.into_iter().map(|(f, _)| f.thread).collect::<Vec<_>>()
    };
    assert_eq!(ids(threads.followed().await.unwrap()), ["r-1", "r-2"]);

    let mut again = follow("r-1", 9);
    again.why = "Review the fix".to_owned();
    threads.follow(&again).await.unwrap();
    let followed = threads.followed().await.unwrap();
    assert_eq!(ids(followed.clone()), ["r-2", "r-1"]);
    assert_eq!(followed[1].0, again);

    assert!(threads.unfollow("r-1").await.unwrap());
    assert!(!threads.unfollow("r-1").await.unwrap());
    assert_eq!(ids(threads.followed().await.unwrap()), ["r-2"]);
}

#[tokio::test]
async fn a_follow_on_another_machine_is_not_this_thread_s() {
    let (_dir, db) = database().await;
    let threads = db.threads();
    let mut elsewhere = follow("r-1", 0);
    elsewhere.machine = "laptop".to_owned();
    threads.follow(&elsewhere).await.unwrap();
    let recorded = threads
        .record(&thread("r-1", ThreadStatus::Failed, 1))
        .await
        .unwrap();
    assert_eq!((recorded.followed, recorded.ending), (None, None));
    assert_eq!(threads.followed().await.unwrap(), [(elsewhere, None)]);
}

fn on(machine: &str, id: &str, status: ThreadStatus, minutes: i64) -> CodingThread {
    CodingThread::new(machine, id, format!("Thread {id}"), status, at(minutes))
}

#[tokio::test]
async fn active_threads_are_the_top_level_running_queued_or_waiting_ones_of_those_machines() {
    let (_dir, db) = database().await;
    let threads = db.threads();
    let mut helper = on("laptop", "h", ThreadStatus::Running, 9);
    helper.parent = Some("a".to_owned());
    for look in [
        on("laptop", "a", ThreadStatus::Running, 1),
        on("laptop", "b", ThreadStatus::Waiting, 3),
        on("laptop", "c", ThreadStatus::Queued, 2),
        on("laptop", "d", ThreadStatus::Completed, 8),
        on("runner", "e", ThreadStatus::Running, 7),
        helper,
    ] {
        threads.record(&look).await.unwrap();
    }
    let machines = ["laptop".to_owned()];
    let ids = |list: Vec<CodingThread>| list.into_iter().map(|t| t.id).collect::<Vec<_>>();
    assert_eq!(
        ids(threads.active_on(&machines, 30).await.unwrap()),
        ["b", "c", "a"]
    );
    assert_eq!(
        ids(threads.active_on(&machines, 2).await.unwrap()),
        ["b", "c"]
    );
    assert_eq!(threads.active_on(&[], 30).await.unwrap(), Vec::new());

    assert_eq!(
        ids(threads.all(false, None).await.unwrap()),
        ["d", "e", "b", "c", "a"]
    );
    assert_eq!(
        ids(threads
            .all(true, Some(&ThreadStatus::Running))
            .await
            .unwrap()),
        ["h", "e", "a"]
    );
    assert_eq!(
        ids(threads.on_machine("laptop").await.unwrap()),
        ["a", "b", "c", "d", "h"]
    );

    let keep = ["a".to_owned(), "h".to_owned()];
    assert_eq!(threads.forget_others("laptop", &keep).await.unwrap(), 3);
    assert_eq!(ids(threads.on_machine("laptop").await.unwrap()), ["a", "h"]);
    assert_eq!(
        ids(threads.on_machine("runner").await.unwrap()),
        ["e"],
        "another machine's kept"
    );
}
