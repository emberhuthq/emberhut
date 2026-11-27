//! The app updating itself (ADR 0005, "The update"): the release feed on the app's own server,
//! read at launch and every 4 hours; the newest newer build for this Mac chosen; its payload
//! downloaded under a hard cap and checked (size, SHA-256, the update signature with its
//! trusted comment, the bundle's code signature, team, notarization and `Info.plist`) before
//! anything of the running app is touched; then, on Restart to Update or after 10 idle minutes,
//! the atomic swap and the old copy watching the new one start, rolling back if it doesn't.
//!
//! Only a Mac release build updates itself. A development build (no `EMBERHUT_BUILD`), an app that
//! isn't signed with a Developer ID, and the app off macOS never check. No update key is
//! trusted yet ([`keys`]), so today every payload fails its signature check and nothing is
//! installed: the updater fails closed.
//!
//! The pure decisions ([`cadence`], [`choose`], [`trusted`], [`marker`], the record's waits,
//! the watch's verdict) are plain types with their own tests; the store
//! (`store/updates.rs`) holds what the views show and runs the checks on a background thread.

pub(crate) mod cadence;
mod check;
pub(crate) mod choose;
mod download;
mod files;
mod health;
#[cfg(unix)]
mod install;
pub(crate) mod installer;
mod keys;
pub(crate) mod marker;
pub(crate) mod platform;
pub(crate) mod record;
pub(crate) mod self_check;
mod signature;
pub(crate) mod system;
mod trusted;
pub(crate) mod version;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use emberhut_api::releases::Build;

pub(crate) use self::check::{CheckError, Earlier, Found, Ready};
pub(crate) use self::health::report_when_drawn;
use self::installer::{InstallError, Installer, Staged, Watch};
use self::keys::TrustedKey;
use self::system::ThisMac;
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
    /// Updates are for the Mac app.
    #[error("Updates come with the Mac app; here, install a new version by hand")]
    NotTheMacApp,
    /// A development build never updates itself.
    #[error("A development build doesn't update itself")]
    DevelopmentBuild,
    /// The app has no directory to keep its files in.
    #[error("Emberhut has nowhere to keep an update on this Mac")]
    NoFiles,
    /// It runs from a place it can't replace itself in.
    #[error("{0}")]
    Place(String),
    /// It isn't signed with a Developer ID (an ad-hoc signed app fails closed).
    #[error("This copy of Emberhut isn't signed with a Developer ID, so it doesn't update itself")]
    NotDeveloperId,
    /// Something it needs to know couldn't be read.
    #[error("Emberhut can't check for updates: couldn't read {0}")]
    Unreadable(String),
}

/// The updater of a Mac release build: what it is, where it keeps things, whom it trusts, and
/// how it installs.
#[derive(Debug)]
pub(crate) struct Engine {
    running: Running,
    build: Build,
    here: ThisMac,
    keys: &'static [TrustedKey],
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
        if !cfg!(target_os = "macos") {
            return Err(Unavailable::NotTheMacApp);
        }
        let running = Running::this();
        let build = running.build.ok_or(Unavailable::DevelopmentBuild)?;
        let data = files.ok_or(Unavailable::NoFiles)?.dir().to_path_buf();
        let staging = crate::config::cache_dir()
            .ok_or(Unavailable::NoFiles)?
            .join("updates");
        let os = system::os_version()
            .ok_or_else(|| Unavailable::Unreadable("the macOS version".to_owned()))?;
        let installer = mac_installer(&staging, &data)?;
        Ok(Self {
            running,
            build,
            here: ThisMac {
                arch: system::arch(),
                os,
            },
            keys: keys::TRUSTED,
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
}

#[cfg(unix)]
fn mac_installer(staging: &Path, data: &Path) -> Result<Arc<dyn Installer>, Unavailable> {
    Ok(Arc::new(install::MacInstaller::for_running_app(
        staging, data,
    )?))
}

#[cfg(not(unix))]
fn mac_installer(_staging: &Path, _data: &Path) -> Result<Arc<dyn Installer>, Unavailable> {
    Err(Unavailable::NotTheMacApp)
}

#[cfg(test)]
pub(crate) mod tests {
    pub(crate) use super::signature::tests::TestKey;
    use super::*;

    impl Engine {
        /// An updater of build `build` on an arm64 Mac with macOS 14.5, trusting `keys`,
        /// keeping its files in `dir`, installing with `installer`.
        pub(crate) fn for_tests(
            build: u64,
            keys: &'static [TrustedKey],
            dir: &Path,
            installer: Arc<dyn Installer>,
        ) -> Self {
            Self {
                running: Running::from_parts("0.2.0", Some(&build.to_string())),
                build: Build::new(build),
                here: ThisMac {
                    arch: emberhut_api::releases::Arch::Arm64,
                    os: system::OsVersion::parse("14.5").unwrap(),
                },
                keys,
                staging: dir.join("staging"),
                data: dir.to_path_buf(),
                installer,
            }
        }
    }

    #[test]
    fn only_a_mac_release_build_has_an_updater() {
        let dir = tempfile::tempdir().unwrap();
        let files = AppFiles::in_dir(dir.path());
        let unavailable = Engine::for_this_app(Some(&files)).unwrap_err();
        if cfg!(target_os = "macos") {
            assert_ne!(unavailable, Unavailable::NotTheMacApp);
        } else {
            assert_eq!(unavailable, Unavailable::NotTheMacApp);
        }
        assert!(!forward_reopen(), "nothing is watched");
    }
}
