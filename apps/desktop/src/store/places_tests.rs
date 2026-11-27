//! The spotlight's and the places' glue on GPUI's headless test platform with a scripted
//! backend: what each asks the server and when, and what the user's changes send.

use std::sync::Arc;

use emberhut_api::facts::FactsPage;
use emberhut_api::ids::{ArtifactId, ProjectId, RoomId};
use emberhut_api::settings::{ProjectChange, SettingsAction};
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowId};
use serde_json::json;

use super::projects::change_of;
use super::spotlight::Found;
use super::tasks::search::TYPING_PAUSE;
use super::{Page, Store, ThemeState};
use crate::backend::fake::{Call, FakeBackend};
use crate::fixtures::issue;

fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
}

fn calls(backend: &FakeBackend) -> Vec<Call> {
    backend
        .calls()
        .into_iter()
        .filter(|call| !matches!(call, Call::State(..) | Call::File(_)))
        .collect()
}

fn people() -> FactsPage {
    serde_json::from_value(json!({"facts": [
        {"id": "people/a", "category": "people", "title": "Example Person", "text": "Runs the bakery",
         "source": "", "updated": "2026-11-20T10:00:00Z"},
        {"id": "work/b", "category": "work", "title": "Bakery hours", "text": "",
         "source": "", "updated": "2026-11-20T10:00:00Z"}
    ]}))
    .unwrap()
}

#[gpui_kit::test]
fn the_spotlight_lists_the_latest_then_asks_once_the_typing_pauses(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.replies.push_facts(Ok(people()));
    backend.push_found(Ok(vec![issue(1, "todo", json!({}))]));
    backend.push_found(Ok(vec![
        issue(2, "done", json!({})),
        issue(3, "todo", json!({})),
    ]));
    let store = store(cx, &backend);
    store.update(cx, Store::open_spotlight);
    cx.run_until_parked();
    let asked = calls(&backend);
    assert!(
        asked.contains(&Call::FindTasks("q=&limit=12".into())),
        "{asked:?}"
    );
    assert!(asked.contains(&Call::Facts));
    store.update(cx, |store, cx| {
        store.spotlight_type("bak", cx);
        store.spotlight_type("bakery", cx);
    });
    cx.executor().advance_clock(TYPING_PAUSE);
    cx.run_until_parked();
    let finds: Vec<Call> = calls(&backend)
        .into_iter()
        .filter(|call| matches!(call, Call::FindTasks(_)))
        .collect();
    assert_eq!(
        finds,
        [
            Call::FindTasks("q=&limit=12".into()),
            Call::FindTasks("q=bakery&limit=12".into())
        ],
        "only the last words are asked for"
    );
    store.read_with(cx, |store, _| {
        let results = store
            .spotlight()
            .results(store.state(), store.places().people());
        assert_eq!(
            results.people.len(),
            1,
            "people only, and their note matches"
        );
        assert_eq!(
            results.order.last(),
            Some(&Found::Task(emberhut_api::ids::TaskId::new(2))),
            "a done task after an open one"
        );
    });
}

#[gpui_kit::test]
fn a_projects_page_asks_for_its_done_tasks_when_shown(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.push_found(Ok(vec![issue(5, "done", json!({"pid": 2}))]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.column_changed(
            WindowId::from(1),
            vec![Page::Project(ProjectId::new(2))],
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        calls(&backend),
        [Call::FindTasks("q=&limit=200&project=2&closed=1".into())]
    );
    store.read_with(cx, |store, _| {
        let found = store
            .places()
            .project_done(ProjectId::new(2))
            .unwrap()
            .found();
        assert!(matches!(found, super::tasks::search::Found::Tasks(tasks) if tasks.len() == 1));
    });
}

#[gpui_kit::test]
fn a_persons_page_finds_the_tasks_naming_them(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.replies.push_facts(Ok(people()));
    let store = store(cx, &backend);
    let key = store.update(cx, |store, _| {
        store.person_key(&emberhut_api::ids::FactId::from("people/a"))
    });
    store.update(cx, |store, cx| {
        store.column_changed(WindowId::from(1), vec![Page::Person(key)], cx);
    });
    cx.run_until_parked();
    assert_eq!(
        calls(&backend),
        [Call::Facts, Call::FindTasks("q=Example&limit=20".into())]
    );
    store.read_with(cx, |store, _| {
        assert_eq!(
            store.person(key).map(|person| person.title.as_str()),
            Some("Example Person")
        );
    });
}

#[gpui_kit::test]
fn posted_replies_and_project_changes_are_sent_at_once(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    let change = ProjectChange {
        priority: Some(2),
        ..change_of(ProjectId::new(2))
    };
    store.update(cx, |store, cx| {
        store.mark_posted(ArtifactId::new(8), 1, cx);
        store.change_project(change.clone(), cx);
    });
    cx.run_until_parked();
    let sent = calls(&backend);
    assert!(sent.contains(&Call::Settings(SettingsAction::XPosted {
        artifact: ArtifactId::new(8),
        option: 1
    })));
    assert!(sent.contains(&Call::Settings(SettingsAction::ProjectSet(change))));
}
