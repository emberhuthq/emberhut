//! A configured agent's room session, with a scripted CLI: its persona and the fixed core as
//! its prompt (none of the owner's rules), its own MCP servers joined to the server's and
//! allowed, its accounts' secret paths in the environment, only its allowed tools in the
//! CLI's `dontAsk` mode; a disabled agent's room takes no turns. The default agent's session
//! is as it was.
#![expect(
    clippy::unwrap_used,
    clippy::too_many_lines,
    reason = "a test helper, and a test that walks one agent's session from start to end"
)]

use std::sync::Arc;

use emberhut_api::mcp::SERVER_NAME;
use emberhut_core::RoomId;
use emberhut_server::services::claude::{AgentRooms, ClaudeAssistant, FIXED_CORE};
use serde_json::{Value, json};

use crate::common::{Call, TestApp};
use crate::fake_cli::{FakeCli, config, init, is_request, is_user, result};

fn answering() -> Arc<FakeCli> {
    FakeCli::new(|launch, line| {
        if !is_user(line) {
            return Vec::new();
        }
        let session = launch.flag("--resume").unwrap_or("s-1").to_owned();
        vec![init(&session), result("NO_REPLY")]
    })
}

async fn app_with(cli: &Arc<FakeCli>) -> TestApp {
    let cli = Arc::clone(cli);
    TestApp::with_agent(move |db, clock| {
        let data = db.path().parent().unwrap().to_path_buf();
        Arc::new(
            ClaudeAssistant::new(
                config(&[("acct-a", "/accounts/a")]),
                cli,
                db.settings(),
                db.rooms(),
                clock,
            )
            .with_agents(AgentRooms::new(db.agents(), data.join("secrets"), data)),
        ) as _
    })
    .await
}

#[tokio::test]
async fn an_agents_room_runs_as_that_agent() {
    let cli = answering();
    let app = app_with(&cli).await;
    let session = app.sign_in().await;
    let made = app
        .call(Call::post("/api/agents").token(&session).json(&json!({
            "id": "client-a", "name": "Example helper", "principal": "Example Ltd",
            "persona": "You help the example team ship.",
        })))
        .await;
    assert_eq!(made.status, 200, "{}", made.text);
    let room = RoomId::new(made.json()["agent"]["room"].as_i64().unwrap());
    let config_file = app.dir().join("example-admin.json");
    // A configuration file anyone may write: whoever writes it would choose what runs.
    let open_file = app.dir().join("open.json");
    std::fs::write(&open_file, json!({ "command": "anything" }).to_string()).unwrap();
    std::fs::set_permissions(
        &open_file,
        std::os::unix::fs::PermissionsExt::from_mode(0o666),
    )
    .unwrap();
    std::fs::write(
        &config_file,
        json!({ "command": "example-admin", "args": ["--stdio"] }).to_string(),
    )
    .unwrap();
    // Only its owner may write it, whatever the umask the tests run under.
    std::fs::set_permissions(
        &config_file,
        std::os::unix::fs::PermissionsExt::from_mode(0o644),
    )
    .unwrap();
    let tools = app
        .call(
            Call::put("/api/agents/client-a/tools")
                .token(&session)
                .json(&json!({ "tools": [
                    { "kind": "mcp", "name": "example-admin", "config": config_file.display().to_string() },
                    { "kind": "mcp", "name": "missing", "config": "/nowhere/missing.json" },
                    { "kind": "mcp", "name": "open", "config": open_file.display().to_string() },
                ]})),
        )
        .await;
    assert_eq!(tools.status, 200, "{}", tools.text);
    let accounts = app
        .call(
            Call::put("/api/agents/client-a/accounts")
                .token(&session)
                .json(
                    &json!({ "accounts": [{ "label": "admin", "secret": "example-admin-key" }] }),
                ),
        )
        .await;
    assert_eq!(accounts.status, 200, "{}", accounts.text);

    app.state
        .talk()
        .post(
            room,
            "[Slack] Example person · channel C0EXAMPLE · ts 1\nhello".to_owned(),
        )
        .await
        .unwrap();
    app.settle().await;
    let launches = cli.launches();
    assert_eq!(launches.len(), 1);
    let launch = &launches[0];
    assert_eq!(launch.flag("--permission-mode"), Some("dontAsk"));
    assert_eq!(
        launch.flag("--allowedTools"),
        Some(format!("mcp__{SERVER_NAME}__*,mcp__example-admin__*").as_str())
    );
    // Apart from the owner's setup: no settings, memory or hooks, no built-in tools, only the
    // MCP servers named here, none of the installation's extra arguments, a folder of its own.
    assert_eq!(launch.flag("--setting-sources"), Some(""));
    assert_eq!(launch.flag("--tools"), Some(""));
    assert!(
        launch.args.contains(&"--strict-mcp-config".to_owned()),
        "{:?}",
        launch.args
    );
    assert!(
        !launch.args.iter().any(|arg| arg.starts_with("--add-dir")),
        "the installation's arguments are the owner's sessions' only: {:?}",
        launch.args
    );
    let folder = app.dir().join("agents").join("client-a");
    assert_eq!(launch.cwd.as_deref(), Some(folder.as_path()));
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&folder).unwrap().permissions())
            & 0o777,
        0o700
    );
    assert_eq!(
        launch.var("CLAUDE_CONFIG_DIR"),
        Some("/accounts/a"),
        "its sign-in"
    );
    let mcp: Value = serde_json::from_str(launch.flag("--mcp-config").unwrap()).unwrap();
    assert_eq!(
        mcp["mcpServers"]["example-admin"]["command"],
        "example-admin"
    );
    assert_eq!(mcp["mcpServers"][SERVER_NAME]["type"], "http");
    assert!(
        mcp["mcpServers"].get("missing").is_none(),
        "an unreadable server is left out"
    );
    assert!(
        mcp["mcpServers"].get("open").is_none(),
        "a file others may write is left out"
    );
    let secret_path = app.dir().join("secrets").join("example-admin-key");
    assert_eq!(
        launch.var("EMBERHUT_AGENT_ACCOUNT_ADMIN"),
        Some(secret_path.display().to_string().as_str())
    );
    let lines = cli.lines();
    assert!(is_request(&lines[0].1, "initialize"));
    let prompt = lines[0].1["request"]["appendSystemPrompt"]
        .as_str()
        .unwrap();
    assert!(
        prompt.starts_with(
            "You are Example helper, an agent working for Example Ltd.\n\nYou help the example team ship."
        ),
        "{prompt}"
    );
    assert!(prompt.ends_with(FIXED_CORE), "{prompt}");
    assert!(
        !prompt.contains("Answer in one line."),
        "not the owner's configured prompt"
    );
    assert!(
        !prompt.contains("personal assistant"),
        "not the owner's assistant's rules"
    );

    // Disabled: its room takes no turns.
    let off = app
        .call(
            Call::patch("/api/agents/client-a")
                .token(&session)
                .json(&json!({ "enabled": false })),
        )
        .await;
    assert_eq!(off.status, 200);
    let before = cli.said().len();
    app.state
        .talk()
        .post(room, "a check-in".to_owned())
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(
        cli.said().len(),
        before,
        "a disabled agent's room took a turn"
    );

    // The default agent's room is as it was.
    app.state
        .talk()
        .talk(RoomId::MAIN, "Hello".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    let main = cli.launches().pop().unwrap();
    assert_eq!(
        main.flag("--allowedTools"),
        Some(format!("mcp__{SERVER_NAME}__*").as_str())
    );
    assert!(main.var("EMBERHUT_AGENT_ACCOUNT_ADMIN").is_none());
}

#[tokio::test]
async fn a_rooms_session_starts_afresh_when_its_agent_comes_or_changes() {
    let cli = answering();
    let app = app_with(&cli).await;
    let session = app.sign_in().await;
    let db = emberhut_db::Database::connect(&app.url).await.unwrap();
    let room = db
        .rooms()
        .create("Example", app.clock_now())
        .await
        .unwrap()
        .id;
    // The owner's conversation in the room.
    app.state
        .talk()
        .talk(room, "Hello".to_owned(), None)
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(cli.launches().len(), 1);
    // An agent takes the room over: its first turn is a new conversation, as the agent.
    let made = app
        .call(Call::post("/api/agents").token(&session).json(&json!({
            "id": "client-a", "name": "Example helper", "principal": "Example Ltd",
            "room": room.get(),
        })))
        .await;
    assert_eq!(made.status, 200, "{}", made.text);
    app.state
        .talk()
        .post(room, "a check-in".to_owned())
        .await
        .unwrap();
    app.settle().await;
    let launches = cli.launches();
    assert_eq!(launches.len(), 2, "the owner's open session is not reused");
    assert_eq!(
        launches[1].flag("--resume"),
        None,
        "nor its conversation resumed"
    );
    assert_eq!(launches[1].flag("--tools"), Some(""));
    // Its next turn goes on in its own session; a change to the agent starts it afresh.
    app.state
        .talk()
        .post(room, "another".to_owned())
        .await
        .unwrap();
    app.settle().await;
    assert_eq!(cli.launches().len(), 2);
    let changed = app
        .call(
            Call::patch("/api/agents/client-a")
                .token(&session)
                .json(&json!({ "persona": "Be brief." })),
        )
        .await;
    assert_eq!(changed.status, 200, "{}", changed.text);
    app.state
        .talk()
        .post(room, "and again".to_owned())
        .await
        .unwrap();
    app.settle().await;
    let launches = cli.launches();
    assert_eq!(launches.len(), 3, "a changed agent's session isn't reused");
    assert_eq!(launches[2].flag("--resume"), None);
}
