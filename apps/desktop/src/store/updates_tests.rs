//! The updater in the store, on GPUI's headless test platform with scripted GitHub releases and
//! a scripted installer: a check that downloads, verifies and stages a build; the bar and its X;
//! a payload that fails closed; the channels, and a candidate on Stable never going back; GitHub
//! limiting requests; the menu's answers; Install and restart; automatic checks and when a build
//! installs by itself.

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
use crate::updates::channel::Channel;
use crate::updates::github::fake::{Asked, FakeSource};
use crate::updates::github::{FetchError, Listed};
use crate::updates::installer::{InstallError, Installer, Staged, Verdict, Watch};
use crate::updates::tests::TestKey;
use crate::updates::version::Running;

const PAYLOAD: &[u8] = b"a disk image";

/// Where the repository's release files download from.
const DOWNLOADS: &str = "https://github.com/emberhuthq/emberhut/releases/download";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        write!(text, "{byte:02x}").unwrap();
        text
    })
}

/// The address of `tag`'s macOS feed.
fn feed_url(tag: &str) -> String {
    format!("{DOWNLOADS}/{tag}/emberhut-desktop-macos.json")
}

/// The address of `tag`'s disk image.
fn dmg_url(tag: &str) -> String {
    format!(
        "{DOWNLOADS}/{tag}/Emberhut-{}-macos-universal.dmg",
        &tag[1..]
    )
}

/// GitHub's list of `releases` (each its tag, and whether it is marked a pre-release), each
/// with a macOS feed, newest first as GitHub lists them.
fn listed(releases: &[(&str, bool)]) -> Vec<Listed> {
    let releases: Vec<_> = releases
        .iter()
        .map(|(tag, prerelease)| {
            json!({"tag_name": tag, "draft": false, "prerelease": prerelease,
                "assets": [{"name": "emberhut-desktop-macos.json",
                    "browser_download_url": feed_url(tag)}]})
        })
        .collect();
    serde_json::from_value(json!(releases)).unwrap()
}

/// The feed of `tag` (build `build`), its payload signed by `key` with its trusted comment.
fn feed(key: &TestKey, tag: &str, build: u64, notes: &str) -> ReleaseFeed {
    let version = &tag[1..];
    let sha = hex(&Sha256::digest(PAYLOAD));
    let comment = format!(
        "emberhut-desktop macos arm64 {version} build {build} sha256:{sha} notes:{}",
        hex(&Sha256::digest(notes.as_bytes()))
    );
    let asset = |arch: &str| {
        json!({"arch": arch, "url": dmg_url(tag), "size": PAYLOAD.len(), "sha256": sha,
            "signature": key.sign(PAYLOAD, &comment.replace(" arm64 ", &format!(" {arch} ")))})
    };
    serde_json::from_value(
        json!({"format": 1, "product": "emberhut-desktop", "platform": "macos",
        "published": "2026-12-08T10:00:00.000Z", "releases": [
            {"version": version, "build": build, "date": "2026-12-08", "notes": notes,
             "minimumOs": "13.0", "assets": [asset("arm64"), asset("x86_64")]}]}),
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
    github: Arc<FakeSource>,
    installer: Arc<FakeInstaller>,
    store: Entity<Store>,
    key: TestKey,
    _dir: tempfile::TempDir,
}

/// A store whose updater is `running` (`version`, `build`), trusting a test key (or none).
fn setup_as(cx: &mut TestAppContext, running: (&'static str, u64), trusting: bool) -> Setup {
    let backend = Arc::new(FakeBackend::new([]));
    let github = Arc::new(FakeSource::default());
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
        running,
        keys,
        Arc::clone(&github) as _,
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
        github,
        installer,
        store,
        key,
        _dir: dir,
    }
}

/// A store whose updater is 0.1.0 (build 10099), a stable release.
fn setup(cx: &mut TestAppContext, trusting: bool) -> Setup {
    setup_as(cx, ("0.1.0", 10099), trusting)
}

fn check(setup: &Setup, cx: &mut TestAppContext) -> Answer {
    let task = setup.store.update(cx, Store::check_for_updates);
    cx.run_until_parked();
    cx.foreground_executor().block_test(task)
}

fn downloads(setup: &Setup) -> usize {
    setup
        .github
        .asked()
        .iter()
        .filter(|asked| matches!(asked, Asked::Download(..)))
        .count()
}

/// GitHub offers 0.2.0 (20099) on Stable, with 0.1.1 (10199) skipped on the way.
fn offer_0_2_0(setup: &Setup) {
    let github = &setup.github;
    github.push_list(Ok(listed(&[
        ("v0.2.0", false),
        ("v0.2.0-rc.1", true),
        ("v0.1.1", false),
        ("v0.1.0", false),
    ])));
    github.push_feed(
        &feed_url("v0.2.0"),
        feed(&setup.key, "v0.2.0", 20099, "- Rooms open faster (#12)"),
    );
    github.push_feed(
        &feed_url("v0.1.1"),
        feed(&setup.key, "v0.1.1", 10199, "- A fix (#9)"),
    );
    github.push_payload(PAYLOAD);
}

#[gpui_kit::test]
fn a_newer_build_is_downloaded_verified_and_staged_and_the_bar_says_so(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    offer_0_2_0(&setup);

    let answer = check(&setup, cx);

    let Answer::Ready { title, notes } = answer else {
        panic!("{answer:?}")
    };
    assert_eq!(title, "Emberhut 0.2.0 (build 20099) is ready");
    assert!(notes.starts_with("- Rooms open faster (#12)"), "{notes}");
    assert!(
        notes.contains("Earlier releases (unverified):\n0.1.1: - A fix (#9)"),
        "the stable release skipped, not the candidate: {notes}"
    );
    assert_eq!(
        setup.github.asked(),
        [
            Asked::Releases,
            Asked::Feed(feed_url("v0.2.0")),
            Asked::Download(dmg_url("v0.2.0"), PAYLOAD.len() as u64),
            Asked::Feed(feed_url("v0.1.1")),
        ]
    );
    assert_eq!(setup.backend.calls(), [], "nothing asked of the hut");
    assert_eq!(*setup.installer.staged.lock().unwrap(), [Build::new(20099)]);
    setup.store.read_with(cx, |store, _| {
        let bar = store.updates().bar().unwrap();
        assert_eq!(bar.title, "Emberhut 0.2.0 is ready");
        let panel = store.updates().panel(SystemTime::now());
        assert_eq!(panel.version, "0.1.0");
        assert_eq!(panel.build, "Build 10099");
        assert_eq!(panel.ready.as_deref(), Some("0.2.0"));
        assert_eq!(panel.status.as_deref(), Some("Ready to install"));
        let notes = panel.notes.unwrap();
        assert_eq!(notes.title, "In 0.2.0");
        assert_eq!(notes.text, "- Rooms open faster (#12)");
        assert_eq!(notes.earlier.len(), 1);
    });
}

#[gpui_kit::test]
fn the_bar_put_away_stays_away_until_a_newer_build(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    offer_0_2_0(&setup);
    check(&setup, cx);
    setup.store.update(cx, Store::dismiss_update_bar);
    assert_eq!(
        setup.store.read_with(cx, |store, _| store.updates().bar()),
        None
    );
    // The next check finds the same build ready: no download, and the bar stays away.
    offer_0_2_0(&setup);
    setup.store.update(cx, Store::check_updates_now);
    cx.run_until_parked();
    assert_eq!(
        setup.store.read_with(cx, |store, _| store.updates().bar()),
        None
    );
    assert_eq!(downloads(&setup), 1);
}

#[gpui_kit::test]
fn with_no_trusted_key_every_update_fails_closed(cx: &mut TestAppContext) {
    let setup = setup(cx, false);
    offer_0_2_0(&setup);

    let answer = check(&setup, cx);

    assert_eq!(
        answer,
        Answer::Failed(
            "The update didn't verify; Emberhut stays at 0.1.0 (this app trusts no update key yet)"
                .into()
        )
    );
    assert_eq!(*setup.installer.staged.lock().unwrap(), []);
    setup.store.read_with(cx, |store, _| {
        assert_eq!(store.updates().bar(), None);
        assert!(matches!(store.updates().phase(), Phase::Failed(_)));
        assert!(store.updates().panel(SystemTime::now()).problem.is_some());
    });
    // The same payload isn't fetched again.
    offer_0_2_0(&setup);
    let again = check(&setup, cx);
    assert_eq!(
        again,
        Answer::Failed("An update didn't verify lately; Emberhut tries again later".into())
    );
    assert_eq!(downloads(&setup), 1);
}

#[gpui_kit::test]
fn a_payload_signed_for_windows_doesnt_verify_on_a_mac(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    let github = &setup.github;
    github.push_list(Ok(listed(&[("v0.2.0", false)])));
    let mut windows = feed(&setup.key, "v0.2.0", 20099, "- Faster.");
    let sha = hex(&Sha256::digest(PAYLOAD));
    let comment = format!(
        "emberhut-desktop windows arm64 0.2.0 build 20099 sha256:{sha} notes:{}",
        hex(&Sha256::digest(b"- Faster."))
    );
    windows.releases[0].assets[0].signature = setup.key.sign(PAYLOAD, &comment);
    github.push_feed(&feed_url("v0.2.0"), windows);
    github.push_payload(PAYLOAD);
    let Answer::Failed(why) = check(&setup, cx) else {
        panic!("refused")
    };
    assert!(
        why.contains("the signature is for another release"),
        "{why}"
    );
}

#[gpui_kit::test]
fn nothing_newer_or_nothing_published_is_up_to_date(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    assert_eq!(
        check(&setup, cx),
        Answer::Latest {
            running: "0.1.0, build 10099".into(),
            detail: Some("No release on this channel yet".into())
        }
    );
    // This copy's own release is the newest: its notes come from its feed.
    setup
        .github
        .push_list(Ok(listed(&[("v0.1.0", false), ("v0.1.0-rc.1", true)])));
    setup.github.push_feed(
        &feed_url("v0.1.0"),
        feed(&setup.key, "v0.1.0", 10099, "- The first release."),
    );
    assert!(matches!(check(&setup, cx), Answer::Latest { .. }));
    setup.store.read_with(cx, |store, _| {
        let panel = store.updates().panel(SystemTime::now());
        assert_eq!(
            panel.status.as_deref(),
            Some("You're up to date · checked just now")
        );
        let notes = panel.notes.unwrap();
        assert_eq!(
            (notes.title.as_str(), notes.text.as_str()),
            ("In 0.1.0", "- The first release.")
        );
        assert_eq!(panel.waiting, None);
    });
    assert_eq!(downloads(&setup), 0);
}

#[gpui_kit::test]
fn an_unreachable_github_says_so(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    setup
        .github
        .push_list(Err(FetchError::Unreachable("refused".into())));
    assert_eq!(
        check(&setup, cx),
        Answer::Failed("Couldn't reach GitHub".into())
    );
}

#[gpui_kit::test]
fn a_limit_says_when_it_checks_again_and_nothing_asks_before(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    let until = SystemTime::now() + Duration::from_mins(20);
    setup
        .github
        .push_list(Err(FetchError::RateLimited { until }));
    let Answer::Failed(why) = check(&setup, cx) else {
        panic!("limited")
    };
    assert!(
        why.starts_with("GitHub is limiting requests from this network; Emberhut checks again at "),
        "{why}"
    );
    // Neither the button nor the tick asks GitHub again before then.
    let again = check(&setup, cx);
    assert_eq!(again, Answer::Failed(why));
    setup.store.update(cx, Store::tick);
    cx.run_until_parked();
    assert_eq!(setup.github.asked(), [Asked::Releases]);
}

#[gpui_kit::test]
fn a_candidate_follows_candidates_and_on_stable_never_goes_back(cx: &mut TestAppContext) {
    let setup = setup_as(cx, ("0.2.0-rc.1", 20001), true);
    assert_eq!(
        setup
            .store
            .read_with(cx, |store, _| store.updates().channel()),
        Channel::Candidates
    );
    // A newer candidate is offered on the candidates' channel.
    let github = &setup.github;
    github.push_list(Ok(listed(&[
        ("v0.2.0-rc.2", true),
        ("v0.2.0-rc.1", true),
        ("v0.1.0", false),
    ])));
    github.push_feed(
        &feed_url("v0.2.0-rc.2"),
        feed(&setup.key, "v0.2.0-rc.2", 20002, "- Candidate notes."),
    );
    github.push_payload(PAYLOAD);
    assert!(matches!(check(&setup, cx), Answer::Ready { .. }));

    // Back to Stable: the candidate ready is dropped, and the check that follows finds only
    // 0.1.0, older than this copy: nothing is offered, and the page says why.
    github.push_list(Ok(listed(&[
        ("v0.2.0-rc.2", true),
        ("v0.2.0-rc.1", true),
        ("v0.1.0", false),
    ])));
    github.push_feed(
        &feed_url("v0.2.0-rc.1"),
        feed(&setup.key, "v0.2.0-rc.1", 20001, "- This candidate."),
    );
    setup.store.update(cx, |store, cx| {
        store.set_update_channel(Channel::Stable, cx);
    });
    cx.run_until_parked();
    setup.store.read_with(cx, |store, _| {
        assert_eq!(store.updates().bar(), None, "the candidate isn't offered");
        let panel = store.updates().panel(SystemTime::now());
        assert_eq!(panel.channel, Channel::Stable);
        assert_eq!(panel.ready, None);
        let waiting = panel.waiting.unwrap();
        assert!(
            waiting.contains("first stable release after 0.2.0-rc.1"),
            "{waiting}"
        );
        assert_eq!(panel.notes.unwrap().text, "- This candidate.");
    });
    assert_eq!(downloads(&setup), 1, "nothing older is downloaded");

    // The final 0.2.0 comes out: it is above every candidate, and offered.
    github.push_list(Ok(listed(&[
        ("v0.2.0", false),
        ("v0.2.0-rc.2", true),
        ("v0.1.0", false),
    ])));
    github.push_feed(
        &feed_url("v0.2.0"),
        feed(&setup.key, "v0.2.0", 20099, "- The release."),
    );
    github.push_payload(PAYLOAD);
    let Answer::Ready { title, .. } = check(&setup, cx) else {
        panic!("the final is offered")
    };
    assert_eq!(title, "Emberhut 0.2.0 (build 20099) is ready");
}

#[gpui_kit::test]
fn a_feed_that_doesnt_describe_its_tag_is_refused(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    setup.github.push_list(Ok(listed(&[("v0.2.0", false)])));
    // The stable tag's feed lists a candidate's build.
    setup.github.push_feed(
        &feed_url("v0.2.0"),
        feed(&setup.key, "v0.3.0-rc.1", 30001, "- Not this."),
    );
    assert_eq!(
        check(&setup, cx),
        Answer::Failed("The release's update file isn't for this app".into())
    );
    assert_eq!(downloads(&setup), 0);
}

#[gpui_kit::test]
fn with_automatic_checks_off_only_the_button_checks(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    setup.store.update(cx, |store, cx| {
        store.set_update_automatic(false, cx);
        store.tick(cx);
    });
    cx.run_until_parked();
    assert_eq!(setup.github.asked(), [], "the tick doesn't check");
    assert!(!setup.store.read_with(cx, |store, _| {
        store.updates().panel(SystemTime::now()).automatic
    }));
    setup.store.update(cx, Store::check_updates_now);
    cx.run_until_parked();
    assert_eq!(setup.github.asked(), [Asked::Releases]);
    // On again: the tick checks once the last check is a day old (not now, it just checked).
    setup.store.update(cx, |store, cx| {
        store.set_update_automatic(true, cx);
        store.tick(cx);
    });
    cx.run_until_parked();
    assert_eq!(setup.github.asked(), [Asked::Releases]);
}

#[gpui_kit::test]
fn a_build_already_downloaded_installs_at_once_from_the_menu(cx: &mut TestAppContext) {
    let setup = setup(cx, true);
    offer_0_2_0(&setup);
    check(&setup, cx);
    let ended = WATCHES_ENDED.load(Ordering::Relaxed);

    assert_eq!(check(&setup, cx), Answer::Installing);

    assert_eq!(
        *setup.installer.swapped.lock().unwrap(),
        [Build::new(20099)]
    );
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
    let version = option_env!("EMBERHUT_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
    assert!(
        matches!(&answer, Answer::Unavailable { running, .. } if running.starts_with(version)),
        "{answer:?}"
    );
    store.read_with(cx, |store, _| {
        let panel = store.updates().panel(SystemTime::now());
        assert!(!panel.updates, "nothing is checked");
        assert!(!panel.can_check);
        assert_eq!(panel.status, None);
        assert_eq!(panel.version, version);
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
