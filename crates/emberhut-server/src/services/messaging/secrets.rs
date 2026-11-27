//! The secrets directory, as messaging reads it: the owner's tokens for Slack and Gmail, the
//! speech-to-text key and the WhatsApp daemon's token, each a file named for what it holds,
//! read fresh at every use (one saved later is used from then on).

use std::path::{Path, PathBuf};

/// The secrets directory.
#[derive(Clone, Debug)]
pub struct Secrets {
    dir: PathBuf,
}

impl Secrets {
    /// The secrets in `dir`.
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The secret `name`, without surrounding whitespace; empty when there is none (or it
    /// can't be read).
    pub async fn read(&self, name: &str) -> String {
        if !is_name(name) {
            return String::new();
        }
        tokio::fs::read_to_string(self.dir.join(name))
            .await
            .map_or_else(|_| String::new(), |value| value.trim().to_owned())
    }

    /// Writes `value` as the secret `name`, making the directory (for the server's user only)
    /// when it isn't there; a new file is readable by the server's user only.
    ///
    /// # Errors
    ///
    /// The I/O error, or [`std::io::ErrorKind::InvalidInput`] for a name that isn't one.
    pub async fn write(&self, name: &str, value: &str) -> std::io::Result<()> {
        if !is_name(name) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a secret's name",
            ));
        }
        let mut builder = tokio::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(&self.dir).await?;
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(self.dir.join(name)).await?;
        tokio::io::AsyncWriteExt::write_all(&mut file, value.as_bytes()).await?;
        file.sync_all().await
    }

    /// Removes the secret `name`; whether it was there.
    ///
    /// # Errors
    ///
    /// The I/O error, other than its not being there.
    pub async fn remove(&self, name: &str) -> std::io::Result<bool> {
        if !is_name(name) {
            return Ok(false);
        }
        match tokio::fs::remove_file(self.dir.join(name)).await {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// The names of every secret, in order.
    pub async fn names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let Ok(mut entries) = tokio::fs::read_dir(&self.dir).await else {
            return names;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
        names.sort();
        names
    }
}

/// Whether `name` is a secret's file name: lowercase letters, digits and dashes.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn secrets_are_written_read_listed_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Secrets::new(dir.path().join("secrets"));
        assert_eq!(secrets.read("slack-acme-user").await, "");
        assert_eq!(secrets.names().await.len(), 0);
        secrets
            .write("slack-acme-user", " xoxp-1 \n")
            .await
            .unwrap();
        assert_eq!(secrets.read("slack-acme-user").await, "xoxp-1");
        assert_eq!(secrets.names().await, ["slack-acme-user"]);
        assert!(secrets.write("../out", "x").await.is_err());
        assert_eq!(secrets.read("../out").await, "");
        assert!(secrets.remove("slack-acme-user").await.unwrap());
        assert!(!secrets.remove("slack-acme-user").await.unwrap());
    }
}
