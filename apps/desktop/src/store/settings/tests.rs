//! Settings' data in the store, on GPUI's headless test platform with a scripted backend:
//! when the switches are asked for, what a switch does, a fix to a fact and its refusal, a
//! Chrome change's answer, adding the extension to Chrome only on the user's OK, and pausing.

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::chrome::{ChromeAction, ChromeReply};
use emberhut_api::facts::FactsPage;
use emberhut_api::ids::{ChromeProfileId, FactId, RoomId, TrialId};
use emberhut_api::settings::SettingsAction;
use emberhut_api::switches::{SwitchAction, SwitchPage};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::json;

use super::SettingsPane;
use super::chrome::ChromeSheet;
use super::chrome_install::{InstallState, Installer, Written};
use crate::backend::Error;
use crate::backend::fake::{Call, FakeBackend};
use crate::store::{Page, Store, ThemeState};

fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
}

fn switch_page(skill_on: bool) -> SwitchPage {
    serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z", "live": true,
        "mcp": [{"name": "files", "state": "connected", "on": true, "fixed": false}],
        "skills": [{"name": "notes", "description": "", "source": "yours", "on": skill_on}]}))
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
fn the_switches_are_asked_for_while_settings_is_open_in_front(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    for _ in 0..3 {
        backend.replies.push_switches(Ok(switch_page(true)));
    }
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.window.active = true;
        store.start_switches(cx);
    });
    cx.run_until_parked();
    assert_eq!(count(&backend, &Call::Switches), 0, "not off Settings");

    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Settings), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        count(&backend, &Call::Switches),
        1,
        "Settings opened: at once"
    );
    assert!(store.read_with(cx, |store, _| store.switches().page().is_some()));
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(
        count(&backend, &Call::Switches),
        2,
        "then every ten seconds"
    );

    store.update(cx, |store, cx| {
        let pages = vec![
            Page::Settings,
            Page::SettingsPane(SettingsPane::ChromeProfiles),
        ];
        store.column_changed(gpui_kit::WindowId::from(1), pages, cx);
    });
    cx.executor().advance_clock(Duration::from_mins(5));
    cx.run_until_parked();
    assert_eq!(
        count(&backend, &Call::Switches),
        2,
        "not on another of its pages"
    );
}

#[gpui_kit::test]
fn a_switch_goes_one_at_a_time_and_brings_the_page_back(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.replies.push_switches(Ok(switch_page(false)));
    let store = store(cx, &backend);
    let off = SwitchAction::Skill {
        name: "notes".into(),
        on: false,
    };
    store.update(cx, |store, cx| {
        store.switch(off.clone(), cx);
        assert_eq!(store.switches().busy(), Some("notes"));
        store.switch(
            SwitchAction::Reconnect {
                name: "files".into(),
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        backend.calls(),
        [Call::Switch(off)],
        "the second waited its turn"
    );
    store.read_with(cx, |store, _| {
        let feed = store.switches();
        assert_eq!(feed.busy(), None);
        assert!(!feed.page().unwrap().skills[0].on);
    });
}

#[gpui_kit::test]
fn a_forgotten_fact_goes_at_once_and_comes_back_if_refused(cx: &mut TestAppContext) {
    let page: FactsPage = serde_json::from_value(json!({"facts": [
        {"id": "work/desk", "category": "work", "title": "Desk", "text": "Works from home.",
         "source": "", "updated": "2026-11-20T10:00:00Z"}]}))
    .unwrap();
    let backend = Arc::new(FakeBackend::new([]));
    backend.replies.push_facts(Ok(page.clone()));
    backend.replies.push_facts(Ok(page));
    backend.replies.push_settings(Err(Error::Refused {
        status: Some(400),
        message: Some("no such fact".into()),
    }));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::SettingsPane(SettingsPane::AboutYou)), cx);
    });
    cx.run_until_parked();
    let desk = FactId::new("work/desk");
    store.update(cx, |store, cx| {
        store.forget_fact(desk.clone(), cx);
        assert!(store.facts().fact(&desk).is_none(), "gone at once");
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert!(store.facts().fact(&desk).is_some(), "fetched again");
        assert_eq!(
            store.action_problem(),
            Some("Not forgotten: no such fact"),
            "on the app's line for refused actions"
        );
        assert_eq!(store.facts().problem(), None, "the page itself read fine");
    });
    assert_eq!(
        backend.calls(),
        [
            Call::Facts,
            Call::Settings(SettingsAction::FactDrop(desk)),
            Call::Facts
        ]
    );
}

/// A page with one lesson on trial, number 4, and none.
fn trial_pages() -> (FactsPage, FactsPage) {
    let on_trial: FactsPage = serde_json::from_value(json!({"facts": [], "trials": [
        {"id": 4, "text": "Say which file.", "at": "2026-11-20T10:00:00Z",
         "ends": "2026-12-04T10:00:00Z", "said": "Which file?"}]}))
    .unwrap();
    (on_trial, FactsPage::default())
}

#[gpui_kit::test]
fn a_kept_lesson_leaves_the_trial_and_learning_from_you_stays_as_left(cx: &mut TestAppContext) {
    let (on_trial, kept) = trial_pages();
    let backend = Arc::new(FakeBackend::new([]));
    backend.replies.push_facts(Ok(on_trial));
    backend.replies.push_facts(Ok(kept));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::SettingsPane(SettingsPane::AboutYou)), cx);
    });
    cx.run_until_parked();
    store.update(cx, |store, cx| {
        assert!(!store.facts().learning_open(), "folded at first");
        store.toggle_learning(cx);
        store.keep_trial(TrialId::new(4), cx);
        assert!(store.facts().page().unwrap().trials.is_empty(), "at once");
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.action_problem(), None);
        assert!(store.facts().learning_open(), "still open");
    });
    assert_eq!(
        backend.calls(),
        [
            Call::Facts,
            Call::Settings(SettingsAction::TrialKeep(TrialId::new(4))),
            Call::Facts
        ]
    );
}

#[gpui_kit::test]
fn a_server_that_cant_keep_lessons_early_says_so_plainly(cx: &mut TestAppContext) {
    let (on_trial, _) = trial_pages();
    let backend = Arc::new(FakeBackend::new([]));
    backend.replies.push_facts(Ok(on_trial.clone()));
    // An older server takes the body (200) and leaves the lesson on trial.
    backend.replies.push_facts(Ok(on_trial));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::SettingsPane(SettingsPane::AboutYou)), cx);
    });
    cx.run_until_parked();
    store.update(cx, |store, cx| store.keep_trial(TrialId::new(4), cx));
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.facts().page().unwrap().trials.len(), 1, "back");
        assert_eq!(
            store.action_problem(),
            Some(super::about::CANT_KEEP),
            "a plain line, not a failure"
        );
    });
}

#[gpui_kit::test]
fn a_new_profiles_link_opens_its_pairing(cx: &mut TestAppContext) {
    let reply: ChromeReply = serde_json::from_value(json!({"ok": true, "chrome": [],
        "chromePairing": [{"code": "AB-12", "link": "https://example.com/p#AB-12",
            "label": "Work", "name": "work", "until": "2026-11-24T22:30:00Z"}],
        "chromeLatest": "1.0.0",
        "pairing": {"code": "AB-12", "link": "https://example.com/p#AB-12",
            "label": "Work", "name": "work", "until": "2026-11-24T22:30:00Z"}}))
    .unwrap();
    let backend = Arc::new(FakeBackend::new([]));
    backend.replies.push_chrome(Ok(reply));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.state = Some(crate::fixtures::state(json!({})));
        store.show_chrome_sheet(Some(ChromeSheet::Add), cx);
        store.add_chrome("  Work ", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        backend.calls(),
        [Call::Chrome(ChromeAction::Add {
            label: "Work".into()
        })]
    );
    store.read_with(cx, |store, _| {
        assert!(matches!(store.chrome().sheet(), Some(ChromeSheet::Pairing(link)) if link.label == "Work"));
        assert_eq!(store.state().unwrap().chrome_pairing.len(), 1, "into the state");
    });
}

/// Writes nothing: counts the times it was asked, and answers with `answer`.
#[derive(Debug)]
struct FakeInstaller {
    asked: std::sync::atomic::AtomicUsize,
    answer: Result<Written, String>,
}

impl Installer for FakeInstaller {
    fn install(&self) -> Result<Written, String> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.answer.clone()
    }
}

fn installer(answer: Result<Written, String>) -> Arc<FakeInstaller> {
    Arc::new(FakeInstaller {
        asked: std::sync::atomic::AtomicUsize::new(0),
        answer,
    })
}

#[gpui_kit::test]
fn the_extension_is_added_to_chrome_only_on_the_users_ok_then_the_link_waits(
    cx: &mut TestAppContext,
) {
    let written = Written {
        place: "/Users/ada/Library/Application Support/Google/Chrome/External Extensions".into(),
        already: false,
        next: "Chrome shows “New extension added” the next time it starts: click Enable.",
    };
    let fake = installer(Ok(written.clone()));
    let pairing = json!({"code": "AB-12", "link": "https://example.com/p#AB-12",
        "label": "Work", "name": "work", "until": "2026-11-24T22:30:00Z"});
    let reply: ChromeReply = serde_json::from_value(json!({"ok": true, "chrome": [],
        "chromePairing": [pairing], "chromeLatest": "1.0.0", "pairing": pairing}))
    .unwrap();
    let backend = Arc::new(FakeBackend::new([]));
    backend.replies.push_chrome(Ok(reply));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.state = Some(crate::fixtures::state(json!({})));
        store.set_chrome_installer(Arc::clone(&fake) as Arc<dyn Installer>);
        store.show_chrome_sheet(Some(ChromeSheet::Add), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        fake.asked.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the sheet alone writes nothing"
    );
    store.read_with(cx, |store, _| {
        assert_eq!(store.chrome().install(), &InstallState::NotWritten);
    });

    store.update(cx, |store, cx| {
        store.install_chrome_extension(cx);
        assert_eq!(store.chrome().install(), &InstallState::Writing);
        store.install_chrome_extension(cx); // a second press while it writes is ignored
    });
    cx.run_until_parked();
    assert_eq!(fake.asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    store.read_with(cx, |store, _| {
        assert_eq!(store.chrome().install(), &InstallState::Written(written));
        assert_eq!(
            store.chrome().sheet(),
            Some(&ChromeSheet::Add),
            "still naming the profile"
        );
    });

    // Then the profile's link: its page waits for the extension to pair.
    store.update(cx, |store, cx| store.add_chrome("Work", cx));
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let Some(ChromeSheet::Pairing(link)) = store.chrome().sheet() else {
            panic!("the pairing page");
        };
        let waiting = &store.state().unwrap().chrome_pairing;
        assert!(
            waiting.iter().any(|l| l.code == link.code && !l.paired),
            "waiting to pair"
        );
    });
}

#[gpui_kit::test]
fn a_failed_install_says_why(cx: &mut TestAppContext) {
    let fake = installer(Err(
        "This computer has no pkexec to ask for your password.".into()
    ));
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.set_chrome_installer(Arc::clone(&fake) as Arc<dyn Installer>);
        store.install_chrome_extension(cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert!(
            matches!(store.chrome().install(), InstallState::Failed(why) if why.contains("pkexec"))
        );
    });
    assert!(backend.calls().is_empty(), "nothing asked of the server");
}

#[gpui_kit::test]
fn done_on_a_re_pair_link_goes_back_to_its_profile(cx: &mut TestAppContext) {
    let pairing = json!({"code": "CD-34", "link": "https://example.com/p#CD-34",
        "label": "Work", "name": "work", "until": "2026-11-24T22:30:00Z"});
    let reply = |pairing: serde_json::Value| -> ChromeReply {
        serde_json::from_value(json!({"ok": true, "chrome": [], "chromePairing": [],
            "chromeLatest": "1.0.0", "pairing": pairing}))
        .unwrap()
    };
    let backend = Arc::new(FakeBackend::new([]));
    backend.replies.push_chrome(Ok(reply(pairing.clone())));
    backend.replies.push_chrome(Ok(reply(pairing)));
    let store = store(cx, &backend);
    let id = ChromeProfileId::new(2);
    store.update(cx, |store, cx| {
        store.state = Some(crate::fixtures::state(json!({})));
        store.show_chrome_sheet(Some(ChromeSheet::Profile(id)), cx);
        store.chrome_act(ChromeAction::Repair { id }, cx);
    });
    cx.run_until_parked();
    store.update(cx, |store, cx| {
        assert!(matches!(
            store.chrome().sheet(),
            Some(ChromeSheet::Pairing(_))
        ));
        store.close_chrome_sheet(cx);
        assert_eq!(
            store.chrome().sheet(),
            Some(&ChromeSheet::Profile(id)),
            "back to the profile"
        );
        store.close_chrome_sheet(cx);
        assert_eq!(store.chrome().sheet(), None, "then the list");
        store.show_chrome_sheet(Some(ChromeSheet::Add), cx);
        store.add_chrome("Work", cx);
    });
    cx.run_until_parked();
    store.update(cx, |store, cx| {
        store.close_chrome_sheet(cx);
        assert_eq!(
            store.chrome().sheet(),
            None,
            "a new profile's link: the list"
        );
    });
}

#[gpui_kit::test]
fn a_pause_shows_at_once_and_a_refusal_says_so(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend
        .replies
        .push_settings(Err(Error::Unreachable("down".into())));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.set_on_duty(false, cx);
        assert!(!store.is_on_duty(), "paused at once");
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert!(store.is_on_duty(), "the server didn't take it");
        assert_eq!(
            store.on_duty_problem(),
            Some("Emberhut not paused: can't reach the server")
        );
    });
}
