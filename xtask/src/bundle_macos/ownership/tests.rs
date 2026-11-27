use std::fs;
use std::path::{Path, PathBuf};

use super::{Occupant, Refusal, holds_ours, identifier_from_plutil, may_replace, occupant};

/// A bundle directory at `<dir>/<name>` whose Info.plist holds `info`.
fn bundle_in(dir: &Path, name: &str, info: &str) -> PathBuf {
    let bundle = dir.join(name);
    fs::create_dir_all(bundle.join("Contents")).unwrap();
    fs::write(bundle.join("Contents/Info.plist"), info).unwrap();
    bundle
}

/// Stands in for plutil: the test plists hold just the identifier, or nothing.
fn identify(info: &Path) -> anyhow::Result<Option<String>> {
    Ok(identifier_from_plutil(true, &fs::read_to_string(info)?))
}

#[test]
fn our_own_bundle_may_be_replaced() {
    assert_eq!(may_replace(Some("com.emberhut.desktop")), Ok(()));
    assert_eq!(may_replace(Some("com.emberhut.desktop\n")), Ok(()));
}

#[test]
fn another_app_is_never_replaced() {
    for other in [
        "com.example.assistant",
        "com.example.emberhut",
        "com.emberhut.desktop.other",
        "COM.EMBERHUT.DESKTOP",
    ] {
        assert_eq!(
            may_replace(Some(other)),
            Err(Refusal::OtherApp {
                identifier: other.to_owned()
            })
        );
    }
}

#[test]
fn an_unknown_identifier_is_never_replaced() {
    assert_eq!(may_replace(None), Err(Refusal::UnknownIdentifier));
    assert_eq!(may_replace(Some(" ")), Err(Refusal::UnknownIdentifier));
}

#[test]
fn plutil_answers_count_only_when_it_succeeds_with_output() {
    assert_eq!(
        identifier_from_plutil(true, "com.emberhut.desktop\n").as_deref(),
        Some("com.emberhut.desktop")
    );
    assert_eq!(identifier_from_plutil(true, "\n"), None);
    assert_eq!(
        identifier_from_plutil(false, "com.emberhut.desktop\n"),
        None
    );
}

#[test]
fn a_refusal_says_whose_bundle_it_is() {
    let refusal = Refusal::OtherApp {
        identifier: "com.example.other".into(),
    };
    assert_eq!(
        refusal.to_string(),
        "it is another app (com.example.other, not com.emberhut.desktop)"
    );
}

#[test]
fn a_free_path_and_a_bundle_are_told_apart() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        occupant(&dir.path().join("Emberhut.app")).unwrap(),
        Occupant::Free
    );
    let bundle = bundle_in(dir.path(), "Emberhut.app", "com.emberhut.desktop");
    assert_eq!(
        occupant(&bundle).unwrap(),
        Occupant::Bundle(bundle.join("Contents/Info.plist"))
    );
}

#[test]
fn a_file_or_a_bundle_without_info_plist_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Emberhut.app");
    fs::write(&file, "not a bundle").unwrap();
    let err = occupant(&file).unwrap_err().to_string();
    assert!(err.starts_with("refusing to replace"), "{err}");
    let empty = dir.path().join("Empty.app");
    fs::create_dir(&empty).unwrap();
    let err = occupant(&empty).unwrap_err().to_string();
    assert!(err.contains("no Contents/Info.plist"), "{err}");
}

#[test]
#[cfg(unix)]
fn a_symlink_is_refused_even_to_our_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let real = bundle_in(dir.path(), "Real.app", "com.emberhut.desktop");
    let link = dir.path().join("Emberhut.app");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let err = occupant(&link).unwrap_err().to_string();
    assert!(err.contains("not a bundle directory"), "{err}");
}

#[test]
fn holds_ours_accepts_only_this_app() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!holds_ours(&dir.path().join("Emberhut.app"), identify).unwrap());
    let ours = bundle_in(dir.path(), "Emberhut.app", "com.emberhut.desktop");
    assert!(holds_ours(&ours, identify).unwrap());
    let other = bundle_in(dir.path(), "Other.app", "com.example.other");
    let err = holds_ours(&other, identify).unwrap_err().to_string();
    assert!(err.contains("com.example.other"), "{err}");
    let unreadable = bundle_in(dir.path(), "Unreadable.app", "");
    let err = holds_ours(&unreadable, identify).unwrap_err().to_string();
    assert!(err.contains("no readable CFBundleIdentifier"), "{err}");
}
