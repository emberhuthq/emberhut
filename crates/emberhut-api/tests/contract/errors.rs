//! Refusals and acknowledgements.

use emberhut_api::error::{ErrorBody, OkReply};

use crate::support::{assert_round_trip, to_value};

#[test]
fn the_sign_in_refusal_round_trips() {
    let body: ErrorBody = assert_round_trip("error_sign_in.json");
    assert!(body.sign_in);
    assert!(body.ok.is_none());
    assert!(ErrorBody::sign_in_required().sign_in);
}

#[test]
fn other_refusals_round_trip() {
    let gate: ErrorBody = assert_round_trip("error_not_let_in.json");
    assert!(!gate.sign_in);
    assert_eq!(
        to_value(&ErrorBody::new("not one of the owner's devices")),
        to_value(&gate)
    );
    let refused: ErrorBody = assert_round_trip("error_refused.json");
    assert_eq!(refused.ok, Some(false));
}

#[test]
fn acknowledgements_round_trip() {
    assert_eq!(assert_round_trip::<OkReply>("ok.json"), OkReply::OK);
    assert_eq!(OkReply::default(), OkReply::OK);
}
