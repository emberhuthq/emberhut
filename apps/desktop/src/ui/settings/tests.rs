//! Settings' view tests: the front door, the group pages and the pages their rows open.

use std::sync::Arc;

use emberhut_api::agent_provider::{
    AgentProvider, ProviderKind, ProviderModel, ProviderRequest, ProviderTest,
};
use emberhut_api::chrome::ChromeAction;
use emberhut_api::facts::{FactCategory, FactsPage};
use emberhut_api::ids::{ChromeProfileId, FactId, TrialId};
use emberhut_api::settings::{FactChange, FactProfile, SettingsAction};
use emberhut_api::switches::SwitchAction;
use gpui_kit::component::input::InputState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Focusable as _, TestAppContext, VisualTestContext, px};
use serde_json::json;

use super::*;
use crate::backend::Error;
use crate::backend::fake::{Call, FakeBackend};
use crate::store::settings::chrome::ChromeSheet;
use crate::store::settings::entries::Fold;
use crate::ui::report::ReportSize;
use crate::ui::window::MainWindow;

/// What a test window shows: Settings' front door, or one of its pages as the column shows
/// it.
struct Shown {
    view: Entity<SettingsView>,
    pane: Option<SettingsPane>,
}

impl Render for Shown {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let scale = window.scale_factor();
        match self.pane {
            None => self.view.clone().into_any_element(),
            Some(pane) => self.view.update(cx, |view, cx| view.pane(pane, scale, cx)),
        }
    }
}

/// Shows `pane` (or the front door) in the test window instead.
fn show(pane: Option<SettingsPane>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let shown = window.root::<Shown>().flatten().expect("a Settings window");
        shown.update(cx, |shown, cx| {
            shown.pane = pane;
            cx.notify();
        });
        window.render_frame(cx);
    });
}

/// Settings (or one of its pages, `pane`) over a fake backend, in a headless window, and
/// its view.
fn open_settings_view(
    cx: &mut TestAppContext,
    pane: Option<SettingsPane>,
) -> (
    Arc<FakeBackend>,
    Entity<Store>,
    Entity<SettingsView>,
    VisualTestContext,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
        parts::bind_keys(cx);
    });
    let backend = Arc::new(FakeBackend::new([]));
    let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
    let store = cx.new(|_| Store::for_tests(shared));
    let window = cx.add_window(|window, cx| {
        let pairing = cx.new(|cx| PairingForm::new(store.clone(), window, cx));
        let view = cx.new(|cx| SettingsView::new(store.clone(), pairing, window, cx));
        Shown { view, pane }
    });
    let view = window
        .root(cx)
        .unwrap()
        .read_with(cx, |shown, _| shown.view.clone());
    let cx = VisualTestContext::from_window(window.into(), cx);
    (backend, store, view, cx)
}

/// Empties `field`, gives it the focus and types `text` in it, as the user would.
fn type_in(field: &Entity<InputState>, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        field.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        window.render_frame(cx);
    });
    cx.simulate_input(text);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_skills_switch_sends_its_change_from_under_connections_fold(cx: &mut TestAppContext) {
    let (backend, store, view, mut cx) = open_settings_view(cx, Some(SettingsPane::Connections));
    let page = serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z", "live": true,
        "mcp": [], "skills": [{"name": "notes", "description": "Takes notes.",
        "source": "yours", "on": true}]}))
    .unwrap();
    backend.replies.push_switches(Ok(page));
    store.update(&mut cx, |store, cx| {
        store.switch(SwitchAction::Reconnect { name: "x".into() }, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("skill:notes-switch").is_none(), "folded");
        window.click(Fold::Skills.id(), cx);
    });
    assert!(view.read_with(&cx, |view, _| view.folds.contains(&Fold::Skills)));
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("skill:notes-switch", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        backend.calls().last(),
        Some(&Call::Switch(SwitchAction::Skill {
            name: "notes".into(),
            on: false
        }))
    );
}

#[gpui_kit::test]
fn return_is_done_on_a_chrome_profiles_sheet(cx: &mut TestAppContext) {
    let (_backend, store, _, mut cx) = open_settings_view(cx, Some(SettingsPane::ChromeProfiles));
    store.update(&mut cx, |store, cx| {
        store.open_chrome_profile(ChromeProfileId::new(2), cx);
    });
    cx.run_until_parked();
    cx.update(Window::render_frame);
    assert_eq!(
        store.read_with(&cx, |store, _| store.chrome().sheet().cloned()),
        Some(ChromeSheet::Profile(ChromeProfileId::new(2)))
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        store.read_with(&cx, |store, _| store.chrome().sheet().cloned()),
        None
    );
}

#[gpui_kit::test]
fn return_in_the_name_field_renames_and_a_refusal_shows_on_the_sheet(cx: &mut TestAppContext) {
    let (backend, store, view, mut cx) = open_settings_view(cx, Some(SettingsPane::ChromeProfiles));
    let id = ChromeProfileId::new(2);
    store.update(&mut cx, |store, cx| {
        store.put_state_for_tests(crate::fixtures::state(json!({"chrome": [{"id": 2,
            "name": "work", "label": "Work", "connected": true,
            "created": "2026-11-01T09:00:00Z"}]})));
        store.open_chrome_profile(id, cx);
    });
    cx.run_until_parked();
    cx.update(Window::render_frame);
    backend.replies.push_chrome(Err(Error::Refused {
        status: Some(400),
        message: Some("There's already a Chrome profile called Laptop".into()),
    }));
    let field = view.read_with(&cx, |view, cx| view.chrome.read(cx).rename.clone());
    type_in(&field, "Laptop", &mut cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(Window::render_frame);
    assert_eq!(
        backend.calls().last(),
        Some(&Call::Chrome(ChromeAction::Rename {
            id,
            label: "Laptop".into()
        }))
    );
    store.read_with(&cx, |store, _| {
        assert_eq!(
            store.chrome().sheet(),
            Some(&ChromeSheet::Profile(id)),
            "the sheet stays open"
        );
        assert_eq!(
            store.chrome().problem(),
            Some("There's already a Chrome profile called Laptop")
        );
    });
    let kept = field.read_with(&cx, |input, _| input.value().to_string());
    assert_eq!(kept, "Work", "the field goes back to the name kept");
}

#[gpui_kit::test]
fn return_in_fix_a_fact_saves_the_correction(cx: &mut TestAppContext) {
    let (backend, store, view, mut cx) = open_settings_view(cx, Some(SettingsPane::AboutYou));
    let page: FactsPage = serde_json::from_value(json!({"facts": [
        {"id": "work/desk", "category": "work", "title": "Desk", "text": "Works from home.",
         "source": "chat", "updated": "2026-11-20T10:00:00Z"}]}))
    .unwrap();
    backend.replies.push_facts(Ok(page));
    store.update(&mut cx, |store, cx| {
        store.show_in_test_column(Some(Page::SettingsPane(SettingsPane::AboutYou)), cx);
    });
    cx.run_until_parked();
    let desk = FactId::new("work/desk");
    let about = view.read_with(&cx, |view, _| view.about.clone());
    cx.update(|window, cx| {
        let fact = store.read(cx).facts().fact(&desk).cloned().unwrap();
        about.update(cx, |about, cx| about.edit(&fact, window, cx));
        window.render_frame(cx);
    });
    cx.run_until_parked();
    let field = about.read_with(&cx, |about, _| about.text.clone());
    type_in(&field, "Works from the office.", &mut cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        backend.calls(),
        [
            Call::Facts,
            Call::Settings(SettingsAction::FactSet(FactChange {
                id: desk,
                category: FactCategory::Work,
                title: "Desk".into(),
                text: "Works from the office.".into(),
            })),
            Call::Facts,
        ],
        "saved, then read again for what the server keeps"
    );
}

/// What your ember knows about you over a fake backend that answers with `page`, drawn.
fn open_about(
    cx: &mut TestAppContext,
    page: serde_json::Value,
) -> (
    Arc<FakeBackend>,
    Entity<Store>,
    Entity<SettingsView>,
    VisualTestContext,
) {
    let (backend, store, view, mut cx) = open_settings_view(cx, Some(SettingsPane::AboutYou));
    backend
        .replies
        .push_facts(Ok(serde_json::from_value(page).unwrap()));
    store.update(&mut cx, |store, cx| {
        store.show_in_test_column(Some(Page::SettingsPane(SettingsPane::AboutYou)), cx);
    });
    cx.run_until_parked();
    cx.update(Window::render_frame);
    (backend, store, view, cx)
}

/// Two facts in two categories and a lesson on trial, with its line to the user.
fn known() -> serde_json::Value {
    json!({
        "facts": [
            {"id": "people/example", "category": "people", "title": "Example Person",
             "text": "Runs the example team.", "source": "chat",
             "updated": "2026-11-20T10:00:00Z", "important": true},
            {"id": "work/desk", "category": "work", "title": "Desk",
             "text": "Works from home.", "source": "chat", "updated": "2026-11-20T10:00:00Z"}],
        "trials": [{"id": 4, "text": "Before telling the owner a file changed, say which.",
                    "at": "2026-11-20T10:00:00Z", "ends": "2026-12-04T10:00:00Z",
                    "said": "Which file was that?",
                    "lesson": "I say which file a change is in."}]})
}

#[gpui_kit::test]
fn learning_from_you_is_one_folded_line_over_the_chips_and_facts(cx: &mut TestAppContext) {
    let (_backend, store, _, mut cx) = open_about(cx, known());
    let learning = cx
        .debug_bounds(about::LEARNING)
        .expect("Learning from you is drawn");
    let chips = cx.debug_bounds(about::CHIPS).expect("the chips are drawn");
    let fact = cx.debug_bounds(about::FACT_ROW).expect("a fact is drawn");
    assert!(
        learning.bottom() <= chips.top() && chips.bottom() <= fact.top(),
        "lessons, then chips, then facts: {learning:?} {chips:?} {fact:?}"
    );
    assert!(
        cx.debug_bounds(about::TRIAL_ROW).is_none(),
        "folded at first"
    );
    cx.update(|window, cx| window.click(about::LEARNING_TOGGLE, cx));
    cx.run_until_parked();
    cx.update(Window::render_frame);
    assert!(cx.debug_bounds(about::TRIAL_ROW).is_some(), "opened");
    assert!(store.read_with(&cx, |store, _| store.facts().learning_open()));
}

#[gpui_kit::test]
fn keep_keeps_the_lesson_now(cx: &mut TestAppContext) {
    let (backend, store, _, mut cx) = open_about(cx, known());
    store.update(&mut cx, Store::toggle_learning);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("trial-4-keep", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        backend.calls(),
        [
            Call::Facts,
            Call::Settings(SettingsAction::TrialKeep(TrialId::new(4))),
            Call::Facts,
        ]
    );
}

#[gpui_kit::test]
fn drop_asks_once_before_it_drops(cx: &mut TestAppContext) {
    let (backend, store, _, mut cx) = open_about(cx, known());
    store.update(&mut cx, Store::toggle_learning);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("trial-4-drop", cx);
    });
    cx.run_until_parked();
    assert_eq!(backend.calls(), [Call::Facts], "asked, not sent");
    cx.update(|window, cx| window.click("trial-4-confirm", cx));
    cx.run_until_parked();
    assert_eq!(
        backend.calls()[1],
        Call::Settings(SettingsAction::TrialDrop(TrialId::new(4)))
    );
}

#[gpui_kit::test]
fn a_facts_edit_and_forget_buttons_act(cx: &mut TestAppContext) {
    let (backend, _store, view, mut cx) = open_about(cx, known());
    assert!(cx.debug_bounds(about::FACT_EDIT).is_some(), "Edit shows");
    assert!(
        cx.debug_bounds(about::FACT_FORGET).is_some(),
        "Forget shows"
    );
    cx.update(|window, cx| window.click("fact-work/desk-edit", cx));
    cx.run_until_parked();
    let about = view.read_with(&cx, |view, _| view.about.clone());
    let field = about.read_with(&cx, |about, _| about.text.clone());
    assert_eq!(
        field.read_with(&cx, |input, _| input.value().to_string()),
        "Works from home.",
        "Fix a fact opens on it"
    );
    cx.update(|window, cx| window.click("sheet-done", cx));
    cx.run_until_parked();

    cx.update(|window, cx| window.click("fact-work/desk-forget", cx));
    cx.run_until_parked();
    assert_eq!(backend.calls(), [Call::Facts], "asked, not sent");
    cx.update(|window, cx| window.click("fact-work/desk-forget-confirm", cx));
    cx.run_until_parked();
    assert_eq!(
        backend.calls()[1],
        Call::Settings(SettingsAction::FactDrop(FactId::new("work/desk")))
    );
}

/// A fact kept from someone else's message, waiting to be confirmed, and one the user told,
/// pinned.
fn waiting() -> serde_json::Value {
    json!({"facts": [
        {"id": "work/launch", "category": "work", "title": "Launch", "text": "Ships in May.",
         "source": "", "updated": "2026-11-21T10:00:00Z", "origin": "message",
         "from": "chat #40", "pending": true},
        {"id": "work/desk", "category": "work", "title": "Desk", "text": "Works from home.",
         "source": "", "updated": "2026-11-20T10:00:00Z", "origin": "assistant",
         "from": "chat #22", "profile": "pin"}]})
}

#[gpui_kit::test]
fn facts_to_confirm_come_first_apart_and_confirm_sends(cx: &mut TestAppContext) {
    let (backend, _store, _, mut cx) = open_about(cx, waiting());
    let to_confirm = cx
        .debug_bounds(about::TO_CONFIRM)
        .expect("To confirm is drawn");
    let chips = cx.debug_bounds(about::CHIPS).expect("the chips are drawn");
    assert!(
        to_confirm.bottom() <= chips.top(),
        "To confirm first: {to_confirm:?} {chips:?}"
    );
    cx.update(|window, _| {
        assert!(window.try_find("fact-work/launch-confirm").is_some());
        assert!(
            window.try_find("fact-work/launch-edit").is_none(),
            "not listed again with the others"
        );
        assert!(window.try_find("fact-work/desk-edit").is_some());
        assert!(window.try_find("fact-work/desk-confirm").is_none());
    });
    cx.update(|window, cx| window.click("fact-work/launch-confirm", cx));
    cx.run_until_parked();
    assert_eq!(
        backend.calls()[1],
        Call::Settings(SettingsAction::FactConfirm(FactId::new("work/launch")))
    );
}

#[gpui_kit::test]
fn a_fact_to_confirm_is_forgotten_once_asked(cx: &mut TestAppContext) {
    let (backend, _store, _, mut cx) = open_about(cx, waiting());
    cx.update(|window, cx| window.click("fact-work/launch-forget", cx));
    cx.run_until_parked();
    assert_eq!(backend.calls(), [Call::Facts], "asked, not sent");
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("fact-work/launch-forget-confirm", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        backend.calls()[1],
        Call::Settings(SettingsAction::FactDrop(FactId::new("work/launch")))
    );
}

#[gpui_kit::test]
fn why_i_know_it_shows_under_a_fact_that_says(cx: &mut TestAppContext) {
    let (_backend, _store, _, mut cx) = open_about(cx, waiting());
    let why = cx.debug_bounds(about::FACT_WHY).expect("the line is drawn");
    assert!(why.size.height > px(0.));
}

#[gpui_kit::test]
fn no_why_line_shows_when_the_server_keeps_neither(cx: &mut TestAppContext) {
    let (_backend, _store, _, mut cx) = open_about(cx, known());
    assert!(cx.debug_bounds(about::FACT_WHY).is_none());
}

#[gpui_kit::test]
fn the_pin_menu_says_how_a_fact_stands_and_changes_it(cx: &mut TestAppContext) {
    let (backend, _store, _, mut cx) = open_about(cx, waiting());
    assert!(cx.debug_bounds(about::FACT_PROFILE).is_some(), "drawn");
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("fact-work/desk-profile", cx);
    });
    let items = cx.update(|window, cx| {
        window.render_frame(cx);
        let menu = window.within("popup-menu");
        (0_usize..6)
            .filter_map(|ix| menu.try_find(gpui_kit::ElementId::from(ix)))
            .filter_map(|item| item.label().map(str::to_owned))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        items,
        ["Always keep in mind", "Keep out of mind", "As it comes"]
    );
    // Keep out of mind.
    cx.update(|window, cx| window.within("popup-menu").click(1_usize, cx));
    cx.run_until_parked();
    assert_eq!(
        backend.calls()[1],
        Call::Settings(SettingsAction::FactProfile(FactProfile {
            id: FactId::new("work/desk"),
            profile: Some("hide".into()),
        }))
    );
}

#[gpui_kit::test]
fn the_nightly_review_and_the_export_act_from_the_page(cx: &mut TestAppContext) {
    let (downloads, kept) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (backend, store, _, mut cx) = open_about(cx, known());
    let shown = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    store.update(&mut cx, |store, _| {
        store.keep_files_in(kept.path(), &shown);
        store.export_memory_to(downloads.path().to_path_buf());
    });
    assert!(
        cx.debug_bounds(about::NIGHTLY).is_some(),
        "the switch is drawn"
    );
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click(about::NIGHTLY, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        backend.calls()[1],
        Call::Settings(SettingsAction::MemoryNightly(false)),
        "on to begin with: switched off"
    );
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click(about::EXPORT, cx);
    });
    cx.run_until_parked();
    assert!(backend.calls().contains(&Call::MemoryExport));
    assert_eq!(shown.borrow().len(), 1, "shown in Downloads");
    assert_eq!(std::fs::read_dir(downloads.path()).unwrap().count(), 1);
}

#[gpui_kit::test]
fn a_chip_narrows_the_list_to_its_category(cx: &mut TestAppContext) {
    let (_backend, _store, _, mut cx) = open_about(cx, known());
    let shown = |cx: &mut VisualTestContext| {
        cx.update(|window, _| {
            (
                window.try_find("fact-people/example-edit").is_some(),
                window.try_find("fact-work/desk-edit").is_some(),
            )
        })
    };
    assert_eq!(shown(&mut cx), (true, true), "All");
    // All, People, Work.
    cx.update(|window, cx| window.click("about-chip-2", cx));
    cx.run_until_parked();
    cx.update(Window::render_frame);
    assert_eq!(shown(&mut cx), (false, true), "Work only");
    cx.update(|window, cx| window.click("about-chip-0", cx));
    cx.run_until_parked();
    cx.update(Window::render_frame);
    assert_eq!(shown(&mut cx), (true, true), "All again");
}

#[gpui_kit::test]
fn long_rules_and_facts_wrap_inside_a_narrow_window(cx: &mut TestAppContext) {
    let long = "A scan or review is done only when every finding is marked landed or \
                superseded after a check in the code, not when its session says done. "
        .repeat(3);
    let (_backend, store, _, mut cx) = open_about(
        cx,
        json!({
            "facts": [{"id": "work/desk", "category": "work", "title": long, "text": long,
                       "source": "chat", "updated": "2026-11-20T10:00:00Z"}],
            "trials": [{"id": 4, "text": long, "at": "2026-11-20T10:00:00Z",
                        "ends": "2026-12-04T10:00:00Z", "said": long, "lesson": long}]}),
    );
    store.update(&mut cx, Store::toggle_learning);
    cx.simulate_resize(gpui_kit::size(px(560.), px(900.)));
    cx.update(Window::render_frame);
    let edge = px(560.);
    let row = cx
        .debug_bounds(about::TRIAL_ROW)
        .expect("the lesson is drawn");
    let drop = cx
        .debug_bounds(about::TRIAL_DROP)
        .expect("its Drop is drawn");
    let keep = cx
        .debug_bounds(about::TRIAL_KEEP)
        .expect("its Keep is drawn");
    let fact = cx.debug_bounds(about::FACT_ROW).expect("the fact is drawn");
    let forget = cx
        .debug_bounds(about::FACT_FORGET)
        .expect("its Forget is drawn");
    assert!(
        row.right() <= edge,
        "the lesson wraps inside the window: {row:?}"
    );
    for button in [drop, keep] {
        assert!(
            button.right() <= row.right(),
            "the button stays in its row: {button:?} in {row:?}"
        );
        assert!(button.size.width > px(0.), "it has room: {button:?}");
    }
    assert!(
        fact.right() <= edge,
        "the fact wraps inside the window: {fact:?}"
    );
    assert!(
        forget.right() <= fact.right() && forget.size.width > px(0.),
        "Forget stays in its row: {forget:?} in {fact:?}"
    );
}

#[gpui_kit::test]
fn the_thinks_with_pane_tests_a_key_then_uses_it(cx: &mut TestAppContext) {
    let pane = SettingsPane::Credentials;
    let (backend, store, view, mut cx) = open_settings_view(cx, Some(pane));
    let model = |id: &str, name: Option<&str>| ProviderModel {
        id: id.to_owned(),
        name: name.map(str::to_owned),
    };
    // No local Ollama answers the look as the pane opens.
    backend
        .provider
        .push_test(Err(Error::Unreachable("connection refused".into())));
    backend.provider.push_test(Ok(ProviderTest {
        checked: true,
        models: vec![
            model("example-model-a", None),
            model("example-model-b", Some("Example B")),
        ],
    }));
    store.update(&mut cx, |store, cx| {
        store.show_in_test_column(Some(Page::SettingsPane(pane)), cx);
    });
    cx.run_until_parked();
    let mut opened = backend.calls();
    opened.sort_by_key(|call| matches!(call, Call::TestAgentProvider(_)));
    assert_eq!(
        opened,
        [Call::AgentProvider, Call::TestAgentProvider(ollama_look())],
        "read, and Ollama looked for, as it opens"
    );
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("provider-openai", cx);
    });
    cx.run_until_parked();
    let picked = |cx: &VisualTestContext| {
        store.read_with(cx, |store, _| store.credentials().picked().cloned())
    };
    assert_eq!(
        picked(&cx),
        Some(ProviderKind::OpenAi),
        "through the server's OpenAI bridge"
    );
    cx.update(|window, cx| window.click("provider-anthropic", cx));
    cx.run_until_parked();
    assert_eq!(picked(&cx), Some(ProviderKind::Anthropic));

    let key = "sk-ant-example-key-0000";
    let field = view.read_with(&cx, |view, cx| view.credentials.read(cx).key.clone());
    type_in(&field, key, &mut cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("provider-test", cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("provider-works").is_some(), "It works");
        window.click(("provider-model", 1_usize), cx);
        window.render_frame(cx);
        window.click("provider-use", cx);
    });
    cx.run_until_parked();
    let sent = |model: Option<&str>| ProviderRequest {
        provider: ProviderKind::Anthropic,
        api_key: Some(key.to_owned()),
        base_url: None,
        model: model.map(str::to_owned),
        fast_model: None,
    };
    assert_eq!(
        backend.calls()[2..],
        [
            Call::TestAgentProvider(sent(None)),
            Call::SaveAgentProvider(sent(Some("example-model-b"))),
        ]
    );
    let left = field.read_with(&cx, |input, _| input.value().to_string());
    assert_eq!(left, "", "the key is gone from the field once saved");
    store.read_with(&cx, |store, _| {
        assert_eq!(store.credentials().saves(), 1);
        assert!(!store.credentials().has_key());
    });
}

/// The look for a local Ollama as the picker opens.
fn ollama_look() -> ProviderRequest {
    ProviderRequest {
        provider: ProviderKind::Ollama,
        api_key: None,
        base_url: None,
        model: None,
        fast_model: None,
    }
}

#[gpui_kit::test]
fn another_service_asks_which_api_it_speaks(cx: &mut TestAppContext) {
    let pane = SettingsPane::Credentials;
    let (backend, store, view, mut cx) = open_settings_view(cx, Some(pane));
    store.update(&mut cx, |store, cx| {
        store.show_in_test_column(Some(Page::SettingsPane(pane)), cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("provider-openai-compatible", cx);
        window.render_frame(cx);
        assert!(
            window
                .try_find("provider-speaks-openai-compatible")
                .is_some()
        );
    });
    let picked = store.read_with(&cx, |store, _| store.credentials().picked().cloned());
    assert_eq!(
        picked,
        Some(ProviderKind::OpenAiCompatible),
        "OpenAI-style first"
    );
    cx.update(|window, cx| window.click("provider-speaks-anthropic-compatible", cx));
    cx.run_until_parked();
    let (url, key) = view.read_with(&cx, |view, cx| {
        let pane = view.credentials.read(cx);
        (pane.base_url.clone(), pane.key.clone())
    });
    type_in(&url, "https://example.test", &mut cx);
    type_in(&key, "sk-example-key-0000", &mut cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("provider-test", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        backend.calls().last(),
        Some(&Call::TestAgentProvider(ProviderRequest {
            provider: ProviderKind::AnthropicCompatible,
            api_key: Some("sk-example-key-0000".to_owned()),
            base_url: Some("https://example.test".to_owned()),
            model: None,
            fast_model: None,
        }))
    );
}

/// What a server answers with a `DeepSeek` key saved.
fn deepseek_saved() -> AgentProvider {
    AgentProvider {
        provider: Some(ProviderKind::DeepSeek),
        key_set: true,
        key_hint: Some("0000".to_owned()),
        usable: true,
        ..AgentProvider::NONE
    }
}

/// Draws the window in the light appearance, then the dark, finding `ids` in each.
fn draws_light_and_dark(cx: &mut VisualTestContext, ids: &[&'static str]) {
    for dark in [false, true] {
        cx.update(|window, cx| {
            cx.set_global(Pal::new(&crate::theme::built_in(), dark));
            window.render_frame(cx);
            for id in ids {
                assert!(window.try_find(*id).is_some(), "{id}, dark: {dark}");
            }
        });
    }
}

#[gpui_kit::test]
fn the_thinks_with_row_on_your_ember_shows_what_is_saved(cx: &mut TestAppContext) {
    let (backend, store, _, mut cx) = open_settings_view(cx, Some(SettingsPane::Ember));
    backend.provider.push_status(Ok(deepseek_saved()));
    store.update(&mut cx, |store, cx| {
        store.show_in_test_column(Some(Page::SettingsPane(SettingsPane::Ember)), cx);
    });
    cx.run_until_parked();
    draws_light_and_dark(&mut cx, &["credentials-row"]);
    let line = store.read_with(&cx, |store, _| store.credentials().saved().map(saved_line));
    assert_eq!(line.as_deref(), Some("DeepSeek · key ending 0000"));
}

#[gpui_kit::test]
fn the_thinks_with_pane_opens_on_the_saved_provider(cx: &mut TestAppContext) {
    let pane = SettingsPane::Credentials;
    let (backend, store, _, mut cx) = open_settings_view(cx, Some(pane));
    let mut saved = deepseek_saved();
    saved.in_use = false;
    backend.provider.push_status(Ok(saved));
    store.update(&mut cx, |store, cx| {
        store.show_in_test_column(Some(Page::SettingsPane(pane)), cx);
    });
    cx.run_until_parked();
    draws_light_and_dark(
        &mut cx,
        &[
            "provider-anthropic",
            "provider-openai-compatible",
            "provider-test",
            "provider-forget",
            "provider-not-in-use",
        ],
    );
    cx.update(|window, cx| {
        window.click("provider-forget", cx);
        window.render_frame(cx);
        window.click("provider-forget-confirm", cx);
    });
    cx.run_until_parked();
    assert_eq!(backend.calls().last(), Some(&Call::ForgetAgentProvider));
}

#[gpui_kit::test]
fn the_usage_card_on_your_ember_and_where_the_assistant_works_on_advanced_answer(
    cx: &mut TestAppContext,
) {
    let (backend, store, _, mut cx) = open_settings_view(cx, Some(SettingsPane::Ember));
    let page = serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z",
        "summary": {"ok": 1, "warn": 0, "down": 0},
        "groups": [{"id": "runners", "title": "Runners", "footer": null, "items": [
            {"id": "runner", "group": "runners", "name": "Runner", "icon": "server.rack",
             "tint": "teal", "state": "ok", "status": "Online", "detail": "2 sessions",
             "at": null, "error": null, "hint": null, "facts": [], "lists": [],
             "actions": [], "lastError": null}]}]}))
    .unwrap();
    backend.working.push_environment_action(Ok(page));
    store.update(&mut cx, |store, cx| {
        let mut state = crate::fixtures::state(json!({
            "usage": [{"id": "a-1", "name": "Example", "scope": "any", "loggedIn": true,
                "used5h": 80, "resets5h": null, "used7d": 20, "resets7d": null,
                "limitedUntil": null, "checked": null, "places": [], "best": true}]}));
        state.assistant_account = Some(emberhut_api::usage::AssistantAccount {
            on: Some("a-1".to_owned()),
            pin: None,
            mode: "auto".to_owned(),
            pin_out: false,
            rooms: Vec::new(),
            waits: None,
        });
        store.put_state_for_tests(state);
        store.act_on_environment(
            emberhut_api::ids::EnvironmentId::new("runner"),
            emberhut_api::environments::EnvActionKind::Check,
            cx,
        );
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("usage-check-now", cx);
    });
    cx.run_until_parked();
    store.update(&mut cx, |store, cx| store.toggle_usage_row("a-1", cx));
    cx.update(Window::render_frame);
    assert!(store.read_with(&cx, |store, _| store.usage_card().is_open("a-1")));
    show(Some(SettingsPane::Advanced), &mut cx);
    cx.update(|window, _| {
        assert!(window.try_find("env-runner").is_some(), "the runner's row");
    });
}

/// A main window over a fake backend, with Settings' front door open in its column as ⌘,
/// opens it, and its Settings.
fn open_front_door(
    cx: &mut TestAppContext,
) -> (
    Arc<FakeBackend>,
    Entity<Store>,
    Entity<MainWindow>,
    Entity<SettingsView>,
    VisualTestContext,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
        crate::ui::window::bind_keys(cx);
    });
    let backend = Arc::new(FakeBackend::new([]));
    let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
    let store = cx.new(|_| Store::for_tests(shared));
    store.update(cx, |store, cx| {
        store.take_state_for_tests(crate::fixtures::state(json!({})), cx);
    });
    let window = cx
        .add_window(|window, cx| MainWindow::new(store.clone(), ReportSize::default(), window, cx));
    let main = window.root(cx).unwrap();
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.simulate_resize(gpui_kit::size(px(1260.), px(800.)));
    cx.update(|window, cx| crate::ui::routes::open_settings(None, window, cx));
    cx.run_until_parked();
    cx.update(Window::render_frame);
    let view = main.read_with(&cx, |main, _| main.settings_view());
    (backend, store, main, view, cx)
}

/// The page the window's column shows.
fn column_shows(main: &Entity<MainWindow>, cx: &VisualTestContext) -> Option<Page> {
    main.read_with(cx, |main, cx| main.column.read(cx).current())
}

/// Whether the front door's search field has the keyboard.
fn search_focused(view: &Entity<SettingsView>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        view.read(cx)
            .search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    })
}

/// Types `text` into the front door's field (it has the keyboard) and draws the result.
fn search_for(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    cx.run_until_parked();
    cx.update(Window::render_frame);
}

#[gpui_kit::test]
fn the_front_door_opens_with_the_keyboard_in_its_search(cx: &mut TestAppContext) {
    let (_backend, _store, main, view, mut cx) = open_front_door(cx);
    assert_eq!(column_shows(&main, &cx), Some(Page::Settings));
    assert!(
        search_focused(&view, &mut cx),
        "the search has the keyboard"
    );
    cx.update(|window, _| {
        for group in [
            "ember",
            "appearance",
            "devices",
            "connections",
            "sharing",
            "privacy",
        ] {
            assert!(
                window.try_find(format!("settings-chip-{group}")).is_some(),
                "{group}'s chip"
            );
        }
        assert!(
            window.try_find("settings-row-thinks-with").is_some(),
            "suggested: what your ember thinks with"
        );
    });
    draws_light_and_dark(&mut cx, &["settings-search", "settings-rows"]);
}

#[gpui_kit::test]
fn typing_narrows_the_list_to_what_matches(cx: &mut TestAppContext) {
    let (_backend, _store, _main, view, mut cx) = open_front_door(cx);
    search_for("dar", &mut cx);
    assert_eq!(view.read_with(&cx, |view, _| view.query.clone()), "dar");
    cx.update(|window, _| {
        let dark = window.find("settings-row-appearance-mode").bounds();
        let room = window.find("settings-row-dark-room").bounds();
        assert!(dark.top() < room.top(), "title matches first, in order");
        assert!(
            window.try_find("settings-row-thinks-with").is_none(),
            "narrowed"
        );
    });
    search_for("zz", &mut cx);
    cx.update(|window, _| {
        assert!(
            window.try_find("settings-nothing").is_some(),
            "Nothing called that"
        );
    });
}

#[gpui_kit::test]
fn arrows_move_and_return_opens_the_rows_page_washing_its_row(cx: &mut TestAppContext) {
    let (backend, store, main, view, mut cx) = open_front_door(cx);
    search_for("dar", &mut cx);
    let before = backend.calls().len();
    let appearance = store.read_with(&cx, |store, _| store.theme().page().choice.clone());
    cx.simulate_keystrokes("down down up");
    assert_eq!(view.read_with(&cx, |view, _| view.pick), 1);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        column_shows(&main, &cx),
        Some(Page::SettingsPane(SettingsPane::Privacy)),
        "Dark room history lives on Privacy"
    );
    assert_eq!(
        view.read_with(&cx, |view, _| view.flashing()),
        Some("dark-room")
    );
    cx.update(Window::render_frame);
    cx.update(|window, _| assert!(window.try_find("dark-clear").is_some()));
    assert!(
        backend.calls()[before..]
            .iter()
            .all(|call| !matches!(call, Call::Settings(_) | Call::ChangeTheme(..))),
        "opening changed nothing: {:?}",
        &backend.calls()[before..]
    );
    let after = store.read_with(&cx, |store, _| store.theme().page().choice.clone());
    assert_eq!(appearance, after, "the appearance is as it was");
}

#[gpui_kit::test]
fn a_result_under_a_disclosure_opens_it(cx: &mut TestAppContext) {
    let (_backend, _store, main, view, mut cx) = open_front_door(cx);
    search_for("mcp", &mut cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        column_shows(&main, &cx),
        Some(Page::SettingsPane(SettingsPane::Connections))
    );
    view.read_with(&cx, |view, _| {
        assert!(view.folds.contains(&Fold::Tools), "Tools opened");
        assert_eq!(view.flashing(), Some("tools"));
    });
}

#[gpui_kit::test]
fn return_on_a_suggestion_opens_its_page(cx: &mut TestAppContext) {
    let (_backend, _store, main, _view, mut cx) = open_front_door(cx);
    // Nothing waits on the user here: what your ember thinks with comes first.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        column_shows(&main, &cx),
        Some(Page::SettingsPane(SettingsPane::Credentials))
    );
}

#[gpui_kit::test]
fn esc_clears_the_search_then_lets_go_of_it(cx: &mut TestAppContext) {
    let (_backend, _store, main, view, mut cx) = open_front_door(cx);
    search_for("mcp", &mut cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let kept = view.read_with(&cx, |view, cx| {
        (view.query.clone(), view.search.read(cx).value().to_string())
    });
    assert_eq!(kept, (String::new(), String::new()), "cleared");
    assert!(search_focused(&view, &mut cx), "still typing");
    assert_eq!(column_shows(&main, &cx), Some(Page::Settings));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!search_focused(&view, &mut cx), "let go of");
    assert_eq!(column_shows(&main, &cx), Some(Page::Settings), "still open");
}

#[gpui_kit::test]
fn a_chip_opens_its_groups_page_and_back_returns_to_the_search(cx: &mut TestAppContext) {
    let (_backend, _store, main, view, mut cx) = open_front_door(cx);
    search_for("dark", &mut cx);
    for (chip, pane) in [
        ("settings-chip-ember", SettingsPane::Ember),
        ("settings-chip-devices", SettingsPane::Devices),
        ("settings-chip-connections", SettingsPane::Connections),
        ("settings-chip-sharing", SettingsPane::Sharing),
        ("settings-chip-privacy", SettingsPane::Privacy),
        ("settings-chip-advanced", SettingsPane::Advanced),
        ("settings-chip-appearance", SettingsPane::Appearance),
    ] {
        cx.update(|window, cx| window.click(chip, cx));
        cx.run_until_parked();
        assert_eq!(column_shows(&main, &cx), Some(Page::SettingsPane(pane)));
        draws_light_and_dark(&mut cx, &[]);
        cx.dispatch_action(column::GoBack);
        cx.run_until_parked();
        cx.update(Window::render_frame);
        assert_eq!(column_shows(&main, &cx), Some(Page::Settings));
    }
    assert_eq!(
        view.read_with(&cx, |view, _| view.query.clone()),
        "dark",
        "back from a page, the search is as it was"
    );
    assert!(search_focused(&view, &mut cx));
}

#[gpui_kit::test]
fn the_dark_rooms_history_is_cleared_only_once_confirmed(cx: &mut TestAppContext) {
    let (backend, _store, _, mut cx) = open_settings_view(cx, Some(SettingsPane::Privacy));
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("dark-clear", cx);
    });
    cx.run_until_parked();
    assert!(
        !backend
            .calls()
            .iter()
            .any(|call| matches!(call, Call::ClearDark)),
        "asked, not sent"
    );
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("dark-clear-confirm", cx);
    });
    cx.run_until_parked();
    assert!(
        backend
            .calls()
            .iter()
            .any(|call| matches!(call, Call::ClearDark))
    );
}

#[gpui_kit::test]
fn your_embers_page_wraps_inside_a_narrow_window(cx: &mut TestAppContext) {
    let (_backend, store, _, mut cx) = open_settings_view(cx, Some(SettingsPane::Ember));
    store.update(&mut cx, |store, cx| {
        store.show_in_test_column(Some(Page::SettingsPane(SettingsPane::Ember)), cx);
    });
    cx.run_until_parked();
    let edge = px(380.);
    cx.simulate_resize(gpui_kit::size(edge, px(900.)));
    cx.update(Window::render_frame);
    cx.update(|window, _| {
        for id in [
            "on-duty-switch",
            "credentials-row",
            "about-you-row",
            "fold-ember-details",
        ] {
            let bounds = window.find(id).bounds();
            assert!(
                bounds.right() <= edge && bounds.size.width > px(0.),
                "{id} stays inside the window: {bounds:?}"
            );
        }
    });
}

#[gpui_kit::test]
fn on_duty_switches_from_your_embers_page(cx: &mut TestAppContext) {
    let (backend, _store, _, mut cx) = open_settings_view(cx, Some(SettingsPane::Ember));
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click("on-duty-switch", cx);
    });
    cx.run_until_parked();
    assert!(
        backend
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Settings(SettingsAction::SetOn(_)))),
        "{:?}",
        backend.calls()
    );
}

#[gpui_kit::test]
fn appearance_hides_the_agents_moves_them_and_still_opens_their_rooms(cx: &mut TestAppContext) {
    use super::appearance::{AGENTS_ABOVE, AGENTS_BELOW, HIDDEN_AGENT, SHOW_AGENTS};
    use crate::store::AgentsPlace;

    let (_backend, store, main, _, mut cx) = open_front_door(cx);
    store.update(&mut cx, |store, cx| {
        store.take_state_for_tests(
            crate::fixtures::state(json!({
                "rooms": [crate::fixtures::room(1, json!({})), crate::fixtures::room(9, json!({}))],
                "agents": [
                    {"id": "main", "name": "Main", "principal": "the owner", "room": 1,
                     "enabled": true, "pendingApprovals": 0},
                    {"id": "helper", "name": "Example helper", "principal": "Example Ltd",
                     "room": 9, "enabled": true, "pendingApprovals": 0}
                ]
            })),
            cx,
        );
    });
    cx.update(|window, cx| {
        crate::ui::routes::open_settings(Some(SettingsPane::Appearance), window, cx);
    });
    cx.run_until_parked();
    // The row of the agent whose room is 9.
    let hidden_row: &'static str = "hidden-agent-9";
    assert!(hidden_row.starts_with(HIDDEN_AGENT));
    draws_light_and_dark(&mut cx, &[SHOW_AGENTS, AGENTS_BELOW, AGENTS_ABOVE]);
    let now = |cx: &VisualTestContext| {
        store.read_with(cx, |store, _| (store.agents_shown(), store.agents_place()))
    };
    assert_eq!(
        now(&cx),
        (true, AgentsPlace::Below),
        "shown, below the rooms"
    );
    cx.update(|window, cx| {
        assert!(
            window.try_find(hidden_row).is_none(),
            "listed in the sidebar"
        );
        window.click(AGENTS_ABOVE, cx);
    });
    cx.run_until_parked();
    assert_eq!(now(&cx), (true, AgentsPlace::Above));
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click(SHOW_AGENTS, cx);
    });
    cx.run_until_parked();
    assert_eq!(now(&cx), (false, AgentsPlace::Above), "hidden");
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click(AGENTS_BELOW, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        now(&cx),
        (false, AgentsPlace::Above),
        "the place waits for the agents to show"
    );
    draws_light_and_dark(&mut cx, &[SHOW_AGENTS, AGENTS_BELOW]);
    cx.update(|window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(hidden_row).is_some(), "listed here instead");
        window.click(hidden_row, cx);
    });
    cx.run_until_parked();
    let room = store.read_with(&cx, |store, _| store.room());
    assert_eq!(room, emberhut_api::ids::RoomId::new(9), "its room opens");
    assert_eq!(
        column_shows(&main, &cx),
        Some(Page::SettingsPane(SettingsPane::Appearance)),
        "a wide window keeps Settings open beside the chat, as the sidebar's row does"
    );
}
