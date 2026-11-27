//! The whole slice through the real client: `emberhut-client` pairs with a code, polls the state
//! (fresh, then 304), follows the live stream while it talks, reads the reply and marks the
//! room seen, against the server on a real socket.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "the client's side runs in helpers: a failure there fails the test"
)]

use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use emberhut_api::chat::HistoryQuery;
use emberhut_api::ids::RoomId;
use emberhut_api::live::LiveEvent;
use emberhut_api::talk::TalkRequest;
use emberhut_client::{
    Client, ClientConfig, LiveEnd, MemorySessionStore, StatePoll, StateReply, Upload,
};
use emberhut_server::services::pairing::CodeMaker;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::common::{REQUEST_TIMEOUT, Scripted, TestApp};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_client_pairs_polls_talks_follows_and_sees() {
    let app = TestApp::new().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(emberhut_server::run(
        listener,
        app.state.clone(),
        REQUEST_TIMEOUT,
        Duration::from_secs(5),
        async {
            stopped.await.ok();
        },
    ));
    let code = app
        .state
        .pairing()
        .new_code(CodeMaker::CommandLine)
        .await
        .unwrap()
        .code
        .to_string();
    app.assistant
        .answers(&["Thinking"], Some("Hello from the server."));
    app.assistant.hold();
    let assistant = Arc::clone(&app.assistant);

    let client_side = tokio::task::spawn_blocking(move || drive(address, &code, &assistant))
        .await
        .unwrap();

    stop.send(()).unwrap();
    let end = tokio::task::spawn_blocking(move || client_side.join().unwrap())
        .await
        .unwrap();
    assert!(
        matches!(end, Ok(LiveEnd::Closed | LiveEnd::Stopped)),
        "the stream ends with the server: {end:?}"
    );
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

/// The client's side: pair, poll, talk while following the stream, read the reply, see it.
/// Returns the live stream's thread, still following.
fn drive(
    address: std::net::SocketAddr,
    code: &str,
    assistant: &Scripted,
) -> std::thread::JoinHandle<Result<LiveEnd, emberhut_client::Error>> {
    let config = ClientConfig::new(&format!("http://{address}")).unwrap();
    let client = Arc::new(Client::new(&config, MemorySessionStore::new()).unwrap());
    client.pair(code).unwrap();
    assert!(client.is_signed_in());
    poll_twice(&client);

    let (events, received) = mpsc::channel();
    let done = Arc::new(AtomicBool::new(false));
    let follower = {
        let client = Arc::clone(&client);
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            client.follow_live(
                RoomId::MAIN,
                || done.load(Ordering::SeqCst),
                |event| {
                    events.send(event).ok();
                },
            )
        })
    };
    let next = || received.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(next(), LiveEvent::Idle, "the room as it is: nothing runs");

    let reply = client
        .talk(&TalkRequest::text(RoomId::MAIN, "Hello, server"))
        .unwrap();
    assert!(reply.ok && reply.started);
    assert!(matches!(next(), LiveEvent::Working(_)));
    assistant.release();
    while next() != LiveEvent::Idle {}

    read_the_reply_and_see_it(&client);
    let code = client.new_pairing_code().unwrap();
    assert_eq!(code.code.len(), 9);
    done.store(true, Ordering::SeqCst);
    follower
}

/// A first poll gets the state and its `ETag`; a second with the tag gets 304.
fn poll_twice(client: &Client) {
    let StateReply::Fresh { state, etag } = client.state(&StatePoll::new(RoomId::MAIN)).unwrap()
    else {
        panic!("a first poll gets the state");
    };
    assert!(state.room.is_main());
    let etag = etag.expect("the state has an ETag");
    let again = client
        .state(&StatePoll::new(RoomId::MAIN).held(Some(etag)).looking(true))
        .unwrap();
    assert!(matches!(again, StateReply::Unchanged), "{again:?}");
}

/// The reply is in the chat, unread until the room is seen.
fn read_the_reply_and_see_it(client: &Client) {
    let StateReply::Fresh { state, .. } = client.state(&StatePoll::new(RoomId::MAIN)).unwrap()
    else {
        panic!("the state changed");
    };
    let chat: Vec<(bool, String)> = state
        .chat
        .iter()
        .map(|m| (m.mine, m.text.clone()))
        .collect();
    assert_eq!(
        chat,
        [
            (true, "Hello, server".to_owned()),
            (false, "Hello from the server.".to_owned())
        ]
    );
    assert!(state.unread);
    client.seen(RoomId::MAIN).unwrap();
    let StateReply::Fresh { state, .. } = client.state(&StatePoll::new(RoomId::MAIN)).unwrap()
    else {
        panic!("seeing the room changed the state");
    };
    assert!(!state.unread);
}

/// The desktop's file calls through the real client: it uploads a picture, sends it with a
/// message, finds it in the room's history and fetches its bytes, whole and as a video player
/// would.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_client_uploads_sends_pages_back_and_fetches_a_file() {
    let app = TestApp::new().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(emberhut_server::run(
        listener,
        app.state.clone(),
        REQUEST_TIMEOUT,
        Duration::from_secs(5),
        async {
            stopped.await.ok();
        },
    ));
    let code = app
        .state
        .pairing()
        .new_code(CodeMaker::CommandLine)
        .await
        .unwrap()
        .code
        .to_string();

    let url = tokio::task::spawn_blocking(move || {
        let config = ClientConfig::new(&format!("http://{address}")).unwrap();
        let client = Client::new(&config, MemorySessionStore::new()).unwrap();
        client.pair(&code).unwrap();
        let upload = Upload::bytes("a photo.png", b"not really a png".to_vec());
        let file = client
            .upload(&upload, |_| ControlFlow::Continue(()))
            .unwrap();
        assert_eq!(file.name, "a photo.png");
        assert!(file.path.is_some(), "the server says where it keeps it");
        let reply = client
            .talk(&TalkRequest::text(RoomId::MAIN, "Look").with_files(vec![file.clone()]))
            .unwrap();
        assert!(reply.ok);
        let page = client.history(&HistoryQuery::latest(RoomId::MAIN)).unwrap();
        let said = &page.chat[0];
        assert_eq!(said.text, "Look");
        assert_eq!(said.files.len(), 1);
        assert_eq!(said.files[0].url, file.url);
        assert_eq!(client.file(&file.url).unwrap(), b"not really a png");
        let mut part = Vec::new();
        client
            .download(&file.url, &mut part, |_| ControlFlow::Continue(()))
            .unwrap();
        assert_eq!(part, b"not really a png");
        file.url
    })
    .await
    .unwrap();
    assert!(url.starts_with("/uploads/"));
    app.settle().await;
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
