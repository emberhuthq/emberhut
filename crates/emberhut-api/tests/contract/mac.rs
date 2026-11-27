//! The Mac's row.

use emberhut_api::mac::MacStatus;

use crate::support::{assert_round_trip, fixture, from_value, with_unknown_fields};

#[test]
fn reads_the_mac_row_online_and_offline() {
    let online: MacStatus = assert_round_trip("mac_online.json");
    assert!(online.online);
    assert_eq!(online.locked, Some(false));
    assert_eq!(online.idle, Some(312.5));
    let offline: MacStatus = assert_round_trip("mac_offline.json");
    assert_eq!(offline, MacStatus::OFFLINE);
}

#[test]
fn the_mac_row_tolerates_unknown_fields_and_a_null_idle() {
    let mut body = fixture("mac_online.json");
    with_unknown_fields(&mut body);
    assert_eq!(
        from_value::<MacStatus>(body),
        from_value::<MacStatus>(fixture("mac_online.json"))
    );
    let unknown_idle: MacStatus =
        from_value(serde_json::json!({ "online": true, "locked": true, "idle": null }));
    assert_eq!(unknown_idle.idle, None);
}

#[test]
fn the_mac_row_is_written_in_its_wire_shape() {
    let quiet = MacStatus {
        online: true,
        locked: Some(true),
        idle: None,
    };
    assert_eq!(
        serde_json::to_value(&quiet).unwrap(),
        serde_json::json!({ "online": true, "locked": true, "idle": null })
    );
    let offline = MacStatus {
        online: false,
        locked: Some(true),
        idle: Some(3.0),
    };
    assert_eq!(
        serde_json::to_value(&offline).unwrap(),
        serde_json::json!({ "online": false })
    );
}
