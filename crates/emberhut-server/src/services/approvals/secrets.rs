//! The secrets approvals read, each a file in the secrets directory read at every use: the
//! link key (`approval-link-key`, made on first use), an agent channel's bot token, a Slack
//! workspace's signing secret and an approver's webhook secret.

use std::io;
use std::path::PathBuf;

use emberhut_core::RandomSource;
use emberhut_core::approvals::link::LinkKey;
use emberhut_core::secret_link::SecretName;

/// The secret holding the key signed links are signed with.
pub const LINK_KEY: &str = "approval-link-key";

/// Why a secret could not be read or made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecretError {
    /// The file system failed.
    #[error("the secret {name} could not be read or written")]
    Io {
        /// The secret.
        name: String,
        /// Why not.
        #[source]
        source: io::Error,
    },
    /// The link key's file holds something that isn't one.
    #[error("the secret {LINK_KEY} is not a link key")]
    NotALinkKey,
    /// No randomness for a new key.
    #[error("no randomness for a new link key")]
    Random(#[from] emberhut_core::RandomError),
}

/// The secrets directory, as approvals read it.
#[derive(Clone, Debug)]
pub(crate) struct SecretFiles {
    dir: PathBuf,
}

impl SecretFiles {
    /// The secrets in `dir`.
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The secret `name`, without surrounding whitespace; none when there is none (or it is
    /// empty).
    pub(crate) async fn read(&self, name: &SecretName) -> Result<Option<String>, SecretError> {
        self.read_file(name.as_str()).await
    }

    /// The file `name` of the secrets directory: a [`SecretName`] or one of this module's
    /// constants, never a path.
    async fn read_file(&self, name: &str) -> Result<Option<String>, SecretError> {
        match tokio::fs::read_to_string(self.dir.join(name)).await {
            Ok(text) => Ok(Some(text.trim().to_owned()).filter(|text| !text.is_empty())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(SecretError::Io {
                name: name.to_owned(),
                source,
            }),
        }
    }

    /// The link key, if one was made.
    pub(crate) async fn link_key(&self) -> Result<Option<LinkKey>, SecretError> {
        self.read_file(LINK_KEY)
            .await?
            .map(|text| LinkKey::decode(&text).map_err(|_| SecretError::NotALinkKey))
            .transpose()
    }

    /// The link key, made now (32 random bytes, readable by the server's user only) when
    /// there is none yet, or its file is empty (a start that stopped half-way). The key is
    /// written whole to a file of its own and then given its name, so no one ever reads half
    /// a key; of two first uses at once, one names its file and both use that key.
    pub(crate) async fn link_key_or_new(
        &self,
        random: &dyn RandomSource,
    ) -> Result<LinkKey, SecretError> {
        if let Some(key) = self.link_key().await? {
            return Ok(key);
        }
        let key = LinkKey::generate(random)?;
        let dir = self.dir.clone();
        let encoded = key.encode();
        tokio::task::spawn_blocking(move || write_new(&dir, &encoded))
            .await
            .map_err(|error| SecretError::Io {
                name: LINK_KEY.to_owned(),
                source: io::Error::other(error),
            })?
            .map_err(|source| SecretError::Io {
                name: LINK_KEY.to_owned(),
                source,
            })?;
        self.link_key().await?.ok_or(SecretError::NotALinkKey)
    }
}

/// Writes `encoded` as the link key in `dir`: whole, to a file of its own (0600, synced), then
/// named [`LINK_KEY`] unless a key is there already. An empty file there is replaced.
fn write_new(dir: &std::path::Path, encoded: &str) -> io::Result<()> {
    use std::io::Write as _;
    let mut folder = std::fs::DirBuilder::new();
    folder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut folder, 0o700);
    folder.create(dir)?;
    let mut file = tempfile::Builder::new()
        .prefix(".approval-link-key")
        .tempfile_in(dir)?;
    file.write_all(encoded.as_bytes())?;
    file.as_file().sync_all()?;
    let target = dir.join(LINK_KEY);
    match file.persist_noclobber(&target) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            let held = std::fs::read_to_string(&target)?;
            if held.trim().is_empty() {
                error
                    .file
                    .persist(&target)
                    .map(drop)
                    .map_err(|error| error.error)
            } else {
                Ok(())
            }
        }
        Err(error) => Err(error.error),
    }
}

#[cfg(test)]
mod tests {
    use emberhut_core::OsRandom;

    use super::*;

    /// A start that stopped between making the key's file and writing it left it empty: the
    /// next use makes a key, rather than never making a link again.
    #[tokio::test]
    async fn an_empty_link_key_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("secrets");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(LINK_KEY), "").unwrap();
        let secrets = SecretFiles::new(folder.clone());
        let made = secrets.link_key_or_new(&OsRandom).await.unwrap();
        assert_eq!(
            secrets.link_key().await.unwrap().unwrap().encode(),
            made.encode()
        );
    }

    #[tokio::test]
    async fn the_link_key_is_made_once_readable_by_the_server_only() {
        assert!(SecretName::parse(LINK_KEY).is_ok());
        let dir = tempfile::tempdir().unwrap();
        let secrets = SecretFiles::new(dir.path().join("secrets"));
        assert!(secrets.link_key().await.unwrap().is_none());
        let made = secrets.link_key_or_new(&OsRandom).await.unwrap();
        let again = secrets.link_key_or_new(&OsRandom).await.unwrap();
        assert_eq!(made.encode(), again.encode());
        let leftovers = std::fs::read_dir(dir.path().join("secrets"))
            .unwrap()
            .count();
        assert_eq!(leftovers, 1, "no half-written file is left behind");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let permissions = std::fs::metadata(dir.path().join("secrets").join(LINK_KEY))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(permissions & 0o777, 0o600);
        }
    }
}
