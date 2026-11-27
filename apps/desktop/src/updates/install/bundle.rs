//! Where the running app is, and whether it may replace itself there: an `Emberhut.app` bundle that
//! holds this app (`com.emberhut.desktop`), in a folder it can write to, not run from a read-only
//! image or a translocated copy (where macOS runs an app it just downloaded).

use std::path::{Path, PathBuf};

/// The bundle's name.
const BUNDLE: &str = "Emberhut.app";

/// The installed bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Installed {
    /// The bundle, `…/Emberhut.app`.
    pub(crate) app: PathBuf,
    /// The folder it is in.
    pub(crate) dir: PathBuf,
    /// Its executable's name in `Contents/MacOS`.
    executable: String,
}

/// Why the running app can't update itself where it is.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum NotUpdatable {
    /// It isn't running from an `Emberhut.app` bundle.
    #[error("Emberhut isn't running from Emberhut.app")]
    NotABundle,
    /// It runs from a translocated or read-only place.
    #[error("Emberhut runs from a read-only place: move it to Applications first")]
    ReadOnly,
    /// Its folder can't be written to.
    #[error("Emberhut can't write to its folder: move it to Applications first")]
    FolderLocked,
}

impl Installed {
    /// The bundle the executable at `exe` (`…/Emberhut.app/Contents/MacOS/emberhut-desktop`) is in.
    ///
    /// # Errors
    ///
    /// [`NotUpdatable::NotABundle`], or [`NotUpdatable::ReadOnly`] for a translocated copy or
    /// one on a mounted image.
    pub(crate) fn of(exe: &Path) -> Result<Self, NotUpdatable> {
        let executable = exe
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(NotUpdatable::NotABundle)?
            .to_owned();
        let macos = exe.parent().ok_or(NotUpdatable::NotABundle)?;
        let contents = macos.parent().ok_or(NotUpdatable::NotABundle)?;
        let app = contents.parent().ok_or(NotUpdatable::NotABundle)?;
        let dir = app.parent().ok_or(NotUpdatable::NotABundle)?;
        let shaped = macos.file_name().is_some_and(|name| name == "MacOS")
            && contents.file_name().is_some_and(|name| name == "Contents")
            && app.file_name().is_some_and(|name| name == BUNDLE);
        if !shaped {
            return Err(NotUpdatable::NotABundle);
        }
        let text = app.to_string_lossy();
        if text.contains("/AppTranslocation/") || app.starts_with("/Volumes") {
            return Err(NotUpdatable::ReadOnly);
        }
        Ok(Self {
            app: app.to_path_buf(),
            dir: dir.to_path_buf(),
            executable,
        })
    }

    /// Checks that the bundle's folder can be written to.
    ///
    /// # Errors
    ///
    /// [`NotUpdatable::FolderLocked`].
    pub(crate) fn check_folder(&self) -> Result<(), NotUpdatable> {
        rustix::fs::access(&self.dir, rustix::fs::Access::WRITE_OK)
            .map_err(|_| NotUpdatable::FolderLocked)
    }

    /// Where the new bundle is staged, beside the installed one (same volume, so it can be
    /// exchanged with it).
    #[must_use]
    pub(crate) fn staged(&self) -> PathBuf {
        self.dir.join(".Emberhut.app.new")
    }

    /// Where the old bundle waits after the swap, until the new one is healthy.
    #[must_use]
    pub(crate) fn kept_old(&self) -> PathBuf {
        self.dir.join(".Emberhut.app.old")
    }

    /// The bundle's executable.
    #[must_use]
    pub(crate) fn executable(&self) -> PathBuf {
        self.app.join("Contents/MacOS").join(&self.executable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_is_found_from_its_executable() {
        let installed = Installed::of(Path::new(
            "/Applications/Emberhut.app/Contents/MacOS/emberhut-desktop",
        ))
        .unwrap();
        assert_eq!(installed.app, Path::new("/Applications/Emberhut.app"));
        assert_eq!(installed.dir, Path::new("/Applications"));
        assert_eq!(
            installed.staged(),
            Path::new("/Applications/.Emberhut.app.new")
        );
        assert_eq!(
            installed.kept_old(),
            Path::new("/Applications/.Emberhut.app.old")
        );
        assert_eq!(
            installed.executable(),
            Path::new("/Applications/Emberhut.app/Contents/MacOS/emberhut-desktop")
        );
    }

    #[test]
    fn a_bare_binary_a_renamed_bundle_or_a_read_only_place_is_not_updated() {
        for exe in [
            "/opt/build/target/release/emberhut-desktop",
            "/Applications/Emberhut 2.app/Contents/MacOS/emberhut-desktop",
            "/Applications/Emberhut.app/Contents/Resources/emberhut-desktop",
            "emberhut-desktop",
        ] {
            assert_eq!(
                Installed::of(Path::new(exe)),
                Err(NotUpdatable::NotABundle),
                "{exe}"
            );
        }
        for exe in [
            "/private/var/folders/x/AppTranslocation/1234/d/Emberhut.app/Contents/MacOS/emberhut-desktop",
            "/Volumes/Emberhut/Emberhut.app/Contents/MacOS/emberhut-desktop",
        ] {
            assert_eq!(
                Installed::of(Path::new(exe)),
                Err(NotUpdatable::ReadOnly),
                "{exe}"
            );
        }
    }

    #[test]
    fn a_folder_that_cant_be_written_to_is_said() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir
            .path()
            .join("Emberhut.app/Contents/MacOS/emberhut-desktop");
        let installed = Installed::of(&exe).unwrap();
        assert_eq!(installed.check_folder(), Ok(()));
        let missing =
            Installed::of(Path::new("/nonexistent/Emberhut.app/Contents/MacOS/z")).unwrap();
        assert_eq!(missing.check_folder(), Err(NotUpdatable::FolderLocked));
    }
}
