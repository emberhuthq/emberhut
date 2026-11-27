//! How late the server's runtime is, and the work that held it: the rules behind `GET /health`
//! and `GET /api/loop`, with no clock of their own.
//!
//! A probe off the request path asks the runtime to run a step once a second and notes how long
//! the step waited ([`LoopLag::sample`]); work that held the runtime for [`SLOW`] or more is noted
//! by name ([`LoopLag::slow`]). Times are offsets from an origin the caller picks (its start), so
//! every rule is tested at any instant. It keeps the last 300 samples (five minutes of them) and
//! the last 200 slow works, and gives the 95th percentile and the longest of the last minute and
//! of all the samples, in whole milliseconds.

use std::collections::VecDeque;
use std::time::Duration;

/// Work that holds the runtime this long or longer is noted by name.
pub const SLOW: Duration = Duration::from_millis(50);

/// Samples kept: five minutes of one a second.
const SAMPLES_KEPT: usize = 300;

/// Slow works kept.
const SLOW_KEPT: usize = 200;

/// The span of "the last minute".
const MINUTE: Duration = Duration::from_secs(60);

/// A lag this long, the longest of a minute, is logged; its 95th percentile this long for
/// [`STALL_MINUTES`] minutes in a row is a stall.
const LAGGED: Duration = Duration::from_secs(1);

/// Minutes in a row of a high 95th percentile that make a stall.
const STALL_MINUTES: u32 = 3;

/// The lag now and over the last minute and five, in whole milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LagStats {
    /// The latest sample.
    pub ms: u64,
    /// The 95th percentile of the last minute's samples (0 without any).
    pub p95_1m: u64,
    /// The longest of the last minute.
    pub max_1m: u64,
    /// The 95th percentile of every sample kept (the last five minutes).
    pub p95_5m: u64,
    /// The longest of every sample kept.
    pub max_5m: u64,
}

/// Slow work of one name: how many times, how long all told, the longest once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlowGroup {
    /// What it was (see [`route_name`]).
    pub what: String,
    /// How many times.
    pub n: u32,
    /// How long, all told, in whole milliseconds.
    pub ms: u64,
    /// The longest once, in whole milliseconds.
    pub max: u64,
}

/// What a minute's look found ([`LoopLag::tick`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MinuteNote {
    /// The longest lag of the last minute was a second or more: worth a log line.
    pub lagged: bool,
    /// The 95th percentile has been a second or more for three minutes in a row (said once,
    /// on the third).
    pub stalling: bool,
    /// The lag at the time.
    pub stats: LagStats,
    /// The slowest work of the last minute, the three longest in all: `GET /api/state 2× 1.4 s`,
    /// joined with `; `, or `unknown`.
    pub slowest: String,
}

/// The samples and slow works of the last few minutes.
#[derive(Clone, Debug, Default)]
pub struct LoopLag {
    samples: VecDeque<(Duration, Duration)>,
    slow: VecDeque<(Duration, Duration, String)>,
    last: Duration,
    high_minutes: u32,
}

impl LoopLag {
    /// Notes that a step asked for at `at` waited `lag` for the runtime.
    pub fn sample(&mut self, at: Duration, lag: Duration) {
        self.last = lag;
        self.samples.push_back((at, lag));
        while self.samples.len() > SAMPLES_KEPT {
            self.samples.pop_front();
        }
    }

    /// Notes that `what` held the runtime for `took`, ending at `at`; work shorter than
    /// [`SLOW`] is not noted.
    pub fn slow(&mut self, at: Duration, took: Duration, what: impl Into<String>) {
        if took < SLOW {
            return;
        }
        self.slow.push_back((at, took, what.into()));
        while self.slow.len() > SLOW_KEPT {
            self.slow.pop_front();
        }
    }

    /// How many samples are kept.
    #[must_use]
    pub fn samples(&self) -> usize {
        self.samples.len()
    }

    /// The lag as of `now`.
    #[must_use]
    pub fn stats(&self, now: Duration) -> LagStats {
        let minute: Vec<Duration> = self
            .samples
            .iter()
            .filter(|(at, _)| now.saturating_sub(*at) <= MINUTE)
            .map(|(_, lag)| *lag)
            .collect();
        let all: Vec<Duration> = self.samples.iter().map(|(_, lag)| *lag).collect();
        LagStats {
            ms: whole_ms(self.last),
            p95_1m: p95(&minute),
            max_1m: minute.iter().max().map_or(0, |lag| whole_ms(*lag)),
            p95_5m: p95(&all),
            max_5m: all.iter().max().map_or(0, |lag| whole_ms(*lag)),
        }
    }

    /// The slow work of the `window` before `now`, by name, the longest in all first (of two as
    /// long, the one with the longest single run first).
    #[must_use]
    pub fn slow_since(&self, now: Duration, window: Duration) -> Vec<SlowGroup> {
        let mut recent: Vec<&(Duration, Duration, String)> = self
            .slow
            .iter()
            .filter(|(at, _, _)| now.saturating_sub(*at) <= window)
            .collect();
        recent.sort_by_key(|slow| std::cmp::Reverse(slow.1));
        let mut groups: Vec<(String, u32, Duration, Duration)> = Vec::new();
        for (_, took, what) in recent {
            match groups.iter_mut().find(|group| group.0 == *what) {
                Some(group) => {
                    group.1 += 1;
                    group.2 += *took;
                }
                None => groups.push((what.clone(), 1, *took, *took)),
            }
        }
        let mut groups: Vec<SlowGroup> = groups
            .into_iter()
            .map(|(what, n, total, max)| SlowGroup {
                what,
                n,
                ms: whole_ms(total),
                max: whole_ms(max),
            })
            .collect();
        groups.sort_by_key(|group| std::cmp::Reverse(group.ms));
        groups
    }

    /// The look taken once a minute, at `now`: whether the last minute lagged, and whether the
    /// runtime is stalling.
    pub fn tick(&mut self, now: Duration) -> MinuteNote {
        let stats = self.stats(now);
        let slowest = self
            .slow_since(now, MINUTE)
            .iter()
            .take(3)
            .map(|group| {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "milliseconds of a minute's work, far below 2^52"
                )]
                let seconds = group.ms as f64 / 1000.0;
                format!("{} {}× {seconds:.1} s", group.what, group.n)
            })
            .collect::<Vec<_>>()
            .join("; ");
        let high = stats.p95_1m >= whole_ms(LAGGED);
        self.high_minutes = if high { self.high_minutes + 1 } else { 0 };
        MinuteNote {
            lagged: stats.max_1m >= whole_ms(LAGGED),
            stalling: self.high_minutes == STALL_MINUTES,
            stats,
            slowest: if slowest.is_empty() {
                "unknown".to_owned()
            } else {
                slowest
            },
        }
    }
}

/// The 95th percentile of `lags` in whole milliseconds: the smallest that at least 95% of them
/// don't exceed; 0 without any.
fn p95(lags: &[Duration]) -> u64 {
    let mut sorted = lags.to_vec();
    sorted.sort_unstable();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "at most 300 samples: the rank is a small positive whole number"
    )]
    let rank = (sorted.len() as f64 * 0.95).ceil() as usize;
    match rank.checked_sub(1) {
        Some(index) => whole_ms(sorted[index.min(sorted.len() - 1)]),
        None => 0,
    }
}

/// `duration` in milliseconds, rounded to the nearest.
fn whole_ms(duration: Duration) -> u64 {
    let micros = duration.as_micros();
    u64::try_from((micros + 500) / 1000).unwrap_or(u64::MAX)
}

/// A request's name for the slow work list: its method and path, every part of the path that
/// has a digit or is longer than 16 characters (an id, a key) written `:x`.
///
/// ```
/// use emberhut_core::lag::route_name;
///
/// assert_eq!(route_name("POST", "/api/tasks/42/message"), "POST /api/tasks/:x/message");
/// assert_eq!(route_name("GET", "/api/state"), "GET /api/state");
/// ```
#[must_use]
pub fn route_name(method: &str, path: &str) -> String {
    format!("{method} {}", masked_path(path))
}

/// `path` with every part that has a digit or is longer than 16 characters (an id, a key, a
/// link's token) written `:x`, so it can be logged.
///
/// ```
/// use emberhut_core::lag::masked_path;
///
/// assert_eq!(masked_path("/s/Zm9vYmFyYmF6cXV4cXV1eA"), "/s/:x");
/// assert_eq!(masked_path("/api/state"), "/api/state");
/// ```
#[must_use]
pub fn masked_path(path: &str) -> String {
    let parts: Vec<&str> = path
        .split('/')
        .map(|part| {
            if part.chars().count() > 16 || part.chars().any(|c| c.is_ascii_digit()) {
                ":x"
            } else {
                part
            }
        })
        .collect();
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    const fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn nothing_yet_is_all_zero() {
        assert_eq!(LoopLag::default().stats(s(10)), LagStats::default());
        assert_eq!(LoopLag::default().slow_since(s(10), s(300)), []);
    }

    #[test]
    fn the_last_minute_and_every_sample_kept() {
        let mut lag = LoopLag::default();
        lag.sample(s(1), ms(200));
        for n in 0..20 {
            lag.sample(s(100 + n), ms(n));
        }
        let stats = lag.stats(s(120));
        assert_eq!(stats.ms, 19);
        assert_eq!(stats.max_1m, 19);
        // 20 samples in the minute: the 19th smallest is the 95th percentile.
        assert_eq!(stats.p95_1m, 18);
        assert_eq!(stats.max_5m, 200);
        // 21 kept: rank 20 of 0..=19 and 200.
        assert_eq!(stats.p95_5m, 19);
    }

    #[test]
    fn only_the_last_300_samples_are_kept() {
        let mut lag = LoopLag::default();
        lag.sample(s(0), ms(900));
        for n in 0..300 {
            lag.sample(s(1 + n), ms(1));
        }
        assert_eq!(lag.samples(), 300);
        assert_eq!(lag.stats(s(301)).max_5m, 1);
    }

    #[test]
    fn milliseconds_are_rounded() {
        let mut lag = LoopLag::default();
        lag.sample(s(0), Duration::from_micros(1_500));
        assert_eq!(lag.stats(s(0)).ms, 2);
        lag.sample(s(0), Duration::from_micros(1_499));
        assert_eq!(lag.stats(s(0)).ms, 1);
    }

    #[test]
    fn slow_work_by_name_the_longest_in_all_first() {
        let mut lag = LoopLag::default();
        lag.slow(s(10), ms(49), "GET /api/quick");
        lag.slow(s(10), ms(120), "GET /api/state");
        lag.slow(s(11), ms(60), "GET /api/state");
        lag.slow(s(12), ms(150), "POST /api/talk");
        lag.slow(s(1), ms(900), "GET /api/old");
        let groups = lag.slow_since(s(305), s(300));
        assert_eq!(
            groups,
            [
                SlowGroup {
                    what: "GET /api/state".to_owned(),
                    n: 2,
                    ms: 180,
                    max: 120
                },
                SlowGroup {
                    what: "POST /api/talk".to_owned(),
                    n: 1,
                    ms: 150,
                    max: 150
                },
            ]
        );
    }

    #[test]
    fn only_the_last_200_slow_works_are_kept() {
        let mut lag = LoopLag::default();
        lag.slow(s(0), ms(500), "first");
        for _ in 0..200 {
            lag.slow(s(1), ms(50), "later");
        }
        let groups = lag.slow_since(s(2), s(300));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].n, 200);
    }

    #[test]
    fn three_lagging_minutes_in_a_row_are_a_stall_said_once() {
        let mut lag = LoopLag::default();
        lag.slow(s(5), ms(1_400), "GET /api/state");
        lag.sample(s(10), ms(1_200));
        let first = lag.tick(s(60));
        assert!(first.lagged && !first.stalling);
        assert_eq!(first.slowest, "GET /api/state 1× 1.4 s");
        lag.sample(s(70), ms(1_200));
        assert!(!lag.tick(s(120)).stalling);
        lag.sample(s(130), ms(1_200));
        assert!(lag.tick(s(180)).stalling);
        lag.sample(s(190), ms(1_200));
        assert!(!lag.tick(s(240)).stalling, "said once");
        let quiet = lag.tick(s(400));
        assert!(!quiet.lagged && !quiet.stalling);
        assert_eq!(quiet.slowest, "unknown");
    }

    #[test]
    fn route_names_hide_ids_and_keys() {
        assert_eq!(
            route_name("GET", "/uploads/1a2b3c4d-photo.png"),
            "GET /uploads/:x"
        );
        assert_eq!(
            route_name("GET", "/r/abcdefghijklmnopq"),
            "GET /r/:x",
            "longer than 16 characters"
        );
        assert_eq!(route_name("GET", "/"), "GET /");
    }
}
