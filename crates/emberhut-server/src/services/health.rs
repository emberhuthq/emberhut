//! How late the server's runtime is (`GET /health`, `GET /api/loop`).
//!
//! A probe asks the runtime to run a step once a second and notes how long the step waited
//! for a worker: on an idle server that is well under a millisecond, and it grows when work
//! holds the workers. Each request's handler is watched too (the `measure` middleware): a
//! single poll of it that runs [`SLOW`] or longer held a worker that long, and is noted under
//! the request's name (`emberhut_core::lag::route_name`). Once a minute a lagging minute is logged,
//! and three minutes in a row with a 95th percentile of a second or more are logged as a stall.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use emberhut_api::health::{Health, LoopLag as Lag, LoopReport, SlowWork};
use emberhut_core::lag::{LagStats, LoopLag, SLOW, SlowGroup, route_name};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::app::AppState;

/// How often the probe asks the runtime for a step.
const EVERY: Duration = Duration::from_secs(1);

/// Probes in a minute: the minute's look comes after this many.
const PER_MINUTE: u32 = 60;

/// The span of `GET /api/loop`'s slow work.
const REPORTED: Duration = Duration::from_secs(300);

/// The runtime's lag and the work that held it, shared by the probe, the requests and the
/// routes that report them.
#[derive(Clone, Debug)]
pub struct LoopMonitor {
    lag: Arc<Mutex<LoopLag>>,
    origin: Instant,
}

impl Default for LoopMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopMonitor {
    /// A monitor with nothing noted yet; its times count from now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lag: Arc::default(),
            origin: Instant::now(),
        }
    }

    /// `GET /health`'s answer.
    #[must_use]
    pub fn health(&self) -> Health {
        Health {
            ok: true,
            lag: to_api(self.stats()),
        }
    }

    /// `GET /api/loop`'s answer: the lag, and the slow work of the last five minutes by name.
    #[must_use]
    pub fn report(&self) -> LoopReport {
        let now = self.now();
        let slow = self.lag().slow_since(now, REPORTED);
        LoopReport {
            lag: to_api(self.lag().stats(now)),
            slow: slow.into_iter().map(slow_to_api).collect(),
        }
    }

    /// The lag now.
    #[must_use]
    pub fn stats(&self) -> LagStats {
        self.lag().stats(self.now())
    }

    /// How many of the probe's samples are kept (the last five minutes' worth at most).
    #[must_use]
    pub fn samples(&self) -> usize {
        self.lag().samples()
    }

    /// Notes a probe's step, asked for at `asked`, that ran after waiting `waited`.
    pub fn sample(&self, asked: Instant, waited: Duration) {
        let at = asked.saturating_duration_since(self.origin);
        self.lag().sample(at, waited);
    }

    /// Notes that `what` held a worker for `took` (noted only from [`SLOW`] on).
    pub fn note_slow(&self, took: Duration, what: &str) {
        if took >= SLOW {
            let now = self.now();
            self.lag().slow(now, took, what);
        }
    }

    /// The probe: a step once a second until `stopping` is cancelled, and the minute's look
    /// after every sixty. Spawned once by the server.
    pub async fn probe(self, stopping: CancellationToken) {
        let mut every = tokio::time::interval(EVERY);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut probes = 0_u32;
        loop {
            tokio::select! {
                () = stopping.cancelled() => return,
                _ = every.tick() => {}
            }
            let asked = Instant::now();
            let monitor = self.clone();
            drop(tokio::spawn(async move {
                monitor.sample(asked, asked.elapsed());
            }));
            probes += 1;
            if probes.is_multiple_of(PER_MINUTE) {
                self.minute();
            }
        }
    }

    /// The minute's look: a log line when the minute lagged, an error line on a stall.
    fn minute(&self) {
        let now = self.now();
        let note = self.lag().tick(now);
        let stats = note.stats;
        if note.stalling {
            tracing::error!(
                p95_ms = stats.p95_1m,
                slowest = %note.slowest,
                "the server is stalling: its runtime has lagged over a second for 3 minutes"
            );
        } else if note.lagged {
            tracing::warn!(
                p95_ms = stats.p95_1m,
                max_ms = stats.max_1m,
                slowest = %note.slowest,
                "the server's runtime lagged in the last minute"
            );
        }
    }

    fn now(&self) -> Duration {
        Instant::now().saturating_duration_since(self.origin)
    }

    fn lag(&self) -> MutexGuard<'_, LoopLag> {
        // Plain numbers: a panic elsewhere cannot leave them half-written.
        self.lag.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn to_api(stats: LagStats) -> Lag {
    Lag {
        ms: stats.ms,
        p95_1m: stats.p95_1m,
        max_1m: stats.max_1m,
        p95_5m: stats.p95_5m,
        max_5m: stats.max_5m,
    }
}

fn slow_to_api(group: SlowGroup) -> SlowWork {
    SlowWork {
        what: group.what,
        n: group.n,
        ms: group.ms,
        max: group.max,
    }
}

/// Middleware: notes every poll of a request's handler that held a worker for [`SLOW`] or
/// more, under the request's name.
pub(crate) async fn measure(State(app): State<AppState>, request: Request, next: Next) -> Response {
    let what = route_name(request.method().as_str(), request.uri().path());
    Held {
        work: Box::pin(next.run(request)),
        monitor: app.lag().clone(),
        what,
    }
    .await
}

/// A future whose every poll is timed.
struct Held<F> {
    work: Pin<Box<F>>,
    monitor: LoopMonitor,
    what: String,
}

impl<F: Future> Future for Held<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let this = self.get_mut();
        let started = std::time::Instant::now();
        let polled = this.work.as_mut().poll(cx);
        this.monitor.note_slow(started.elapsed(), &this.what);
        polled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn the_probe_samples_once_a_second() {
        let monitor = LoopMonitor::new();
        let stopping = CancellationToken::new();
        let probe = tokio::spawn(monitor.clone().probe(stopping.clone()));
        tokio::time::sleep(Duration::from_millis(3_500)).await;
        stopping.cancel();
        probe.await.unwrap();
        // Ticks at 0, 1, 2 and 3 s; the clock is paused, so no step waited.
        assert_eq!(monitor.lag().samples(), 4);
        assert_eq!(monitor.stats(), LagStats::default());
        assert_eq!(monitor.health().lag.max_5m, 0);
        assert!(monitor.health().ok);
    }

    #[tokio::test]
    async fn work_that_held_a_worker_is_reported_by_name() {
        let monitor = LoopMonitor::new();
        monitor.note_slow(Duration::from_millis(10), "GET /api/quick");
        monitor.note_slow(Duration::from_millis(80), "GET /api/state");
        let held = Held {
            work: Box::pin(async {
                std::thread::sleep(Duration::from_millis(60));
            }),
            monitor: monitor.clone(),
            what: "POST /api/talk".to_owned(),
        };
        held.await;
        let report = monitor.report();
        let names: Vec<&str> = report.slow.iter().map(|work| work.what.as_str()).collect();
        assert_eq!(names, ["GET /api/state", "POST /api/talk"]);
        assert!(report.slow[1].max >= 60);
    }
}
