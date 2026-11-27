//! Pairing, renewal and their answers.

use emberhut_api::auth::{AuthRefusal, AuthReply, PairRequest, PairingCode, RefreshRequest};
use emberhut_api::ids::{DeviceId, InstallId};
use emberhut_identity::PairingLink;

use crate::support::{assert_round_trip, to_value};

#[test]
fn a_pairing_request_round_trips() {
    let request: PairRequest = assert_round_trip("pair_request.json");
    assert_eq!(request.code, "ABCD-EFGH");
    assert_eq!(
        request.install.as_ref().map(InstallId::as_str),
        Some("zr-0123456789abcdef")
    );
}

#[test]
fn a_pairing_request_may_leave_out_everything_but_the_code() {
    let request: PairRequest =
        serde_json::from_value(serde_json::json!({ "code": "ABCD-EFGH" })).unwrap();
    assert_eq!(request.install, None);
    assert_eq!((request.name.as_str(), request.model.as_str()), ("", ""));
    let empty: PairRequest =
        serde_json::from_value(serde_json::json!({ "code": "ABCD-EFGH", "install": "" })).unwrap();
    assert_eq!(empty.install, None, "an empty install is none");
    let nulls: PairRequest = serde_json::from_value(
        serde_json::json!({ "code": "ABCD-EFGH", "name": null, "model": null, "install": null }),
    )
    .unwrap();
    assert_eq!((nulls.name.as_str(), nulls.model.as_str()), ("", ""));
    assert_eq!(nulls.install, None);
}

#[test]
fn a_refresh_reads_an_empty_install_as_none_as_pairing_does() {
    let request: RefreshRequest =
        serde_json::from_value(serde_json::json!({ "refresh": "token", "install": "" })).unwrap();
    assert_eq!(request.install, None);
}

#[test]
fn an_accepted_sign_in_round_trips_and_grants_a_session() {
    let reply: AuthReply = assert_round_trip("auth_accepted.json");
    let grant = reply.clone().into_grant().unwrap();
    assert_eq!(grant.access.expose_secret(), "access-token-for-tests");
    assert_eq!(grant.device, Some(DeviceId::new(3)));
    assert_eq!(
        grant.expires.as_ref().unwrap().as_str(),
        "2026-11-24T22:57:53.410Z"
    );
    assert_eq!(AuthReply::accepted(grant), reply);
    assert!(
        !format!("{reply:?}").contains("token-for-tests"),
        "tokens never show in Debug"
    );
}

#[test]
fn a_refused_sign_in_round_trips_and_says_why() {
    let reply: AuthReply = assert_round_trip("auth_refused.json");
    assert_eq!(
        reply.clone().into_grant().unwrap_err(),
        AuthRefusal {
            status: 401,
            error: "That code isn't right. Check it, or make a new one.".to_owned()
        }
    );
    assert_eq!(
        to_value(&AuthReply::refused(
            401,
            "That code isn't right. Check it, or make a new one."
        )),
        to_value(&reply)
    );
    let locked: AuthReply = assert_round_trip("auth_too_many.json");
    assert_eq!(locked.into_grant().unwrap_err().status, 429);
}

#[test]
fn a_refresh_request_round_trips() {
    let request: RefreshRequest = assert_round_trip("refresh_request.json");
    assert_eq!(request.refresh.expose_secret(), "refresh-token-for-tests");
    let bare: RefreshRequest = serde_json::from_str(r#"{"refresh":"x"}"#).unwrap();
    assert!(bare.install.is_none());
    assert_eq!(to_value(&bare), serde_json::json!({ "refresh": "x" }));
}

#[test]
fn a_pairing_code_round_trips() {
    let code: PairingCode = assert_round_trip("pair_code.json");
    assert!(code.ok);
    assert_eq!(code.code.len(), 9);
    assert_eq!(
        code.link, None,
        "a server that predates the link leaves it out"
    );
}

#[test]
fn a_pairing_code_s_link_holds_the_same_code() {
    let code: PairingCode = assert_round_trip("pair_code_link.json");
    let link = PairingLink::parse(code.link.as_deref().unwrap()).unwrap();
    assert_eq!(link.code(), code.code);
    assert_eq!(link.routes(), ["http://192.0.2.10:18990"]);
}
