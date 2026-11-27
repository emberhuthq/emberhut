//! A phone's push address (`POST /api/devices`) and the sign-in status
//! (`GET /api/auth/status`).

use emberhut_api::auth::{AuthMode, AuthStatus};
use emberhut_api::push::PushRegistration;

use crate::support::assert_round_trip;

#[test]
fn an_iphone_and_an_android_phone_register() {
    let iphone: PushRegistration = assert_round_trip("push_iphone.json");
    assert!(!iphone.is_android());
    assert_eq!(iphone.env.as_deref(), Some("production"));
    let android: PushRegistration = assert_round_trip("push_android.json");
    assert!(android.is_android());
    assert_eq!(android.env, None);
}

#[test]
fn a_registration_without_a_token_reads_as_an_empty_one() {
    let empty: PushRegistration = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, PushRegistration::default());
}

#[test]
fn the_status_is_the_auth_info_and_whether_this_device_is_signed_in() {
    let status: AuthStatus = assert_round_trip("auth_status.json");
    assert!(status.signed_in);
    assert_eq!(status.info.mode, AuthMode::Enforce);
    assert_eq!(status.info.devices[0].id.get(), 3);
}
