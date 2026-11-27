//! The updater in the store, on GPUI's headless test platform with a scripted backend and
//! installer: a check that downloads, verifies and stages a build; the bar and its X; a payload
//! that fails closed; the menu's answers; Restart to Update; when a build installs by itself.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use emberhut_api::ids::RoomId;
use emberhut_api::releases::{Build, Release, ReleaseFeed};
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use serde_json::json;
use sha2::{Digest as _, Sha256};

use super::{Answer, Phase, ago, installs_alone};
use crate::backend::fake::{Call, FakeBackend};
use crate::store::{Store, ThemeState};
use crate::updates::Engine;
use crate::updates::installer::{InstallError, Installer, Staged, Verdict, Watch};
use crate::updates::tests::TestKey;
use crate::updates::version::Running;

const PAYLOAD: &[u8] = b"a disk image";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        write!(text, "{byte:02x}").unwrap();
        text
    })
}

/// A feed offering build 15 (`version`), its payload signed by `key` with its trusted comment.
fn feed(key: &TestKey, version: &str) -> ReleaseFeed {
    let notes = "- Rooms open faster.";
    let sha = hex(&Sha256::digest(PAYLOAD));
    let comment = format!(
        "emberhut-desktop macos arm64 {version} build 15 sha256:{sha} notes:{}",
        hex(&Sha256::digest(notes.as_bytes()))
    );
    serde_json::from_value(
        json!({"format": 1, "product": "emberhut-desktop", "platform": "macos",
        "published": "2026-12-08T10:00:00.000Z", "releases": [
            {"version": version, "build": 15, "date": "2026-12-08", "notes": notes,
             "minimumOs": "13.0", "assets": [{"arch": "arm64",
                "url": "/api/releases/files/Emberhut_15_macos_arm64.dmg", "size": PAYLOAD.len(),
                "sha256": sha, "signature": key.sign(PAYLOAD, &comment)}]},
            {"version": "0.2.1", "build": 13, "date": "2026-12-01", "notes": "- A fix.",
             "minimumOs": "13.0", "assets": []}]}),
    )
    .unwrap()
}

/// An installer that stages and swaps nothing, noting what it was asked.
#[derive(Debug, Default)]
struct FakeInstaller {
    staged: Mutex<Vec<Build>>,
    swapped: Mutex<Vec<Build>>,
    /// The backend its watches look at when they start the new copy.
    backend: std::sync::OnceLock<Arc<FakeBackend>>,
}

impl Installer for FakeInstaller {
    fn stage(&self, payload: &std::path::Path, release: &Release) -> Result<Staged, InstallError> {
        assert_eq!(std::fs::read(payload).unwrap(), PAYLOAD);
        self.staged.lock().unwrap().push(release.build);
        Ok(Staged {
            build: release.build,
            version: release.version.clone(),
        })
    }

    fn swap(&self, staged: &Staged, _running: Running) -> Result<Box<dyn Watch>, InstallError> {
        self.swapped.lock().unwrap().push(staged.build);
        Ok(Box::new(Kept(self.backend.get().cloned())))
    }
}

/// A watch whose new copy is kept at once, noting whether the old copy's backend had stopped
/// by then.
struct Kept(Option<Arc<FakeBackend>>);

/// Whether the backend had stopped when the last watch started the new copy.
static STOPPED_BEFORE_THE_NEW_COPY: AtomicBool = AtomicBool::new(false);

impl Watch for Kept {
    fn run(self: Box<Self>, _stop: &AtomicBool) -> Verdict {
        let stopped = self
            .0
            .is_some_and(|backend| backend.calls().contains(&Call::Stop));
        STOPPED_BEFORE_THE_NEW_COPY.store(stopped, Ordering::Relaxed);
        Verdict::Kept
    }
}

static WATCHES_ENDED: AtomicUsize = AtomicUsize::new(0);

fn watch_ended(_: &mut gpui_kit::App) {
    WATCHES_ENDED.fetch_add(1, Ordering::Relaxed);
}

struct Setup {
    backend: Arc<FakeBackend>,
    installer: Arc<FakeInstaller>,
    store: Entity<Store>,
    key: TestKey,
    _dir: tempfile::TempDir,
}

/// A store whose updater is build 12 of 0.2.0, trusting a test key (or none).
fn setup(cx: &mut TestAppContext, trusting: bool) -> Setup {
    let backend = Arc::new(FakeBackend::new([]));
    let installer = Arc::new(FakeInstaller::default());
    installer.backend.set(Arc::clone(&backend)).unwrap();
    let key = TestKey::new();
    let keys: &'static [_] = if trusting {
        Box::leak(vec![key.trusted].into_boxed_slice())
    } else {
        &[]
    };
    let dir = tempfile::tempdir().unwrap();
    let engine = Arc::new(Engine::for_tests(
        12,
        keys,
        dir.path(),
        Arc::clone(&installer) as Arc<dyn Installer>,
    ));
    let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
    let store = cx.new(|_| {
        let mut store = Store::idle(shared, None, ThemeState::load(None), RoomId::MAIN);
        store.use_test_updater(engine, watch_ended);
        store
    });
    Setup {
        backend,
        installer,
        store,
        key,
        _dir: dir,
    }
}

fn check(setup: &Setup, cx: &mut TestAppContext) -> Answer {
    let task = setup.store.update(cx, Store::check_for_updates);
    cx.run_until_parked();
    cx.foreground_executor().block_test(task)
}

#[gpui_kit::test]
fn a_newer_build_is_downloaded_verified_and_staged_and_the_bar_says_so(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    setup
        .backend
        .releases
        .push_feed(Ok(Some(feed(&setup.key, "0.3.0"))));
    setup.backend.releases.push_payload(Ok(PAYLOAD.to_vec()));

    let answer = check(&setup, cx);

    let Answer::Ready { title, notes } = answer else {
        panic!("{answer:?}")
    };
    assert_eq!(title, "Emberhut 0.3.0 (build 15) is ready");
    assert!(notes.starts_with("- Rooms open faster."), "{notes}");
    assert!(
        notes.contains("Earlier releases (unverified):\n0.2.1: - A fix."),
        "{notes}"
    );
    assert_eq!(
        setup.backend.calls(),
        [
            Call::ReleaseFeed(1),
            Call::DownloadRelease(
                "/api/releases/files/Emberhut_15_macos_arm64.dmg".into(),
                PAYLOAD.len() as u64
            ),
        ]
    );
    assert_eq!(*setup.installer.staged.lock().unwrap(), [Build::new(15)]);
    setup.store.read_with(cx, |store, _| {
        let bar = store.updates().bar().unwrap();
        assert_eq!(bar.title, "Emberhut 0.3.0 is ready");
        let rows = store.updates().rows(SystemTime::now());
        assert_eq!(rows.version, "0.2.0 (build 12)");
        assert_eq!(rows.ready.as_deref(), Some("Build 15"));
        assert_eq!(rows.last_checked.as_deref(), Some("just now"));
    });
}

#[gpui_kit::test]
fn the_bar_put_away_stays_away_until_a_newer_build(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    setup
        .backend
        .releases
        .push_feed(Ok(Some(feed(&setup.key, "0.2.0"))));
    setup.backend.releases.push_payload(Ok(PAYLOAD.to_vec()));
    check(&setup, cx);
    let title = setup
        .store
        .read_with(cx, |store, _| store.updates().bar().map(|bar| bar.title));
    assert_eq!(title.as_deref(), Some("Build 15 is ready"), "same version");
    setup.store.update(cx, Store::dismiss_update_bar);
    assert_eq!(
        setup.store.read_with(cx, |store, _| store.updates().bar()),
        None
    );
    // The next check finds the same build ready: no download, and the bar stays away.
    setup
        .backend
        .releases
        .push_feed(Ok(Some(feed(&setup.key, "0.2.0"))));
    setup.store.update(cx, Store::check_updates).detach();
    cx.run_until_parked();
    assert_eq!(
        setup.store.read_with(cx, |store, _| store.updates().bar()),
        None
    );
    let downloads = setup
        .backend
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::DownloadRelease(..)))
        .count();
    assert_eq!(downloads, 1);
}

#[gpui_kit::test]
fn with_no_trusted_key_every_update_fails_closed(cx: &mut TestAppContext) {
    let setup = setup(cx, false);
    setup
        .backend
        .releases
        .push_feed(Ok(Some(feed(&setup.key, "0.3.0"))));
    setup.backend.releases.push_payload(Ok(PAYLOAD.to_vec()));

    let answer = check(&setup, cx);

    assert_eq!(
        answer,
        Answer::Failed(
            "The update didn't verify; Emberhut stays at 0.2.0 (this app trusts no update key yet)"
                .into()
        )
    );
    assert_eq!(*setup.installer.staged.lock().unwrap(), []);
    setup.store.read_with(cx, |store, _| {
        assert_eq!(store.updates().bar(), None);
        assert!(matches!(store.updates().phase(), Phase::Failed(_)));
    });
    // The same payload isn't fetched again.
    setup
        .backend
        .releases
        .push_feed(Ok(Some(feed(&setup.key, "0.3.0"))));
    let again = check(&setup, cx);
    assert_eq!(
        again,
        Answer::Failed("An update didn't verify lately; Emberhut tries again later".into())
    );
    let downloads = setup
        .backend
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::DownloadRelease(..)))
        .count();
    assert_eq!(downloads, 1);
}

#[gpui_kit::test]
fn nothing_newer_or_nothing_published_is_the_latest(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    assert_eq!(
        check(&setup, cx),
        Answer::Latest {
            running: "0.2.0, build 12".into(),
            detail: Some("Your server has published no update yet".into())
        }
    );
    let mut old = feed(&setup.key, "0.3.0");
    old.releases
        .retain(|release| release.build < Build::new(15));
    setup.backend.releases.push_feed(Ok(Some(old)));
    let Answer::Latest { detail, .. } = check(&setup, cx) else {
        panic!("the latest")
    };
    assert!(
        detail.is_some_and(|detail| detail.starts_with("Published ")),
        "the feed's age"
    );
}

#[gpui_kit::test]
fn an_unreachable_server_says_so(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    setup
        .backend
        .releases
        .push_feed(Err(crate::backend::Error::Unreachable("refused".into())));
    assert_eq!(
        check(&setup, cx),
        Answer::Failed("Couldn't reach your server".into())
    );
}

#[gpui_kit::test]
fn a_build_already_downloaded_installs_at_once_from_the_menu(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    setup
        .backend
        .releases
        .push_feed(Ok(Some(feed(&setup.key, "0.3.0"))));
    setup.backend.releases.push_payload(Ok(PAYLOAD.to_vec()));
    check(&setup, cx);
    let ended = WATCHES_ENDED.load(Ordering::Relaxed);

    assert_eq!(check(&setup, cx), Answer::Installing);

    assert_eq!(*setup.installer.swapped.lock().unwrap(), [Build::new(15)]);
    assert_eq!(
        WATCHES_ENDED.load(Ordering::Relaxed),
        ended + 1,
        "the watch ran, then the app quits"
    );
    assert!(
        STOPPED_BEFORE_THE_NEW_COPY.load(Ordering::Relaxed),
        "the old copy saved its session and stopped its requests before the new one started"
    );
    cx.run_until_parked();
    let calls = setup.backend.calls();
    let stopped = calls.iter().position(|call| *call == Call::Stop).unwrap();
    assert_eq!(
        calls[stopped + 1..],
        [],
        "nothing asked of the server after"
    );
}

#[gpui_kit::test]
fn a_copy_that_doesnt_update_itself_says_why(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
    let store = cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN));
    let task = store.update(cx, Store::check_for_updates);
    let answer = cx.foreground_executor().block_test(task);
    assert!(
        matches!(&answer, Answer::Unavailable { running, .. } if running.starts_with(env!("CARGO_PKG_VERSION"))),
        "{answer:?}"
    );
    store.read_with(cx, |store, _| {
        let rows = store.updates().rows(SystemTime::now());
        assert_eq!(rows.last_checked, None, "nothing is checked");
        assert!(!rows.can_check);
    });
}

#[test]
fn a_ready_build_installs_alone_only_after_ten_idle_minutes_with_nothing_being_written() {
    let minutes = |n: u64| Some(Duration::from_secs(n * 60));
    assert!(installs_alone(minutes(10), false));
    assert!(!installs_alone(minutes(9), false));
    assert!(
        !installs_alone(minutes(30), true),
        "a message is being written"
    );
    assert!(!installs_alone(None, false), "the system doesn't say");
}

#[test]
fn times_read_as_how_long_ago() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
    let before = |seconds| now - Duration::from_secs(seconds);
    assert_eq!(ago(before(5), now), "just now");
    assert_eq!(ago(before(60), now), "1 minute ago");
    assert_eq!(ago(before(150), now), "2 minutes ago");
    assert_eq!(ago(before(3 * 3600), now), "3 hours ago");
    assert_eq!(ago(before(86_400), now), "1 day ago");
    assert_eq!(ago(now + Duration::from_secs(9), now), "just now");
}
