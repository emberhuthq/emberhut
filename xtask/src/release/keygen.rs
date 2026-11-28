//! `release keygen`: a new update key, as a minisign key pair. The secret half is written
//! unencrypted (CI signs without a password prompt) to a file only its owner may read; it is
//! never printed.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, ensure};

use crate::release::feed::UpdateKey;

/// The two files a new key pair is written to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyFiles {
    /// The secret key, `<name>.key`, mode 0600.
    pub(crate) secret: PathBuf,
    /// The public key, `<name>.pub`, in minisign's `minisign.pub` format.
    pub(crate) public: PathBuf,
}

/// The files of the key pair `name` in `dir`.
#[must_use]
pub(crate) fn key_files(dir: &Path, name: &str) -> KeyFiles {
    KeyFiles {
        secret: dir.join(format!("{name}.key")),
        public: dir.join(format!("{name}.pub")),
    }
}

/// Writes `text` to a new file at `path` that only its owner can read and write.
fn write_private(path: &Path, text: &str) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("writing {}", path.display()))
}

/// Makes a key pair and writes it to `dir` as `name`, refusing to replace either file.
/// Returns the key and the files.
///
/// # Errors
///
/// Fails when a file exists already or can't be written.
pub(crate) fn keygen(dir: &Path, name: &str) -> anyhow::Result<(UpdateKey, KeyFiles)> {
    let files = key_files(dir, name);
    for path in [&files.secret, &files.public] {
        ensure!(
            !path.exists(),
            "{} exists; a new key never replaces one",
            path.display()
        );
    }
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let pair = minisign::KeyPair::generate_unencrypted_keypair().context("making a key pair")?;
    let secret = pair
        .sk
        .to_box(Some("Emberhut update key (minisign secret key)"))
        .context("encoding the secret key")?
        .to_string();
    let public = pair
        .pk
        .to_box()
        .context("encoding the public key")?
        .to_string();
    write_private(&files.secret, &secret)?;
    fs::write(&files.public, public)
        .with_context(|| format!("writing {}", files.public.display()))?;
    Ok((UpdateKey::from_pair(pair), files))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::keygen;
    use crate::release::feed::UpdateKey;

    #[test]
    fn a_new_key_reads_back_and_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let (key, files) = keygen(dir.path(), "emberhut-update").unwrap();
        assert!(files.secret.ends_with("emberhut-update.key"));
        let read = UpdateKey::read(&files.secret).unwrap();
        assert_eq!(read.public_base64(), key.public_base64());
        assert_eq!(read.id(), key.id());
        let public = fs::read_to_string(&files.public).unwrap();
        assert_eq!(public.lines().nth(1), Some(key.public_base64().as_str()));
        assert!(public.contains(&key.id()), "{public}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&files.secret).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let err = keygen(dir.path(), "emberhut-update").unwrap_err();
        assert!(err.to_string().contains("never replaces"), "{err}");
    }
}
