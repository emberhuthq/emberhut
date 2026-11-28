//! The agent runtime: a CLI session per room behind `POST /api/talk`, with a scripted CLI in
//! place of the real one. A turn's message goes to the room's session and its result comes
//! back to the chat; the session is kept and resumed; the stop button interrupts it; a
//! session that dies fails its turn and comes back on the same conversation.

use emberhut_api::mcp::SERVER_NAME;
use emberhut_api::paths;
use emberhut_api::settings::SettingsReply;
use emberhut_api::state::State;
use emberhut_core::RoomId;
use emberhut_server::services::claude::MCP_TOKEN_VAR;
use serde_json::{Value, json};

use crate::common::{Call, TestApp};
use crate::fake_cli::{
    EXIT, FakeCli, agent, config, init, is_request, is_user, result, step, until,
};

/// A CLI that answers every message with `Echo: <text>`, after a step; its session is `s-1`,
/// or the one it resumes.
fn echoing() -> std::sync::Arc<FakeCli> {
    FakeCli::new(|launch, line| {
        if !is_user(line) {
            return Vec::new();
        }
        let session = launch.flag("--resume").unwrap_or("s-1").to_owned();
        let said = line["message"]["content"].as_str().unwrap_or_default();
        vec![
            init(&session),
            step("Let me look. Then more.", "Bash"),
            result(&format!("Echo: {said}")),
        ]
    })
}

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: a failure fails the test"
)]
async fn chat(app: &TestApp, room: RoomId) -> Vec<(bool, bool, String)> {
    app.state
        .state()
        .state(room)
        .await
        .unwrap()
        .chat
        .into_iter()
        .map(|line| (line.mine, line.event, line.text))
        .collect()
}

#[tokio::test]
async fn a_message_is_answered_by_the_room_s_cli_session() {
    let cli = echoing();
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    let session = app.sign_in().await;
    let said = app
        .call(
            Call::post(paths::TALK)
                .token(&session)
                .json(&json!({ "text": "Hello" })),
        )
        .await;
    assert_eq!(said.status, 200);
    app.settle().await;
    assert_eq!(
        chat(&app, RoomId::MAIN).await,
        [
            (true, false, "Hello".to_owned()),
            (false, false, "Echo: Hello".to_owned())
        ]
    );

    // How the CLI was started: stream-json both ways, the model, the permission mode, the
    // configured extra arguments and directory, and the server's MCP endpoint with Main's
    // own agent token.
    let launches = cli.launches();
    assert_eq!(launches.len(), 1);
    let launch = &launches[0];
    assert_eq!(launch.program.to_str(), Some("agent-cli"));
    assert_eq!(launch.flag("--output-format"), Some("stream-json"));
    assert_eq!(launch.flag("--input-format"), Some("stream-json"));
    assert_eq!(launch.flag("--model"), Some("a-model"));
    assert_eq!(launch.flag("--permission-mode"), Some("dontAsk"));
    assert_eq!(
        launch.flag("--allowedTools"),
        Some(format!("mcp__{SERVER_NAME}__*").as_str()),
        "only the server's own tools run without asking"
    );
    assert_eq!(launch.flag("--resume"), None, "a new conversation");
    assert!(launch.args.contains(&"--add-dir=/work".to_owned()));
    assert_eq!(
        launch.cwd.as_deref().and_then(|d| d.to_str()),
        Some("/work")
    );
    assert_eq!(launch.var("MCP_TOOL_TIMEOUT"), Some("120000"));
    assert!(launch.env_remove.contains(&"CLAUDECODE".to_owned()));
    // The token is in the CLI's environment, which the configuration names: never on its
    // command line, where any local user could read it.
    let mcp: Value = serde_json::from_str(launch.flag("--mcp-config").unwrap()).unwrap();
    let token = app.state.agent_tokens().for_room(RoomId::MAIN).unwrap();
    assert_eq!(
        mcp["mcpServers"][SERVER_NAME],
        json!({ "type": "http", "url": format!("http://127.0.0.1:18990{}", paths::MCP_ROOM),
            "headers": { "Authorization": "Bearer ${EMBERHUT_MCP_TOKEN}" } })
    );
    assert_eq!(launch.var(MCP_TOKEN_VAR), Some(token.as_str()));
    assert!(
        launch.args.iter().all(|arg| !arg.contains(token.as_str())),
        "the token is not among the arguments"
    );
    assert_eq!(
        app.state.agent_tokens().room_of(token.as_str()),
        Some(RoomId::MAIN)
    );

    // The first line is the initialize request, with the server's rules, the room and the
    // configured prompt after the CLI's own system prompt.
    let lines = cli.lines();
    assert!(is_request(&lines[0].1, "initialize"));
    let append = lines[0].1["request"]["appendSystemPrompt"]
        .as_str()
        .unwrap();
    assert!(append.contains("Event:"), "{append}");
    assert!(append.contains("NO_REPLY"), "{append}");
    assert!(append.contains("Rooms: you are in Main"), "{append}");
    assert!(append.contains("Answer in one line."), "{append}");
    // The built-in memory rules, then what the assistant always knows about the owner.
    assert!(
        append.contains("Memory. What you know about the owner"),
        "{append}"
    );
    assert!(append.contains("search it before you ask"), "{append}");
    assert!(
        append.ends_with(
            "About the user: nothing kept yet. Keep what will matter later with fact_set."
        ),
        "{append}"
    );
    assert_eq!(cli.said(), ["Hello"]);
}

#[tokio::test]
async fn bypass_permissions_is_an_opt_in_that_still_allows_the_server_s_tools() {
    let cli = echoing();
    let mut settings = config(&[]);
    settings.permission_mode = Some("bypassPermissions".to_owned());
    let app = TestApp::with_agent(agent(&cli, settings)).await;
    app.state
        .talk()
        .talk(RoomId::MAIN, "Hello".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let launch = &cli.launches()[0];
    assert_eq!(launch.flag("--permission-mode"), Some("bypassPermissions"));
    assert_eq!(
        launch.flag("--allowedTools"),
        Some(format!("mcp__{SERVER_NAME}__*").as_str())
    );

    // No mode at all: the CLI's own default, the server's tools still allowed.
    let cli = echoing();
    let mut settings = config(&[]);
    settings.permission_mode = None;
    let app = TestApp::with_agent(agent(&cli, settings)).await;
    app.state
        .talk()
        .talk(RoomId::MAIN, "Hello".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let launch = &cli.launches()[0];
    assert_eq!(launch.flag("--permission-mode"), None);
    assert!(launch.flag("--allowedTools").is_some());
}

#[tokio::test]
async fn the_session_is_kept_between_turns_and_resumed_after_a_restart() {
    let cli = echoing();
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    let talk = app.state.talk();
    talk.talk(RoomId::MAIN, "one".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    talk.talk(RoomId::MAIN, "two".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(cli.launches().len(), 1, "one session for both turns");
    assert_eq!(cli.said(), ["one", "two"]);

    // A new runtime over the same database (a restart) resumes the kept conversation.
    let again = echoing();
    let restarted = emberhut_server::services::claude::ClaudeAssistant::new(
        config(&[]),
        again.clone(),
        emberhut_db::Database::connect(&app.url)
            .await
            .unwrap()
            .settings(),
        emberhut_db::Database::connect(&app.url)
            .await
            .unwrap()
            .rooms(),
        std::sync::Arc::new(emberhut_core::SystemClock),
    );
    let live_turn = emberhut_server::services::talk::Turn {
        room: RoomId::MAIN,
        said: emberhut_core::message::Message {
            id: emberhut_core::MessageId::new(1),
            room: RoomId::MAIN,
            author: emberhut_core::message::Author::User,
            text: "three".to_owned(),
            created_at: app.clock_now(),
            reply_to: None,
        },
        history: Vec::new(),
        files: Vec::new(),
        stopping: tokio_util::sync::CancellationToken::new(),
        prompt: "three".to_owned(),
        from_owner: true,
        interrupt: tokio_util::sync::CancellationToken::new(),
        agent_token: None,
    };
    let answer = emberhut_server::services::talk::Assistant::respond(
        &restarted,
        live_turn,
        emberhut_server::services::talk::LiveSink::new(app.state.live().clone(), RoomId::MAIN),
    )
    .await
    .unwrap();
    assert_eq!(answer.as_deref(), Some("Echo: three"));
    assert_eq!(again.launches()[0].flag("--resume"), Some("s-1"));
}

#[tokio::test]
async fn the_stop_button_interrupts_the_turn_and_no_reply_comes() {
    let cli = FakeCli::new(|_, line| {
        if is_user(line) {
            // A turn that goes on until it is interrupted.
            vec![init("s-1"), step("Working on it.", "Bash")]
        } else if is_request(line, "interrupt") {
            vec![result("[Request interrupted by user]")]
        } else {
            Vec::new()
        }
    });
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    let session = app.sign_in().await;
    app.state
        .talk()
        .talk(RoomId::MAIN, "Do the long thing".to_owned(), None)
        .await
        .unwrap();
    until(|| cli.said().len() == 1).await;
    until(|| {
        app.state
            .live()
            .now(RoomId::MAIN)
            .and_then(|live| live.doing)
            .as_deref()
            == Some("Working on it.")
    })
    .await;
    let reply: SettingsReply = app
        .call(
            Call::post(paths::SETTINGS)
                .token(&session)
                .json(&json!({ "interrupt": true })),
        )
        .await
        .read_as();
    assert!(reply.ok);
    app.settle().await;
    assert!(
        cli.lines()
            .iter()
            .any(|(_, line)| is_request(line, "interrupt")),
        "the CLI was asked to stop"
    );
    assert_eq!(
        chat(&app, RoomId::MAIN).await,
        [
            (true, false, "Do the long thing".to_owned()),
            (false, true, "Stopped.".to_owned())
        ]
    );
    let state: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert!(!state.busy);
    assert!(
        state
            .did
            .iter()
            .any(|line| line.text == "Stopped by the owner")
    );
}

#[tokio::test]
async fn a_session_that_dies_fails_its_turn_and_comes_back_on_the_same_conversation() {
    let cli = FakeCli::new(|launch, line| {
        if !is_user(line) {
            return Vec::new();
        }
        if launch.flag("--resume").is_none() {
            vec![init("s-9"), EXIT.to_owned()]
        } else {
            vec![init("s-9"), result("Back again")]
        }
    });
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    let talk = app.state.talk();
    talk.talk(RoomId::MAIN, "first".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    talk.talk(RoomId::MAIN, "second".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(
        chat(&app, RoomId::MAIN).await,
        [
            (true, false, "first".to_owned()),
            (
                false,
                true,
                "Something went wrong while answering. Try again.".to_owned()
            ),
            (true, false, "second".to_owned()),
            (false, false, "Back again".to_owned())
        ]
    );
    let launches = cli.launches();
    assert_eq!(launches.len(), 2);
    assert_eq!(launches[1].flag("--resume"), Some("s-9"));
}

#[tokio::test]
async fn no_reply_puts_nothing_in_the_chat_and_an_event_reaches_the_session() {
    let cli = FakeCli::new(|_, line| {
        if !is_user(line) {
            return Vec::new();
        }
        let said = line["message"]["content"].as_str().unwrap_or_default();
        let answer = if said.starts_with("Event:") {
            "NO_REPLY"
        } else {
            "Hi"
        };
        vec![init("s-1"), result(answer)]
    });
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    app.state.routines().set(Some(&[]), None).await.unwrap();
    let session = app.sign_in().await;
    let told = app
        .call(
            Call::post("/api/events")
                .token(&session)
                .json(&json!({ "text": "The nightly job finished", "kind": "job" })),
        )
        .await;
    assert_eq!(told.status, 200);
    // Not urgent: it gathers for 90 s before it wakes the room.
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert!(cli.said().is_empty(), "none");
    app.advance_minutes(2);
    app.state.wake().tick().await.unwrap();
    app.settle().await;
    assert_eq!(cli.said(), ["Event: - [job] The nightly job finished"]);
    let state: State = app
        .call(Call::get(paths::STATE).token(&session))
        .await
        .read_as();
    assert_eq!(state.pending, 0, "the event was read");
    let texts: Vec<&str> = state.chat.iter().map(|line| line.text.as_str()).collect();
    assert_eq!(texts, ["- [job] The nightly job finished"]);
    assert!(state.chat[0].event);
    assert!(
        state
            .did
            .iter()
            .any(|line| line.text == "Checked · nothing for the owner")
    );
}

#[tokio::test]
async fn sessions_open_at_once_are_capped_and_main_s_stays() {
    let cli = echoing();
    let mut settings = config(&[]);
    settings.max_sessions = 2;
    let app = TestApp::with_agent(agent(&cli, settings)).await;
    let rooms = app.state.rooms();
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let first = db.rooms().create("Atlas", app.clock_now()).await.unwrap();
    let second = db.rooms().create("Birch", app.clock_now()).await.unwrap();
    assert_eq!(
        rooms.find("atlas").await.unwrap().map(|r| r.id),
        Some(first.id)
    );
    let talk = app.state.talk();
    for room in [RoomId::MAIN, first.id, second.id] {
        talk.talk(room, "hi".to_owned(), None).await.unwrap();
        app.settle().await;
    }
    assert_eq!(cli.launches().len(), 3);
    // Atlas's session gave way to Birch's; Main's stayed.
    talk.talk(RoomId::MAIN, "again".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(cli.launches().len(), 3, "Main's session was still open");
    talk.talk(first.id, "again".to_owned(), None).await.unwrap();
    app.settle().await;
    assert_eq!(cli.launches().len(), 4, "Atlas's session started again");
    let append = cli.lines().into_iter().find_map(|(index, line)| {
        (index == 1 && is_request(&line, "initialize")).then(|| {
            line["request"]["appendSystemPrompt"]
                .as_str()
                .unwrap()
                .to_owned()
        })
    });
    assert!(append.unwrap().contains("Rooms: you are in Atlas."));
}

#[tokio::test]
async fn a_session_in_a_turn_counts_toward_the_cap() {
    // Main's turn goes on until it is stopped; the others answer at once.
    let cli = FakeCli::new(|_, line| {
        if is_user(line) {
            if line["message"]["content"] == "hold" {
                vec![init("s-1"), step("Working on it.", "Bash")]
            } else {
                vec![init("s-2"), result("Hi")]
            }
        } else if is_request(line, "interrupt") {
            vec![result("")]
        } else {
            Vec::new()
        }
    });
    let mut settings = config(&[]);
    settings.max_sessions = 2;
    let app = TestApp::with_agent(agent(&cli, settings)).await;
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let first = db.rooms().create("Atlas", app.clock_now()).await.unwrap();
    let second = db.rooms().create("Birch", app.clock_now()).await.unwrap();
    let talk = app.state.talk();
    talk.talk(RoomId::MAIN, "hold".to_owned(), None)
        .await
        .unwrap();
    until(|| cli.said().len() == 1).await;
    for room in [first.id, second.id] {
        talk.talk(room, "hi".to_owned(), None).await.unwrap();
        until(|| !talk.busy(room)).await;
    }
    assert_eq!(cli.launches().len(), 3);
    // Main's session, in its turn, and Atlas's made two: Birch's closed Atlas's.
    assert_eq!(cli.stopped(), [1]);
    app.state.stopping().cancel();
    assert!(talk.finish(std::time::Duration::from_secs(5)).await);
}

#[tokio::test]
async fn a_room_s_idle_session_closes_at_the_tick() {
    let cli = echoing();
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    app.state.routines().set(Some(&[]), None).await.unwrap();
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let room = db.rooms().create("Atlas", app.clock_now()).await.unwrap();
    let talk = app.state.talk();
    for room in [RoomId::MAIN, room.id] {
        talk.talk(room, "hi".to_owned(), None).await.unwrap();
        app.settle().await;
    }
    app.advance_minutes(29);
    app.state.wake().tick().await.unwrap();
    assert!(cli.stopped().is_empty(), "not idle long enough");
    app.advance_minutes(2);
    app.state.wake().tick().await.unwrap();
    assert_eq!(cli.stopped(), [1], "Atlas's closed, Main's stays");
}

#[tokio::test]
async fn a_kept_session_that_died_between_turns_is_started_again_for_the_turn() {
    // The first CLI answers once, then is gone; the next one answers.
    let cli = FakeCli::new(|launch, line| {
        if !is_user(line) {
            return Vec::new();
        }
        let said = line["message"]["content"].as_str().unwrap_or_default();
        if launch.flag("--resume").is_none() && said == "second" {
            return vec![EXIT.to_owned()];
        }
        vec![init("s-4"), result(&format!("Echo: {said}"))]
    });
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    let talk = app.state.talk();
    talk.talk(RoomId::MAIN, "first".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    talk.talk(RoomId::MAIN, "second".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(
        chat(&app, RoomId::MAIN).await,
        [
            (true, false, "first".to_owned()),
            (false, false, "Echo: first".to_owned()),
            (true, false, "second".to_owned()),
            (false, false, "Echo: second".to_owned())
        ]
    );
    let launches = cli.launches();
    assert_eq!(launches.len(), 2);
    assert_eq!(launches[1].flag("--resume"), Some("s-4"));
}

#[tokio::test]
async fn a_kept_session_that_dies_after_a_tool_ran_fails_its_turn_without_sending_it_again() {
    // The first CLI answers once; at the second message it runs a tool, then is gone.
    let cli = FakeCli::new(|launch, line| {
        if !is_user(line) {
            return Vec::new();
        }
        let said = line["message"]["content"].as_str().unwrap_or_default();
        if launch.flag("--resume").is_none() && said == "second" {
            return vec![step("Deleting the branch.", "Bash"), EXIT.to_owned()];
        }
        vec![init("s-5"), result(&format!("Echo: {said}"))]
    });
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    let talk = app.state.talk();
    talk.talk(RoomId::MAIN, "first".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    talk.talk(RoomId::MAIN, "second".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(
        chat(&app, RoomId::MAIN).await,
        [
            (true, false, "first".to_owned()),
            (false, false, "Echo: first".to_owned()),
            (true, false, "second".to_owned()),
            (
                false,
                true,
                "Something went wrong while answering. Try again.".to_owned()
            )
        ]
    );
    assert_eq!(cli.launches().len(), 1, "not started again for the turn");
    assert_eq!(cli.said(), ["first", "second"], "the message went once");
}

#[tokio::test]
async fn shutting_down_stops_the_running_turn() {
    let cli = FakeCli::new(|_, line| {
        if is_user(line) {
            vec![init("s-1"), step("Working on it.", "Bash")]
        } else if is_request(line, "interrupt") {
            vec![result("")]
        } else {
            Vec::new()
        }
    });
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    app.state
        .talk()
        .talk(RoomId::MAIN, "Long job".to_owned(), None)
        .await
        .unwrap();
    until(|| cli.said().len() == 1).await;
    app.state.stopping().cancel();
    assert!(
        app.state
            .talk()
            .finish(std::time::Duration::from_secs(5))
            .await,
        "the turn ended within the grace"
    );
    assert!(
        cli.lines()
            .iter()
            .any(|(_, line)| is_request(line, "interrupt"))
    );
}

#[tokio::test]
async fn the_session_s_background_tasks_are_the_state_s_tasks() {
    let cli = FakeCli::new(|_, line| {
        if !is_user(line) {
            return Vec::new();
        }
        let said = line["message"]["content"].as_str().unwrap_or_default();
        if said.contains("start") {
            vec![
                init("s-1"),
                json!({ "type": "system", "subtype": "task_started", "task_id": "bg-1" })
                    .to_string(),
                json!({
                    "type": "system", "subtype": "background_tasks_changed",
                    "tasks": [
                        { "task_id": "bg-1", "description": "Build the site", "task_type": "local_bash" },
                        { "task_id": "bg-0", "description": "Watch", "task_type": "local_bash", "ambient": true },
                        { "task_id": "bg-2", "description": "A sub-agent", "task_type": "local_agent", "parent_task_id": "bg-1" }
                    ]
                })
                .to_string(),
                result("Started it."),
            ]
        } else {
            vec![
                json!({
                    "type": "system", "subtype": "task_notification", "task_id": "bg-1",
                    "status": "completed", "summary": "The site is built."
                })
                .to_string(),
                result("Done."),
            ]
        }
    });
    let app = TestApp::with_agent(agent(&cli, config(&[]))).await;
    let session = app.sign_in().await;
    let talk = |text: &'static str| {
        let session = session.clone();
        let app = &app;
        async move {
            app.call(
                Call::post(paths::TALK)
                    .token(&session)
                    .json(&json!({ "text": text })),
            )
            .await;
            app.settle().await;
            let state: State = app
                .call(Call::get(paths::STATE).token(&session))
                .await
                .read_as();
            state.tasks
        }
    };

    let running = talk("start the build").await;
    assert_eq!(running.len(), 1, "ambient tasks and sub-tasks are left out");
    assert_eq!(running[0].id, "bg-1");
    assert_eq!(running[0].description, "Build the site");
    assert_eq!(running[0].kind, "local_bash");
    assert!(running[0].is_running());
    assert_eq!(
        running[0].room,
        Some(emberhut_api::ids::RoomId::new(RoomId::MAIN.get()))
    );

    let ended = talk("how is it going").await;
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].status.as_str(), "completed");
    assert_eq!(ended[0].summary.as_deref(), Some("The site is built."));
    assert!(ended[0].ended_at.is_some());
    let did: Vec<String> = app
        .state
        .state()
        .state(RoomId::MAIN)
        .await
        .unwrap()
        .did
        .into_iter()
        .map(|d| d.text)
        .collect();
    assert!(
        did.contains(&"Finished: Build the site".to_owned()),
        "{did:?}"
    );
}

#[tokio::test]
async fn a_background_task_is_stopped_through_its_room_s_session() {
    use std::sync::{Arc, Mutex};

    use emberhut_server::services::claude::ClaudeAssistant;
    use emberhut_server::services::workers_view::AssistantTasks;

    let cli = FakeCli::new(|_, line| {
        if !is_user(line) {
            return Vec::new();
        }
        vec![
            init("s-1"),
            json!({
                "type": "system", "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "bg-1", "description": "Build", "task_type": "local_bash" }]
            })
            .to_string(),
            result("Started."),
        ]
    });
    let made: Arc<Mutex<Option<Arc<ClaudeAssistant>>>> = Arc::default();
    let app = TestApp::with_agent(crate::fake_cli::agent_kept(
        &cli,
        config(&[]),
        Arc::clone(&made),
    ))
    .await;
    let session = app.sign_in().await;
    app.call(
        Call::post(paths::TALK)
            .token(&session)
            .json(&json!({ "text": "build it" })),
    )
    .await;
    app.settle().await;
    let runtime = made.lock().unwrap().clone().unwrap();
    assert!(!runtime.stop("bg-9").await, "no such task");
    assert!(runtime.stop("bg-1").await);
    until(|| {
        cli.lines()
            .iter()
            .any(|(_, line)| is_request(line, "stop_task"))
    })
    .await;
    let stops: Vec<Value> = cli
        .lines()
        .into_iter()
        .map(|(_, line)| line)
        .filter(|line| is_request(line, "stop_task"))
        .collect();
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["request"]["task_id"], "bg-1");
}
