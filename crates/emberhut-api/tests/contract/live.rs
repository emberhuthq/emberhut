//! The live stream's events, one fixture per kind.

use emberhut_api::live::{Live, LiveEvent};

use crate::support::assert_round_trip;

#[test]
fn a_step_is_working() {
    let event: LiveEvent = assert_round_trip("live_working.json");
    assert_eq!(event.doing(), Some("Reading your new messages"));
}

#[test]
fn a_turn_without_a_step_yet_is_working() {
    let event: LiveEvent = assert_round_trip("live_started.json");
    assert_eq!(event, LiveEvent::Working(Live::default()));
    assert_eq!(event.doing(), None);
}

#[test]
fn null_is_idle() {
    let event: LiveEvent = assert_round_trip("live_idle.json");
    assert_eq!(
        event,
        LiveEvent::Idle,
        "`data: null`: nothing is being written"
    );
}
