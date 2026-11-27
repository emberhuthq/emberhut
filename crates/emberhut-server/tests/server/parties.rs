//! Parties: AI personas talking turn by turn through a model, the person's controls, the live
//! stream, and the assistant's `party_start` and `parties` tools.
#![expect(clippy::unwrap_used, reason = "tests: a failure fails the test")]

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use emberhut_api::parties::PartyPage;
use emberhut_api::party::{PartiesPage, PartyEvent as PartyLiveEvent, PartyReply};
use emberhut_server::services::chat_model::{
    ChatModel, ChatModelError, ChatPiece, ChatReply, ChatRequest, Pieces,
};
use futures_util::future::BoxFuture;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::common::{Call, TestApp, text};

/// The model of the tests: each persona's turn answers with the next scripted reply (or a
/// stock line), the supervisor with its scripted pick; a held turn says half a word and waits
/// to be stopped.
#[derive(Debug, Default)]
pub(crate) struct FakeModel {
    state: Mutex<FakeState>,
    /// Told when a held turn has said its half word.
    pub(crate) speaking: Notify,
    released: Notify,
}

#[derive(Debug, Default)]
struct FakeState {
    requests: Vec<ChatRequest>,
    replies: VecDeque<Result<String, String>>,
    supervisor: Option<String>,
    held: bool,
    down: bool,
}

impl FakeModel {
    /// The next turns answer these, in order.
    pub(crate) fn replies(&self, replies: &[Result<&str, &str>]) {
        self.state.lock().unwrap().replies = replies
            .iter()
            .map(|r| r.map(str::to_owned).map_err(str::to_owned))
            .collect();
    }

    /// The supervisor answers this.
    pub(crate) fn supervisor(&self, answer: &str) {
        self.state.lock().unwrap().supervisor = Some(answer.to_owned());
    }

    /// The next turns say half a word and wait until stopped.
    pub(crate) fn hold(&self, held: bool) {
        self.state.lock().unwrap().held = held;
    }

    /// Lets a held turn end, with its half word as its answer.
    pub(crate) fn release(&self) {
        self.state.lock().unwrap().held = false;
        self.released.notify_one();
    }

    /// Whether its health check fails.
    pub(crate) fn down(&self, down: bool) {
        self.state.lock().unwrap().down = down;
    }

    /// What it was asked.
    pub(crate) fn requests(&self) -> Vec<ChatRequest> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl ChatModel for FakeModel {
    fn chat<'a>(
        &'a self,
        request: ChatRequest,
        pieces: Pieces<'a>,
        stop: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ChatReply, ChatModelError>> {
        Box::pin(async move {
            let supervising = request.model == emberhut_core::party::SUPERVISOR_MODEL
                && request
                    .system
                    .starts_with("You run a lively group conversation");
            let (held, reply) = {
                let mut state = self.state.lock().unwrap();
                state.requests.push(request);
                let reply = if supervising {
                    Ok(state
                        .supervisor
                        .clone()
                        .unwrap_or_else(|| "{\"next\": \"p1\"}".to_owned()))
                } else {
                    state
                        .replies
                        .pop_front()
                        .unwrap_or_else(|| Ok("A fine point, and here is mine.".to_owned()))
                };
                (state.held && !supervising, reply)
            };
            if held {
                pieces(ChatPiece::Text("Half a tho".to_owned()));
                self.speaking.notify_one();
                tokio::select! {
                    () = stop.cancelled() => return Err(ChatModelError::Cancelled),
                    () = self.released.notified() => {
                        return Ok(ChatReply {
                            text: "Half a tho".to_owned(),
                            tokens_in: 10,
                            tokens_out: 1,
                        });
                    }
                }
            }
            match reply {
                Ok(text) => {
                    pieces(ChatPiece::Text(text.clone()));
                    Ok(ChatReply {
                        text,
                        tokens_in: 1000,
                        tokens_out: 100,
                    })
                }
                Err(error) => Err(ChatModelError::Stream(error)),
            }
        })
    }

    fn is_up(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move { !self.state.lock().unwrap().down })
    }
}

/// A new party of Nora and Victor, as the app makes one.
async fn start(app: &TestApp, max_turns: u32) -> PartyReply {
    let session = app.sign_in().await;
    let made = app
        .call(Call::post("/api/parties").token(&session).json(&json!({
            "topic": "Should cities ban cars?",
            "personas": [
                { "name": "Nora", "role": "An optimist.", "model": "claude-opus-5-5" },
                { "name": "Victor", "role": "", "model": "claude-haiku-5-5" },
            ],
            "maxTurns": max_turns,
        })))
        .await;
    assert_eq!(made.status, 200, "{}", made.text);
    made.read_as()
}

async fn page(app: &TestApp, id: emberhut_api::ids::PartyId) -> PartyPage {
    let session = app.sign_in().await;
    let got = app
        .call(Call::get(&format!("/api/parties/{id}")).token(&session))
        .await;
    assert_eq!(got.status, 200, "{}", got.text);
    got.read_as()
}

async fn post(app: &TestApp, path: &str, body: &Value) -> crate::common::Answer {
    let session = app.sign_in().await;
    app.call(Call::post(path).token(&session).json(body)).await
}

#[tokio::test]
pub(crate) async fn a_party_talks_turn_by_turn_until_its_turn_cap() {
    let app = TestApp::new().await;
    app.party_model.replies(&[
        Ok("**Nora**: Cars out, trees in."),
        Ok("Who pays for that?"),
    ]);
    let made = start(&app, 3).await;
    assert!(made.ok);
    let party = made.party.unwrap();
    assert_eq!(party.personas[0].id.as_str(), "p1");
    assert_eq!(party.personas[1].color, "orange");
    app.state.parties().idle().await;

    let got = page(&app, party.id).await;
    assert_eq!(got.party.status.as_str(), "done");
    assert_eq!(got.party.why.as_deref(), Some("Reached its 3 turns"));
    assert_eq!(got.party.turns, 3);
    let lines: Vec<(&str, &str)> = got
        .messages
        .iter()
        .map(|m| (m.who.as_str(), m.text.as_str()))
        .collect();
    assert_eq!(
        lines,
        vec![
            ("owner", "Should cities ban cars?"),
            ("p1", "Cars out, trees in."),
            ("p2", "Who pays for that?"),
            ("p1", "A fine point, and here is mine."),
        ]
    );
    // Opus at $4/$20 twice, Haiku at $0.10/$0.50 once, 1,000 in and 100 out each: $0.01215,
    // shown to four places.
    assert!((got.party.cost - 0.0122).abs() < 1e-9, "{}", got.party.cost);
    assert_eq!(got.party.tokens, 3300);
    let asked = app.party_model.requests();
    assert!(asked[1].system.contains("You are Victor"));
    assert_eq!(asked[1].model, "claude-haiku-5-5");

    let session = app.sign_in().await;
    let list = app.call(Call::get("/api/parties").token(&session)).await;
    assert_eq!(list.status, 200);
    let list: PartiesPage = list.read_as();
    assert_eq!(list.parties.len(), 1);
    assert_eq!(
        list.parties[0].last.as_deref(),
        Some("Nora: A fine point, and here is mine.")
    );
    assert_eq!(list.presets.len(), 8);
    assert_eq!(list.models.len(), 3);
    assert_eq!(list.defaults.as_ref().unwrap().max_turns, 12);
    assert_eq!(list.recent.len(), 2, "the person's own personas");
}

#[tokio::test]
pub(crate) async fn a_party_without_a_session_or_with_a_bad_cast_is_refused() {
    let app = TestApp::new().await;
    for (method, path) in [
        ("GET", "/api/parties"),
        ("POST", "/api/parties"),
        ("GET", "/api/parties/1"),
        ("GET", "/api/parties/1/live"),
        ("POST", "/api/parties/1/say"),
        ("POST", "/api/parties/1/pause"),
        ("POST", "/api/parties/1/resume"),
        ("POST", "/api/parties/1/stop"),
        ("POST", "/api/parties/1/caps"),
        ("POST", "/api/parties/1/delete"),
    ] {
        let call = if method == "GET" {
            Call::get(path)
        } else {
            Call::post(path).json(&json!({}))
        };
        assert_eq!(app.call(call).await.status, 401, "{method} {path}");
    }
    // Report mode lets a request without a session read, never act: a party runs a model, at
    // a cost.
    let report = TestApp::with_mode(emberhut_server::config::AuthMode::Report).await;
    for path in [
        "/api/parties",
        "/api/parties/1/say",
        "/api/parties/1/pause",
        "/api/parties/1/resume",
        "/api/parties/1/stop",
        "/api/parties/1/caps",
        "/api/parties/1/delete",
    ] {
        let refused = report.call(Call::post(path).json(&json!({}))).await;
        assert_eq!(refused.status, 401, "{path}");
        assert_eq!(refused.json()["signIn"], true, "{path}");
    }
    assert_eq!(report.call(Call::get("/api/parties")).await.status, 200);
    let refused = post(
        &app,
        "/api/parties",
        &json!({ "topic": "t", "personas": [{ "name": "Solo" }] }),
    )
    .await;
    assert_eq!(refused.status, 400);
    assert_eq!(
        refused.json(),
        json!({ "ok": false, "error": "a party has 2 to 5 personas" })
    );
    let session = app.sign_in().await;
    let too_big = app
        .call(
            Call::post("/api/parties")
                .token(&session)
                .raw(&format!("{{\"topic\":\"{}\"}}", "x".repeat(3_000_000))),
        )
        .await;
    assert_eq!(too_big.status, 413);
    for action in ["say", "caps"] {
        let path = format!("/api/parties/1/{action}");
        let big = format!("{{\"text\":\"{}\"}}", "x".repeat(3_000_000));
        let got = app.call(Call::post(&path).token(&session).raw(&big)).await;
        assert_eq!(got.status, 413, "{path}");
        let got = app
            .call(Call::post(&path).token(&session).raw("not json"))
            .await;
        assert_eq!(got.status, 400, "{path}");
    }
}

#[tokio::test]
pub(crate) async fn a_party_that_is_not_there_is_404() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    for path in ["/api/parties/9", "/api/parties/9/live", "/api/parties/x"] {
        let got = app.call(Call::get(path).token(&session)).await;
        assert_eq!(got.status, 404, "{path}");
        assert_eq!(got.json(), json!({ "ok": false, "error": "no such party" }));
    }
    for action in ["say", "pause", "resume", "stop", "caps", "delete"] {
        let path = format!("/api/parties/9/{action}");
        let got = post(&app, &path, &json!({ "text": "hi" })).await;
        assert_eq!(got.status, 404, "{path}");
    }
}

#[tokio::test]
pub(crate) async fn the_person_stops_steers_resumes_sets_caps_and_deletes() {
    let app = TestApp::new().await;
    app.party_model.hold(true);
    let party = start(&app, 12).await.party.unwrap();
    tokio::time::timeout(Duration::from_secs(5), app.party_model.speaking.notified())
        .await
        .expect("Nora starts speaking");
    let path = |action: &str| format!("/api/parties/{}/{action}", party.id);

    let paused = post(&app, &path("pause"), &json!({})).await;
    assert_eq!(paused.status, 200);
    let paused: PartyReply = paused.read_as();
    let paused = paused.party.unwrap();
    assert_eq!(paused.status.as_str(), "paused");
    assert_eq!(
        paused.why.as_deref(),
        Some("Nora finishes the turn, then it waits")
    );
    assert_eq!(paused.speaking.unwrap().text, "Half a tho");

    let stopped: PartyReply = post(&app, &path("stop"), &json!({})).await.read_as();
    assert_eq!(stopped.party.unwrap().status.as_str(), "stopped");
    app.state.parties().idle().await;
    let got = page(&app, party.id).await;
    let cut = got.messages.last().unwrap();
    assert_eq!((cut.text.as_str(), cut.cut), ("Half a tho", true));

    let blank = post(&app, &path("say"), &json!({ "text": "  " })).await;
    assert_eq!(blank.status, 400);
    assert_eq!(blank.json()["error"], "say something");

    // Speaking to a stopped party sets it talking again; a direction is for everyone, so the
    // one who hasn't spoken yet goes next.
    app.party_model.hold(false);
    let capped: PartyReply = post(&app, &path("caps"), &json!({ "maxTurns": 2 }))
        .await
        .read_as();
    assert_eq!(capped.party.unwrap().max_turns, 2);
    let said = post(
        &app,
        &path("say"),
        &json!({ "text": "Victor, argue the other side", "kind": "steer" }),
    )
    .await;
    assert_eq!(said.status, 200);
    app.state.parties().idle().await;
    let got = page(&app, party.id).await;
    let steer = got
        .messages
        .iter()
        .find(|m| m.kind.as_str() == "steer")
        .unwrap();
    assert_eq!(steer.who.as_str(), "owner");
    assert_eq!(got.messages.last().unwrap().who.as_str(), "p2");
    let asked = app.party_model.requests();
    assert!(
        asked
            .last()
            .unwrap()
            .system
            .contains("argue the other side")
    );
    assert_eq!(got.party.status.as_str(), "done");
    assert_eq!(got.party.why.as_deref(), Some("Reached its 2 turns"));

    // Words that name a persona: they answer. The cap was reached, so it gets 10 more turns.
    let said = post(&app, &path("say"), &json!({ "text": "Nora, your answer?" })).await;
    let resumed: PartyReply = said.read_as();
    let resumed = resumed.party.unwrap();
    assert_eq!(resumed.status.as_str(), "running");
    assert_eq!(resumed.max_turns, 12);
    app.state.parties().idle().await;
    let got = page(&app, party.id).await;
    let after = got
        .messages
        .iter()
        .position(|m| m.text == "Nora, your answer?")
        .unwrap();
    assert_eq!(got.messages[after + 1].who.as_str(), "p1", "Nora was named");
    assert_eq!(got.party.turns, 12);
    let resumed: PartyReply = post(&app, &path("resume"), &json!({})).await.read_as();
    assert_eq!(resumed.party.unwrap().status.as_str(), "running");
    app.state.parties().idle().await;
    assert_eq!(page(&app, party.id).await.party.turns, 22);

    let deleted = post(&app, &path("delete"), &json!({})).await;
    assert_eq!(deleted.status, 200);
    assert_eq!(deleted.json(), json!({ "ok": true, "party": null }));
    let session = app.sign_in().await;
    let gone = app
        .call(Call::get(&format!("/api/parties/{}", party.id)).token(&session))
        .await;
    assert_eq!(gone.status, 404);
}

#[tokio::test]
async fn a_failed_turn_pauses_the_party_with_why() {
    let app = TestApp::new().await;
    app.party_model.replies(&[Err("Workers down")]);
    let party = start(&app, 5).await.party.unwrap();
    app.state.parties().idle().await;
    let got = page(&app, party.id).await;
    assert_eq!(got.party.status.as_str(), "paused");
    assert_eq!(
        got.party.why.as_deref(),
        Some("Nora couldn't speak (Workers down)")
    );
}

#[tokio::test]
async fn the_supervisor_picks_among_three_and_may_end_the_talk() {
    let app = TestApp::new().await;
    let session = app.sign_in().await;
    app.party_model
        .supervisor("{\"next\": \"p3\", \"end\": true}");
    let made: PartyReply = app
        .call(Call::post("/api/parties").token(&session).json(&json!({
            "topic": "Tea or coffee",
            "personas": [{ "name": "Ann" }, { "name": "Bo" }, { "name": "Cy" }],
        })))
        .await
        .read_as();
    let party = made.party.unwrap();
    app.state.parties().idle().await;
    let got = page(&app, party.id).await;
    // Three in the first round; then the supervisor picks Cy (who just spoke is not a choice,
    // so the next in turn, Ann, speaks) until two rounds are done, and then ends it.
    assert_eq!(got.party.status.as_str(), "done");
    assert_eq!(got.party.why.as_deref(), Some("They reached the end of it"));
    assert_eq!(got.party.turns, 6);
}

#[tokio::test]
pub(crate) async fn the_live_stream_starts_with_a_snapshot_then_follows_the_talk() {
    let app = TestApp::new().await;
    app.party_model.hold(true);
    let party = start(&app, 12).await.party.unwrap();
    tokio::time::timeout(Duration::from_secs(5), app.party_model.speaking.notified())
        .await
        .expect("Nora starts speaking");
    let session = app.sign_in().await;
    let response = app
        .send(
            Call::get(&format!("/api/parties/{}/live", party.id))
                .token(&session)
                .request(),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body();
    let mut buffer = String::new();
    let mut events = Vec::new();
    let mut stopped = false;
    while events.len() < 4 {
        if let Some(end) = buffer.find("\n\n") {
            let block: String = buffer.drain(..end + 2).collect();
            if let Some(data) = block.strip_prefix("data: ") {
                let event: PartyLiveEvent = serde_json::from_str(data.trim_end()).unwrap();
                events.push(event);
            }
            if events.len() == 1 && !stopped {
                stopped = true;
                post(&app, &format!("/api/parties/{}/stop", party.id), &json!({})).await;
            }
            continue;
        }
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("an event within 5 s")
            .unwrap()
            .unwrap();
        if let Ok(chunk) = frame.into_data() {
            buffer.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    }
    match &events[0] {
        PartyLiveEvent::Snapshot { party, messages } => {
            assert_eq!(
                party.as_ref().unwrap().speaking.as_ref().unwrap().name,
                "Nora"
            );
            assert_eq!(messages.len(), 1);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, PartyLiveEvent::Party { party: Some(party) } if party.status.as_str() == "stopped"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, PartyLiveEvent::Speaking { speaking: None }))
    );
}

#[tokio::test]
async fn the_assistant_starts_a_party_and_reads_it() {
    let app = TestApp::new().await;
    let started = app
        .call_tool(
            "party_start",
            json!({
                "topic": "Is remote work here to stay?",
                "personas": [{ "name": "victor" }, { "name": "Ann", "role": "A manager." }],
                "max_turns": 2,
            }),
        )
        .await;
    assert_eq!(started.is_error, None);
    assert_eq!(
        text(&started),
        "Party 1 started: Is remote work here to stay?, with Victor and Ann. It's on the \
         user's Party screen; up to 2 turns."
    );
    app.state.parties().idle().await;
    let list = app.call_tool("parties", json!({})).await;
    assert!(text(&list).starts_with("Party 1: Is remote work here to stay? · done · 2 turns"));
    let one = app.call_tool("parties", json!({ "id": 1 })).await;
    let one = text(&one);
    assert!(
        one.contains("(done: Reached its 2 turns), 2/2 turns"),
        "{one}"
    );
    assert!(
        one.contains("The host: Is remote work here to stay?"),
        "{one}"
    );
    assert!(one.contains("Victor: A fine point"), "{one}");
    let none = app.call_tool("parties", json!({ "id": 7 })).await;
    assert_eq!(none.is_error, Some(true));
    let refused = app
        .call_tool(
            "party_start",
            json!({ "topic": "Alone", "personas": [{ "name": "Solo" }] }),
        )
        .await;
    assert_eq!(refused.is_error, Some(true));
    assert_eq!(text(&refused), "a party has 2 to 5 personas");
}

/// What `pub` here needs: a turn runs on with its model, and the shutdown cuts it.
#[tokio::test]
async fn a_restart_pauses_a_party_left_running() {
    let app = TestApp::new().await;
    app.party_model.hold(true);
    let party = start(&app, 12).await.party.unwrap();
    tokio::time::timeout(Duration::from_secs(5), app.party_model.speaking.notified())
        .await
        .expect("Nora starts speaking");
    app.state.stopping().cancel();
    app.state.parties().idle().await;
    assert_eq!(app.state.parties().pause_left_running().await.unwrap(), 1);
    let got = page(&app, party.id).await;
    assert_eq!(got.party.status.as_str(), "paused");
    assert_eq!(got.party.why.as_deref(), Some("The server restarted"));
}
