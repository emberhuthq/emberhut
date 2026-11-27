//! The newest builds in the state, and the release step's report of one.

use emberhut_api::builds::{BuildPlatform, BuildReport, LatestBuilds};
use emberhut_api::ids::RoomId;
use emberhut_api::settings::{SettingsReply, SettingsRequest};
use emberhut_api::state::State;
use serde_json::json;

use crate::support::{fixture, from_value, read, to_value};

#[test]
fn reads_the_builds_in_the_state() {
    let state: State = read("state.json");
    assert_eq!(
        state.builds,
        Some(LatestBuilds {
            ios: Some(120),
            mac: Some(121)
        })
    );
    assert_eq!(read::<State>("state_older.json").builds, None);
}

#[test]
fn no_build_yet_is_written_as_nulls() {
    assert_eq!(
        to_value(&LatestBuilds::default()),
        json!({ "ios": null, "mac": null })
    );
    assert_eq!(
        from_value::<LatestBuilds>(json!({})),
        LatestBuilds::default()
    );
}

#[test]
fn a_build_report_reads_as_the_release_step_sends_it() {
    let body = json!({ "build": { "platform": "ios", "number": 12 } });
    let report: SettingsRequest = from_value(body.clone());
    assert_eq!(
        report,
        SettingsRequest {
            build: Some(BuildReport {
                platform: BuildPlatform::Ios,
                number: 12
            }),
            ..SettingsRequest::default()
        }
    );
    assert_eq!(to_value(&report), body);
    let in_a_room: SettingsRequest =
        from_value(json!({ "build": { "platform": "mac", "number": 3 }, "room": 2 }));
    assert_eq!(in_a_room.room, Some(RoomId::new(2)));
}

#[test]
fn the_reply_is_ok_with_the_state_spread_in() {
    let reply = SettingsReply {
        ok: true,
        state: read("state.json"),
    };
    let written = to_value(&reply);
    assert_eq!(written["ok"], true);
    assert_eq!(written["builds"], fixture("state.json")["builds"]);
    assert_eq!(from_value::<SettingsReply>(written), reply);
}
