//! Slack's live events over Socket Mode, against a stand-in Slack socket on loopback: every
//! envelope is acknowledged at once, a message event is kept in the mirror, a rotated socket
//! is opened again and a dropped one or a failed open after a doubling pause, and a workspace
//! whose messages flow live has its notifications from the Mac skipped.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers: a failure here fails the test that called them"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use emberhut_core::messaging::slack::SlackMessage;
use futures_util::{SinkExt as _, StreamExt as _};
use jiff::SignedDuration;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message;

use crate::messaging_fakes::Kit;
use crate::slack::connect;

/// What the stand-in socket does next on the connection open at the time.
enum Out {
    /// Sends this envelope.
    Send(Value),
    /// Drops the connection without a close frame.
    Drop,
}

/// A stand-in for Slack's Socket Mode endpoint: it says hello on each connection, sends what
/// the test hands it, and reports every frame the server sends back.
struct FakeSocket {
    url: String,
    out: mpsc::UnboundedSender<Out>,
    acks: mpsc::UnboundedReceiver<String>,
    connections: Arc<AtomicUsize>,
}

impl FakeSocket {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/link", listener.local_addr().unwrap());
        let (out, script) = mpsc::unbounded_channel::<Out>();
        let (heard, acks) = mpsc::unbounded_channel();
        let script = Arc::new(Mutex::new(script));
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&connections);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    continue;
                };
                counted.fetch_add(1, Ordering::SeqCst);
                let hello = json!({ "type": "hello", "num_connections": 1 });
                ws.send(Message::text(hello.to_string())).await.unwrap();
                let mut script = script.lock().await;
                loop {
                    tokio::select! {
                        next = script.recv() => match next {
                            Some(Out::Send(envelope)) => {
                                if ws.send(Message::text(envelope.to_string())).await.is_err() {
                                    break;
                                }
                            }
                            Some(Out::Drop) | None => break,
                        },
                        frame = ws.next() => match frame {
                            Some(Ok(Message::Text(text))) => {
                                drop(heard.send(text.as_str().to_owned()));
                            }
                            Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                            Some(Ok(_)) => {}
                        },
                    }
                }
            }
        });
        Self {
            url,
            out,
            acks,
            connections,
        }
    }

    fn send(&self, envelope: Value) {
        self.out.send(Out::Send(envelope)).unwrap();
    }

    fn drop_connection(&self) {
        self.out.send(Out::Drop).unwrap();
    }

    /// The next frame the server sent back, within 3 seconds (Slack's limit for an ack).
    async fn ack(&mut self) -> Value {
        let frame = tokio::time::timeout(Duration::from_secs(3), self.acks.recv())
            .await
            .expect("an acknowledgement within 3 seconds")
            .unwrap();
        serde_json::from_str(&frame).unwrap()
    }

    /// Waits until `count` connections were made.
    async fn connections_reach(&self, count: usize) {
        until(|| async { self.connections.load(Ordering::SeqCst) >= count }).await;
    }
}

/// Waits (5 seconds at most) until `check` holds.
async fn until<F, Fut>(check: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..500 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("not within 5 seconds");
}

/// A kit whose live socket is opened again after 20 ms (then 40, 80 ...).
async fn quick() -> Kit {
    Kit::with(|deps| deps.slack_live_pause = Duration::from_millis(20)).await
}

/// A message event from Ann in DM D1, sent `minutes` before the kit's now, as Slack wraps it.
fn message(kit: &Kit, envelope: &str, minutes: i64, text: &str) -> (Value, String) {
    let ts = format!(
        "{}.000200",
        (kit.now() - SignedDuration::from_mins(minutes)).as_second()
    );
    let envelope = json!({
        "envelope_id": envelope,
        "type": "events_api",
        "accepts_response_payload": false,
        "payload": {
            "type": "event_callback",
            "event": {
                "type": "message", "channel": "D1", "user": "UANN", "text": text,
                "ts": ts, "channel_type": "im"
            }
        }
    });
    (envelope, format!("Acme:D1:{ts}"))
}

/// The message `id` in the mirror, once it is there.
async fn kept(kit: &Kit, id: &str) -> SlackMessage {
    let db = emberhut_db::Database::connect(&kit.app.url).await.unwrap();
    let store = db.slack();
    until(|| {
        let store = store.clone();
        async move { store.get(id).await.unwrap().is_some() }
    })
    .await;
    store.get(id).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_live_message_is_acknowledged_at_once_and_kept() {
    let mut socket = FakeSocket::start().await;
    let kit = quick().await;
    kit.secret("slack-acme-app", "xapp-test");
    kit.slack.answers(
        "apps.connections.open",
        json!({ "ok": true, "url": socket.url }),
    );
    let kit = connect(kit).await;
    socket.connections_reach(1).await;

    let (envelope, id) = message(&kit, "env-1", 1, "see you live <@UME>");
    socket.send(envelope);
    assert_eq!(socket.ack().await, json!({ "envelope_id": "env-1" }));
    let message = kept(&kit, &id).await;
    assert_eq!(message.workspace, "Acme");
    assert_eq!(message.who, "Ann");
    assert_eq!(message.body, "see you live @Me Myself");
    assert_eq!(message.sender.as_deref(), Some("Ann"));
    assert_eq!(message.channel.as_deref(), Some("D1"));
    assert!(!message.from_me);

    // An envelope of another kind is acknowledged too, and keeps nothing.
    socket.send(json!({
        "envelope_id": "env-2", "type": "slash_commands", "payload": {}
    }));
    assert_eq!(socket.ack().await, json!({ "envelope_id": "env-2" }));

    // The workspace now comes in live: its notifications from the Mac are skipped, another
    // workspace's still kept.
    let at = (kit.now() - SignedDuration::from_mins(1)).to_string();
    kit.mac.answers(
        "/notifications",
        &json!([
            { "id": "n-acme", "title": "Acme", "subtitle": "Ann", "body": "see you", "at": at },
            { "id": "n-beta", "title": "Beta", "subtitle": "#ops", "body": "deploy", "at": at }
        ]),
    );
    kit.app.state.messaging().collect().await;
    let db = emberhut_db::Database::connect(&kit.app.url).await.unwrap();
    assert!(db.slack().get("n-acme").await.unwrap().is_none());
    assert!(db.slack().get("n-beta").await.unwrap().is_some());
    // One socket per workspace, however often the workspaces are connected again.
    assert_eq!(kit.slack.calls("apps.connections.open").len(), 1);
}

#[tokio::test]
async fn a_rotated_or_dropped_socket_is_opened_again() {
    let mut socket = FakeSocket::start().await;
    let kit = quick().await;
    kit.secret("slack-acme-app", "xapp-test");
    kit.slack.answers(
        "apps.connections.open",
        json!({ "ok": true, "url": socket.url }),
    );
    let kit = connect(kit).await;
    socket.connections_reach(1).await;

    // Slack rotates the socket: a new one is opened.
    socket.send(json!({ "type": "disconnect", "reason": "refresh_requested" }));
    socket.connections_reach(2).await;
    let (envelope, id) = message(&kit, "env-3", 1, "after the rotation");
    socket.send(envelope);
    assert_eq!(socket.ack().await, json!({ "envelope_id": "env-3" }));
    assert_eq!(kept(&kit, &id).await.body, "after the rotation");

    // It drops: a new one after the pause.
    socket.drop_connection();
    socket.connections_reach(3).await;
    let (envelope, id) = message(&kit, "env-4", 1, "after the drop");
    socket.send(envelope);
    assert_eq!(socket.ack().await, json!({ "envelope_id": "env-4" }));
    assert_eq!(kept(&kit, &id).await.body, "after the drop");
    assert_eq!(kit.slack.calls("apps.connections.open").len(), 3);
}

#[tokio::test]
async fn a_socket_that_will_not_open_is_tried_again_with_longer_pauses() {
    let mut socket = FakeSocket::start().await;
    let kit = quick().await;
    kit.secret("slack-acme-app", "xapp-test");
    kit.slack.answers(
        "apps.connections.open",
        json!({ "ok": false, "error": "invalid_auth" }),
    );
    let kit = connect(kit).await;
    // 20 ms, then 40, then 80: three tries in well under a second.
    until(|| async { kit.slack.calls("apps.connections.open").len() >= 3 }).await;
    assert_eq!(socket.connections.load(Ordering::SeqCst), 0);

    kit.slack.answers(
        "apps.connections.open",
        json!({ "ok": true, "url": socket.url }),
    );
    socket.connections_reach(1).await;
    let (envelope, id) = message(&kit, "env-5", 1, "at last");
    socket.send(envelope);
    assert_eq!(socket.ack().await, json!({ "envelope_id": "env-5" }));
    assert_eq!(kept(&kit, &id).await.body, "at last");
}

#[tokio::test]
async fn without_an_app_token_no_socket_is_opened() {
    let kit = connect(quick().await).await;
    kit.app.state.messaging().collect().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(kit.slack.calls("apps.connections.open").len(), 0);
}

#[tokio::test]
async fn the_socket_closes_when_the_server_stops() {
    let socket = FakeSocket::start().await;
    let kit = quick().await;
    kit.secret("slack-acme-app", "xapp-test");
    kit.slack.answers(
        "apps.connections.open",
        json!({ "ok": true, "url": socket.url }),
    );
    let kit = connect(kit).await;
    socket.connections_reach(1).await;
    kit.app.state.messaging().slack().stop_live();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        socket.connections.load(Ordering::SeqCst),
        1,
        "not opened again"
    );
    assert_eq!(kit.slack.calls("apps.connections.open").len(), 1);
}
