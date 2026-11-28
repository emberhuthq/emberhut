//! The app updating itself (ADR 0005, "The update", as ADR 0032 moves releases to GitHub): the
//! repository's GitHub Releases, read at launch and then once a day when automatic checks are
//! on; the newest release on the chosen channel (Stable, or Release candidates) whose build is
//! above this one; its payload for this computer downloaded under a hard cap and checked (size,
//! SHA-256, and the update signature with its trusted comment; on macOS also the bundle's code
//! signature, team, notarization and `Info.plist`) before anything of the running app is
//! touched; then, on Install and restart or after 10 idle minutes, the install. On macOS that is
//! the atomic swap with the old copy watching the new one start and rolling back if it doesn't;
//! on Windows, the release's `.msi` run once the app has quit, and the app started again.
//!
//! Only a release build (one with `EMBERHUT_BUILD`) on macOS or Windows updates itself: a
//! development build, a Mac app that isn't signed with a Developer ID, a Windows app that isn't
//! the per-user install, and the app on Linux never check.
//!
//! The pure decisions ([`cadence`], [`channel`], [`choose`], [`tag`], the GitHub list read
//! ([`github`]), [`trusted`], [`marker`], the record's waits, the watch's verdict) are plain
//! types with their own tests; the store (`store/updates.rs`) holds what the views show and
//! runs the checks on a background thread.

pub(crate) mod cadence;
pub(crate) mod channel;
mod check;
pub(crate) mod choose;
mod download;
mod files;
pub(crate) mod github;
mod health;
#[cfg(unix)]
mod install;
// The Mac's install step (`install`) is built on Unix only; on Windows (which installs with
// `msi`), what only it uses is marked as such where it is.
pub(crate) mod installer;
mod keys;
#[cfg_attr(
    all(windows, not(test)),
    expect(dead_code, reason = "used by the install step, built on Unix only")
)]
pub(crate) mod marker;
#[cfg(any(windows, test))]
mod msi;
#[cfg_attr(
    windows,
    expect(
        dead_code,
        unused_imports,
        reason = "a Dock click passed on to a new copy is the install step's, built on Unix only"
    )
)]
pub(crate) mod platform;
pub(crate) mod record;
pub(crate) mod self_check;
mod signature;
pub(crate) mod system;
pub(crate) mod tag;
mod trusted;
pub(crate) mod version;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use emberhut_api::releases::{Build, Platform};

pub(crate) use self::check::{CheckError, Earlier, Found, Ready};
use self::github::Source;
pub(crate) use self::health::report_when_drawn;
use self::installer::{InstallError, Installer, Staged, Watch};
use self::keys::TrustedKey;
use self::system::ThisMachine;
use self::version::Running;
use crate::config::AppFiles;

/// The Dock was clicked while this copy watches the new one start: the click goes to the new
/// copy. Whether it did.
#[must_use]
pub(crate) fn forward_reopen() -> bool {
    #[cfg(unix)]
    if let Some(pid) = install::watchdog::watched() {
        return platform::activate(pid);
    }
    false
}

/// Why this copy doesn't update itself, worded for Settings and the dialog.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Unavailable {
    /// The app updates itself on macOS and Windows only.
    #[error("Emberhut updates itself on macOS and Windows; here, install a new version by hand")]
    ByHand,
    /// A development build never updates itself.
    #[error("Development builds don't update themselves.")]
    DevelopmentBuild,
    /// The app has no directory to keep its files in.
    #[error("Emberhut has nowhere to keep an update on this computer")]
    NoFiles,
    /// It runs from a place it can't replace itself in.
    #[error("{0}")]
    Place(String),
    /// It isn't signed with a Developer ID (an ad-hoc signed app fails closed).
    #[cfg_attr(
        windows,
        expect(dead_code, reason = "found by the install step, built on Unix only")
    )]
    #[error("This copy of Emberhut isn't signed with a Developer ID, so it doesn't update itself")]
    NotDeveloperId,
    /// Something it needs to know couldn't be read.
    #[error("Emberhut can't check for updates: couldn't read {0}")]
    Unreadable(String),
}

/// The updater of a release build: what it is, where releases come from, where it keeps things,
/// whom it trusts, and how it installs.
#[derive(Debug)]
pub(crate) struct Engine {
    running: Running,
    build: Build,
    here: ThisMachine,
    keys: &'static [TrustedKey],
    /// Where releases are listed and fetched.
    source: Arc<dyn Source>,
    /// The owner-only folder payloads are downloaded to.
    staging: PathBuf,
    /// The app's directory: the record and the marker.
    data: PathBuf,
    installer: Arc<dyn Installer>,
}

impl Engine {
    /// The updater of the running app, its files in `files`.
    ///
    /// # Errors
    ///
    /// Why this copy doesn't update itself.
    pub(crate) fn for_this_app(files: Option<&AppFiles>) -> Result<Self, Unavailable> {
        let platform = this_platform().ok_or(Unavailable::ByHand)?;
        let running = Running::this();
        let build = running.build.ok_or(Unavailable::DevelopmentBuild)?;
        let data = files.ok_or(Unavailable::NoFiles)?.dir().to_path_buf();
        let staging = crate::config::cache_dir()
            .ok_or(Unavailable::NoFiles)?
            .join("updates");
        let os = system::os_version().ok_or_else(|| {
            Unavailable::Unreadable(format!("the {} version", system::system_name(&platform)))
        })?;
        let installer = installer_for_this_app(&staging, &data)?;
        Ok(Self {
            running,
            build,
            here: ThisMachine {
                platform,
                arch: system::arch(),
                os,
            },
            keys: keys::TRUSTED,
            source: Arc::new(github::Github::new()),
            staging,
            data,
            installer,
        })
    }

    /// This copy.
    #[must_use]
    pub(crate) fn running(&self) -> Running {
        self.running
    }

    /// Where the record is kept.
    #[must_use]
    pub(crate) fn record_path(&self) -> PathBuf {
        self.data.join(record::FILE)
    }

    /// Where the marker is kept.
    #[must_use]
    pub(crate) fn marker_path(&self) -> PathBuf {
        self.data.join(marker::FILE)
    }

    /// What the marker says of the last update, for this launch: the update that didn't
    /// start, when this copy is the build it rolled back to.
    #[must_use]
    pub(crate) fn rolled_back(&self) -> Option<String> {
        let found = marker::Marker::read(&self.marker_path())?;
        (found.state == marker::State::RolledBack && found.from == self.build).then(|| {
            format!(
                "The update to Emberhut {} didn't start; Emberhut stays at {}",
                found.to_version, found.from_version
            )
        })
    }

    /// Swaps the staged build in, handing back the watch to run.
    ///
    /// # Errors
    ///
    /// The swap failed: the running app is as it was.
    pub(crate) fn swap(&self, staged: &Staged) -> Result<Box<dyn Watch>, InstallError> {
        self.installer.swap(staged, self.running)
    }

    /// The staging folder.
    fn staging(&self) -> &Path {
        &self.staging
    }

    /// The name build `build`'s payload is downloaded under.
    fn payload_name(&self, build: Build) -> String {
        let extension = if self.here.platform == Platform::Windows {
            "msi"
        } else {
            "dmg"
        };
        format!("Emberhut-build-{build}.{extension}")
    }
}

/// The system this copy updates on, if it updates itself on it.
fn this_platform() -> Option<Platform> {
    if cfg!(target_os = "macos") {
        Some(Platform::Macos)
    } else if cfg!(windows) {
        Some(Platform::Windows)
    } else {
        None
    }
}

#[cfg(unix)]
fn installer_for_this_app(staging: &Path, data: &Path) -> Result<Arc<dyn Installer>, Unavailable> {
    Ok(Arc::new(install::MacInstaller::for_running_app(
        staging, data,
    )?))
}

#[cfg(windows)]
fn installer_for_this_app(staging: &Path, _data: &Path) -> Result<Arc<dyn Installer>, Unavailable> {
    Ok(Arc::new(msi::MsiInstaller::for_running_app(staging)?))
}

#[cfg(not(any(unix, windows)))]
fn installer_for_this_app(
    _staging: &Path,
    _data: &Path,
) -> Result<Arc<dyn Installer>, Unavailable> {
    Err(Unavailable::ByHand)
}

#[cfg(test)]
pub(crate) mod tests {
    pub(crate) use super::signature::tests::TestKey;
    use super::*;

    impl Engine {
        /// An updater of `version` (`build`) on an arm64 Mac with macOS 14.5, trusting `keys`,
        /// reading releases from `source`, keeping its files in `dir`, installing with
        /// `installer`.
        pub(crate) fn for_tests(
            (version, build): (&'static str, u64),
            keys: &'static [TrustedKey],
            source: Arc<dyn Source>,
            dir: &Path,
            installer: Arc<dyn Installer>,
        ) -> Self {
            Self {
                running: Running::from_parts(version, Some(&build.to_string())),
                build: Build::new(build),
                here: ThisMachine {
                    platform: Platform::Macos,
                    arch: emberhut_api::releases::Arch::Arm64,
                    os: system::OsVersion::parse("14.5").unwrap(),
                },
                keys,
                source,
                staging: dir.join("staging"),
                data: dir.to_path_buf(),
                installer,
            }
        }
    }

    #[test]
    fn only_a_release_build_on_macos_or_windows_has_an_updater() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        let unavailable = Engine::for_this_app(Some(&files)).unwrap_err();
        if cfg!(any(target_os = "macos", windows)) {
            assert_ne!(unavailable, Unavailable::ByHand);
        } else {
            assert_eq!(unavailable, Unavailable::ByHand);
        }
        assert!(!forward_reopen(), "nothing is watched");
    }
}
