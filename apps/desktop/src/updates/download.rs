//! Fetching a payload: into an owner-only folder in the app's cache, only with room for it
//! (three times its size: the image, the bundle copied out, a margin), cut off past
//! `min(size, 1 GiB)` (the feed's size, and a ceiling no feed can raise), and kept only with
//! exactly the feed's size and SHA-256. On any error the partial file goes.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use emberhut_api::releases::Asset;

use super::github::{FetchError, Source};
use super::trusted::hex;

/// The largest payload the app takes, whatever a feed says.
pub(crate) const CEILING: u64 = 1 << 30;

/// Why a payload wasn't kept.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DownloadError {
    /// The feed lists it larger than an update may be.
    #[error("the update is {size} bytes, more than an update may be")]
    TooLarge {
        /// Its size in the feed.
        size: u64,
    },
    /// This computer has no room for it.
    #[error("there isn't room for the update ({need} bytes free are needed)")]
    NoSpace {
        /// The free space needed.
        need: u64,
    },
    /// GitHub couldn't be asked, or refused.
    #[error(transparent)]
    Fetch(FetchError),
    /// It came larger than the feed said, or with another size.
    #[error("the update isn't the size its release listed")]
    WrongSize,
    /// Its bytes aren't the ones the feed names.
    #[error("the update isn't the file its release listed")]
    WrongHash,
    /// The folder or the file couldn't be written.
    #[error("couldn't save the update")]
    Local(#[source] io::Error),
}

impl DownloadError {
    /// Whether the payload itself failed a check (rather than the network or this computer).
    #[must_use]
    pub(crate) fn is_verification(&self) -> bool {
        matches!(
            self,
            Self::TooLarge { .. } | Self::WrongSize | Self::WrongHash
        )
    }
}

/// A payload on disk with the feed's size and SHA-256.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Payload {
    /// Where it is.
    pub(crate) path: PathBuf,
    /// Its SHA-256.
    pub(crate) sha256: [u8; 32],
}

/// The free space a payload of `size` bytes needs.
#[must_use]
pub(crate) fn space_needed(size: u64) -> u64 {
    size.saturating_mul(3)
}

/// Fetches `asset` from `source` into `dir` (made owner-only) as `name`, `free` bytes being
/// free there (`None`: unknown, not checked), telling `progress` how many bytes have come.
///
/// # Errors
///
/// What [`DownloadError`] lists.
pub(crate) fn fetch(
    source: &dyn Source,
    asset: &Asset,
    name: &str,
    (dir, free): (&Path, Option<u64>),
    progress: &dyn Fn(u64),
) -> Result<Payload, DownloadError> {
    if asset.size > CEILING {
        return Err(DownloadError::TooLarge { size: asset.size });
    }
    let need = space_needed(asset.size);
    if free.is_some_and(|free| free < need) {
        return Err(DownloadError::NoSpace { need });
    }
    private_dir(dir).map_err(DownloadError::Local)?;
    let path = dir.join(name);
    let partial = dir.join(format!("{name}.part"));
    let fetched = fetch_into(source, asset, &partial, progress).and_then(|sha256| {
        std::fs::rename(&partial, &path)
            .map(|()| sha256)
            .map_err(DownloadError::Local)
    });
    match fetched {
        Ok(sha256) => Ok(Payload { path, sha256 }),
        Err(error) => {
            remove(&partial);
            Err(error)
        }
    }
}

fn fetch_into(
    source: &dyn Source,
    asset: &Asset,
    partial: &Path,
    progress: &dyn Fn(u64),
) -> Result<[u8; 32], DownloadError> {
    let mut file = owner_only_file(partial).map_err(DownloadError::Local)?;
    let got = source
        .download(&asset.url, &mut file, asset.size.min(CEILING), progress)
        .map_err(|error| match error {
            FetchError::TooLarge => DownloadError::WrongSize,
            other => DownloadError::Fetch(other),
        })?;
    file.sync_all().map_err(DownloadError::Local)?;
    if got.size != asset.size {
        return Err(DownloadError::WrongSize);
    }
    if !hex(&got.sha256).eq_ignore_ascii_case(asset.sha256.trim()) {
        return Err(DownloadError::WrongHash);
    }
    Ok(got.sha256)
}

/// Removes the file at `path`, if there is one.
pub(crate) fn remove(path: &Path) {
    if let Err(error) = std::fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        log::warn!("couldn't remove {}: {error}", path.display());
    }
}

/// Makes `dir` (and what leads to it), readable by its owner only.
fn private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    Ok(())
}

/// A new file at `path`, readable by its owner only, replacing one there.
fn owner_only_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// The free space where `dir` is (or will be), if the system says.
#[must_use]
pub(crate) fn free_space(dir: &Path) -> Option<u64> {
    let near = dir.ancestors().find(|place| place.exists())?;
    free_at(near)
}

#[cfg(unix)]
fn free_at(place: &Path) -> Option<u64> {
    rustix::fs::statvfs(place)
        .inspect_err(|error| log::warn!("can't read the free space: {error}"))
        .ok()
        .map(|stats| stats.f_bavail.saturating_mul(stats.f_frsize))
}

#[cfg(not(unix))]
fn free_at(_place: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use sha2::{Digest as _, Sha256};

    use super::*;
    use crate::updates::github::fake::FakeSource;

    const NAME: &str = "Emberhut-build-15.dmg";

    fn asset(bytes: &[u8]) -> Asset {
        Asset {
            arch: emberhut_api::releases::Arch::Arm64,
            url: "https://github.com/emberhuthq/emberhut/releases/download/v0.3.0/Emberhut.dmg"
                .into(),
            size: bytes.len() as u64,
            sha256: hex(&Sha256::digest(bytes).into()),
            signature: String::new(),
        }
    }

    #[test]
    fn a_payload_with_the_feeds_size_and_hash_is_kept_in_a_private_folder() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("updates");
        let source = FakeSource::default();
        source.push_payload(b"a disk image");
        let payload = fetch(
            &source,
            &asset(b"a disk image"),
            NAME,
            (&staging, None),
            &|_| {},
        )
        .unwrap();
        assert_eq!(std::fs::read(&payload.path).unwrap(), b"a disk image");
        assert_eq!(
            payload.sha256,
            <[u8; 32]>::from(Sha256::digest(b"a disk image"))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&staging), 0o700);
            assert_eq!(mode(&payload.path), 0o600);
        }
    }

    #[test]
    fn other_bytes_or_another_size_are_refused_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let source = FakeSource::default();
        source.push_payload(b"another image");
        let listed = asset(b"a disk image!");
        let error = fetch(&source, &listed, NAME, (dir.path(), None), &|_| {}).unwrap_err();
        assert!(matches!(error, DownloadError::WrongHash), "{error:?}");
        assert!(error.is_verification());
        source.push_payload(b"short");
        let error = fetch(&source, &listed, NAME, (dir.path(), None), &|_| {}).unwrap_err();
        assert!(matches!(error, DownloadError::WrongSize), "{error:?}");
        source.push_payload(b"a disk image!! and more");
        let error = fetch(&source, &listed, NAME, (dir.path(), None), &|_| {}).unwrap_err();
        assert!(
            matches!(error, DownloadError::WrongSize),
            "cut off: {error:?}"
        );
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(left.len(), 0, "nothing stays: {left:?}");
    }

    #[test]
    fn no_room_or_a_size_past_the_ceiling_downloads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let source = FakeSource::default();
        let listed = asset(b"a disk image");
        let error = fetch(&source, &listed, NAME, (dir.path(), Some(35)), &|_| {}).unwrap_err();
        assert!(
            matches!(error, DownloadError::NoSpace { need: 36 }),
            "{error:?}"
        );
        assert!(!error.is_verification());
        let mut huge = listed;
        huge.size = CEILING + 1;
        let error = fetch(&source, &huge, NAME, (dir.path(), None), &|_| {}).unwrap_err();
        assert!(matches!(error, DownloadError::TooLarge { .. }), "{error:?}");
        assert_eq!(source.asked(), [], "nothing asked of GitHub");
    }

    #[test]
    fn a_fetch_failure_isnt_the_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let source = FakeSource::default();
        let error = fetch(&source, &asset(b"x"), NAME, (dir.path(), None), &|_| {}).unwrap_err();
        assert!(matches!(error, DownloadError::Fetch(_)), "{error:?}");
        assert!(!error.is_verification());
    }

    #[cfg(unix)]
    #[test]
    fn the_free_space_is_read_where_the_folder_will_be() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(&dir.path().join("not/yet")).is_some_and(|free| free > 0));
    }
}
