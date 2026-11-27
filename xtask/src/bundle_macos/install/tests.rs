use std::fs;
use std::path::{Path, PathBuf};

use anyhow::bail;
use tempfile::TempDir;

use super::{InstallPaths, Occupancy, Step, System, install_into, steps};

/// How the stand-in for ditto behaves.
#[derive(Debug, Clone, Copy)]
enum Copy {
    /// Copies the whole bundle.
    Whole,
    /// Writes part of the bundle, then fails.
    FailHalfway,
    /// Reports success without writing anything, so the rename into place fails.
    Nothing,
}

/// A stand-in for plutil and ditto: test Info.plists hold just the bundle id.
#[derive(Debug)]
struct Fake(Copy);

impl System for Fake {
    fn identifier(&self, info_plist: &Path) -> anyhow::Result<Option<String>> {
        let identifier = fs::read_to_string(info_plist)?.trim().to_owned();
        Ok((!identifier.is_empty()).then_some(identifier))
    }

    fn copy_bundle(&self, from: &Path, to: &Path) -> anyhow::Result<()> {
        match self.0 {
            Copy::Whole => copy_tree(from, to),
            Copy::FailHalfway => {
                fs::create_dir_all(to.join("Contents"))?;
                bail!("the disk is full")
            }
            Copy::Nothing => Ok(()),
        }
    }
}

fn copy_tree(from: &Path, to: &Path) -> anyhow::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Writes a bundle at `path` with bundle id `identifier` and a marker of which copy it is.
fn bundle_at(path: &Path, identifier: &str, copy: &str) {
    fs::create_dir_all(path.join("Contents")).unwrap();
    fs::write(path.join("Contents/Info.plist"), identifier).unwrap();
    fs::write(path.join("Contents/copy"), copy).unwrap();
}

fn copy_at(path: &Path) -> String {
    fs::read_to_string(path.join("Contents/copy")).unwrap()
}

/// A scratch dir with the bundle to install (`src/Emberhut.app`, copy "new") and an empty
/// `apps` dir standing in for /Applications.
fn setup() -> (TempDir, PathBuf, InstallPaths) {
    let scratch = tempfile::tempdir().unwrap();
    let bundle = scratch.path().join("src/Emberhut.app");
    bundle_at(&bundle, "com.emberhut.desktop", "new");
    let apps = scratch.path().join("apps");
    fs::create_dir(&apps).unwrap();
    let paths = InstallPaths::in_dir(&apps);
    (scratch, bundle, paths)
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn it_installs_to_exactly_applications_emberhut_app() {
    let paths = InstallPaths::in_dir(Path::new("/Applications"));
    assert_eq!(paths.dest, Path::new("/Applications/Emberhut.app"));
    assert_eq!(paths.new, Path::new("/Applications/.Emberhut.app.new"));
    assert_eq!(paths.old, Path::new("/Applications/.Emberhut.app.old"));
}

#[test]
fn the_steps_follow_what_is_there() {
    let free = Occupancy {
        dest: false,
        stale_new: false,
        stale_old: false,
    };
    assert_eq!(steps(free), [Step::CopyToNew, Step::MoveNewIntoPlace]);
    let installed = Occupancy { dest: true, ..free };
    assert_eq!(
        steps(installed),
        [
            Step::CopyToNew,
            Step::MoveOldAside,
            Step::MoveNewIntoPlace,
            Step::RemoveOld
        ]
    );
    let stale = Occupancy {
        dest: true,
        stale_new: true,
        stale_old: true,
    };
    assert_eq!(
        steps(stale),
        [
            Step::RemoveStaleNew,
            Step::RemoveStaleOld,
            Step::CopyToNew,
            Step::MoveOldAside,
            Step::MoveNewIntoPlace,
            Step::RemoveOld
        ]
    );
    let only_old = Occupancy {
        stale_old: true,
        ..free
    };
    assert_eq!(
        steps(only_old),
        [
            Step::RestoreOld,
            Step::CopyToNew,
            Step::MoveOldAside,
            Step::MoveNewIntoPlace,
            Step::RemoveOld
        ]
    );
}

#[test]
fn a_fresh_install() {
    let (_scratch, bundle, paths) = setup();
    let dir = paths.dest.parent().unwrap();
    let installed = install_into(dir, &bundle, &Fake(Copy::Whole)).unwrap();
    assert_eq!(installed, paths.dest);
    assert_eq!(copy_at(&paths.dest), "new");
    assert_eq!(names_in(dir), ["Emberhut.app"]);
}

#[test]
fn our_old_app_is_replaced_and_nothing_else_is_touched() {
    let (_scratch, bundle, paths) = setup();
    let dir = paths.dest.parent().unwrap();
    bundle_at(&paths.dest, "com.emberhut.desktop", "old");
    bundle_at(&dir.join("Other.app"), "com.example.other", "other");
    install_into(dir, &bundle, &Fake(Copy::Whole)).unwrap();
    assert_eq!(copy_at(&paths.dest), "new");
    assert_eq!(copy_at(&dir.join("Other.app")), "other");
    assert_eq!(names_in(dir), ["Emberhut.app", "Other.app"]);
}

#[test]
fn stale_copies_of_ours_are_cleared() {
    let (_scratch, bundle, paths) = setup();
    let dir = paths.dest.parent().unwrap();
    bundle_at(&paths.dest, "com.emberhut.desktop", "installed");
    bundle_at(&paths.new, "com.emberhut.desktop", "stale new");
    bundle_at(&paths.old, "com.emberhut.desktop", "stale old");
    install_into(dir, &bundle, &Fake(Copy::Whole)).unwrap();
    assert_eq!(copy_at(&paths.dest), "new");
    assert_eq!(names_in(dir), ["Emberhut.app"]);
}

#[test]
fn another_app_at_any_install_path_is_left_alone() {
    for foreign in [0, 1, 2] {
        let (_scratch, bundle, paths) = setup();
        let dir = paths.dest.parent().unwrap();
        let target = [&paths.dest, &paths.new, &paths.old][foreign];
        bundle_at(target, "com.example.other", "foreign");
        let before = names_in(dir);
        let err = install_into(dir, &bundle, &Fake(Copy::Whole)).unwrap_err();
        assert!(err.to_string().contains("com.example.other"), "{err:#}");
        assert_eq!(copy_at(target), "foreign");
        assert_eq!(names_in(dir), before);
    }
}

#[test]
fn a_failed_copy_leaves_the_old_app_in_place() {
    let (_scratch, bundle, paths) = setup();
    let dir = paths.dest.parent().unwrap();
    bundle_at(&paths.dest, "com.emberhut.desktop", "old");
    let err = install_into(dir, &bundle, &Fake(Copy::FailHalfway)).unwrap_err();
    assert!(format!("{err:#}").contains("the disk is full"), "{err:#}");
    assert!(
        err.to_string().contains("holds the app installed before"),
        "{err:#}"
    );
    assert_eq!(copy_at(&paths.dest), "old");
    assert_eq!(names_in(dir), ["Emberhut.app"]);
}

#[test]
fn a_failed_rename_into_place_moves_the_old_app_back() {
    let (_scratch, bundle, paths) = setup();
    let dir = paths.dest.parent().unwrap();
    bundle_at(&paths.dest, "com.emberhut.desktop", "old");
    let err = install_into(dir, &bundle, &Fake(Copy::Nothing)).unwrap_err();
    assert!(format!("{err:#}").contains("moving"), "{err:#}");
    assert!(
        err.to_string().contains("holds the app installed before"),
        "{err:#}"
    );
    assert_eq!(copy_at(&paths.dest), "old");
    assert_eq!(names_in(dir), ["Emberhut.app"]);
}

#[test]
fn a_lone_old_copy_is_kept_when_the_copy_fails() {
    // What a failed rollback leaves: Emberhut.app missing, the only copy of the app at .old.
    let (_scratch, bundle, paths) = setup();
    let dir = paths.dest.parent().unwrap();
    bundle_at(&paths.old, "com.emberhut.desktop", "old");
    let err = install_into(dir, &bundle, &Fake(Copy::FailHalfway)).unwrap_err();
    assert!(format!("{err:#}").contains("the disk is full"), "{err:#}");
    assert_eq!(copy_at(&paths.dest), "old");
    assert_eq!(names_in(dir), ["Emberhut.app"]);
}

#[test]
fn a_lone_old_copy_is_replaced_when_the_copy_succeeds() {
    let (_scratch, bundle, paths) = setup();
    let dir = paths.dest.parent().unwrap();
    bundle_at(&paths.old, "com.emberhut.desktop", "old");
    install_into(dir, &bundle, &Fake(Copy::Whole)).unwrap();
    assert_eq!(copy_at(&paths.dest), "new");
    assert_eq!(names_in(dir), ["Emberhut.app"]);
}
