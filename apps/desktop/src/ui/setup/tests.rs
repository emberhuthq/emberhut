//! The setup's view tests: the welcome in the window's place, its first step's two ways, the
//! picker on "Light it", the ember holding still under Reduce Motion, Say hello, the quiet line
//! over the chat, and the words on its screens.

use std::sync::Arc;

use emberhut_api::setup::{SetupItemId, SetupState};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{ElementId, TestAppContext, VisualTestContext, size};
use serde_json::json;

use super::steps::{FIND_TITLE, REACH_LINES, SHARE_LINE, WHATSAPP_NEXT};
use super::*;
use crate::backend::fake::setup::page;
use crate::backend::fake::{Call, FakeBackend};
use crate::hut::KeepLit;
use crate::store::Reconnect;
use crate::store::gate::Gate;
use crate::store::setup::tests::FakeHome;
use crate::store::setup::{LocalHome, NoLocalHome, SetupStep};
use crate::ui::report::ReportSize;
use crate::ui::sign_in::PAIRING_HINT;
use crate::ui::window::MainWindow;

/// A main window over a store on `backend`, with the setup begun as on a fresh install
/// (`fresh`) or not.
fn open<'a>(
    backend: &Arc<FakeBackend>,
    fresh: bool,
    cx: &'a mut TestAppContext,
) -> (Entity<Store>, &'a mut VisualTestContext) {
    open_with(backend, fresh, NoLocalHome, cx)
}

/// [`open`], with `home` as Emberhut on this computer.
fn open_with<'a>(
    backend: &Arc<FakeBackend>,
    fresh: bool,
    home: impl LocalHome + 'static,
    cx: &'a mut TestAppContext,
) -> (Entity<Store>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_global(Pal::new(&crate::theme::built_in(), false));
    });
    let shared = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    let store =
        cx.new(|_| Store::for_setup_tests(shared, None, std::rc::Rc::new(home), Reconnect::none()));
    store.update(cx, |store, cx| store.begin_setup(fresh, cx));
    let (_, cx) = cx.add_window_view(|window, cx| {
        MainWindow::new(store.clone(), ReportSize::default(), window, cx)
    });
    cx.simulate_resize(size(px(1280.), px(860.)));
    cx.run_until_parked();
    (store, cx)
}

fn shows(cx: &mut VisualTestContext, id: &'static str) -> bool {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
}

#[gpui_kit::test]
fn a_fresh_install_shows_the_welcome_with_every_step(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let (store, cx) = open(&backend, true, cx);
    assert_eq!(store.read_with(cx, |store, _| store.gate()), Gate::Setup);
    for step in 1..=5 {
        let id = ElementId::Name(format!("setup-step-{step}").into());
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(id.clone()).is_some(), "step {step}");
        });
    }
    assert!(shows(cx, "setup-home-here"));
    assert!(!shows(cx, "ember"), "no sidebar behind the setup");
    let label = cx.update(|window, _| {
        window
            .find("setup-ember")
            .label()
            .map_or_else(String::new, str::to_owned)
    });
    assert_eq!(label, "Your ember. Not lit yet.");
}

#[gpui_kit::test]
fn another_computer_leads_into_the_pairing_form(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let (store, cx) = open(&backend, true, cx);
    assert!(!shows(cx, "setup-find"));
    cx.update(|window, cx| window.click("setup-home-elsewhere", cx));
    cx.update(|window, cx| window.click("setup-continue", cx));
    assert!(shows(cx, "setup-find"), "Find your ember");
    // Without a home here, Continue on Here says so and offers the same form.
    store.update(cx, |store, cx| {
        store.choose_home(crate::store::setup::HomeChoice::Here, cx);
    });
    assert!(!shows(cx, "setup-find"));
    cx.update(|window, cx| window.click("setup-continue", cx));
    cx.run_until_parked();
    assert!(shows(cx, "setup-home-problem"));
    assert!(shows(cx, "setup-find"));
}

#[gpui_kit::test]
fn light_it_shows_the_picker_and_saving_lights_the_ember(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).with_session());
    backend
        .setup
        .push_page(Ok(page(&[(SetupItemId::Provider, SetupState::Missing)])));
    let (store, cx) = open(&backend, false, cx);
    store.update(cx, |store, cx| {
        store.take_state_for_tests(crate::fixtures::state(json!({})), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().open()),
        Some(SetupStep::Light)
    );
    assert!(shows(cx, "provider-anthropic"), "the picker, in the step");
    assert!(backend.calls().contains(&Call::AgentProvider));
    cx.update(|window, cx| window.click("provider-anthropic", cx));
    store.update(cx, |store, cx| {
        store.type_provider_key("sk-ant-example", cx);
        store.test_provider(cx);
    });
    cx.run_until_parked();
    store.update(cx, Store::save_provider);
    cx.run_until_parked();
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().open()),
        Some(SetupStep::Reach),
        "the save went through: the step is done"
    );
    assert_eq!(store.read_with(cx, |store, _| store.setup().glow()), 2);
}

#[gpui_kit::test]
fn the_embers_glow_holds_still_under_reduce_motion(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).with_session());
    let (store, cx) = open(&backend, false, cx);
    store.update(cx, |store, cx| {
        store.preview_setup(Some(SetupStep::Reach), cx);
    });
    assert!(shows(cx, ember::BREATHING));
    assert!(!shows(cx, ember::STILL));
    cx.update(|_, cx| motion::reduce_for_tests(true, cx));
    assert!(shows(cx, ember::STILL));
    assert!(!shows(cx, ember::BREATHING));
}

#[gpui_kit::test]
fn say_hello_opens_the_app_and_the_setup_stays_away(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).with_session());
    let (store, cx) = open(&backend, false, cx);
    store.update(cx, |store, cx| {
        store.take_state_for_tests(crate::fixtures::state(json!({})), cx);
        store.preview_setup(None, cx);
    });
    assert!(shows(cx, "setup-lit"));
    assert!(shows(cx, "setup-summary"));
    cx.update(|window, cx| window.click("setup-say-hello", cx));
    cx.run_until_parked();
    assert_eq!(store.read_with(cx, |store, _| store.gate()), Gate::Open);
    assert!(shows(cx, "ember"), "the sidebar: the app is open");
    assert!(!shows(cx, "unlit-bar"), "lit: no line over the chat");
}

#[gpui_kit::test]
fn an_ember_that_goes_out_after_the_setup_gets_a_quiet_line(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).with_session());
    backend
        .setup
        .push_page(Ok(page(&[(SetupItemId::Provider, SetupState::Missing)])));
    let (store, cx) = open(&backend, false, cx);
    store.update(cx, |store, cx| {
        store.take_state_for_tests(crate::fixtures::state(json!({})), cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.click("setup-later", cx));
    cx.run_until_parked();
    assert_eq!(store.read_with(cx, |store, _| store.gate()), Gate::Open);
    assert!(shows(cx, "unlit-bar"));
}

#[test]
fn the_setups_words_keep_the_machinery_out() {
    let mut lines = vec![
        LEAD,
        FOOTER,
        FIND_HINT,
        FIND_TITLE,
        SHARE_LINE,
        WHATSAPP_NEXT,
    ];
    lines.extend(REACH_LINES);
    for line in lines {
        let lower = line.to_lowercase();
        for hidden in ["server", "runner", "route", "token", "terminal"] {
            assert!(!lower.contains(hidden), "{line:?} says {hidden:?}");
        }
    }
}

#[test]
fn find_your_ember_names_only_screens_that_exist() {
    // The screen a code comes from is the one the sign-in hint names, in Settings › Devices.
    let source = "Settings › Devices › Pair another device";
    assert!(FIND_HINT.contains(source));
    assert!(PAIRING_HINT.contains(source));
    for missing in ["Show a pairing code", "Settings › People"] {
        assert!(!FIND_HINT.contains(missing), "{missing:?}");
        assert!(!SHARE_LINE.contains(missing), "{missing:?}");
    }
    assert!(SHARE_LINE.contains("Settings › Sharing"));
}

#[gpui_kit::test]
fn step_ones_change_opens_how_the_ember_stays_lit(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).at("http://127.0.0.1:18990"));
    backend
        .setup
        .push_page(Ok(page(&[(SetupItemId::Provider, SetupState::Missing)])));
    let home = FakeHome::at("http://127.0.0.1:18990", Some("WXYZ-2345")).keeping(Ok(()));
    let keeps = std::rc::Rc::clone(&home.keeps);
    let (store, cx) = open_with(&backend, true, home, cx);
    assert!(
        !shows(cx, "setup-keep-change"),
        "nothing to change before it lives here"
    );
    cx.update(|window, cx| window.click("setup-continue", cx));
    cx.run_until_parked();
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().open()),
        Some(SetupStep::Light)
    );
    assert!(shows(cx, "setup-keep-change"));
    let sheet = |cx: &mut VisualTestContext| {
        store.read_with(cx, |store, _| store.setup().shows_keep_sheet())
    };
    assert!(!sheet(cx));
    cx.update(|window, cx| window.click("setup-keep-change", cx));
    cx.run_until_parked();
    assert!(sheet(cx));
    assert!(shows(cx, "setup-keep-when-closed"));
    cx.update(|window, cx| window.click("setup-keep-at-login", cx));
    cx.run_until_parked();
    assert_eq!(
        keeps.borrow().last(),
        Some(&KeepLit {
            when_closed: true,
            at_login: false,
        })
    );
    cx.update(|window, cx| window.click("sheet-done", cx));
    cx.run_until_parked();
    assert!(!sheet(cx));
}

#[gpui_kit::test]
fn a_hut_that_doesnt_start_again_shows_why_with_try_again(cx: &mut TestAppContext) {
    use crate::store::setup::Relit;
    let backend = Arc::new(
        FakeBackend::new([])
            .at("http://127.0.0.1:18990")
            .with_session(),
    );
    let home = FakeHome::at("http://127.0.0.1:18990", None);
    home.relights.borrow_mut().push_back(Relit::Failed(
        "The hut stopped while it was starting.".to_owned(),
    ));
    let relit_at = std::rc::Rc::clone(&home.relit_at);
    let (store, cx) = open_with(&backend, false, home, cx);
    assert!(!shows(cx, "gate-hut-retry"), "connecting, as usual");
    store.update(cx, Store::relight_home);
    cx.run_until_parked();
    assert!(shows(cx, "gate-hut-problem"));
    cx.update(|window, cx| window.click("gate-hut-retry", cx));
    cx.run_until_parked();
    assert_eq!(relit_at.borrow().len(), 2);
    assert!(!shows(cx, "gate-hut-retry"), "started: connecting again");
}
