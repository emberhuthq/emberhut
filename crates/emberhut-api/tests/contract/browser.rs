//! The assistant's browser: its state, a change's answer, the live view's session and the
//! toolbar's requests, as the wire carries them.

use emberhut_api::browser::{
    BrowserControl, BrowserInput, BrowserReply, BrowserSessionReply, StreamMode,
};
use emberhut_api::ids::{BrowserDeviceId, BrowserTabId, TaskId};
use emberhut_api::state::State;
use serde_json::json;

use crate::support::{assert_round_trip, fixture, from_value, read, to_value};

#[test]
fn reads_the_browser_in_the_state() {
    let state: State = read("state.json");
    let browser = state.browser.unwrap();
    assert!(browser.connected && !browser.chrome_ui);
    assert_eq!(browser.control, BrowserControl::Person);
    assert_eq!(browser.task, Some(TaskId::new(12)));
    assert_eq!(
        browser.handoff.as_deref(),
        Some("Sign in to the example site")
    );
    assert!(browser.is_unclaimed());
    assert_eq!(
        browser.shown_tab().map(|tab| tab.id),
        Some(BrowserTabId::new(1))
    );
    assert!(!browser.file_request.unwrap().multiple);
}

#[test]
fn an_older_server_has_no_browser() {
    let state: State = read("state_older.json");
    assert!(state.browser.is_none());
}

#[test]
fn a_change_answers_with_the_browser_in_the_csharp_shape() {
    let reply: BrowserReply = assert_round_trip("browser_reply.json");
    assert!(reply.ok && reply.browser.chrome_ui);
    assert!(reply.browser.is_mine(&BrowserDeviceId::new("device-1")));
    assert!(!reply.browser.is_mine(&BrowserDeviceId::new("device-2")));
    assert!(reply.browser.file_request.is_none());
}

#[test]
fn a_session_reads_and_writes_the_csharp_shape() {
    let reply: BrowserSessionReply = assert_round_trip("browser_live_view.json");
    let session = reply.session;
    assert_eq!(session.mode, StreamMode::Control);
    assert_eq!(session.screen.map(|screen| screen.width), Some(2880));
    assert_eq!(
        session.signed_url().as_deref(),
        Some("https://viewer:example-password@example.org:8447/")
    );
    let mut body = fixture("browser_live_view.json");
    body["screen"] = json!(null);
    body["mode"] = json!("view");
    let reply: BrowserSessionReply = from_value(body);
    assert!(reply.session.screen.is_none() && !reply.session.is_control());
}

#[test]
fn every_input_is_written_as_the_csharp_server_reads_it() {
    let inputs = [
        BrowserInput::Takeover {
            device: BrowserDeviceId::new("device-1"),
        },
        BrowserInput::Handback,
        BrowserInput::Back,
        BrowserInput::Forward,
        BrowserInput::Reload,
        BrowserInput::Go {
            url: "example.com".to_owned(),
        },
        BrowserInput::Tab {
            id: BrowserTabId::new(3),
        },
        BrowserInput::NewTab,
        BrowserInput::CloseTab {
            id: BrowserTabId::new(3),
        },
        BrowserInput::Chrome { show: true },
    ];
    assert_eq!(to_value(&inputs), fixture("browser_input.json"));
    let read: Vec<BrowserInput> = read("browser_input.json");
    assert_eq!(read, inputs);
}
