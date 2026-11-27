//! Taking the new bundle out of the disk image: attached read-only at a random place, never
//! shown in the Finder or opened, the bundle copied out with `ditto`, and the image detached
//! again whatever happened.

use std::io;
use std::path::{Path, PathBuf};

use super::tools::{self, ToolError};

/// The `hdiutil` tool.
const HDIUTIL: &str = "/usr/bin/hdiutil";
/// The `ditto` tool.
const DITTO: &str = "/usr/bin/ditto";
/// The bundle's name in the image.
const BUNDLE: &str = "Emberhut.app";

/// Why the bundle couldn't be taken out.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DmgError {
    /// A tool failed.
    #[error(transparent)]
    Tool(#[from] ToolError),
    /// The mount folder couldn't be made or read.
    #[error("couldn't prepare a place to open the update")]
    Local(#[source] io::Error),
    /// The image didn't mount where it was asked to, or holds no `Emberhut.app`.
    #[error("the update holds no {BUNDLE}")]
    NoBundle,
}

/// Copies `Emberhut.app` out of the image at `dmg` to `to` (which must not exist), mounting it in a
/// random folder inside `mounts`.
///
/// # Errors
///
/// What [`DmgError`] lists; `to` is left out (or as `ditto` left it: the caller removes it).
pub(crate) fn copy_out(dmg: &Path, mounts: &Path, to: &Path) -> Result<(), DmgError> {
    std::fs::create_dir_all(mounts).map_err(DmgError::Local)?;
    let before = folders(mounts).map_err(DmgError::Local)?;
    tools::run(
        HDIUTIL,
        [
            "attach".as_ref(),
            "-nobrowse".as_ref(),
            "-readonly".as_ref(),
            "-noautoopen".as_ref(),
            "-mountrandom".as_ref(),
            mounts.as_os_str(),
            dmg.as_os_str(),
        ],
    )?;
    let mounted = folders(mounts)
        .map_err(DmgError::Local)?
        .into_iter()
        .find(|folder| !before.contains(folder));
    let Some(mount) = mounted else {
        return Err(DmgError::NoBundle);
    };
    let copied = copy_bundle(&mount, to);
    detach(&mount);
    copied
}

fn copy_bundle(mount: &Path, to: &Path) -> Result<(), DmgError> {
    let bundle = mount.join(BUNDLE);
    if !bundle.is_dir() {
        return Err(DmgError::NoBundle);
    }
    tools::run(DITTO, [bundle.as_os_str(), to.as_os_str()])?;
    Ok(())
}

/// Detaches the image at `mount`, forcing it if a plain detach fails.
fn detach(mount: &Path) {
    let plain = tools::run(HDIUTIL, ["detach".as_ref(), mount.as_os_str()]);
    if let Err(error) = plain {
        log::warn!("{error}; forcing the update's image off");
        if let Err(error) = tools::run(
            HDIUTIL,
            ["detach".as_ref(), "-force".as_ref(), mount.as_os_str()],
        ) {
            log::error!("the update's image stays attached: {error}");
        }
    }
}

/// The folders in `dir`.
fn folders(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            found.push(entry.path());
        }
    }
    Ok(found)
}
