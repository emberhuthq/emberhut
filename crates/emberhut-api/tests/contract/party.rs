//! Party and the Dark room: their pages, requests, answers and stream events.

use emberhut_api::dark::{DarkEvent, DarkRequest, DarkRole, DarkRoom};
use emberhut_api::ids::{DarkMessageId, PartyId, PartyLineId, PersonaId};
use emberhut_api::party::{
    LineKind, NewParty, PartiesPage, PartyAction, PartyEvent, PartyReply, PartySay, PartyStatus,
    SayKind,
};
use emberhut_api::paths;

use crate::support::{
    assert_round_trip, assert_writes_shape, fixture, from_value, with_unknown_fields,
};

#[test]
fn reads_the_parties_page() {
    let page: PartiesPage = assert_writes_shape("parties.json", &[]);
    let [talking, done] = page.parties.as_slice() else {
        panic!("two parties")
    };
    assert_eq!(talking.status, PartyStatus::Running);
    assert_eq!(talking.speaking.as_ref().unwrap().text, "Only if");
    assert_eq!(
        talking.persona(&PersonaId::new("p2")).unwrap().name,
        "Victor"
    );
    assert_eq!(done.why.as_deref(), Some("Reached its 12 turns"));
    assert!(done.last.is_none());
    assert_eq!(page.presets[0].color, "", "a preset has no colour");
    assert_eq!(page.recent[0].id.as_str(), "recent1");
    assert!((page.models[2].price_in - 0.1).abs() < f64::EPSILON);
    assert_eq!(page.defaults.unwrap().max_tokens, 150_000);
}

#[test]
fn the_parties_page_tolerates_unknown_fields() {
    let mut body = fixture("parties.json");
    with_unknown_fields(&mut body);
    assert_eq!(
        from_value::<PartiesPage>(body),
        from_value::<PartiesPage>(fixture("parties.json"))
    );
}

#[test]
fn a_new_party_and_a_line_round_trip() {
    let new: NewParty = assert_round_trip("party_new.json");
    assert_eq!(new.personas.len(), 2);
    assert!(new.title.is_none(), "no title: the server makes one");
    let say: PartySay = assert_round_trip("party_say.json");
    assert_eq!(say.kind, SayKind::Steer);
}

#[test]
fn reads_the_answers_to_the_controls() {
    let reply: PartyReply = assert_writes_shape("party_reply.json", &[]);
    let party = reply.party.unwrap();
    assert_eq!(party.id, PartyId::new(3));
    assert_eq!(party.status, PartyStatus::Paused);
    let deleted: PartyReply = assert_writes_shape("party_deleted.json", &[]);
    assert!(deleted.ok && deleted.party.is_none());
}

#[test]
fn reads_the_live_streams_events() {
    let PartyEvent::Snapshot { party, messages } = assert_writes_shape("party_snapshot.json", &[])
    else {
        panic!("a snapshot")
    };
    assert_eq!(party.unwrap().turns, 1);
    assert_eq!(messages[0].kind, LineKind::Say);
    assert_eq!(messages[1].model.as_deref(), Some("claude-opus-5-5"));
    let PartyEvent::Message { message } = assert_writes_shape("party_message.json", &[]) else {
        panic!("a message")
    };
    assert_eq!(message.id, PartyLineId::new(12));
    assert!(message.cut);
    let PartyEvent::Speaking { speaking } = assert_writes_shape("party_speaking.json", &[]) else {
        panic!("who is speaking")
    };
    assert_eq!(speaking.unwrap().text, "");
}

#[test]
fn an_event_of_a_kind_this_contract_doesnt_know_doesnt_read() {
    let unknown = serde_json::json!({ "type": "confetti" });
    assert!(serde_json::from_value::<PartyEvent>(unknown).is_err());
}

#[test]
fn each_control_has_its_route() {
    let id = PartyId::new(7);
    let say = PartyAction::Say(PartySay {
        text: "Hello".into(),
        kind: SayKind::Say,
    });
    assert_eq!(paths::party_action(id, &say), "/api/parties/7/say");
    assert_eq!(
        paths::party_action(id, &PartyAction::Delete),
        "/api/parties/7/delete"
    );
    assert_eq!(paths::party_live(id), "/api/parties/7/live");
}

#[test]
fn reads_the_dark_room() {
    let room: DarkRoom = assert_round_trip("dark.json");
    assert_eq!(
        room.place.as_deref(),
        Some("DeepSeek V4.1 Flash on Cloudflare")
    );
    assert_eq!(room.chat[1].role, DarkRole::Assistant);
    let request: DarkRequest = assert_round_trip("dark_request.json");
    assert!(request.text.ends_with("dog?"));
    let done: DarkEvent = assert_round_trip("dark_done.json");
    assert_eq!(done.done.unwrap().id, DarkMessageId::new(4));
    let delta: DarkEvent = from_value(serde_json::json!({ "delta": "Bis" }));
    assert_eq!(delta.delta.as_deref(), Some("Bis"));
    assert!(delta.done.is_none() && delta.error.is_none() && delta.thinking.is_none());
}

#[test]
fn the_dark_room_of_an_older_server_has_no_place() {
    let mut body = fixture("dark.json");
    body.as_object_mut().unwrap().remove("where");
    with_unknown_fields(&mut body);
    assert!(from_value::<DarkRoom>(body).place.is_none());
}
