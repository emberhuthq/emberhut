//! The switch to the fallback server: the watch's count on a simulated clock, the backend's
//! routing and its Try again over fakes, and the watcher's thread on short timings. No real
//! server, network or keychain.

use std::sync::atomic::AtomicU32;
use std::time::Instant;

use super::*;
use crate::backend::fake::{Call, FakeBackend};

const PRIMARY: &str = "http://primary.example";
const FALLBACK: &str = "http://fallback.example";

/// A server's health, set by the test, and how many times it was asked.
#[derive(Clone, Default)]
struct Health {
    up: Arc<AtomicBool>,
    asked: Arc<AtomicU32>,
}

impl Health {
    fn up() -> Self {
        let health = Self::default();
        health.set(true);
        health
    }

    fn set(&self, up: bool) {
        self.up.store(up, Ordering::SeqCst);
    }

    fn asked(&self) -> u32 {
        self.asked.load(Ordering::SeqCst)
    }

    fn probe(&self) -> Probe {
        let health = self.clone();
        Box::new(move || {
            health.asked.fetch_add(1, Ordering::SeqCst);
            health.up.load(Ordering::SeqCst)
        })
    }
}

struct Pair {
    primary: Arc<FakeBackend>,
    fallback: Arc<FakeBackend>,
}

impl Pair {
    fn new() -> Self {
        Self {
            primary: Arc::new(FakeBackend::new([]).at(PRIMARY)),
            fallback: Arc::new(FakeBackend::new([]).at(FALLBACK)),
        }
    }

    fn failover(&self, health: &Health, mark: Option<PathBuf>) -> Failover {
        Failover::new(
            Arc::clone(&self.primary) as Arc<dyn Backend>,
            Arc::clone(&self.fallback) as Arc<dyn Backend>,
            [health.probe(), Health::up().probe()],
            mark,
        )
    }
}

/// Runs the watch on a simulated clock from 0 s, the primary answering as `up` says at each
/// second; the second the app turns to the fallback, if it does by `until`.
fn simulate(up: impl Fn(u64) -> bool, until: u64) -> Option<u64> {
    let mut watch = Watch::starting(Timing::DEFAULT);
    let mut now = Duration::ZERO;
    while now.as_secs() <= until {
        match watch.heard(up(now.as_secs())) {
            Next::CheckIn(after) => now += after,
            Next::TurnToFallback => return Some(now.as_secs()),
        }
    }
    None
}

#[test]
fn a_primary_that_answers_is_kept() {
    assert_eq!(simulate(|_| true, 3600), None);
}

#[test]
fn three_failed_checks_in_a_row_turn_to_the_fallback() {
    // Checked at 0, 15, 30 … s: down from 100 s, the checks at 105, 120 and 135 s fail.
    assert_eq!(simulate(|second| second < 100, 3600), Some(135));
}

#[test]
fn a_check_that_answers_starts_the_count_again() {
    // Down for two checks (105 and 120 s) at a time, answering at the third: never three in
    // a row.
    let flaky = |second: u64| second < 100 || second.is_multiple_of(45);
    assert_eq!(simulate(flaky, 3600), None);
}

#[test]
fn at_start_up_a_silent_primary_gets_two_quick_retries() {
    // Checked at 0 s, then retried at 2 and 4 s.
    assert_eq!(simulate(|_| false, 60), Some(4));
    // Answering at the last retry: kept.
    assert_eq!(simulate(|second| second >= 4, 3600), None);
    let mut watch = Watch::starting(Timing::DEFAULT);
    assert_eq!(watch.heard(false), Next::CheckIn(Duration::from_secs(2)));
    assert_eq!(watch.heard(true), Next::CheckIn(Duration::from_secs(15)));
}

#[test]
fn every_call_goes_to_the_server_being_served() {
    let (pair, health) = (Pair::new(), Health::up());
    let failover = pair.failover(&health, None);
    assert_eq!(failover.serving(), Serving::Primary);
    assert_eq!(failover.server().as_str(), PRIMARY);
    failover.machines().unwrap();
    assert_eq!(pair.primary.calls(), [Call::Machines]);

    health.set(false);
    let mut watch = Watch::answered(Timing::DEFAULT);
    for _ in 0..2 {
        assert!(matches!(
            failover.shared.check(&mut watch),
            Next::CheckIn(_)
        ));
        assert_eq!(failover.serving(), Serving::Primary);
    }
    assert_eq!(failover.shared.check(&mut watch), Next::TurnToFallback);
    assert_eq!(
        failover.serving(),
        Serving::Fallback {
            primary: ServerUrl::parse(PRIMARY).unwrap(),
            fallback: ServerUrl::parse(FALLBACK).unwrap(),
        }
    );
    assert_eq!(failover.server().as_str(), FALLBACK);
    failover.machines().unwrap();
    assert_eq!(pair.fallback.calls(), [Call::Machines]);
    assert_eq!(pair.primary.calls(), [Call::Machines], "nothing more to it");
}

#[test]
fn try_again_turns_back_only_when_the_primary_answers() {
    let (pair, health) = (Pair::new(), Health::default());
    let dir = tempfile::tempdir().unwrap();
    let mark = dir.path().join("fallback.json");
    let failover = pair.failover(&health, Some(mark.clone()));
    failover.shared.turn_to_fallback();
    assert!(mark.exists(), "kept for the next launch");

    assert!(!failover.try_primary(), "still silent");
    assert!(matches!(failover.serving(), Serving::Fallback { .. }));
    assert!(mark.exists());

    health.set(true);
    assert!(failover.try_primary());
    assert_eq!(failover.serving(), Serving::Primary);
    assert!(!mark.exists(), "forgotten once back");
    assert!(failover.try_primary(), "on the primary: nothing to try");
}

#[test]
fn a_relaunch_stays_on_the_fallback_for_the_same_primary_only() {
    let (pair, health) = (Pair::new(), Health::up());
    let dir = tempfile::tempdir().unwrap();
    let mark = dir.path().join("fallback.json");
    pair.failover(&health, Some(mark.clone()))
        .shared
        .turn_to_fallback();

    let relaunched = pair.failover(&health, Some(mark.clone()));
    assert!(
        matches!(relaunched.serving(), Serving::Fallback { .. }),
        "never back by itself at launch, even when the primary answers"
    );
    drop(relaunched);

    std::fs::write(&mark, r#"{"primary": "http://another.example"}"#).unwrap();
    let repointed = pair.failover(&health, Some(mark.clone()));
    assert_eq!(
        repointed.serving(),
        Serving::Primary,
        "a mark for another server"
    );
    std::fs::write(&mark, "not json").unwrap();
    assert_eq!(
        pair.failover(&health, Some(mark)).serving(),
        Serving::Primary
    );
}

#[test]
fn stopping_stops_both() {
    let (pair, health) = (Pair::new(), Health::up());
    pair.failover(&health, None).stop().unwrap();
    assert_eq!(pair.primary.calls(), [Call::Stop]);
    assert_eq!(pair.fallback.calls(), [Call::Stop]);
}

/// Waits up to 5 s for `done`.
fn eventually(done: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    done()
}

const QUICK: Timing = Timing {
    every: Duration::from_millis(10),
    failures: 3,
    start_retries: 2,
    retry_after: Duration::from_millis(5),
    probe_within: Duration::from_millis(5),
};

#[test]
fn the_watcher_turns_to_the_fallback_and_stays_there() {
    let (pair, health) = (Pair::new(), Health::up());
    let failover = Failover::start(
        Arc::clone(&pair.primary) as Arc<dyn Backend>,
        Arc::clone(&pair.fallback) as Arc<dyn Backend>,
        [health.probe(), Health::up().probe()],
        None,
        QUICK,
    );
    assert!(eventually(|| health.asked() >= 3), "checked every 10 ms");
    assert_eq!(failover.serving(), Serving::Primary);

    health.set(false);
    assert!(eventually(|| failover.serving() != Serving::Primary));

    // While the fallback answers the primary isn't asked, so nothing turns back, even once
    // the primary answers.
    health.set(true);
    let asked = health.asked();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(health.asked(), asked);
    assert!(matches!(failover.serving(), Serving::Fallback { .. }));

    // Try again turns back, and the watch goes on.
    assert!(failover.try_primary());
    assert!(eventually(|| health.asked() >= asked + 4));
    assert_eq!(failover.serving(), Serving::Primary);
    failover.stop().unwrap();
}

#[test]
fn a_primary_silent_at_start_up_turns_to_the_fallback_at_once() {
    let (pair, health) = (Pair::new(), Health::default());
    let failover = Failover::start(
        Arc::clone(&pair.primary) as Arc<dyn Backend>,
        Arc::clone(&pair.fallback) as Arc<dyn Backend>,
        [health.probe(), Health::up().probe()],
        None,
        QUICK,
    );
    assert!(eventually(|| failover.serving() != Serving::Primary));
    assert_eq!(health.asked(), 3, "a first check and two retries");
    failover.stop().unwrap();
}

#[test]
fn a_silent_fallback_turns_back_to_the_primary_once_it_answers() {
    let (pair, primary, fallback) = (Pair::new(), Health::up(), Health::default());
    let dir = tempfile::tempdir().unwrap();
    let mark = dir.path().join("fallback.json");
    std::fs::write(&mark, format!(r#"{{"primary": "{PRIMARY}"}}"#)).unwrap();
    // Relaunched on the fallback, which is down too, while the primary is down as well.
    primary.set(false);
    let failover = Failover::start(
        Arc::clone(&pair.primary) as Arc<dyn Backend>,
        Arc::clone(&pair.fallback) as Arc<dyn Backend>,
        [primary.probe(), fallback.probe()],
        Some(mark.clone()),
        QUICK,
    );
    assert!(
        eventually(|| primary.asked() >= 2),
        "asked once the fallback is silent"
    );
    assert!(
        fallback.asked() >= 3,
        "only after three silent checks of the fallback"
    );
    assert!(matches!(failover.serving(), Serving::Fallback { .. }));
    assert!(mark.exists());

    primary.set(true);
    assert!(eventually(|| failover.serving() == Serving::Primary));
    assert!(!mark.exists(), "forgotten once back");
    failover.stop().unwrap();
}
