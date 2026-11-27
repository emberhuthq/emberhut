//! Where the assistant works, in the store, on GPUI's headless test platform with a scripted
//! backend: when the environments and the jobs are asked for, what an action does, the usage
//! checks, and picking the assistant's account.

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::environments::{EnvActionKind, EnvActionRequest, EnvPage};
use emberhut_api::ids::{EnvironmentId, MachineJobId, RoomId};
use emberhut_api::jobs::{JobAction, JobsPage};
use emberhut_api::usage::UseAccountRequest;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::json;

use crate::backend::Error;
use crate::backend::fake::{Call, FakeBackend};
use crate::store::{Page, Store, ThemeState};

fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
}

/// The assistant's account, on account `id` and set to Auto.
fn assistant_on(id: &str) -> emberhut_api::usage::AssistantAccount {
    emberhut_api::usage::AssistantAccount {
        on: Some(id.to_owned()),
        pin: None,
        mode: "auto".to_owned(),
        pin_out: false,
        rooms: Vec::new(),
        waits: None,
    }
}

fn env_page(note: Option<&str>) -> EnvPage {
    serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z",
        "summary": {"ok": 1, "warn": 0, "down": 0},
        "groups": [{"id": "connections", "title": "Connections", "footer": null, "items": [
            {"id": "gmail:work", "group": "connections", "name": "Mail", "icon": "envelope.fill",
             "tint": "red", "state": "ok", "status": "Fine", "detail": "", "at": null,
             "error": null, "hint": null, "facts": [], "lists": [],
             "actions": [{"id": "link", "label": "Reconnect"},
                         {"id": "unlink", "label": "Unlink"}],
             "lastError": null}]}],
        "link": "https://example.com/once", "note": note}))
    .unwrap()
}

fn jobs_page() -> JobsPage {
    serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z", "mac": {"ok": true},
        "jobs": [{"id": "unit:a", "kind": "unit", "place": "Linux PC", "name": "a",
                  "title": "A run", "what": "Codex run", "task": null, "state": "running",
                  "stop": true, "clear": false}]}))
    .unwrap()
}

fn count(backend: &FakeBackend, wanted: &Call) -> usize {
    backend
        .calls()
        .iter()
        .filter(|call| *call == wanted)
        .count()
}

#[gpui_kit::test]
fn the_environments_are_asked_for_while_settings_or_their_page_shows(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    for _ in 0..4 {
        backend.working.push_environments(Ok(env_page(None)));
    }
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.window.active = true;
        store.start_environments(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        count(&backend, &Call::Environments),
        0,
        "nothing shows them"
    );
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Settings), cx);
    });
    cx.run_until_parked();
    assert_eq!(count(&backend, &Call::Environments), 1, "Settings opened");
    let key = store.update(cx, |store, cx| {
        let key = store.environment_key(EnvironmentId::new("gmail:work"));
        store.show_in_test_column(Some(Page::Environment(key)), cx);
        key
    });
    cx.run_until_parked();
    assert_eq!(count(&backend, &Call::Environments), 2, "its page opened");
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(count(&backend, &Call::Environments), 3, "every ten seconds");
    store.read_with(cx, |store, _| {
        let feed = store.environments();
        assert_eq!(
            feed.id_of(key).map(EnvironmentId::as_str),
            Some("gmail:work")
        );
    });
    store.update(cx, |store, cx| store.show_in_test_column(None, cx));
    cx.executor().advance_clock(Duration::from_mins(2));
    cx.run_until_parked();
    assert_eq!(count(&backend, &Call::Environments), 3, "shut: no more");
}

#[gpui_kit::test]
fn an_action_keeps_what_it_made_and_unlink_asks_first(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend
        .working
        .push_environment_action(Ok(env_page(Some("Open the link."))));
    backend.working.push_environment_action(Err(Error::Refused {
        status: Some(400),
        message: Some("No token for this workspace".into()),
    }));
    let store = store(cx, &backend);
    let mail = EnvironmentId::new("gmail:work");
    store.update(cx, |store, cx| {
        store.act_on_environment(mail.clone(), EnvActionKind::Link, cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let outcome = store.environments().outcome(&mail).unwrap();
        assert_eq!(outcome.link.as_deref(), Some("https://example.com/once"));
        assert_eq!(outcome.note.as_deref(), Some("Open the link."));
        assert!(store.environments().page().is_some());
    });
    store.update(cx, |store, cx| store.ask_unlink(Some(mail.clone()), cx));
    assert!(store.read_with(cx, |store, _| store.environments().is_unlinking(&mail)));
    store.update(cx, |store, cx| {
        store.act_on_environment(mail.clone(), EnvActionKind::Unlink, cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let feed = store.environments();
        assert!(!feed.is_unlinking(&mail));
        assert_eq!(feed.problem(), Some("No token for this workspace"));
        assert_eq!(feed.busy(), None);
    });
    assert_eq!(
        backend.calls().last(),
        Some(&Call::EnvironmentAction(EnvActionRequest {
            id: mail,
            kind: EnvActionKind::Unlink,
        }))
    );
}

#[gpui_kit::test]
fn usage_is_checked_at_launch_and_on_check_now(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, Store::start_usage_checks);
    cx.run_until_parked();
    assert_eq!(count(&backend, &Call::CheckUsage), 1, "at launch");
    store.update(cx, |store, _| store.check_usage_now());
    cx.run_until_parked();
    assert_eq!(count(&backend, &Call::CheckUsage), 2, "Check now");
    store.update(cx, |store, _| store.working_forward());
    cx.run_until_parked();
    assert_eq!(count(&backend, &Call::CheckUsage), 3, "back to the front");
}

#[gpui_kit::test]
fn a_picked_account_shows_at_once_and_a_refusal_says_so(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        let mut state = crate::fixtures::state(json!({}));
        state.assistant_account = Some(assistant_on("a-1"));
        store.put_state_for_tests(state);
        store.use_account(UseAccountRequest::pin("a-2"), cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let account = store.state().unwrap().assistant_account.as_ref().unwrap();
        assert_eq!(account.pin.as_deref(), Some("a-2"));
        assert!(!store.usage_card().is_picking());
    });
    backend.working.push_use_account(Err(Error::Refused {
        status: Some(400),
        message: Some("no such account".into()),
    }));
    store.update(cx, |store, cx| {
        store.use_account(UseAccountRequest::auto(), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        store.read_with(cx, |store, _| store.action_problem().map(str::to_owned)),
        Some("Account not switched: no such account".to_owned())
    );
    assert_eq!(
        backend.calls().last(),
        Some(&Call::UseAccount(UseAccountRequest::auto()))
    );
}

#[gpui_kit::test]
fn the_jobs_follow_their_page_and_a_stop_reads_them_fresh(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    for _ in 0..3 {
        backend.working.push_jobs(Ok(jobs_page()));
    }
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.window.active = true;
        store.start_jobs(cx);
    });
    cx.run_until_parked();
    assert_eq!(count(&backend, &Call::Jobs(false)), 0, "nothing shows them");
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Jobs), cx);
    });
    cx.run_until_parked();
    assert_eq!(count(&backend, &Call::Jobs(false)), 1, "the list opened");
    let id = MachineJobId::new("unit:a");
    backend.working.push_job_action(Err(Error::Refused {
        status: Some(400),
        message: Some("that job isn't running anymore".into()),
    }));
    store.update(cx, |store, cx| {
        store.confirm_job(Some((id.clone(), JobAction::Stop)), cx);
        store.act_on_job(id.clone(), JobAction::Stop, cx);
    });
    cx.run_until_parked();
    assert!(
        backend
            .calls()
            .contains(&Call::JobAction(JobAction::Stop, id.clone()))
    );
    assert_eq!(
        count(&backend, &Call::Jobs(true)),
        1,
        "fresh after the stop"
    );
    store.read_with(cx, |store, _| {
        let feed = store.jobs();
        assert_eq!(feed.refused(&id), Some("that job isn't running anymore"));
        assert_eq!(feed.confirming(&id), None);
        assert_eq!(feed.busy(), None);
    });
}
