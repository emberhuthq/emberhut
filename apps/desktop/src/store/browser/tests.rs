//! The browser's store on GPUI's headless test platform with a scripted backend: opening the
//! window claims a browser handed over, a new session comes whenever control changes for this
//! device, an input's answer is the browser's state at once, and a file goes to the page.

use std::sync::Arc;
use std::time::Instant;

use emberhut_api::browser::{BrowserControl, BrowserInfo, BrowserInput, StreamMode};
use emberhut_api::ids::{BrowserDeviceId, RoomId};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::{Value, json};

use super::{Link, Showing};
use crate::backend::fake::browser::session;
use crate::backend::fake::{Call, FakeBackend};
use crate::backend::{Error, FreshState, StateReply};
use crate::fixtures::state;
use crate::store::Store;
use crate::wire::ASSISTANT_DRIVER;

fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    cx.new(|_| Store::for_tests(backend))
}

fn take_browser(store: &Entity<Store>, cx: &mut TestAppContext, browser: &Value) {
    store.update(cx, |store, cx| {
        let reply = StateReply::Fresh(FreshState {
            state: Box::new(state(json!({ "browser": browser.clone() }))),
            etag: None,
        });
        store.apply(RoomId::MAIN, Ok(reply), Instant::now(), cx);
    });
}

fn browser_calls(backend: &FakeBackend) -> Vec<Call> {
    backend
        .calls()
        .into_iter()
        .filter(|call| {
            matches!(
                call,
                Call::BrowserSession(_) | Call::BrowserInput(_) | Call::BrowserUpload(..)
            )
        })
        .collect()
}

fn device(store: &Entity<Store>, cx: &mut TestAppContext) -> BrowserDeviceId {
    store.read_with(cx, |store, _| store.browser_device().clone())
}

fn mine(device: &BrowserDeviceId) -> BrowserInfo {
    BrowserInfo {
        control: BrowserControl::Person,
        driver: Some("You".to_owned()),
        device: Some(device.clone()),
        ..BrowserInfo::default()
    }
}

#[gpui_kit::test]
fn opening_asks_for_a_session_and_claims_a_browser_handed_over(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    take_browser(
        &store,
        cx,
        &json!({"control": "owner", "handoff": "Sign in", "device": ""}),
    );
    let here = device(&store, cx);
    backend.browser.push_input(Ok(mine(&here)));
    // The two asks may reach the server in either order: both get the same answer.
    backend
        .browser
        .push_session(Ok(session(StreamMode::Control)));
    backend
        .browser
        .push_session(Ok(session(StreamMode::Control)));
    store.update(cx, |store, cx| store.open_browser(Showing::Stream, cx));
    cx.run_until_parked();
    let calls = browser_calls(&backend);
    let takeover = Call::BrowserInput(BrowserInput::Takeover {
        device: here.clone(),
    });
    assert_eq!(calls.iter().filter(|call| **call == takeover).count(), 1);
    // One on opening, and one more once the takeover's answer made this device the driver.
    let sessions = Call::BrowserSession(here.clone());
    assert_eq!(calls.iter().filter(|call| **call == sessions).count(), 2);
    store.read_with(cx, |store, _| {
        assert!(store.browser_is_mine());
        assert!(matches!(store.browser_link(), Link::Ready(s) if s.is_control()));
    });
}

#[gpui_kit::test]
fn a_session_comes_again_only_when_control_changes(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    take_browser(&store, cx, &json!({"driver": ASSISTANT_DRIVER, "task": 12}));
    store.update(cx, |store, cx| store.open_browser(Showing::Stream, cx));
    cx.run_until_parked();
    take_browser(&store, cx, &json!({"driver": ASSISTANT_DRIVER, "task": 13}));
    cx.run_until_parked();
    assert_eq!(
        browser_calls(&backend).len(),
        1,
        "still watching: the same session"
    );
    let here = device(&store, cx);
    take_browser(
        &store,
        cx,
        &json!({"control": "owner", "driver": "You", "device": here.as_str()}),
    );
    cx.run_until_parked();
    assert_eq!(browser_calls(&backend).len(), 2, "now driving: a new one");
    store.update(cx, Store::close_browser);
    take_browser(&store, cx, &json!({"driver": ASSISTANT_DRIVER}));
    cx.run_until_parked();
    assert_eq!(browser_calls(&backend).len(), 2, "closed: none");
    store.read_with(cx, |store, _| {
        assert_eq!(store.browser_link(), &Link::Connecting);
    });
}

#[gpui_kit::test]
fn a_machine_without_a_live_view_says_so(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.browser.push_session(Err(Error::Refused {
        status: Some(503),
        message: Some("The live view isn't set up on this machine".to_owned()),
    }));
    let store = store(cx, &backend);
    take_browser(&store, cx, &json!({}));
    store.update(cx, |store, cx| store.open_browser(Showing::Stream, cx));
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.browser_link(), &Link::Unavailable);
    });
}

#[gpui_kit::test]
fn giving_back_sends_only_what_this_device_holds(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    take_browser(
        &store,
        cx,
        &json!({"control": "owner", "driver": "You", "device": "other"}),
    );
    store.update(cx, Store::give_back_browser);
    cx.run_until_parked();
    assert_eq!(browser_calls(&backend), [], "another device holds it");
    let here = device(&store, cx);
    take_browser(
        &store,
        cx,
        &json!({"control": "owner", "driver": "You", "device": here.as_str()}),
    );
    store.update(cx, Store::give_back_browser);
    cx.run_until_parked();
    assert_eq!(
        browser_calls(&backend),
        [Call::BrowserInput(BrowserInput::Handback)]
    );
    store.read_with(cx, |store, _| {
        assert!(
            !store.browser_is_mine(),
            "the answer is the browser's state at once"
        );
    });
}

#[gpui_kit::test]
fn a_refused_input_keeps_the_servers_words(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.browser.push_input(Err(Error::Refused {
        status: Some(409),
        message: Some("Take over first".to_owned()),
    }));
    let store = store(cx, &backend);
    take_browser(&store, cx, &json!({}));
    store.update(cx, |store, cx| store.browser_act(BrowserInput::Reload, cx));
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.browser_problem(), Some("Take over first"));
    });
}

#[gpui_kit::test]
fn a_file_goes_to_the_page_under_its_name(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scan.pdf");
    std::fs::write(&path, b"%PDF").unwrap();
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    take_browser(
        &store,
        cx,
        &json!({"fileRequest": {"multiple": false, "at": "2026-11-24T22:00:00Z"}}),
    );
    store.update(cx, |store, cx| store.send_to_browser(path, cx));
    store.read_with(cx, |store, _| {
        assert_eq!(store.browser_sending(), Some("scan.pdf"));
    });
    cx.run_until_parked();
    assert_eq!(
        browser_calls(&backend),
        [Call::BrowserUpload("scan.pdf".to_owned(), 4)]
    );
    store.read_with(cx, |store, _| {
        assert_eq!(store.browser_sending(), None);
        assert!(
            store.browser().unwrap().file_request.is_none(),
            "the page has it"
        );
    });
}

#[gpui_kit::test]
fn a_file_the_page_no_longer_wants_says_so(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("photo.jpg");
    std::fs::write(&path, b"\xFF\xD8").unwrap();
    let backend = Arc::new(FakeBackend::new([]));
    backend.browser.push_upload(Err(Error::Refused {
        status: Some(409),
        message: Some("The page isn't asking for a file".to_owned()),
    }));
    let store = store(cx, &backend);
    take_browser(&store, cx, &json!({}));
    store.update(cx, |store, cx| store.send_to_browser(path, cx));
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.browser_sending(), None);
        assert_eq!(
            store.browser_problem(),
            Some("The page isn't asking for a file")
        );
    });
}
