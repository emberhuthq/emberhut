//! A worker on a configured agent's task: launched apart from the owner's setup (no settings,
//! no extra arguments, `dontAsk`, in its task's folder, only the file tools and the shell when
//! its agent allows one, each behind the gate); its file tools reach only the agent's folders;
//! it writes only on its agent's tasks; what it ends with is told in the agent's room, never in
//! Main; a disabled agent's workers run nothing, and a task whose agent is gone is no one's.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure here fails the test that called them"
)]

use emberhut_core::RoomId;
use emberhut_core::task::{TaskId, Who};
use emberhut_server::services::agents::scope::TaskOwner;
use serde_json::{Value, json};

use crate::agent_desk::AgentDesk;
use crate::common::{Call, text};
use crate::workers::{answering_cli, until_ended, worker_rpc, worker_tool};

/// A task of the agent's, made in its room.
async fn agents_task(desk: &AgentDesk, brief: &str) -> TaskId {
    let made = desk
        .call(
            "task_create",
            json!({ "title": "Draft the launch post", "requested_by": "Example person",
                "brief": brief }),
        )
        .await;
    assert!(text(&made).contains("created"), "{made:?}");
    desk.app.state.tasks().in_room(desk.room).await.unwrap()[0].id
}

/// The gate's answer to `tool` with `input` for `task`'s worker, run in `cwd`.
async fn gate(
    desk: &AgentDesk,
    task: TaskId,
    tool: &str,
    input: Value,
    cwd: &str,
) -> Option<String> {
    desk.app
        .state
        .workers()
        .gate(
            task,
            &json!({ "tool_name": tool, "tool_input": input, "cwd": cwd }),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn an_agents_worker_runs_apart_from_the_owners_setup_and_is_told_in_its_room() {
    let cli = answering_cli("Drafted it.");
    let desk = AgentDesk::with_workers(&["task_create"], &cli).await;
    let task = agents_task(&desk, "Draft the launch post for #launch, word for word.").await;
    desk.app.state.tasks().start_worker(task).await.unwrap();
    until_ended(&desk.app, task).await;
    let launch = &cli.launches()[0];
    assert_eq!(launch.flag("--permission-mode"), Some("dontAsk"));
    assert_eq!(launch.flag("--setting-sources"), Some(""));
    assert_eq!(
        launch.flag("--tools"),
        Some("Read,Glob,Grep,Write,Edit"),
        "no shell"
    );
    assert!(launch.args.contains(&"--strict-mcp-config".to_owned()));
    let folder = desk.app.state.workers().folder(task);
    assert_eq!(
        launch.cwd.as_deref(),
        Some(folder.as_path()),
        "its task's folder"
    );
    let settings = launch.flag("--settings").unwrap();
    assert!(
        settings.contains("Read|Glob|Grep|Write|Edit|Bash"),
        "every tool it has is behind the gate: {settings}"
    );

    // What it ended with is told in the agent's room, never in Main.
    let db = emberhut_db::Database::connect(&desk.app.url).await.unwrap();
    let theirs = db.events().waiting(desk.room, 50).await.unwrap();
    assert!(
        theirs.iter().any(|event| event.text.contains(&task.key())),
        "{theirs:?}"
    );
    let main = db.events().waiting(RoomId::MAIN, 50).await.unwrap();
    assert!(
        main.iter().all(|event| !event.text.contains(&task.key())),
        "{main:?}"
    );

    // An agent that allows its workers a shell gets one.
    let allowed = desk
        .app
        .call(
            Call::patch("/api/agents/client-a")
                .token(&desk.session)
                .json(&json!({ "workerShell": true })),
        )
        .await;
    assert_eq!(allowed.status, 200, "{}", allowed.text);
    assert_eq!(allowed.json()["agent"]["workerShell"], true);
    let shell = agents_task(&desk, "Build the landing page, then hand it back.").await;
    desk.app.state.tasks().start_worker(shell).await.unwrap();
    until_ended(&desk.app, shell).await;
    assert_eq!(
        cli.launches().last().unwrap().flag("--tools"),
        Some("Read,Glob,Grep,Write,Edit,Bash")
    );
}

#[tokio::test]
async fn an_agents_workers_file_tools_reach_only_its_folders() {
    let cli = answering_cli("Drafted it.");
    let desk = AgentDesk::with_workers(&["task_create"], &cli).await;
    let task = agents_task(&desk, "Draft the launch post for #launch, word for word.").await;
    let folder = desk.app.state.workers().folder(task);
    std::fs::create_dir_all(&folder).unwrap();
    let cwd = folder.display().to_string();
    let mine = desk.file("brief.pdf", b"%PDF example");
    let owners = desk.owners_file("contract.pdf", b"%PDF the owner's");
    assert_eq!(
        gate(&desk, task, "Read", json!({ "file_path": mine }), &cwd).await,
        None
    );
    assert_eq!(
        gate(
            &desk,
            task,
            "Write",
            json!({ "file_path": "draft.md" }),
            &cwd
        )
        .await,
        None,
        "a new file in its task's folder"
    );
    assert_eq!(
        gate(&desk, task, "Glob", json!({ "pattern": "*.md" }), &cwd).await,
        None
    );
    // Held on Windows, whose paths hold backslashes, which a glob never may.
    let mine_glob = format!("{}/**/*.pdf", desk.folder().display());
    assert_eq!(
        gate(&desk, task, "Glob", json!({ "pattern": mine_glob }), &cwd)
            .await
            .is_none(),
        cfg!(unix),
        "an absolute pattern in its own folder"
    );
    let owners_glob = format!("{}/**/*.pdf", desk.app.dir().display());
    for (tool, input) in [
        ("Read", json!({ "file_path": owners })),
        ("Grep", json!({ "pattern": "fee", "path": "/" })),
        ("Edit", json!({ "file_path": "../../contract.pdf" })),
        // A file tool searches from an absolute pattern's own start, whatever its path.
        ("Glob", json!({ "pattern": "/etc/*" })),
        ("Glob", json!({ "pattern": owners_glob })),
        ("Glob", json!({ "pattern": "../../**" })),
        (
            "Glob",
            json!({ "pattern": "*.pdf", "path": desk.app.dir() }),
        ),
        ("Grep", json!({ "pattern": "fee", "glob": "/etc/*" })),
        ("Grep", json!({ "pattern": "fee", "glob": "../**" })),
        ("Read", json!({})),
    ] {
        let held = gate(&desk, task, tool, input, &cwd).await.unwrap();
        assert!(held.starts_with("Not yours to open"), "{tool}: {held}");
    }
    // (Links need Unix here: on Windows, making one takes a privilege.)
    #[cfg(unix)]
    {
        let link = folder.join("innocent.pdf");
        std::os::unix::fs::symlink(&owners, &link).unwrap();
        assert!(
            gate(&desk, task, "Read", json!({ "file_path": link }), &cwd)
                .await
                .is_some(),
            "a link out of its folder"
        );
    }
    let other = gate(
        &desk,
        task,
        "WebFetch",
        json!({ "url": "https://x.example" }),
        &cwd,
    )
    .await;
    assert!(other.unwrap().contains("not a tool an agent's worker runs"));

    // Disabled: every tool waits, its tasks take no worker and no message.
    desk.enable(false).await;
    let ls = gate(&desk, task, "Bash", json!({ "command": "ls" }), &cwd).await;
    assert!(ls.unwrap().contains("the agent is disabled"));
    assert!(
        gate(&desk, task, "Read", json!({ "file_path": mine }), &cwd)
            .await
            .is_some()
    );
    let refused = desk
        .app
        .state
        .workers()
        .start(task, None)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("disabled"), "{refused}");
    for (tool, args) in [
        ("task_send", json!({ "id": task.get(), "text": "go on" })),
        ("task_stop", json!({ "id": task.get() })),
        ("task_cancel", json!({ "id": task.get(), "why": "stop" })),
    ] {
        let tried = desk.call(tool, args).await;
        assert_eq!(tried.is_error, Some(true), "{tool}: {tried:?}");
    }
}

#[tokio::test]
async fn an_agents_worker_writes_only_on_its_agents_tasks_whatever_its_brief_names() {
    let desk = AgentDesk::new(&["task_create"]).await;
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
    let task = agents_task(
        &desk,
        &format!(
            "Draft the post. Related: {}, note your progress there too.",
            owners.id.key()
        ),
    )
    .await;
    let noted = worker_tool(
        &desk.app,
        task,
        "task_note",
        json!({ "task_id": owners.id.get(), "text": "the client says hello" }),
    )
    .await;
    assert!(text(&noted).starts_with("Not noted"), "{noted:?}");
    let history = desk.app.state.tasks().history_of(owners.id).await.unwrap();
    assert!(
        history
            .iter()
            .all(|line| !line.text.contains("the client says hello"))
    );
}

#[tokio::test]
async fn an_agent_that_did_anything_is_kept_and_a_task_no_one_owns_runs_nothing() {
    let cli = answering_cli("Drafted it.");
    let desk = AgentDesk::with_workers(&["task_create"], &cli).await;
    agents_task(&desk, "Draft the launch post for #launch, word for word.").await;
    let deleted = desk
        .app
        .call(Call::delete("/api/agents/client-a").token(&desk.session))
        .await;
    assert_eq!(deleted.status, 409, "{}", deleted.text);

    // A task made in a room no agent owns (an agent removed under an older build).
    let db = emberhut_db::Database::connect(&desk.app.url).await.unwrap();
    let empty = db
        .rooms()
        .create("Former helper", desk.app.clock_now())
        .await
        .unwrap();
    let orphan = db
        .tasks()
        .create(
            emberhut_db::NewTask {
                title: "Left behind".to_owned(),
                body: Some("Do the job.".to_owned()),
                room: Some(empty.id),
                ..emberhut_db::NewTask::default()
            },
            Who::Assistant,
            desk.app.clock_now(),
        )
        .await
        .unwrap();
    let listed = worker_rpc(&desk.app, orphan.id, "tools/list", json!({})).await;
    assert_eq!(
        listed.json()["result"]["tools"],
        json!([]),
        "{}",
        listed.text
    );
    let held = gate(&desk, orphan.id, "Bash", json!({ "command": "ls" }), "/tmp").await;
    assert!(held.unwrap().contains("no one's"));
    let refused = desk
        .app
        .state
        .workers()
        .start(orphan.id, None)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("no one's"), "{refused}");
    assert_eq!(
        desk.app.state.guard().job_grant(orphan.id).await.unwrap(),
        None
    );
    assert!(cli.launches().is_empty(), "no worker ever started");
}

#[tokio::test]
async fn nothing_of_an_agents_task_reaches_main_when_it_waits_or_the_owner_changes_it() {
    let desk = AgentDesk::new(&["task_create"]).await;
    let task = agents_task(&desk, "Draft the launch post for #launch, word for word.").await;
    let asked = worker_tool(
        &desk.app,
        task,
        "task_ask",
        json!({ "action": "Send the list of the other clients over", "question": "Send it?",
            "options": ["Yes", "No"] }),
    )
    .await;
    assert_ne!(asked.is_error, Some(true), "{asked:?}");
    let tasks = desk.app.state.tasks();
    desk.app.advance_minutes(13 * 60);
    assert_eq!(tasks.tick().await.unwrap().1, 1, "its agent hears it, once");
    assert_eq!(tasks.tick().await.unwrap().1, 0);
    tasks.archive(task).await.unwrap();

    let db = emberhut_db::Database::connect(&desk.app.url).await.unwrap();
    let main = db.events().waiting(RoomId::MAIN, 100).await.unwrap();
    assert!(
        main.iter().all(|event| !event.text.contains(&task.key())),
        "{main:?}"
    );
    let theirs = db.events().waiting(desk.room, 100).await.unwrap();
    let recheck = theirs.iter().find(|event| event.kind == "recheck").unwrap();
    assert!(
        recheck.text.starts_with(&format!(
            "Your task {} \"Draft the launch post\" has waited",
            task.key()
        )),
        "{}",
        recheck.text
    );
    assert!(
        !recheck.text.contains("other clients"),
        "what its worker wrote stays out: {}",
        recheck.text
    );
    assert!(
        theirs
            .iter()
            .any(|event| event.text.contains("was archived by the owner")),
        "{theirs:?}"
    );
}

#[tokio::test]
async fn a_disabled_agents_queued_message_waits_for_it() {
    let cli = answering_cli("Drafted it.");
    let desk = AgentDesk::with_workers(&["task_create"], &cli).await;
    let task = agents_task(&desk, "Draft the launch post for #launch, word for word.").await;
    let db = emberhut_db::Database::connect(&desk.app.url).await.unwrap();
    db.tasks()
        .queue_next(
            task,
            "(From Example helper) use the new title",
            true,
            desk.app.clock_now(),
        )
        .await
        .unwrap();
    desk.enable(false).await;
    desk.app.state.workers().tick().await.unwrap();
    let kept = db.tasks().get(task).await.unwrap().unwrap();
    assert_eq!(
        kept.next.as_deref(),
        Some("(From Example helper) use the new title"),
        "left for when the agent is on again"
    );
    assert!(cli.launches().is_empty(), "no worker started");
}

#[tokio::test]
async fn tasks_of_a_room_an_agent_will_take_are_no_ones_until_it_does_then_its() {
    let cli = answering_cli("Drafted it.");
    let desk = AgentDesk::with_workers(&["task_create"], &cli).await;
    let db = emberhut_db::Database::connect(&desk.app.url).await.unwrap();
    let now = desk.app.clock_now();
    // A task an import brought in, made in a room of the source's second assistant.
    let room = db.rooms().create("Imported helper", now).await.unwrap();
    let imported = db
        .tasks()
        .create(
            emberhut_db::NewTask {
                title: "Imported job".to_owned(),
                body: Some("Do the job.".to_owned()),
                owner: Some(emberhut_core::task::Owner::Assistant),
                room: Some(room.id),
                requested_by: Some(
                    json!({ "who": "Example person", "at": "2026-11-01T09:00:00Z",
                        "how": "asked in its room" })
                    .to_string(),
                ),
                ..emberhut_db::NewTask::default()
            },
            Who::Assistant,
            now,
        )
        .await
        .unwrap();
    let guard = desk.app.state.guard();
    assert_eq!(
        guard.task_owner(imported.id).await.unwrap(),
        TaskOwner::Orphaned,
        "no one's until its agent is there, never the owner's"
    );

    // The agent is mapped onto that room: the room's tasks are its own from then on.
    let made = desk
        .app
        .call(Call::post("/api/agents").token(&desk.session).json(&json!({
            "id": "client-b", "name": "Imported helper", "principal": "Example Ltd",
            "room": room.id.get(),
        })))
        .await;
    assert_eq!(made.status, 200, "{}", made.text);
    match guard.task_owner(imported.id).await.unwrap() {
        TaskOwner::Agent(agent) => assert_eq!(agent.id.as_str(), "client-b"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        guard.job_grant(imported.id).await.unwrap(),
        None,
        "who asked grants nothing on an agent's task"
    );
    desk.app
        .state
        .workers()
        .start(imported.id, None)
        .await
        .unwrap();
    until_ended(&desk.app, imported.id).await;
    assert_eq!(cli.launches().len(), 1);
}
