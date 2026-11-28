//! The Claude Code driver against the real command line: a turn, then an interrupt.
//!
//! Runs only when `EMBERHUT_LIVE_CLAUDE` names the `claude` command to use (it spends a little
//! of an account's use); `CLAUDE_CONFIG_DIR` picks the account as usual.

use std::time::Duration;

use emberhut_runner::engine::claude::ClaudeCode;
use emberhut_runner::engine::{AgentOutput, Engine as _, Launch};
use serde_json::Value;

#[tokio::test]
async fn a_turn_then_an_interrupt() {
    let Ok(command) = std::env::var("EMBERHUT_LIVE_CLAUDE") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let launch = Launch {
        cwd: dir.path().to_path_buf(),
        model: Some("haiku".to_owned()),
        effort: Some("low".to_owned()),
        ..Launch::default()
    };
    let (agent, mut out) = ClaudeCode::new(command.into()).start(&launch).unwrap();

    let init = agent
        .request(
            serde_json::json!({ "subtype": "initialize" }),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert!(init.get("account").is_some(), "{init}");
    let usage = agent
        .request(
            serde_json::json!({ "subtype": "get_usage", "skip_behaviors": true }),
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    assert!(usage.get("rate_limits").is_some(), "{usage}");

    agent
        .say("Reply with the single word OK. Do not use any tools.")
        .await
        .unwrap();
    let mut kinds = Vec::new();
    let result = loop {
        match tokio::time::timeout(Duration::from_secs(120), out.recv())
            .await
            .unwrap()
            .unwrap()
        {
            AgentOutput::Message(m) => {
                let t = m
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                kinds.push(t.clone());
                if t == "result" {
                    break m;
                }
            }
            AgentOutput::Permission { tool, .. } => panic!("asked about {tool}"),
            AgentOutput::Exited { why } => panic!("ended: {why}; saw {kinds:?}"),
        }
    };
    assert_eq!(
        result.get("subtype").and_then(Value::as_str),
        Some("success"),
        "{result}"
    );
    assert!(kinds.contains(&"assistant".to_owned()), "{kinds:?}");

    agent
        .say("Count slowly from 1 to 400, one number per line, no tools.")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    agent.interrupt().await.unwrap();
    let ended = loop {
        match tokio::time::timeout(Duration::from_secs(60), out.recv())
            .await
            .unwrap()
            .unwrap()
        {
            AgentOutput::Message(m) if m.get("type").and_then(Value::as_str) == Some("result") => {
                break m;
            }
            AgentOutput::Exited { why } => panic!("ended: {why}"),
            _ => {}
        }
    };
    eprintln!(
        "after interrupt: {}",
        ended
            .get("subtype")
            .and_then(Value::as_str)
            .unwrap_or_default()
    );
    agent.close().await;
}
