//! The runner's side of the runner protocol, against the server's own types (`emberhut-api`):
//! a whole runner in-process, driven over its HTTP API like the server drives it, with a stand-in
//! agent that speaks Claude Code's stream-json and a stand-in server that keeps everything it is
//! sent. Each answer and each event is read with the types the server and the apps read it with.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::http::Uri;
use emberhut_api::machines::{Cpu, Memory};
use emberhut_api::runner::actions::SentToSession;
use emberhut_api::runner::new_session::{BranchList, FolderList, OpenedSession};
use emberhut_api::runner::steps::StepKind;
use emberhut_api::runner::transcript::{SessionLive, Transcript};
use emberhut_runner::config::Config;
use emberhut_runner::engine::claude::piped;
use emberhut_runner::engine::{Agent, AgentOutput, Engine, EngineError, Launch, SessionStart};
use emberhut_runner::runner::{Options, Runner};
use futures_util::StreamExt as _;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc;

const TOKEN: &str = "test-token-0123456789";

// ── The stand-in agent ──────────────────────────────────────────────────────────────────────────

/// Speaks Claude Code's stream-json. A message with `LIMIT` in it ends on a usage limit when the
/// session runs on the account whose folder is `limited`; one with `SLOW` waits for an interrupt;
/// any other gets an answer with thinking, words as they're typed, a tool call and its result.
#[derive(Debug, Default)]
struct Scripted {
    limited: Option<PathBuf>,
    launches: Mutex<Vec<Launch>>,
}

impl Engine for Scripted {
    fn start(
        &self,
        launch: &Launch,
    ) -> Result<(Agent, mpsc::UnboundedReceiver<AgentOutput>), EngineError> {
        self.launches.lock().unwrap().push(launch.clone());
        let (runner_in, agent_in) = tokio::io::duplex(1 << 20);
        let (mut agent_out, runner_out) = tokio::io::duplex(1 << 20);
        let dir = launch
            .set_env
            .iter()
            .find(|(k, _)| k == "CLAUDE_CONFIG_DIR")
            .map(|(_, v)| PathBuf::from(v));
        let limited = self.limited.is_some() && dir == self.limited;
        let session = match &launch.session {
            SessionStart::New(id) | SessionStart::Resume(id) => id.clone(),
            SessionStart::Fresh => "00000000-0000-4000-8000-000000000000".to_owned(),
        };
        tokio::spawn(async move {
            let mut lines = BufReader::new(agent_in).lines();
            let mut waiting = false;
            while let Ok(Some(line)) = lines.next_line().await {
                let m: Value = serde_json::from_str(&line).unwrap();
                let mut say = Vec::new();
                match m["type"].as_str() {
                    Some("control_request") => {
                        let id = m["request_id"].clone();
                        let answer = match m["request"]["subtype"].as_str() {
                            Some("initialize") => {
                                json!({ "account": { "email": "one@example.com", "subscriptionType": "max" } })
                            }
                            Some("get_usage") => {
                                json!({ "rate_limits": { "five_hour": { "utilization": 12.0, "resets_at": "2099-01-01T00:00:00Z" } } })
                            }
                            _ => json!({}),
                        };
                        say.push(json!({ "type": "control_response", "response": { "subtype": "success", "request_id": id, "response": answer } }));
                        if m["request"]["subtype"] == "interrupt" && waiting {
                            waiting = false;
                            say.push(json!({ "type": "result", "subtype": "error_during_execution", "is_error": true, "result": "" }));
                        }
                    }
                    Some("user") => {
                        let text = m["message"]["content"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned();
                        say.push(
                            json!({ "type": "system", "subtype": "init", "session_id": session }),
                        );
                        if text.contains("LIMIT") && limited {
                            say.push(json!({ "type": "result", "subtype": "success", "is_error": true, "result": "You've hit your limit · resets 3pm" }));
                        } else if text.contains("SLOW") {
                            say.push(json!({ "type": "assistant", "message": { "content": [ { "type": "text", "text": "Starting." } ] } }));
                            waiting = true;
                        } else {
                            say.extend(answer());
                        }
                    }
                    _ => {}
                }
                for s in say {
                    let mut b = s.to_string().into_bytes();
                    b.push(b'\n');
                    if agent_out.write_all(&b).await.is_err() {
                        return;
                    }
                }
            }
        });
        Ok(piped(runner_in, runner_out))
    }
}

fn answer() -> Vec<Value> {
    let ev = |e: Value| json!({ "type": "stream_event", "event": e });
    vec![
        ev(json!({ "type": "content_block_start", "content_block": { "type": "thinking" } })),
        ev(
            json!({ "type": "content_block_delta", "delta": { "type": "thinking_delta", "thinking": "Looking at it" } }),
        ),
        json!({ "type": "assistant", "message": { "content": [
            { "type": "thinking", "thinking": "Looking at it." },
            { "type": "tool_use", "id": "call-1", "name": "Bash", "input": { "command": "cargo test" } },
        ] } }),
        json!({ "type": "user", "message": { "content": [ { "type": "tool_result", "tool_use_id": "call-1", "content": "test result: ok" } ] } }),
        ev(json!({ "type": "content_block_start", "content_block": { "type": "text" } })),
        ev(
            json!({ "type": "content_block_delta", "delta": { "type": "text_delta", "text": "All " } }),
        ),
        ev(
            json!({ "type": "content_block_delta", "delta": { "type": "text_delta", "text": "green." } }),
        ),
        ev(json!({ "type": "message_delta", "usage": { "output_tokens": 9 } })),
        json!({ "type": "assistant", "message": { "content": [ { "type": "text", "text": "All green.\nSTATUS: done" } ] } }),
        json!({ "type": "result", "subtype": "success", "is_error": false, "result": "All green.\nSTATUS: done",
                "usage": { "input_tokens": 10, "output_tokens": 9 }, "total_cost_usd": 0.01 }),
    ]
}

// ── The stand-in server ─────────────────────────────────────────────────────────────────────────

type Heard = Arc<Mutex<Vec<(String, Value)>>>;

async fn server() -> (String, Heard) {
    let heard: Heard = Arc::default();
    let keep = Arc::clone(&heard);
    let app = axum::Router::new().fallback(move |uri: Uri, body: Bytes| {
        let keep = Arc::clone(&keep);
        async move {
            let v = serde_json::from_slice(&body).unwrap_or(Value::Null);
            keep.lock().unwrap().push((uri.path().to_owned(), v));
            if uri.path() == "/api/runner/mcp" {
                r#"{"mcpServers":{}}"#
            } else {
                r#"{"ok":true}"#
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{at}"), heard)
}

fn events(heard: &Heard, kind: &str) -> Vec<Value> {
    heard
        .lock()
        .unwrap()
        .iter()
        .filter(|(p, v)| p == "/api/runner/events" && v["kind"] == kind)
        .map(|(_, v)| v.clone())
        .collect()
}

// ── The runner ──────────────────────────────────────────────────────────────────────────────────

struct Hut {
    _dir: tempfile::TempDir,
    work: PathBuf,
    base: String,
    http: reqwest::Client,
    heard: Heard,
    engine: Arc<Scripted>,
    runner: Option<Runner>,
}

async fn hut(limit_first: bool) -> Hut {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let (server, heard) = server().await;
    let accounts: Vec<Value> = ["a", "b"]
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let folder = dir.path().join(format!("agent-{id}"));
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join(".credentials.json"), "{}").unwrap();
            json!({ "id": id, "name": format!("Account {id}"), "configDir": folder, "order": i + 1 })
        })
        .collect();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("accounts.json"),
        serde_json::to_string(&accounts).unwrap(),
    )
    .unwrap();
    std::fs::write(home.join("token"), TOKEN).unwrap();
    std::fs::write(
        home.join("config.json"),
        json!({ "name": "build", "server": server, "port": 0, "listen": ["127.0.0.1"], "machineEvery": 1000,
                "worktreesDir": dir.path().join("worktrees") })
        .to_string(),
    )
    .unwrap();
    let h = home.display().to_string();
    let user = dir.path().display().to_string();
    let env = move |k: &str| match k {
        "EMBERHUT_RUNNER_HOME" => Some(h.clone()),
        "HOME" => Some(user.clone()),
        _ => None,
    };
    let config = Config::load(&env, "build").unwrap();
    let engine = Arc::new(Scripted {
        limited: limit_first.then(|| dir.path().join("agent-a")),
        ..Scripted::default()
    });
    let options = Options {
        engine: Some(engine.clone()),
        credential_files: true,
        no_probe: true,
        no_samples: false,
        tmp: Some(dir.path().join("tmp")),
    };
    let runner = Runner::start(config, options).await.unwrap();
    let base = format!("http://127.0.0.1:{}", runner.port());
    Hut {
        _dir: dir,
        work,
        base,
        http: emberhut_runner::client::client().unwrap(),
        heard,
        engine,
        runner: Some(runner),
    }
}

impl Hut {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut r = self
            .http
            .request(method.parse().unwrap(), format!("{}{path}", self.base))
            .bearer_auth(TOKEN);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let r = r.send().await.unwrap();
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    async fn until(&self, what: &str, done: impl Fn(&Hut) -> bool) {
        for _ in 0..200 {
            if done(self) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("never: {what}; heard {:#?}", self.heard.lock().unwrap());
    }

    async fn close(mut self) {
        if let Some(r) = self.runner.take() {
            r.close().await;
        }
    }
}

fn git_repo(at: &Path) {
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "first",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(&args)
                .current_dir(at)
                .status()
                .unwrap()
                .success()
        );
    }
}

// ── The tests ───────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one session's whole life, checked step by step"
)]
async fn a_turn_reports_and_reads_back_in_the_servers_shapes() {
    let h = hut(false).await;
    let (status, opened) = h
        .call("POST", "/sessions", Some(json!({ "cwd": h.work, "title": "Tidy", "prompt": "Run the tests", "meta": { "direct": true }, "by": "owner", "effort": "high" })))
        .await;
    assert_eq!(status, 200, "{opened}");
    let reply: OpenedSession = serde_json::from_value(
        json!({ "ok": true, "id": opened["id"], "turn": opened["turn"], "session": opened }),
    )
    .unwrap();
    let id = reply
        .session
        .as_ref()
        .expect("the opened session")
        .id
        .to_string();
    assert!(emberhut_runner::wire::is_session_id(&id));
    assert_eq!(opened["turn"], format!("{id}:1"));

    h.until("the turn ends", |h| {
        !events(&h.heard, "turn_finished").is_empty()
    })
    .await;
    for kind in [
        "session",
        "turn_queued",
        "turn_started",
        "activity",
        "turn_finished",
    ] {
        let e = events(&h.heard, kind);
        assert!(!e.is_empty(), "no {kind} event");
        assert_eq!(e[0]["machine"], "build");
        assert_eq!(e[0]["session"]["id"], id.as_str());
        assert!(e[0]["at"].as_str().unwrap().ends_with('Z'));
    }
    let finished = &events(&h.heard, "turn_finished")[0];
    assert_eq!(finished["turn"]["outcome"], "completed");
    assert_eq!(finished["turn"]["last"], "All green.\nSTATUS: done");
    assert_eq!(finished["session"]["meta"]["direct"], true);
    assert_eq!(events(&h.heard, "turn_queued")[0]["turn"]["by"], "owner");

    let (_, t) = h
        .call(
            "GET",
            &format!("/sessions/{id}/transcript?after=&limit="),
            None,
        )
        .await;
    let transcript: Transcript = serde_json::from_value(t.clone()).unwrap();
    let kinds: Vec<StepKind> = transcript.events.iter().map(|s| s.kind.clone()).collect();
    for k in [
        StepKind::User,
        StepKind::Thinking,
        StepKind::Tool,
        StepKind::Result,
        StepKind::Text,
        StepKind::End,
    ] {
        assert!(kinds.contains(&k), "no {k:?} step in {kinds:?}");
    }
    let tool = transcript
        .events
        .iter()
        .find(|s| s.kind == StepKind::Tool)
        .unwrap();
    assert_eq!(tool.summary.as_deref(), Some("cargo test"));
    assert_eq!(
        tool.call.as_ref().map(ToString::to_string).as_deref(),
        Some("call-1")
    );
    assert!(!transcript.earlier);
    assert_eq!(transcript.session.status, "completed");

    // The live stream: the head first, then every step after the one asked for.
    let r = h
        .http
        .get(format!("{}/sessions/{id}/live?after=1", h.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["content-type"], "text/event-stream");
    let mut body = r.bytes_stream();
    let mut text = String::new();
    while !text.contains("\"end\"") {
        text.push_str(&String::from_utf8_lossy(
            &tokio::time::timeout(Duration::from_secs(5), body.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
        ));
    }
    let live: Vec<SessionLive> = text
        .split("\n\n")
        .filter_map(|f| f.strip_prefix("data: "))
        .map(|d| serde_json::from_str(d).unwrap())
        .collect();
    assert!(matches!(live[0], SessionLive::Head { .. }));
    assert!(
        live.iter()
            .skip(1)
            .all(|e| matches!(e, SessionLive::Step { step } if step.seq > 1))
    );

    let (_, sent) = h
        .call(
            "POST",
            &format!("/sessions/{id}/send"),
            Some(json!({ "prompt": "Again", "by": "assistant" })),
        )
        .await;
    let sent: SentToSession = serde_json::from_value(sent).unwrap();
    assert_eq!(sent.turn.map(|t| t.to_string()), Some(format!("{id}:2")));
    h.until("the second turn ends", |h| {
        events(&h.heard, "turn_finished").len() >= 2
    })
    .await;
    let second = h.engine.launches.lock().unwrap().len();
    assert_eq!(second, 1, "the second turn runs in the same agent process");

    let (_, settings) = h
        .call(
            "POST",
            &format!("/sessions/{id}/settings"),
            Some(json!({ "effort": "max" })),
        )
        .await;
    assert_eq!(settings["id"], id.as_str());
    assert_eq!(settings["effort"], "max");

    git_repo(&h.work);
    let (_, folders) = h
        .call("GET", &format!("/folders?path={}", h.work.display()), None)
        .await;
    let folders: FolderList = serde_json::from_value(folders).unwrap();
    assert!(folders.git == Some(true), "{folders:?}");
    let (_, root) = h.call("GET", "/folders", None).await;
    let _root: FolderList = serde_json::from_value(root).unwrap();
    let (_, branches) = h
        .call("GET", &format!("/branches?cwd={}", h.work.display()), None)
        .await;
    let branches: BranchList = serde_json::from_value(branches).unwrap();
    assert_eq!(branches.current, "main");

    h.until("a machine sample", |h| {
        h.heard
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| p == "/api/runner/machine")
    })
    .await;
    let sample = h
        .heard
        .lock()
        .unwrap()
        .iter()
        .find(|(p, _)| p == "/api/runner/machine")
        .map(|(_, v)| v.clone())
        .unwrap();
    assert_eq!(sample["machine"], "build");
    let _cpu: Cpu = serde_json::from_value(sample["cpu"].clone()).unwrap();
    if !sample["memory"].is_null() {
        let _memory: Memory = serde_json::from_value(sample["memory"].clone()).unwrap();
    }
    h.close().await;
}

#[tokio::test]
async fn at_a_limit_the_session_moves_to_the_next_account_and_carries_on() {
    let h = hut(true).await;
    let (status, opened) = h
        .call(
            "POST",
            "/sessions",
            Some(json!({ "cwd": h.work, "prompt": "LIMIT test", "title": "Limits" })),
        )
        .await;
    assert_eq!(status, 200, "{opened}");
    assert_eq!(opened["account"], "a");
    h.until("the turn ends", |h| {
        !events(&h.heard, "turn_finished").is_empty()
    })
    .await;
    let moved = &events(&h.heard, "account")[0];
    assert_eq!(
        (moved["from"].as_str(), moved["to"].as_str()),
        (Some("a"), Some("b"))
    );
    let finished = &events(&h.heard, "turn_finished")[0];
    assert_eq!(finished["turn"]["outcome"], "completed");
    assert_eq!(finished["turn"]["accounts"], json!(["a", "b"]));
    assert_eq!(finished["session"]["account"], "b");
    let (_, accounts) = h.call("GET", "/accounts", None).await;
    let a = accounts["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == "a")
        .unwrap()
        .clone();
    assert!(
        a["limitedUntil"].is_string() && a["limitedWhy"].as_str().unwrap().contains("limit"),
        "{a}"
    );
    h.close().await;
}

#[tokio::test]
async fn an_interrupt_ends_the_running_turn_and_the_next_one_runs() {
    let h = hut(false).await;
    let (_, opened) = h
        .call(
            "POST",
            "/sessions",
            Some(json!({ "cwd": h.work, "prompt": "SLOW work" })),
        )
        .await;
    let id = opened["id"].as_str().unwrap().to_owned();
    h.until("the turn starts", |h| {
        !events(&h.heard, "turn_started").is_empty()
    })
    .await;
    let (_, queued) = h
        .call(
            "POST",
            &format!("/sessions/{id}/send"),
            Some(json!({ "prompt": "after it" })),
        )
        .await;
    assert_eq!(queued["queued"], true);
    let (_, stopped) = h
        .call("POST", &format!("/sessions/{id}/interrupt"), None)
        .await;
    assert_eq!(stopped["running"], true);
    h.until("both turns end", |h| {
        events(&h.heard, "turn_finished").len() >= 2
    })
    .await;
    let outcomes: Vec<Value> = events(&h.heard, "turn_finished")
        .iter()
        .map(|e| e["turn"]["outcome"].clone())
        .collect();
    assert_eq!(outcomes, [json!("interrupted"), json!("completed")]);
    h.close().await;
}

#[tokio::test]
async fn only_the_token_gets_in_and_refusals_are_words() {
    let h = hut(false).await;
    let r = h
        .http
        .get(format!("{}/health", h.base))
        .send()
        .await
        .unwrap();
    let health: Value = r.json().await.unwrap();
    assert_eq!(
        (health["ok"].as_bool(), health["machine"].as_str()),
        (Some(true), Some("build"))
    );
    let r = h
        .http
        .get(format!("{}/accounts", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    let r = h
        .http
        .get(format!("{}/accounts", h.base))
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    let (status, refused) = h
        .call(
            "POST",
            "/sessions",
            Some(json!({ "cwd": "/no/such/folder", "prompt": "x" })),
        )
        .await;
    assert_eq!(status, 400);
    assert!(
        refused["error"].as_str().unwrap().starts_with("no folder"),
        "{refused}"
    );
    let (status, _) = h.call("GET", "/sessions/r-00000000/transcript", None).await;
    assert_eq!(status, 404);
    let (status, refused) = h
        .call(
            "POST",
            "/sessions",
            Some(json!({ "cwd": h.work, "prompt": "x", "effort": "loud" })),
        )
        .await;
    assert_eq!(
        (status, refused["error"].as_str()),
        (400, Some("no effort loud"))
    );
    h.close().await;
}
