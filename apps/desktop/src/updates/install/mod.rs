//! Installing a verified payload on macOS: the new bundle taken out of the disk image and
//! checked beside the installed one (step 4 of ADR 0005's checks), then the atomic swap and
//! the hand-over to the old copy, which watches the new one start (the watchdog).
//!
//! The steps run the system's own tools and calls. They build on every Unix so the Linux build
//! type-checks and tests them, but only a Mac release build ever runs them: elsewhere there is
//! no installer ([`super::Engine`] has none).

mod bundle;
mod codesign;
mod dmg;
mod plist;
mod swap;
mod tools;
pub(crate) mod watchdog;

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use emberhut_api::releases::{Build, Release};

use self::bundle::Installed;
use self::codesign::Team;
use self::watchdog::Watchdog;
use super::Unavailable;
use super::installer::{InstallError, Installer, Staged, Watch};
use super::marker::{self, Marker, State};
use super::version::Running;

/// The installer of a Developer ID signed `Emberhut.app`.
#[derive(Debug)]
pub(crate) struct MacInstaller {
    installed: Installed,
    team: Team,
    /// Where disk images are mounted.
    mounts: PathBuf,
    /// The marker's path.
    marker: PathBuf,
}

impl MacInstaller {
    /// The installer for the running app, mounting images in `staging` and keeping the marker
    /// in the app's directory `data`.
    ///
    /// # Errors
    ///
    /// Why this copy can't update itself.
    pub(crate) fn for_running_app(staging: &Path, data: &Path) -> Result<Self, Unavailable> {
        let exe = std::env::current_exe().map_err(|error| {
            Unavailable::Unreadable(format!("where Emberhut runs from: {error}"))
        })?;
        let installed = Installed::of(&exe).map_err(|why| Unavailable::Place(why.to_string()))?;
        installed
            .check_folder()
            .map_err(|why| Unavailable::Place(why.to_string()))?;
        let team = codesign::team_of(&installed.app)
            .map_err(|error| Unavailable::Unreadable(format!("this copy's signature: {error}")))?
            .ok_or(Unavailable::NotDeveloperId)?;
        Ok(Self {
            installed,
            team,
            mounts: staging.join("mnt"),
            marker: data.join(marker::FILE),
        })
    }
}

impl Installer for MacInstaller {
    fn stage(&self, payload: &Path, release: &Release) -> Result<Staged, InstallError> {
        let staged = self.installed.staged();
        remove_bundle(&staged).map_err(InstallError::Local)?;
        let wanted = Staged {
            build: release.build,
            version: release.version.clone(),
        };
        let checked = dmg::copy_out(payload, &self.mounts, &staged)
            .map_err(|error| match error {
                dmg::DmgError::NoBundle => InstallError::NoBundle,
                other => InstallError::Image(Box::new(other)),
            })
            .and_then(|()| self.verify(&staged, &wanted));
        refuse_on_error(&staged, checked)?;
        Ok(wanted)
    }

    fn swap(&self, staged: &Staged, running: Running) -> Result<Box<dyn Watch>, InstallError> {
        let Some(from) = running.build else {
            return Err(InstallError::NotStaged);
        };
        let (app, new, old) = (
            &self.installed.app,
            self.installed.staged(),
            self.installed.kept_old(),
        );
        if !new.is_dir() {
            return Err(InstallError::NotStaged);
        }
        // Checked again right before it is swapped in: the staged bundle may have waited hours
        // in a folder others can write to, and has never run, so the system doesn't guard it
        // yet. Whatever changed since it was staged is refused here.
        refuse_on_error(&new, self.verify(&new, staged))?;
        remove_bundle(&old).map_err(InstallError::Local)?;
        swap::exchange(app, &new).map_err(InstallError::Local)?;
        let mut marker = Marker {
            from,
            from_version: running.version.to_owned(),
            to: staged.build,
            to_version: staged.version.clone(),
            state: State::Pending,
            reason: None,
            at: 0,
            watchdog: std::process::id(),
            new_copy: None,
        };
        marker.set(State::Pending, SystemTime::now());
        let kept = std::fs::rename(&new, &old).and_then(|()| marker.write(&self.marker));
        if let Err(error) = kept {
            // Put the running copy's bundle back before saying no.
            let back = if old.is_dir() { &old } else { &new };
            if let Err(undo) = swap::exchange(app, back) {
                log::error!("the update's swap couldn't be undone: {undo}");
            }
            return Err(InstallError::Local(error));
        }
        Ok(Box::new(Watchdog::new(
            self.installed.clone(),
            self.marker.clone(),
            marker,
        )))
    }
}

impl MacInstaller {
    /// Checks the bundle at `app` as `wanted`: its signature whole and strict, this copy's team
    /// and notarization ([`codesign::check`]), and its `Info.plist` this app at `wanted`'s version
    /// and build.
    fn verify(&self, app: &Path, wanted: &Staged) -> Result<(), InstallError> {
        codesign::check(app, &self.team)
            .map_err(|error| InstallError::Signature(Box::new(error)))?;
        let info = plist::read(app).map_err(|error| InstallError::Bundle(Box::new(error)))?;
        plist::matches(&info, &wanted.version, wanted.build)
            .map_err(|error| InstallError::Bundle(Box::new(error)))
    }
}

/// `checked`, the bundle at `staged` removed when it failed.
fn refuse_on_error(staged: &Path, checked: Result<(), InstallError>) -> Result<(), InstallError> {
    if checked.is_err()
        && let Err(cleanup) = remove_bundle(staged)
    {
        log::error!("couldn't remove the refused bundle: {cleanup}");
    }
    checked
}

/// Removes the bundle at `path`, if there is one.
fn remove_bundle(path: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// What the copy of build `me` does once its first frame is drawn, as the marker in `data`
/// says ([`marker::on_first_frame`]): reports itself healthy, and tidies up when no watchdog
/// is left to.
pub(crate) fn report_first_frame(data: &Path, me: Build) {
    let path = data.join(marker::FILE);
    let Some(mut found) = Marker::read(&path) else {
        return;
    };
    let watchdog_runs = watchdog::runs(found.watchdog);
    let step = marker::on_first_frame(Some(&found), me, watchdog_runs);
    if matches!(
        step,
        marker::OnFirstFrame::Report | marker::OnFirstFrame::ReportAndTidy
    ) {
        found.set(State::Healthy, SystemTime::now());
        if let Err(error) = found.write(&path) {
            log::error!("couldn't report the update healthy: {error}");
            return;
        }
        log::info!("the update to build {me} started");
    }
    if matches!(
        step,
        marker::OnFirstFrame::ReportAndTidy | marker::OnFirstFrame::Tidy
    ) {
        tidy(&path);
    }
}

/// Removes the old bundle and the marker, the new build being kept.
fn tidy(marker: &Path) {
    let installed = std::env::current_exe()
        .map_err(|error| error.to_string())
        .and_then(|exe| Installed::of(&exe).map_err(|error| error.to_string()));
    match installed {
        Ok(installed) => {
            if let Err(error) = remove_bundle(&installed.kept_old()) {
                log::error!("couldn't remove the old bundle: {error}");
            }
        }
        Err(error) => log::warn!("no old bundle to remove: {error}"),
    }
    if let Err(error) = std::fs::remove_file(marker) {
        log::warn!("couldn't remove the update marker: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A staged bundle that no longer checks out when it is to be swapped in (here, unsigned) is
    /// refused and removed, and the installed app is left as it was.
    #[test]
    fn a_staged_bundle_is_checked_again_right_before_the_swap() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Emberhut.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("build"), "12").unwrap();
        let installed = Installed::of(&app.join("Contents/MacOS/emberhut-desktop")).unwrap();
        let staged = installed.staged();
        std::fs::create_dir_all(staged.join("Contents/MacOS")).unwrap();
        std::fs::write(staged.join("build"), "15").unwrap();
        let mac = MacInstaller {
            installed,
            team: Team::parse("ABCDE12345").unwrap(),
            mounts: dir.path().join("mnt"),
            marker: dir.path().join(marker::FILE),
        };
        let running = Running {
            version: "0.2.0",
            build: Some(Build::new(12)),
        };
        let wanted = Staged {
            build: Build::new(15),
            version: "0.3.0".into(),
        };
        let Err(error) = mac.swap(&wanted, running) else {
            panic!("an unsigned bundle was swapped in");
        };
        assert!(matches!(error, InstallError::Signature(_)), "{error:?}");
        assert!(!staged.exists(), "the refused bundle is removed");
        assert_eq!(std::fs::read_to_string(app.join("build")).unwrap(), "12");
        assert!(!dir.path().join(marker::FILE).exists(), "no swap recorded");
    }
}
