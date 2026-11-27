//! The setup's tests: when it shows, how its steps go and what the ember's glow is at each,
//! putting steps off, the home here through a fake [`LocalHome`] (the move to its address,
//! keeping it lit, the Change sheet), and that an ending is kept.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use emberhut_api::ids::RoomId;
use emberhut_api::setup::{SetupItemId, SetupState};
use gpui_kit::{App, Entity, Task, TestAppContext};
use serde_json::json;

use super::*;
use crate::backend::fake::setup::page;
use crate::backend::fake::{Call, FakeBackend};
use crate::backend::{FreshState, StateReply};
use crate::fixtures::state;
use crate::store::gate::Gate;
use crate::store::{Connected, Connector, Reconnect};

/// What the step's lines call this computer here: macOS says "this Mac".
const HERE_WORDS: &str = if cfg!(target_os = "macos") {
    "this Mac"
} else {
    "this computer"
};

/// Emberhut on this computer as a test wants it: whether it can live here, what starting it,
/// asking it for a code and keeping it lit answer (`None`: not the app's to keep), how often
/// it was started and how it was asked to stay lit.
pub(crate) struct FakeHome {
    pub(crate) available: bool,
    pub(crate) started: Result<&'static str, &'static str>,
    pub(crate) code: Result<Option<&'static str>, &'static str>,
    pub(crate) kept: Option<Result<(), &'static str>>,
    pub(crate) starts: Rc<Cell<usize>>,
    pub(crate) keeps: Rc<RefCell<Vec<KeepLit>>>,
    /// What each launch's relight finds, in order (then nothing here); the addresses asked.
    pub(crate) relights: Rc<RefCell<std::collections::VecDeque<Relit>>>,
    pub(crate) relit_at: Rc<RefCell<Vec<ServerUrl>>>,
}

impl FakeHome {
    /// A home here that starts at `address`, hands over `code` and isn't the app's to keep lit.
    pub(crate) fn at(address: &'static str, code: Option<&'static str>) -> Self {
        Self {
            available: true,
            started: Ok(address),
            code: Ok(code),
            kept: None,
            starts: Rc::default(),
            keeps: Rc::default(),
            relights: Rc::default(),
            relit_at: Rc::default(),
        }
    }

    /// The same home, kept lit as `kept` says.
    pub(crate) fn keeping(self, kept: Result<(), &'static str>) -> Self {
        Self {
            kept: Some(kept),
            ..self
        }
    }
}

impl LocalHome for FakeHome {
    fn available(&self) -> bool {
        self.available
    }

    fn start(&self, _: &mut App) -> Task<Result<ServerUrl, String>> {
        self.starts.set(self.starts.get() + 1);
        Task::ready(
            self.started
                .map(|address| ServerUrl::parse(address).expect("an address"))
                .map_err(str::to_owned),
        )
    }

    fn pair_code(&self, _: &mut App) -> Task<Result<Option<String>, String>> {
        Task::ready(
            self.code
                .map(|code| code.map(str::to_owned))
                .map_err(str::to_owned),
        )
    }

    fn keep_lit(&self, keep: KeepLit, _: &mut App) -> Option<Task<Result<(), String>>> {
        let kept = self.kept?;
        self.keeps.borrow_mut().push(keep);
        Some(Task::ready(kept.map_err(str::to_owned)))
    }

    fn relight(&self, server: &ServerUrl, _: &mut App) -> Task<Relit> {
        self.relit_at.borrow_mut().push(server.clone());
        Task::ready(
            self.relights
                .borrow_mut()
                .pop_front()
                .unwrap_or(Relit::NotHere),
        )
    }
}

/// Makes a fake backend at each address the store moves to, and keeps them.
#[derive(Default)]
pub(crate) struct FakeConnector {
    pub(crate) made: Mutex<Vec<Arc<FakeBackend>>>,
}

impl FakeConnector {
    /// The backends made, in order.
    pub(crate) fn made(&self) -> Vec<Arc<FakeBackend>> {
        self.made.lock().unwrap().clone()
    }
}

impl Connector for FakeConnector {
    fn connect(&self, server: &ServerUrl) -> Result<Connected, Error> {
        let backend = Arc::new(FakeBackend::new([]).at(server.as_str()));
        backend.setup.push_page(Ok(provider(SetupState::Missing)));
        self.made.lock().unwrap().push(Arc::clone(&backend));
        Ok(Connected {
            backend,
            disk: None,
        })
    }
}

const HERE: &str = "http://127.0.0.1:18990";

fn store_with(
    backend: &Arc<FakeBackend>,
    files: Option<AppFiles>,
    home: impl LocalHome + 'static,
    cx: &mut TestAppContext,
) -> Entity<Store> {
    store_moving(backend, files, home, Reconnect::none(), cx)
}

fn store_moving(
    backend: &Arc<FakeBackend>,
    files: Option<AppFiles>,
    home: impl LocalHome + 'static,
    reconnect: Reconnect,
    cx: &mut TestAppContext,
) -> Entity<Store> {
    let shared = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    cx.new(|_| Store::for_setup_tests(shared, files, Rc::new(home), reconnect))
}

/// Whether `backend` was handed a pairing code.
fn paired(backend: &FakeBackend) -> Vec<String> {
    backend
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Pair(code) => Some(code),
            _ => None,
        })
        .collect()
}

fn here(store: &Entity<Store>, cx: &TestAppContext) -> Here {
    store.read_with(cx, |store, _| store.setup().here().clone())
}

fn home_line(store: &Entity<Store>, cx: &TestAppContext) -> StepState {
    store.read_with(cx, |store, _| store.setup().state(SetupStep::Home).clone())
}

fn store(backend: &Arc<FakeBackend>, cx: &mut TestAppContext) -> Entity<Store> {
    store_with(backend, None, NoLocalHome, cx)
}

/// A fresh state from the poll, with `extra`'s fields.
fn poll(store: &Entity<Store>, extra: serde_json::Value, cx: &mut TestAppContext) {
    store.update(cx, |store, cx| {
        let fresh = FreshState {
            state: Box::new(state(extra)),
            etag: None,
        };
        store.apply(
            RoomId::MAIN,
            Ok(StateReply::Fresh(fresh)),
            Instant::now(),
            cx,
        );
    });
    cx.run_until_parked();
}

/// A checklist whose provider is `state`.
fn provider(state: SetupState) -> SetupPage {
    page(&[(SetupItemId::Provider, state)])
}

/// The signed-in devices, as the state lists them.
fn devices(names: &[(i64, &str)]) -> serde_json::Value {
    let devices: Vec<_> = names
        .iter()
        .map(|(id, name)| json!({"id": id, "name": name, "model": null, "created": null, "seen": null}))
        .collect();
    json!({"auth": {"mode": "enforce", "bound": true, "devices": devices}})
}

fn open(store: &Entity<Store>, cx: &TestAppContext) -> Option<SetupStep> {
    store.read_with(cx, |store, _| store.setup().open())
}

fn glow(store: &Entity<Store>, cx: &TestAppContext) -> u8 {
    store.read_with(cx, |store, _| store.setup().glow())
}

fn gate(store: &Entity<Store>, cx: &TestAppContext) -> Gate {
    store.read_with(cx, |store, _| store.gate())
}

#[test]
fn every_direction_names_the_first_screen_and_home_first_is_taken() {
    assert_eq!(DIRECTION, SetupDirection::HomeFirst);
    for direction in [
        SetupDirection::HomeFirst,
        SetupDirection::AccountFirst,
        SetupDirection::ShowFirst,
    ] {
        assert_eq!(first_screen(direction), SetupStep::Home, "{direction:?}");
    }
}

#[test]
fn the_steps_read_in_the_embers_voice() {
    let names: Vec<_> = SetupStep::ALL.into_iter().map(SetupStep::name).collect();
    assert_eq!(
        names,
        [
            "Give your ember a home",
            "Light it",
            "Let it reach you",
            "Show it your world",
            "Share your ember",
        ]
    );
    let optional: Vec<_> = SetupStep::ALL
        .into_iter()
        .map(SetupStep::optional)
        .collect();
    assert_eq!(optional, [false, false, true, true, true]);
    assert_eq!(
        and_list(&["Gmail", "Slack", "Chrome"]),
        "Gmail, Slack and Chrome"
    );
    assert_eq!(and_list(&["Gmail"]), "Gmail");
}

#[test]
fn no_word_from_behind_the_scenes_reaches_the_steps() {
    let mut words: Vec<&str> = SetupStep::ALL
        .into_iter()
        .flat_map(|step| [step.name(), step.line()])
        .collect();
    words.extend(NOTES);
    words.extend(
        Tile::ALL
            .into_iter()
            .flat_map(|tile| [tile.name(), tile.line()]),
    );
    words.extend([WAITING, CANT_HERE]);
    for line in words {
        let lower = line.to_lowercase();
        for hidden in ["server", "runner", "route", "token", "terminal"] {
            assert!(!lower.contains(hidden), "{line:?} says {hidden:?}");
        }
    }
}

#[gpui_kit::test]
fn a_fresh_install_opens_on_the_welcome_not_an_endless_connecting(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(&backend, cx);
    store.update(cx, |store, cx| store.begin_setup(true, cx));
    assert_eq!(gate(&store, cx), Gate::Setup);
    assert_eq!(open(&store, cx), Some(SetupStep::Home));
    assert_eq!(glow(&store, cx), 0, "asleep and dim");
    store.update(cx, |store, cx| {
        let down = || Err(Error::Unreachable("refused".into()));
        let start = Instant::now();
        store.apply(RoomId::MAIN, down(), start, cx);
        store.apply(RoomId::MAIN, down(), start + super::super::gate::QUIET, cx);
    });
    assert_eq!(
        gate(&store, cx),
        Gate::Setup,
        "nothing answers: still the welcome"
    );
}

#[gpui_kit::test]
fn a_configured_install_keeps_connecting_and_not_your_device(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let store = store(&backend, cx);
    store.update(cx, |store, cx| store.begin_setup(false, cx));
    assert_eq!(gate(&store, cx), Gate::Connecting);
    store.update(cx, Store::refuse_for_tests);
    assert_eq!(gate(&store, cx), Gate::NotYourDevice);
    assert!(
        !backend.calls().contains(&Call::Setup),
        "the checklist is read only with a session"
    );
}

#[gpui_kit::test]
fn an_unlit_ember_opens_the_setup_on_light_it_and_the_picker_looks(cx: &mut TestAppContext) {
    for unlit in [SetupState::Missing, SetupState::Off] {
        let backend = Arc::new(FakeBackend::new([]).with_session());
        backend.setup.push_page(Ok(provider(unlit.clone())));
        let store = store(&backend, cx);
        store.update(cx, |store, cx| store.begin_setup(false, cx));
        assert_eq!(gate(&store, cx), Gate::Connecting);
        poll(&store, json!({}), cx);
        assert_eq!(gate(&store, cx), Gate::Setup, "{unlit:?}");
        assert_eq!(open(&store, cx), Some(SetupStep::Light));
        assert!(store.read_with(cx, |store, _| store.setup().is_done(SetupStep::Home)));
        assert_eq!(glow(&store, cx), 1, "a spark: it has a home");
        let calls = backend.calls();
        assert!(
            calls.contains(&Call::AgentProvider),
            "what is saved is read"
        );
        assert!(
            calls
                .iter()
                .any(|call| matches!(call, Call::TestAgentProvider(_))),
            "a local Ollama is looked for: {calls:?}"
        );
    }
}

#[gpui_kit::test]
fn a_lit_ember_opens_the_app(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).with_session());
    backend.setup.push_page(Ok(provider(SetupState::Ready)));
    let store = store(&backend, cx);
    store.update(cx, |store, cx| store.begin_setup(false, cx));
    poll(&store, json!({}), cx);
    assert_eq!(gate(&store, cx), Gate::Open);
    assert_eq!(
        backend
            .calls()
            .iter()
            .filter(|call| **call == Call::Setup)
            .count(),
        1
    );
    poll(&store, json!({}), cx);
    assert_eq!(
        backend
            .calls()
            .iter()
            .filter(|call| **call == Call::Setup)
            .count(),
        1,
        "asked once a launch"
    );
}

#[gpui_kit::test]
fn a_server_without_the_checklist_opens_the_app(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).with_session());
    backend.setup.push_page(Err(Error::Refused {
        status: Some(404),
        message: None,
    }));
    let store = store(&backend, cx);
    store.update(cx, |store, cx| store.begin_setup(false, cx));
    poll(&store, json!({}), cx);
    assert_eq!(gate(&store, cx), Gate::Open);
}

#[gpui_kit::test]
fn each_step_done_warms_the_ember_to_lit(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).at(HERE));
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let store = store(&backend, cx);
    store.update(cx, |store, cx| store.begin_setup(true, cx));
    assert_eq!(
        (open(&store, cx), glow(&store, cx)),
        (Some(SetupStep::Home), 0)
    );
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().note()),
        "Not lit yet."
    );

    // Another computer: its code, typed in the form.
    store.update(cx, |store, cx| {
        store.choose_home(HomeChoice::Elsewhere, cx);
        store.continue_home(cx);
        assert!(store.setup().shows_elsewhere(), "the pairing form shows");
        store.pair("ABCD-EFGH".to_owned(), cx, |_, _| {});
    });
    cx.run_until_parked();
    assert_eq!(
        (open(&store, cx), glow(&store, cx)),
        (Some(SetupStep::Light), 1)
    );
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().state(SetupStep::Home).clone()),
        StepState::Done(format!(
            "It lives on {}.",
            place(&ServerUrl::parse(HERE).unwrap())
        ))
    );
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().note()),
        "A spark."
    );

    poll(&store, devices(&[(1, "This computer")]), cx);
    store.update(cx, Store::setup_lit);
    cx.run_until_parked();
    assert_eq!(
        (open(&store, cx), glow(&store, cx)),
        (Some(SetupStep::Reach), 2)
    );
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().state(SetupStep::Light).clone()),
        StepState::Done("Lit.".to_owned())
    );
    assert!(
        backend.calls().contains(&Call::NewPairingCode),
        "the phone's code is made"
    );

    poll(
        &store,
        devices(&[(1, "This computer"), (2, "Example iPhone")]),
        cx,
    );
    assert_eq!(
        (open(&store, cx), glow(&store, cx)),
        (Some(SetupStep::World), 3)
    );
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().state(SetupStep::Reach).clone()),
        StepState::Done("Example iPhone can reach it.".to_owned())
    );
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().note()),
        "Getting warmer."
    );

    store.update(cx, Store::world_done);
    assert_eq!(
        open(&store, cx),
        Some(SetupStep::Share),
        "nothing connected: put off"
    );
    store.update(cx, Store::finish_setup);
    assert_eq!((open(&store, cx), glow(&store, cx)), (None, BRIGHTEST));
    assert_eq!(gate(&store, cx), Gate::Setup, "the lit screen");
    assert!(
        store
            .read_with(cx, |store, _| store.setup_summary())
            .contains("can reach you on Example iPhone")
    );
    store.update(cx, Store::finish_setup);
    assert_eq!(gate(&store, cx), Gate::Open);
}

#[gpui_kit::test]
fn a_phone_is_new_only_beside_the_devices_signed_in_before(cx: &mut TestAppContext) {
    let backend = Arc::new(
        FakeBackend::new([])
            .with_session()
            .signed_in_as(DeviceId::new(1)),
    );
    let store = store(&backend, cx);
    // "Let it reach you" opens before any state came: the first one only says who was in.
    store.update(cx, |store, cx| {
        store.preview_setup(Some(SetupStep::Reach), cx);
    });
    poll(
        &store,
        devices(&[(1, "This computer"), (3, "Example laptop")]),
        cx,
    );
    assert_eq!(open(&store, cx), Some(SetupStep::Reach));
    poll(
        &store,
        devices(&[
            (1, "This computer"),
            (3, "Example laptop"),
            (4, "Example phone"),
        ]),
        cx,
    );
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().state(SetupStep::Reach).clone()),
        StepState::Done("Example phone can reach it.".to_owned())
    );
}

#[gpui_kit::test]
fn the_optional_steps_can_wait(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).with_session());
    let store = store(&backend, cx);
    store.update(cx, |store, cx| {
        store.preview_setup(Some(SetupStep::Reach), cx);
    });
    assert_eq!(glow(&store, cx), 2);
    store.update(cx, Store::setup_later);
    assert_eq!(open(&store, cx), Some(SetupStep::World));
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().state(SetupStep::Reach).clone()),
        StepState::Skipped
    );
    assert_eq!(glow(&store, cx), 2, "a step put off doesn't warm it");
    store.update(cx, Store::setup_later);
    store.update(cx, Store::setup_later);
    assert_eq!(open(&store, cx), None, "every step behind: the lit screen");
}

#[gpui_kit::test]
fn a_connection_from_the_world_step_is_said(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).with_session());
    backend.setup.push_page(Ok(page(&[
        (SetupItemId::Mac, SetupState::Ready),
        (SetupItemId::Gmail, SetupState::Missing),
        (SetupItemId::Slack, SetupState::Error),
        (SetupItemId::Chrome, SetupState::Missing),
    ])));
    let store = store(&backend, cx);
    store.update(cx, |store, cx| {
        store.preview_setup(Some(SetupStep::World), cx);
    });
    cx.run_until_parked();
    store.update(cx, |store, cx| {
        let tile = |store: &Store, tile| store.setup().tile(tile, false);
        assert_eq!(tile(store, Tile::Calendar), TileState::Connected);
        assert_eq!(tile(store, Tile::Slack), TileState::Failing);
        assert_eq!(tile(store, Tile::Gmail), TileState::Open);
        store.ask_tile(Tile::Gmail, cx);
        assert_eq!(tile(store, Tile::Gmail), TileState::Waiting);
        assert_eq!(store.setup().tile(Tile::Chrome, true), TileState::Connected);
        store.world_done(cx);
        assert_eq!(
            store.setup().state(SetupStep::World),
            &StepState::Done("Connected: Calendar.".to_owned())
        );
        assert!(store.setup_summary().contains("can read your calendar"));
        assert_eq!(
            store.setup_waiting().as_deref(),
            Some("Gmail, Slack and Chrome are waiting in Settings › Connections whenever you want them.")
        );
    });
}

#[gpui_kit::test]
fn a_home_here_starts_and_pairs_with_its_own_code(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).at(HERE));
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let home = FakeHome::at(HERE, Some("WXYZ-2345"));
    let starts = Rc::clone(&home.starts);
    let store = store_with(&backend, None, home, cx);
    store.update(cx, |store, cx| {
        store.begin_setup(true, cx);
        store.continue_home(cx);
    });
    cx.run_until_parked();
    assert_eq!(starts.get(), 1);
    assert!(
        backend
            .calls()
            .contains(&Call::Pair("WXYZ-2345".to_owned()))
    );
    assert_eq!(open(&store, cx), Some(SetupStep::Light));
}

#[gpui_kit::test]
fn a_home_here_that_hands_no_code_asks_for_one(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).at(HERE));
    let store = store_with(&backend, None, FakeHome::at(HERE, None), cx);
    store.update(cx, |store, cx| {
        store.begin_setup(true, cx);
        store.continue_home(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        store.read_with(cx, |store, _| store.setup().here().clone()),
        Here::TypeCode
    );
    assert_eq!(open(&store, cx), Some(SetupStep::Home));
}

#[gpui_kit::test]
fn without_a_home_here_another_computer_is_offered(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).at(HERE));
    let home = FakeHome {
        available: false,
        ..FakeHome::at(HERE, None)
    };
    let starts = Rc::clone(&home.starts);
    let store = store_with(&backend, None, home, cx);
    store.update(cx, |store, cx| {
        store.begin_setup(true, cx);
        store.continue_home(cx);
    });
    cx.run_until_parked();
    assert_eq!(starts.get(), 0, "nothing is started");
    store.read_with(cx, |store, _| {
        assert_eq!(store.setup().here(), &Here::Failed(CANT_HERE.to_owned()));
        assert!(store.setup().shows_elsewhere());
    });
}

#[gpui_kit::test]
fn a_home_here_that_fails_to_start_says_why(cx: &mut TestAppContext) {
    for (started, why) in [
        (
            Err("It stopped as it started."),
            "It stopped as it started.",
        ),
        (
            Ok("http://127.0.0.1:9000"),
            "Emberhut started here at http://127.0.0.1:9000",
        ),
    ] {
        let backend = Arc::new(FakeBackend::new([]).at(HERE));
        let home = FakeHome {
            started,
            ..FakeHome::at(HERE, Some("WXYZ-2345"))
        };
        let store = store_with(&backend, None, home, cx);
        store.update(cx, |store, cx| {
            store.begin_setup(true, cx);
            store.continue_home(cx);
        });
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            let Here::Failed(said) = store.setup().here() else {
                panic!("{:?}", store.setup().here());
            };
            assert!(said.starts_with(why), "{said}");
            assert!(store.setup().shows_elsewhere());
            assert_eq!(store.setup().open(), Some(SetupStep::Home));
        });
        assert!(
            !backend
                .calls()
                .iter()
                .any(|call| matches!(call, Call::Pair(_))),
            "no code goes anywhere"
        );
    }
}

#[gpui_kit::test]
fn a_finished_setup_is_kept_and_never_shows_again(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let files = AppFiles::in_dir(dir.path());
    let backend = Arc::new(FakeBackend::new([]).with_session());
    let store = store_with(&backend, Some(files.clone()), NoLocalHome, cx);
    store.update(cx, |store, cx| {
        store.preview_setup(None, cx);
        store.finish_setup(cx);
    });
    cx.run_until_parked();
    let kept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(files.setup()).unwrap()).unwrap();
    assert_eq!(kept, json!({"ended": "finished"}));

    // The next launch: a fresh install's welcome doesn't come back, nor does an unlit ember
    // bring the whole setup back: a quiet line says so.
    let backend = Arc::new(FakeBackend::new([]));
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let store = store_with(&backend, Some(files.clone()), NoLocalHome, cx);
    store.update(cx, |store, cx| store.begin_setup(true, cx));
    assert_eq!(gate(&store, cx), Gate::Connecting);
    store.read_with(cx, |store, _| {
        assert_eq!(store.setup().ended(), Some(Ending::Finished));
    });
    let backend = Arc::new(FakeBackend::new([]).with_session());
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let store = store_with(&backend, Some(files), NoLocalHome, cx);
    store.update(cx, |store, cx| store.begin_setup(false, cx));
    poll(&store, json!({}), cx);
    assert_eq!(gate(&store, cx), Gate::Open);
    store.read_with(cx, |store, _| {
        assert_eq!(store.setup().lit(), LitCheck::Unlit);
        assert!(store.shows_unlit_line());
    });
}

#[gpui_kit::test]
fn light_it_put_off_ends_the_setup_unlit(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let files = AppFiles::in_dir(dir.path());
    let backend = Arc::new(FakeBackend::new([]).with_session());
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let store = store_with(&backend, Some(files.clone()), NoLocalHome, cx);
    store.update(cx, |store, cx| store.begin_setup(false, cx));
    poll(&store, json!({}), cx);
    assert_eq!(open(&store, cx), Some(SetupStep::Light));
    store.update(cx, Store::setup_later);
    cx.run_until_parked();
    assert_eq!(gate(&store, cx), Gate::Open);
    assert!(store.read_with(cx, |store, _| store.shows_unlit_line()));
    let kept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(files.setup()).unwrap()).unwrap();
    assert_eq!(kept, json!({"ended": "skipped"}));
}

#[gpui_kit::test]
fn a_home_here_at_another_address_moves_the_app_there_pairs_and_lights(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let files = AppFiles::in_dir(dir.path());
    let backend = Arc::new(FakeBackend::new([]).at(HERE));
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let connector = Arc::new(FakeConnector::default());
    let home = FakeHome::at("http://127.0.0.1:18995", Some("WXYZ-2345")).keeping(Ok(()));
    let store = store_moving(
        &backend,
        Some(files.clone()),
        home,
        Reconnect::new(Arc::clone(&connector) as Arc<dyn Connector>, None),
        cx,
    );
    store.update(cx, |store, cx| {
        store.start_poll(cx);
        store.begin_setup(true, cx);
    });
    cx.run_until_parked();
    let polled = |backend: &FakeBackend| {
        backend
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::State(..)))
            .count()
    };
    assert_eq!(polled(&backend), 1, "the poll ran on the first address");
    store.update(cx, Store::continue_home);
    cx.run_until_parked();

    // The address is kept for the next launch, and the app talks to it now.
    let kept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(files.config()).unwrap()).unwrap();
    assert_eq!(kept, json!({"server": "http://127.0.0.1:18995"}));
    let made = connector.made();
    let [moved] = made.as_slice() else {
        panic!("one backend made, not {}", made.len());
    };
    store.read_with(cx, |store, _| {
        assert_eq!(store.server().as_str(), "http://127.0.0.1:18995");
        assert!(Arc::ptr_eq(
            store.backend(),
            &(Arc::clone(moved) as Arc<dyn crate::backend::Backend>)
        ));
    });
    assert!(
        polled(moved) >= 1,
        "the poll started again on the new address"
    );
    assert_eq!(polled(&backend), 1, "and stopped on the old one");

    // Paired through the normal sign-in, with the code the home made: on to "Light it".
    assert_eq!(paired(moved), ["WXYZ-2345"]);
    assert!(
        paired(&backend).is_empty(),
        "no code goes to the old address"
    );
    assert!(store.read_with(cx, |store, _| store.is_signed_in()));
    assert_eq!(open(&store, cx), Some(SetupStep::Light));
    assert_eq!(here(&store, cx), Here::Idle);
}

#[gpui_kit::test]
fn localhost_and_the_loopback_address_on_one_port_are_one_place(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).at("http://localhost:18990"));
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let connector = Arc::new(FakeConnector::default());
    let store = store_moving(
        &backend,
        None,
        FakeHome::at(HERE, Some("WXYZ-2345")),
        Reconnect::new(Arc::clone(&connector) as Arc<dyn Connector>, None),
        cx,
    );
    store.update(cx, |store, cx| {
        store.begin_setup(true, cx);
        store.continue_home(cx);
    });
    cx.run_until_parked();
    assert!(connector.made().is_empty(), "no move");
    assert_eq!(
        store
            .read_with(cx, |store, _| store.server().clone())
            .as_str(),
        "http://localhost:18990"
    );
    assert_eq!(paired(&backend), ["WXYZ-2345"]);
    assert_eq!(open(&store, cx), Some(SetupStep::Light));
}

#[gpui_kit::test]
fn the_variable_that_named_the_server_wins_and_says_so(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let files = AppFiles::in_dir(dir.path());
    let backend = Arc::new(FakeBackend::new([]).at(HERE));
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let connector = Arc::new(FakeConnector::default());
    let store = store_moving(
        &backend,
        Some(files.clone()),
        FakeHome::at("http://127.0.0.1:18995", Some("WXYZ-2345")).keeping(Ok(())),
        Reconnect::new(
            Arc::clone(&connector) as Arc<dyn Connector>,
            Some("EMBERHUT_URL"),
        ),
        cx,
    );
    store.update(cx, |store, cx| {
        store.begin_setup(true, cx);
        store.continue_home(cx);
    });
    cx.run_until_parked();
    let Here::Failed(said) = here(&store, cx) else {
        panic!("{:?}", here(&store, cx));
    };
    for part in [
        "EMBERHUT_URL",
        "http://127.0.0.1:18995",
        HERE,
        "Unset EMBERHUT_URL",
    ] {
        assert!(said.contains(part), "{said:?} names {part}");
    }
    assert!(connector.made().is_empty(), "no move");
    assert!(!files.config().exists(), "nothing kept");
    assert!(paired(&backend).is_empty(), "no code goes anywhere");
    store.read_with(cx, |store, _| {
        assert_eq!(store.server().as_str(), HERE);
        assert!(
            store.setup().shows_elsewhere(),
            "another computer is offered"
        );
        assert_eq!(store.setup().open(), Some(SetupStep::Home));
    });
}

#[gpui_kit::test]
fn a_home_here_is_kept_lit_while_closed_and_from_login(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).at(HERE));
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let home = FakeHome::at(HERE, Some("WXYZ-2345")).keeping(Ok(()));
    let keeps = Rc::clone(&home.keeps);
    let store = store_with(&backend, None, home, cx);
    store.update(cx, |store, cx| {
        store.begin_setup(true, cx);
        store.continue_home(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        *keeps.borrow(),
        [KeepLit {
            when_closed: true,
            at_login: true
        }]
    );
    assert_eq!(
        home_line(&store, cx),
        StepState::Done(format!(
            "It lives on {HERE_WORDS} and stays lit when the window is closed."
        ))
    );
    assert_eq!(open(&store, cx), Some(SetupStep::Light));

    // Change: both off, and the line says it runs while the app does.
    store.update(cx, Store::open_keep_sheet);
    assert!(store.read_with(cx, |store, _| store.setup().shows_keep_sheet()));
    store.update(cx, |store, cx| {
        store.change_keep_lit(KeepLit::default(), cx);
    });
    cx.run_until_parked();
    assert_eq!(keeps.borrow().last(), Some(&KeepLit::default()));
    assert_eq!(
        home_line(&store, cx),
        StepState::Done(format!("It lives on {HERE_WORDS} while Emberhut is open."))
    );
    store.update(cx, Store::close_keep_sheet);
    assert!(!store.read_with(cx, |store, _| store.setup().shows_keep_sheet()));
}

#[gpui_kit::test]
fn a_home_here_that_cant_stay_lit_without_the_app_still_pairs(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).at(HERE));
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let home = FakeHome::at(HERE, Some("WXYZ-2345")).keeping(Err("No service manager."));
    let store = store_with(&backend, None, home, cx);
    store.update(cx, |store, cx| {
        store.begin_setup(true, cx);
        store.continue_home(cx);
    });
    cx.run_until_parked();
    assert_eq!(paired(&backend), ["WXYZ-2345"]);
    assert_eq!(open(&store, cx), Some(SetupStep::Light));
    assert_eq!(
        home_line(&store, cx),
        StepState::Done(format!("It lives on {HERE_WORDS} while Emberhut is open."))
    );

    // Turned on from the sheet, it says why it can't, and nothing changes.
    store.update(cx, |store, cx| {
        store.open_keep_sheet(cx);
        store.change_keep_lit(KEEP_LIT, cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        assert_eq!(store.setup().keep_problem(), Some("No service manager."));
        assert_eq!(store.setup().keep(), Some(KeepLit::default()));
    });
}

#[gpui_kit::test]
fn a_home_that_isnt_the_apps_to_keep_has_no_change(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).at(HERE));
    backend.setup.push_page(Ok(provider(SetupState::Missing)));
    let store = store_with(&backend, None, FakeHome::at(HERE, Some("WXYZ-2345")), cx);
    store.update(cx, |store, cx| {
        store.begin_setup(true, cx);
        store.continue_home(cx);
    });
    cx.run_until_parked();
    assert_eq!(open(&store, cx), Some(SetupStep::Light));
    assert_eq!(
        home_line(&store, cx),
        StepState::Done(format!("It lives on {HERE_WORDS}."))
    );
    store.update(cx, Store::open_keep_sheet);
    store.read_with(cx, |store, _| {
        assert_eq!(store.setup().keep(), None);
        assert!(!store.setup().shows_keep_sheet());
    });
}

#[gpui_kit::test]
fn a_relaunch_with_the_hut_out_starts_it_and_connects(cx: &mut TestAppContext) {
    let down = Err(Error::Unreachable("refused".into()));
    let up = Ok(StateReply::Fresh(FreshState {
        state: Box::new(state(json!({}))),
        etag: None,
    }));
    let backend = Arc::new(FakeBackend::new([down, up]).at(HERE).with_session());
    let home = FakeHome::at(HERE, None);
    home.relights
        .borrow_mut()
        .push_back(Relit::Started(ServerUrl::parse(HERE).unwrap()));
    let relit_at = Rc::clone(&home.relit_at);
    let store = store_with(&backend, None, home, cx);
    store.update(cx, |store, cx| {
        store.start_poll(cx);
        store.begin_setup(false, cx);
    });
    cx.run_until_parked();
    assert_eq!(gate(&store, cx), Gate::Connecting, "the hut is out");
    store.update(cx, Store::relight_home);
    cx.run_until_parked();
    assert_eq!(relit_at.borrow().len(), 1);
    assert_eq!(relit_at.borrow()[0].as_str(), HERE);
    assert_eq!(
        gate(&store, cx),
        Gate::Open,
        "started, then the poll went at once"
    );
    assert_eq!(
        store.read_with(cx, |store, _| store.relight_problem().map(str::to_owned)),
        None
    );
}

#[gpui_kit::test]
fn a_relaunch_elsewhere_or_pinned_starts_nothing(cx: &mut TestAppContext) {
    // An address that isn't the hut's: the home says so, and the app keeps connecting.
    let backend = Arc::new(FakeBackend::new([]).at("https://ember.example.com"));
    let home = FakeHome::at(HERE, None);
    let relit_at = Rc::clone(&home.relit_at);
    let store = store_with(&backend, None, home, cx);
    store.update(cx, Store::relight_home);
    cx.run_until_parked();
    assert_eq!(relit_at.borrow().len(), 1);
    assert_eq!(gate(&store, cx), Gate::Connecting);
    assert_eq!(
        store.read_with(cx, |store, _| store.relight_problem().map(str::to_owned)),
        None
    );

    // EMBERHUT_URL named the server: the home isn't even asked.
    let backend = Arc::new(FakeBackend::new([]).at(HERE));
    let home = FakeHome::at(HERE, None);
    let relit_at = Rc::clone(&home.relit_at);
    let connector = Arc::new(FakeConnector::default()) as Arc<dyn Connector>;
    let store = store_moving(
        &backend,
        None,
        home,
        Reconnect::new(connector, Some("EMBERHUT_URL")),
        cx,
    );
    store.update(cx, Store::relight_home);
    cx.run_until_parked();
    assert!(relit_at.borrow().is_empty());
}

#[gpui_kit::test]
fn a_hut_that_doesnt_start_again_says_why_and_tries_again(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]).at(HERE).with_session());
    let home = FakeHome::at(HERE, None);
    home.relights.borrow_mut().extend([
        Relit::Failed("The hut didn't answer within 60 seconds.".to_owned()),
        Relit::Started(ServerUrl::parse(HERE).unwrap()),
    ]);
    let relit_at = Rc::clone(&home.relit_at);
    let store = store_with(&backend, None, home, cx);
    store.update(cx, Store::relight_home);
    cx.run_until_parked();
    let problem = |cx: &TestAppContext| {
        store.read_with(cx, |store, _| store.relight_problem().map(str::to_owned))
    };
    assert_eq!(
        problem(cx).as_deref(),
        Some("The hut didn't answer within 60 seconds.")
    );
    assert_eq!(gate(&store, cx), Gate::Connecting);
    store.update(cx, Store::relight_home);
    cx.run_until_parked();
    assert_eq!(relit_at.borrow().len(), 2, "Try again asks again");
    assert_eq!(problem(cx), None);
}
