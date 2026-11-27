//! Voice notes as text: a WhatsApp or iMessage voice note in what the Mac helper answers reads
//! as its transcript, once; a failed one says so and is tried again later; one over the length
//! limit is skipped; and the transcripts are found by `messages_search`.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers: a failure fails the test that called them"
)]

use jiff::SignedDuration;
use serde_json::{Value, json};

use crate::common::text;
use crate::messaging_fakes::Kit;

/// A kit whose Mac has one chat with Bo, holding a voice note of `seconds`.
fn with_note(kit: &Kit, seconds: f64) {
    let at = (kit.now() - SignedDuration::from_mins(10)).to_string();
    kit.mac.answers(
        "/read",
        &json!([
            { "chat": "wa:12", "chatName": "Bo", "at": at, "fromMe": false, "sender": "Bo", "text": "see below" },
            {
                "chat": "wa:12", "chatName": "Bo", "at": at, "fromMe": false, "sender": "Bo", "text": "",
                "audio": { "id": "wa:m1", "name": "voice.opus", "seconds": seconds }
            }
        ]),
    );
    kit.mac.raw(
        "/media",
        200,
        "audio/ogg; codecs=opus",
        b"OggS....",
        Some("12"),
    );
}

async fn read(kit: &Kit) -> Value {
    let read = kit
        .app
        .call_tool("messages_read", json!({ "chat": "wa:12" }))
        .await;
    serde_json::from_str(text(&read)).unwrap()
}

#[tokio::test]
async fn a_voice_note_reads_as_what_was_said() {
    let kit = Kit::new().await;
    with_note(&kit, 12.0);
    kit.transcriber.hears("  call me back when you can ");
    let messages = read(&kit).await;
    assert_eq!(
        messages[0]["text"], "see below",
        "other messages as they came"
    );
    assert_eq!(
        messages[1]["text"],
        "[voice, 0:12] call me back when you can"
    );
    assert!(
        messages[1].get("audio").is_none(),
        "the audio field isn't passed on"
    );
    let heard = kit.transcriber.heard();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].0, "voice.ogg", "named for its type");
    assert!(heard[0].2.contains("Names that may come up: Bo."));
    assert!(
        kit.mac
            .paths()
            .iter()
            .any(|p| p.starts_with("/media?id=wa%3Am1&max=600"))
    );

    read(&kit).await;
    assert_eq!(kit.transcriber.heard().len(), 1, "transcribed once");

    kit.mac.answers("/search", &json!([]));
    let found = kit
        .app
        .call_tool("messages_search", json!({ "q": "call me back" }))
        .await;
    let found = text(&found);
    assert!(
        found.contains("WhatsApp and iMessage voice notes that match (what was said)"),
        "{found}"
    );
    assert!(
        found.contains(r#""text":"[voice, 0:12] call me back when you can""#),
        "{found}"
    );
    assert!(found.contains(r#""source":"WhatsApp""#));
}

#[tokio::test]
async fn a_failed_voice_note_says_so_and_is_tried_again_later() {
    let kit = Kit::new().await;
    with_note(&kit, 12.0);
    kit.transcriber.fails("the service is busy");
    let messages = read(&kit).await;
    assert_eq!(messages[1]["text"], "[voice.opus] (couldn't transcribe)");
    assert_eq!(
        kit.transcriber.heard().len(),
        2,
        "a hiccup is tried again at once"
    );

    kit.transcriber.hears("all good now");
    kit.app.state.messaging().voice().tick().await.unwrap();
    kit.app
        .state
        .messaging()
        .voice()
        .settle(std::time::Duration::from_secs(5), None)
        .await;
    assert_eq!(kit.transcriber.heard().len(), 2, "not before its time");
    kit.later(61);
    kit.app.state.messaging().voice().tick().await.unwrap();
    kit.app
        .state
        .messaging()
        .voice()
        .settle(std::time::Duration::from_secs(5), None)
        .await;
    let messages = read(&kit).await;
    assert_eq!(messages[1]["text"], "[voice, 0:12] all good now");
}

#[tokio::test]
async fn a_voice_note_over_the_limit_is_skipped() {
    let kit = Kit::new().await;
    with_note(&kit, 900.0);
    kit.transcriber.hears("never heard");
    let messages = read(&kit).await;
    assert_eq!(
        messages[1]["text"],
        "[voice, 15:00] voice.opus (not transcribed: over 10 minutes)"
    );
    assert_eq!(kit.transcriber.heard().len(), 0);

    let kit = Kit::new().await;
    with_note(&kit, 0.0);
    kit.mac.raw(
        "/media",
        413,
        "application/json",
        br#"{"seconds":700}"#,
        None,
    );
    let messages = read(&kit).await;
    assert_eq!(
        messages[1]["text"], "[voice, 11:40] voice.opus (not transcribed: over 10 minutes)",
        "the Mac said it is too long"
    );
}
