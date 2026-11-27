//! The sessions' store on GPUI's headless test platform with a scripted backend: a session
//! followed live and shared by its pages, a message, Stop and settings, the filter kept, the
//! new-session sheet's folders and branches, and a draft that opens its session.

use std::sync::Arc;
use std::time::{Duration, Instant};

use emberhut_api::ids::{RoomId, SessionId};
use emberhut_api::runner::actions::{Effort, SentToSession, SessionSettings};
use emberhut_api::runner::new_session::{BranchList, FolderList};
use emberhut_api::runner::steps::{SessionStep, StepKind};
use emberhut_api::runner::transcript::{SessionLive, Transcript};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::json;

use super::filter::{Choice, SessionFilter, Who};
use super::kept::SheetChoices;
use super::places::SessionPlace;
use super::sheet::{Listing, Setup};
use super::view::SessionRef;
use super::watch::SessionWatch;
use crate::backend::fake::{Call, FakeBackend};
use crate::backend::{Backend, Error};
use crate::config::AppFiles;
use crate::store::{Store, ThemeState};

fn store(
    cx: &mut TestAppContext,
    backend: &Arc<FakeBackend>,
    files: Option<AppFiles>,
) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn Backend>;
    cx.new(|_| Store::idle(backend, files, ThemeState::load(None), RoomId::MAIN))
}

fn transcript() -> Transcript {
    serde_json::from_value(json!({
        "session": {"id": "r-1", "title": "A session", "status": "running", "running": true,
            "meta": {"direct": true}},
        "events": [{"seq": 4, "turn": "r-1:1", "kind": "text", "text": "Hi"}],
        "earlier": false
    }))
    .unwrap()
}

/// Runs the app until `done` holds (the stream's thread sends on its own time), a second at
/// most.
fn wait_for(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        cx.run_until_parked();
        if done(cx) || Instant::now() > deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn reference() -> SessionRef {
    SessionRef::whole(SessionId::from("r-1"))
}

fn watch(cx: &mut TestAppContext, store: &Entity<Store>) -> Entity<SessionWatch> {
    let watch = store.update(cx, |store, cx| store.watch_session(&reference(), cx));
    wait_for(cx, |cx| {
        watch.read_with(cx, |w, _| w.view().is_loaded() && w.view().is_connected())
    });
    watch
}

#[gpui_kit::test]
fn a_session_loads_then_follows_live_and_is_shared_by_its_pages(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let backend = Arc::new(FakeBackend::new([]));
    backend.sessions.put_transcript(transcript());
    backend.sessions.push_stream(Ok(vec![SessionLive::Step {
        step: SessionStep::new(5, StepKind::Text),
    }]));
    let store = store(cx, &backend, None);
    let watch = watch(cx, &store);
    wait_for(cx, |cx| {
        watch.read_with(cx, |w, _| w.view().steps().len() == 2)
    });
    watch.read_with(cx, |watch, _| {
        assert_eq!(watch.view().steps().len(), 2);
        assert_eq!(watch.view().seen(), 5);
        assert!(watch.problem().is_none());
    });
    let again = store.update(cx, |store, cx| store.watch_session(&reference(), cx));
    assert_eq!(
        again.entity_id(),
        watch.entity_id(),
        "one stream for both pages"
    );
    assert!(backend.calls().contains(&Call::FollowSession(
        SessionId::from("r-1"),
        "after=4".into()
    )));
}

#[gpui_kit::test]
fn a_message_goes_and_one_that_doesnt_comes_back(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let backend = Arc::new(FakeBackend::new([]));
    backend.sessions.put_transcript(transcript());
    let store = store(cx, &backend, None);
    let watch = watch(cx, &store);
    backend.sessions.push_send(Ok(SentToSession {
        ok: true,
        turn: None,
        queued: true,
    }));
    watch.update(cx, |watch, cx| watch.send("  Carry on ", Vec::new(), cx));
    watch.read_with(cx, |watch, _| assert!(watch.is_sending()));
    cx.run_until_parked();
    watch.read_with(cx, |watch, _| {
        assert_eq!(watch.sent(), 1);
        assert!(!watch.is_sending());
    });
    backend.sessions.push_send(Err(Error::Refused {
        status: Some(409),
        message: Some("it's busy".into()),
    }));
    watch.update(cx, |watch, cx| watch.send("Again", Vec::new(), cx));
    cx.run_until_parked();
    watch.update(cx, |watch, _| {
        assert_eq!(watch.problem(), Some("Not sent: it's busy"));
        assert_eq!(
            watch.take_unsent().map(|(text, _)| text).as_deref(),
            Some("Again")
        );
        assert_eq!(watch.take_unsent(), None, "given back once");
    });
    watch.update(cx, |watch, cx| watch.send("   ", Vec::new(), cx));
    let sent: Vec<Call> = backend
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::SendToSession(..)))
        .collect();
    assert_eq!(
        sent,
        [
            Call::SendToSession(SessionId::from("r-1"), "Carry on".into()),
            Call::SendToSession(SessionId::from("r-1"), "Again".into())
        ],
        "a blank message isn't sent"
    );
}

#[gpui_kit::test]
fn stop_stop_this_turn_and_settings_say_when_they_fail(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let backend = Arc::new(FakeBackend::new([]));
    backend.sessions.put_transcript(transcript());
    let store = store(cx, &backend, None);
    let watch = watch(cx, &store);
    watch.update(cx, SessionWatch::interrupt);
    cx.run_until_parked();
    backend.sessions.push_action(Err(Error::Refused {
        status: Some(502),
        message: Some("the runner didn't answer".into()),
    }));
    watch.update(cx, SessionWatch::stop);
    cx.run_until_parked();
    watch.read_with(cx, |watch, _| {
        assert_eq!(
            watch.problem(),
            Some("Not stopped: the runner didn't answer")
        );
    });
    let settings = SessionSettings {
        model: None,
        effort: Some(Effort::Max),
    };
    watch.update(cx, |watch, cx| watch.change(settings.clone(), cx));
    cx.run_until_parked();
    watch.read_with(cx, |watch, _| assert_eq!(watch.problem(), None));
    let calls = backend.calls();
    let id = SessionId::from("r-1");
    assert!(calls.contains(&Call::InterruptSession(id.clone())));
    assert!(calls.contains(&Call::StopSession(id.clone())));
    assert!(calls.contains(&Call::ChangeSession(id, settings)));
}

#[gpui_kit::test]
fn the_filter_and_the_sheets_choices_are_kept(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let files = AppFiles::in_dir(dir.path());
    let backend = Arc::new(FakeBackend::new([]));
    let first = store(cx, &backend, Some(files.clone()));
    let mut filter = SessionFilter::default();
    filter.toggle(&Choice::Worker(Who::Direct));
    first.update(cx, |store, cx| store.set_session_filter(filter.clone(), cx));
    let mut setup = Setup::from_choices(&SheetChoices::default());
    setup.machine = "runner-b".into();
    setup.cwd = "/work".into();
    let key = first.update(cx, |store, cx| store.draft_session(setup.clone(), cx));
    first.read_with(cx, |store, _| {
        assert_eq!(
            store.session_place(key),
            Some(&SessionPlace::Draft(setup.clone()))
        );
        assert!(store.draft(key).is_some());
    });
    // The writes go off the UI thread: done before the next launch reads them.
    cx.run_until_parked();
    let second = store(cx, &backend, Some(files));
    second.read_with(cx, |store, _| assert_eq!(store.session_filter(), &filter));
    let sheet = second.update(cx, |store, cx| store.new_session_sheet(cx));
    sheet.read_with(cx, |sheet, _| {
        assert_eq!(
            sheet.setup().machine.as_str(),
            "runner-b",
            "the last machine"
        );
    });
}

#[gpui_kit::test]
fn the_sheet_fetches_folders_and_branches_for_its_machine(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let list: FolderList = serde_json::from_value(json!({
        "recent": [{"path": "/work/recent"}], "projects": [{"path": "/work/project", "name": "P"}]
    }))
    .unwrap();
    backend.sessions.push_folders(Ok(list));
    backend.sessions.push_branches(Ok(BranchList {
        repo: "/work/recent".into(),
        current: "main".into(),
        branches: vec!["main".into(), "fix".into()],
    }));
    let store = store(cx, &backend, None);
    let sheet = store.update(cx, |store, cx| store.new_session_sheet(cx));
    cx.run_until_parked();
    sheet.read_with(cx, |sheet, _| {
        assert_eq!(sheet.setup().cwd, "/work/recent", "the most recent folder");
        assert!(matches!(sheet.roots(), Listing::Ready(_)));
    });
    sheet.update(cx, |sheet, cx| sheet.set_worktree(true, cx));
    cx.run_until_parked();
    sheet.read_with(cx, |sheet, _| {
        assert_eq!(sheet.branches().map(|b| b.branches.len()), Some(2));
    });
    sheet.update(cx, |sheet, cx| sheet.set_machine("runner-b", cx));
    cx.run_until_parked();
    sheet.read_with(cx, |sheet, _| {
        assert_eq!(
            sheet.setup().cwd,
            "",
            "a new machine starts its folder over"
        );
        assert!(sheet.branches().is_none());
    });
    let asked: Vec<Call> = backend
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Folders(_) | Call::Branches(_)))
        .collect();
    assert_eq!(
        asked,
        [
            Call::Folders("machine=runner".into()),
            Call::Branches("machine=runner&cwd=%2Fwork%2Frecent".into()),
            Call::Folders("machine=runner-b".into()),
        ]
    );
}

#[gpui_kit::test]
fn a_drafts_first_message_opens_its_session(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend, None);
    let mut setup = Setup::from_choices(&SheetChoices::default());
    setup.cwd = "/work".into();
    let key = store.update(cx, |store, cx| store.draft_session(setup, cx));
    let draft = store.read_with(cx, |store, _| store.draft(key)).unwrap();
    backend.sessions.push_open(Err(Error::Refused {
        status: Some(409),
        message: Some("no such folder".into()),
    }));
    draft.update(cx, |draft, cx| draft.open("Fix it", Vec::new(), cx));
    cx.run_until_parked();
    draft.update(cx, |draft, _| {
        assert_eq!(draft.problem(), Some("Not opened: no such folder"));
        assert_eq!(
            draft.take_unsent().map(|(t, _)| t).as_deref(),
            Some("Fix it")
        );
    });
    draft.update(cx, |draft, cx| draft.open("Fix it", Vec::new(), cx));
    cx.run_until_parked();
    draft.read_with(cx, |draft, _| {
        assert_eq!(draft.opened(), Some(&SessionId::from("r-new")));
        assert!(!draft.is_opening());
    });
    assert!(backend.calls().iter().any(
        |call| matches!(call, Call::OpenSession(open) if open.cwd == "/work" && open.text == "Fix it")
    ));
}

#[gpui_kit::test]
fn a_t3_threads_steps_are_asked_for(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.sessions.push_t3(Ok(serde_json::from_value(json!({
        "title": "Old", "status": "running", "items": [{"type": "reasoning", "text": "Hmm"}]
    }))
    .unwrap()));
    let store = store(cx, &backend, None);
    let watch = store.update(cx, |store, cx| {
        store.watch_t3("t3-mac".into(), "t-1".into(), cx)
    });
    cx.run_until_parked();
    watch.read_with(cx, |watch, _| {
        assert!(watch.has_asked());
        assert_eq!(watch.activity().map(|a| a.items.len()), Some(1));
        assert_eq!(
            (watch.machine().as_str(), watch.thread()),
            ("t3-mac", "t-1")
        );
    });
    assert!(
        backend
            .calls()
            .contains(&Call::Thread("machine=t3-mac&thread=t-1".into()))
    );
}
