use std::io::{self, Write as _};

use zip::write::SimpleFileOptions;

use super::*;

/// The id of the key `YWJj` (the bytes `abc`).
const ID: &str = "lkhibglpipabmpokebebeanofnkocccd";

fn manifest(version: &str, key: &str) -> String {
    format!(r#"{{"manifest_version":3,"name":"Emberhut","version":"{version}","key":"{key}"}}"#)
}

/// A zip of `files` (name, bytes), deflated.
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in files {
        if name.ends_with('/') {
            writer.add_directory(*name, options).unwrap();
        } else {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}

fn folder(root: &Path) -> ExtensionFolder {
    ExtensionFolder::new(root.join("Emberhut").join("Extension"), ID)
}

#[test]
fn a_emberhut_zip_is_installed_and_its_version_read() {
    let root = tempfile::tempdir().unwrap();
    let folder = folder(root.path());
    assert_eq!(folder.installed(), None);
    let zip = zip_of(&[
        ("manifest.json", manifest("1.2.3", "YWJj").as_bytes()),
        ("js/", b""),
        ("js/background.js", b"chrome.runtime;"),
    ]);
    let installed = folder.install(&zip).unwrap();
    assert_eq!(installed.version, "1.2.3");
    assert_eq!(folder.installed(), Some(installed.clone()));
    assert_eq!(
        fs::read_to_string(installed.path.join("js/background.js")).unwrap(),
        "chrome.runtime;"
    );
}

#[test]
fn a_new_version_replaces_the_old_one_whole() {
    let root = tempfile::tempdir().unwrap();
    let folder = folder(root.path());
    folder
        .install(&zip_of(&[
            ("manifest.json", manifest("1.0.0", "YWJj").as_bytes()),
            ("old.js", b"old"),
        ]))
        .unwrap();
    // A zip with one top folder, as a build tool makes it.
    let installed = folder
        .install(&zip_of(&[
            (
                "emberhut-extension/manifest.json",
                manifest("1.1.0", "YWJj").as_bytes(),
            ),
            ("emberhut-extension/new.js", b"new"),
        ]))
        .unwrap();
    assert_eq!(installed.version, "1.1.0");
    assert!(installed.path.join("new.js").is_file());
    assert!(!installed.path.join("old.js").exists());
    // Nothing is left beside it.
    let beside: Vec<String> = fs::read_dir(root.path().join("Emberhut"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(beside, ["Extension"]);
}

#[test]
fn another_extension_s_key_is_refused_and_the_folder_kept() {
    let root = tempfile::tempdir().unwrap();
    let folder = folder(root.path());
    folder
        .install(&zip_of(&[(
            "manifest.json",
            manifest("1.0.0", "YWJj").as_bytes(),
        )]))
        .unwrap();
    let other = zip_of(&[("manifest.json", manifest("9.9.9", "eHl6").as_bytes())]);
    let ExtensionError::Refused(why) = folder.install(&other).unwrap_err() else {
        panic!("refused")
    };
    assert!(why.contains("not Emberhut's"), "{why}");
    let keyless = zip_of(&[("manifest.json", br#"{"version":"2.0.0"}"#.as_slice())]);
    assert!(matches!(
        folder.install(&keyless),
        Err(ExtensionError::Refused(_))
    ));
    assert_eq!(folder.installed().unwrap().version, "1.0.0");
}

#[test]
fn entries_outside_the_folder_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let folder = folder(root.path());
    for name in ["../escape.js", "a/../../escape.js", "/abs.js"] {
        let zip = zip_of(&[
            ("manifest.json", manifest("1.0.0", "YWJj").as_bytes()),
            (name, b"x"),
        ]);
        let ExtensionError::Refused(why) = folder.install(&zip).unwrap_err() else {
            panic!("refused")
        };
        assert!(why.contains("outside"), "{name}: {why}");
    }
    assert!(!root.path().join("escape.js").exists());
    assert!(!root.path().join("Emberhut").join("escape.js").exists());
    assert_eq!(folder.installed(), None);
}

#[test]
fn links_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let folder = folder(root.path());
    let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    writer.start_file("manifest.json", options).unwrap();
    writer
        .write_all(manifest("1.0.0", "YWJj").as_bytes())
        .unwrap();
    writer
        .add_symlink("passwd", "/etc/passwd", options)
        .unwrap();
    let zip = writer.finish().unwrap().into_inner();
    let ExtensionError::Refused(why) = folder.install(&zip).unwrap_err() else {
        panic!("refused")
    };
    assert!(why.contains("link"), "{why}");
    assert_eq!(folder.installed(), None);
}

#[test]
fn not_a_zip_or_too_big_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let folder = folder(root.path());
    assert!(matches!(
        folder.install(b"not a zip"),
        Err(ExtensionError::Refused(_))
    ));
    let big = vec![0_u8; MAX_ZIP + 1];
    let ExtensionError::Refused(why) = folder.install(&big).unwrap_err() else {
        panic!("refused")
    };
    assert_eq!(why, "the zip is over 20 MB");
    let none = zip_of(&[("readme.txt", b"hi")]);
    assert!(matches!(
        folder.install(&none),
        Err(ExtensionError::Refused(_))
    ));
}

#[test]
fn a_zip_bomb_stops_at_the_unpacked_limit() {
    let root = tempfile::tempdir().unwrap();
    let folder = folder(root.path());
    let zeros = vec![0_u8; 101 * 1024 * 1024];
    let zip = zip_of(&[
        ("manifest.json", manifest("1.0.0", "YWJj").as_bytes()),
        ("big.bin", &zeros),
    ]);
    assert!(zip.len() < MAX_ZIP);
    let ExtensionError::Refused(why) = folder.install(&zip).unwrap_err() else {
        panic!("refused")
    };
    assert!(why.contains("unpacks to more than"), "{why}");
    assert_eq!(folder.installed(), None);
}

#[test]
fn leftovers_of_an_interrupted_install_are_cleared() {
    let root = tempfile::tempdir().unwrap();
    let folder = folder(root.path());
    let left = root.path().join("Emberhut").join(".Extension.new-abc");
    fs::create_dir_all(&left).unwrap();
    folder
        .install(&zip_of(&[(
            "manifest.json",
            manifest("1.0.0", "YWJj").as_bytes(),
        )]))
        .unwrap();
    assert!(!left.exists());
}
