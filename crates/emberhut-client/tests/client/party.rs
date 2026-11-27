//! Party and the Dark room: what the client sends and how it reads the answers and streams.

use emberhut_api::dark::DarkRequest;
use emberhut_api::ids::PartyId;
use emberhut_api::party::{CastMember, NewParty, PartyAction, PartyEvent, PartySay, SayKind};
use emberhut_client::{Error, LiveEnd};
use serde_json::json;

use crate::support::{Reply, TestServer, fixture, fresh};

#[test]
fn reads_the_parties_and_starts_one() {
    let server = TestServer::start(|request| match request.method.as_str() {
        "GET" => Reply::Json(200, fixture("parties.json")),
        _ => Reply::Json(200, fixture("party_reply.json")),
    });
    let client = server.client(fresh("r1"));
    assert_eq!(client.parties().unwrap().parties.len(), 2);
    let new = NewParty {
        topic: "Tea or coffee".into(),
        max_turns: 12,
        max_tokens: 150_000,
        personas: vec![CastMember {
            name: "Nora".into(),
            role: "An optimist.".into(),
            model: "claude-opus-5-5".into(),
        }],
        title: None,
    };
    let reply = client.start_party(&new).unwrap();
    assert_eq!(reply.party.unwrap().id, PartyId::new(3));
    let posted = &server.requests_to("/api/parties")[1];
    assert_eq!(posted.method, "POST");
    assert_eq!(
        posted.json(),
        json!({"topic": "Tea or coffee", "maxTurns": 12, "maxTokens": 150_000,
               "personas": [{"name": "Nora", "role": "An optimist.", "model": "claude-opus-5-5"}]})
    );
}

#[test]
fn a_refused_party_carries_the_servers_reason() {
    let server = TestServer::start(|_| {
        Reply::Json(
            400,
            json!({"ok": false, "error": "a party has 2 to 5 personas"}),
        )
    });
    let error = server
        .client(fresh("r1"))
        .start_party(&NewParty {
            topic: "x".into(),
            max_turns: 12,
            max_tokens: 150_000,
            personas: Vec::new(),
            title: None,
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("a party has 2 to 5 personas"),
        "{error}"
    );
}

#[test]
fn each_control_posts_to_its_route() {
    let server = TestServer::start(|request| {
        if request.path.ends_with("/delete") {
            Reply::Json(200, fixture("party_deleted.json"))
        } else {
            Reply::Json(200, fixture("party_reply.json"))
        }
    });
    let client = server.client(fresh("r1"));
    let id = PartyId::new(3);
    let say = PartyAction::Say(PartySay {
        text: "Now argue the other side".into(),
        kind: SayKind::Steer,
    });
    assert!(client.act_on_party(id, &say).unwrap().party.is_some());
    assert!(
        client
            .act_on_party(id, &PartyAction::Pause)
            .unwrap()
            .party
            .is_some()
    );
    assert!(
        client
            .act_on_party(id, &PartyAction::Delete)
            .unwrap()
            .party
            .is_none()
    );
    assert_eq!(
        server.requests_to("/api/parties/3/say")[0].json(),
        json!({"text": "Now argue the other side", "kind": "steer"})
    );
    assert_eq!(
        server.requests_to("/api/parties/3/pause")[0].json(),
        json!({})
    );
    assert_eq!(
        server.requests_to("/api/parties/3/delete")[0].method,
        "POST"
    );
}

#[test]
fn follows_a_partys_stream() {
    const STREAM: [&str; 3] = [
        "data: {\"type\":\"speaking\",\"speaking\":{\"who\":\"p2\",\"name\":\"Victor\",\"text\":\"\"}}\n\n",
        ": ping\n\n",
        "data: {\"type\":\"confetti\"}\n\ndata: {\"type\":\"speaking\",\"speaking\":null}\n\n",
    ];
    let server = TestServer::start(|_| Reply::Stream(STREAM.to_vec()));
    let mut events = Vec::new();
    let end = server
        .client(fresh("r1"))
        .follow_party(PartyId::new(3), || false, |e| events.push(e))
        .unwrap();
    assert_eq!(end, LiveEnd::Closed);
    assert_eq!(events.len(), 2, "the unknown kind is skipped");
    assert!(matches!(
        &events[1],
        PartyEvent::Speaking { speaking: None }
    ));
    let sent = &server.requests_to("/api/parties/3/live")[0];
    assert_eq!(sent.header("accept"), Some("text/event-stream"));
}

#[test]
fn a_stopped_client_ends_a_partys_and_the_dark_rooms_streams() {
    const STREAM: [&str; 3] = [
        "data: {\"type\":\"speaking\",\"speaking\":{\"who\":\"p2\",\"name\":\"Victor\",\"text\":\"\"}}\n\n",
        ": ping\n\n",
        "data: {\"type\":\"speaking\",\"speaking\":null}\n\n",
    ];
    const ANSWER: [&str; 2] = [
        "data: {\"delta\":\"Bis\"}\n\n",
        ": ping\n\ndata: {\"delta\":\"cuit.\"}\n\n",
    ];
    let server = TestServer::start(|request| match request.method.as_str() {
        "POST" => Reply::Stream(ANSWER.to_vec()),
        _ => Reply::Stream(STREAM.to_vec()),
    });
    let client = server.client(fresh("r1"));
    let mut written = String::new();
    client
        .send_dark(&DarkRequest { text: "Hi".into() }, |event| {
            written.extend(event.delta);
            client.stop().unwrap();
        })
        .unwrap();
    assert_eq!(written, "Bis", "ended at the item after the stop");
    let mut events = 0;
    assert!(matches!(
        client.follow_party(PartyId::new(3), || false, |_| events += 1),
        Err(Error::Stopped)
    ));
    assert_eq!(events, 0);
    assert!(
        server.requests_to("/api/parties/3/live").is_empty(),
        "nothing sent after"
    );
}

#[test]
fn a_party_stream_ends_once_the_client_stops() {
    const STREAM: [&str; 3] = [
        "data: {\"type\":\"speaking\",\"speaking\":{\"who\":\"p2\",\"name\":\"Victor\",\"text\":\"\"}}\n\n",
        ": ping\n\n",
        "data: {\"type\":\"speaking\",\"speaking\":null}\n\n",
    ];
    let server = TestServer::start(|_| Reply::Stream(STREAM.to_vec()));
    let client = server.client(fresh("r1"));
    let mut events = 0;
    let end = client
        .follow_party(
            PartyId::new(3),
            || false,
            |_| {
                events += 1;
                client.stop().unwrap();
            },
        )
        .unwrap();
    assert_eq!(end, LiveEnd::Stopped);
    assert_eq!(events, 1);
}

#[test]
fn a_stream_of_a_party_the_server_lacks_is_refused() {
    let server =
        TestServer::start(|_| Reply::Json(404, json!({"ok": false, "error": "no such party"})));
    let error = server
        .client(fresh("r1"))
        .follow_party(PartyId::new(9), || false, |_| {})
        .unwrap_err();
    assert!(
        matches!(error, Error::Status { status: 404, .. }),
        "{error}"
    );
}

#[test]
fn the_dark_room_reads_streams_and_clears() {
    const ANSWER: [&str; 3] = [
        "data: {\"thinking\":true}\n\n",
        "data: {\"delta\":\"Bis\"}\n\ndata: {\"delta\":\"cuit.\"}\n\n",
        "data: {\"done\":{\"id\":4,\"role\":\"assistant\",\"text\":\"Biscuit.\",\"at\":\"2026-11-25T10:01:03.000Z\"}}\n\n",
    ];
    let server = TestServer::start(|request| match request.method.as_str() {
        "GET" => Reply::Json(200, fixture("dark.json")),
        "POST" => Reply::Stream(ANSWER.to_vec()),
        _ => Reply::Json(200, json!({"ok": true})),
    });
    let client = server.client(fresh("r1"));
    assert_eq!(client.dark().unwrap().chat.len(), 2);
    let mut written = String::new();
    let mut done = None;
    client
        .send_dark(
            &DarkRequest {
                text: "A dog's name?".into(),
            },
            |event| {
                written.extend(event.delta);
                done = done.take().or(event.done);
            },
        )
        .unwrap();
    assert_eq!(written, "Biscuit.");
    assert_eq!(done.unwrap().text, "Biscuit.");
    client.clear_dark().unwrap();
    let requests = server.requests_to("/api/dark");
    assert_eq!(requests[1].json(), json!({"text": "A dog's name?"}));
    assert_eq!(requests[2].method, "DELETE");
}

#[test]
fn a_dark_message_while_one_is_answered_is_refused_with_the_servers_words() {
    let server = TestServer::start(|_| {
        Reply::Json(409, json!({"error": "A reply is still being written."}))
    });
    let error = server
        .client(fresh("r1"))
        .send_dark(&DarkRequest { text: "Hi".into() }, |_| {})
        .unwrap_err();
    assert!(error.to_string().contains("still being written"), "{error}");
}
