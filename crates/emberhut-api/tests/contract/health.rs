//! The health check and the runtime's lag.

use emberhut_api::health::{Health, LoopReport};

use crate::support::assert_round_trip;

#[test]
fn the_health_check_round_trips() {
    let health: Health = assert_round_trip("health.json");
    assert!(health.ok);
    assert_eq!(health.lag.max_5m, 61);
    assert_eq!(health.lag.p95_1m, 2);
}

#[test]
fn the_loop_report_round_trips() {
    let report: LoopReport = assert_round_trip("loop.json");
    assert_eq!(report.lag.max_5m, 120);
    assert_eq!(report.slow.len(), 2);
    assert_eq!(report.slow[0].what, "GET /api/state");
    assert_eq!(
        (report.slow[0].n, report.slow[0].ms, report.slow[0].max),
        (2, 180, 120)
    );
}
