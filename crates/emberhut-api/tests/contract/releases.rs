//! The release feed, as ADR 0005 writes it: a desktop feed with its payloads, a phone's feed
//! without any, and a newer server's fields.

use emberhut_api::paths;
use emberhut_api::releases::{Arch, Build, FORMAT, Platform, ReleaseFeed};

use crate::support::{assert_round_trip, fixture, from_value, read, with_unknown_fields};

#[test]
fn reads_the_desktop_feed() {
    let feed: ReleaseFeed = assert_round_trip("release_feed.json");
    assert_eq!(feed.format, FORMAT);
    assert_eq!(feed.product, "emberhut-desktop");
    assert_eq!(feed.platform, Platform::Macos);
    assert_eq!(feed.published.as_str(), "2026-12-08T10:00:00.000Z");
    let newest = feed.newest().unwrap();
    assert_eq!(newest.build, Build::new(15));
    assert_eq!(newest.version, "0.3.0");
    assert_eq!(newest.notes, "- Rooms open faster.\n- …");
    assert_eq!(newest.minimum_os.as_deref(), Some("13.0"));
    let asset = newest.asset(&Arch::Arm64).unwrap();
    assert_eq!(
        asset.url,
        "/api/releases/files/Emberhut_0.3.0_15_macos_arm64.dmg"
    );
    assert_eq!(asset.size, 41_234_567);
    assert_eq!(asset.sha256.len(), 64);
    assert!(
        asset
            .signature
            .contains("trusted comment: emberhut-desktop macos arm64 0.3.0 build 15")
    );
    assert!(newest.asset(&Arch::X86_64).is_none(), "arm64 only");
    assert_eq!(feed.releases[1].assets, []);
}

#[test]
fn a_phones_feed_has_no_payloads_and_no_minimum() {
    let feed: ReleaseFeed = read("release_feed_phone.json");
    assert_eq!(feed.platform, Platform::Android);
    let [release] = feed.releases.as_slice() else {
        panic!("one release")
    };
    assert_eq!(release.assets, []);
    assert_eq!(release.minimum_os, None);
}

#[test]
fn the_feed_tolerates_unknown_fields_and_words() {
    let mut body = fixture("release_feed.json");
    with_unknown_fields(&mut body);
    assert_eq!(
        from_value::<ReleaseFeed>(body),
        from_value::<ReleaseFeed>(fixture("release_feed.json"))
    );
    let mut newer = fixture("release_feed.json");
    newer["platform"] = "visionos".into();
    newer["releases"][0]["assets"][0]["arch"] = "riscv64".into();
    let feed = from_value::<ReleaseFeed>(newer);
    assert_eq!(feed.platform, Platform::Other("visionos".into()));
    assert_eq!(
        feed.releases[0].assets[0].arch,
        Arch::Other("riscv64".into())
    );
}

#[test]
fn a_feed_with_nothing_listed_still_reads() {
    let mut body = fixture("release_feed_phone.json");
    body.as_object_mut().unwrap().remove("releases");
    let feed = from_value::<ReleaseFeed>(body);
    assert_eq!(feed.releases, []);
    assert!(feed.newest().is_none());
}

#[test]
fn the_feeds_path_names_its_format_and_platform() {
    assert_eq!(
        paths::release_feed(1, &Platform::Macos),
        "/api/releases/v1/macos"
    );
    assert_eq!(
        paths::release_feed(2, &Platform::Android),
        "/api/releases/v2/android"
    );
}
