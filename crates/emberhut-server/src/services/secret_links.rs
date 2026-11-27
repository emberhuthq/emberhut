//! One-time secret links: a page where the person types the secrets the assistant asked for
//! (an API token, a password), so they land in the server's secrets directory and never in a
//! chat.
//!
//! The assistant makes a link for a few named secrets and a line saying why
//! ([`SecretLinkService::create`], the `secret_link` tool). The link is the key: it works
//! once, for a day. Sending its form writes each secret that was filled in to its own file in
//! the secrets directory (readable by the server's user only), uses the link up, and notes in
//! Main which secrets were saved (never their values).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use emberhut_core::message::{Author, NewMessage};
use emberhut_core::secret_link::{
    InvalidLinkRequest, LINK_LIFETIME, LinkToken, SecretLinkRecord, SecretName, link_names,
    looks_like_token,
};
use emberhut_core::{Clock, RandomError, RandomSource, RoomId, SecretHash};
use emberhut_db::{DbError, MessageStore, SecretLinkStore};
use jiff::Timestamp;

/// Why a link could not be made or used.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecretLinkError {
    /// The names asked for can't be a link's.
    #[error(transparent)]
    Invalid(#[from] InvalidLinkRequest),
    /// No token could be made.
    #[error("could not make a link")]
    Random(#[from] RandomError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
    /// A secret could not be written to its file.
    #[error("could not write the secret {name}")]
    Write {
        /// The secret's name.
        name: SecretName,
        /// Why not.
        #[source]
        source: std::io::Error,
    },
}

/// A link that was made: where to send the person, and until when it works.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedLink {
    /// The page's address.
    pub url: String,
    /// When it stops working.
    pub expires_at: Timestamp,
}

/// What sending a link's form did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Saved {
    /// The link had expired, was used already, or never existed.
    Gone,
    /// The link is used up; these secrets were filled in and written (maybe none).
    Saved(Vec<SecretName>),
}

/// What the service is built from.
#[derive(Debug)]
pub struct SecretLinkParts {
    /// The links.
    pub links: SecretLinkStore,
    /// Where the note of what was saved goes.
    pub messages: MessageStore,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// Where tokens get their randomness.
    pub random: Arc<dyn RandomSource>,
    /// The secrets directory.
    pub secrets_dir: PathBuf,
    /// The address the person's devices reach the server at, without a final `/`.
    pub public_url: String,
}

/// Makes secret links, shows them and saves what is typed into them.
#[derive(Clone, Debug)]
pub struct SecretLinkService {
    inner: Arc<SecretLinkParts>,
}

impl SecretLinkService {
    /// A service from its parts.
    #[must_use]
    pub fn new(parts: SecretLinkParts) -> Self {
        Self {
            inner: Arc::new(parts),
        }
    }

    /// The secrets directory.
    #[must_use]
    pub fn secrets_dir(&self) -> &Path {
        &self.inner.secrets_dir
    }

    /// A new link asking for `names`, because of `why`: the `secret_link` tool. Links that
    /// have expired are deleted on the way.
    ///
    /// # Errors
    ///
    /// [`SecretLinkError::Invalid`] for names a link can't ask for, [`SecretLinkError::Random`]
    /// or [`SecretLinkError::Database`].
    pub async fn create<S: AsRef<str>>(
        &self,
        names: &[S],
        why: &str,
    ) -> Result<IssuedLink, SecretLinkError> {
        let names = link_names(names)?;
        let now = self.inner.clock.now();
        self.inner.links.delete_expired(now).await?;
        let token = LinkToken::generate(self.inner.random.as_ref())?;
        let expires_at = now + LINK_LIFETIME;
        self.inner
            .links
            .insert(&token.hash(), &names, why, now, expires_at)
            .await?;
        tracing::info!(
            names = %joined(&names),
            "made a secret link"
        );
        Ok(IssuedLink {
            url: format!("{}/s/{}", self.inner.public_url, token.as_str()),
            expires_at,
        })
    }

    /// The link whose token this is, while it works.
    ///
    /// # Errors
    ///
    /// [`SecretLinkError::Database`].
    pub async fn open(&self, token: &str) -> Result<Option<SecretLinkRecord>, SecretLinkError> {
        if !looks_like_token(token) {
            return Ok(None);
        }
        let now = self.inner.clock.now();
        Ok(self
            .inner
            .links
            .find(&SecretHash::of(token))
            .await?
            .filter(|link| link.works_at(now)))
    }

    /// Saves what was typed into the link's form (`form`, its fields in order): each of the
    /// link's secrets whose field is not blank, trimmed, to its file. The link is used up
    /// first, so two sends of one form never both write.
    ///
    /// # Errors
    ///
    /// [`SecretLinkError::Database`], or [`SecretLinkError::Write`] when a file can't be
    /// written (the link is used up by then).
    pub async fn save(
        &self,
        token: &str,
        form: &[(String, String)],
    ) -> Result<Saved, SecretLinkError> {
        let Some(link) = self.open(token).await? else {
            return Ok(Saved::Gone);
        };
        if !self.inner.links.take(link.id).await? {
            return Ok(Saved::Gone);
        }
        let mut saved = Vec::new();
        for name in &link.names {
            let value = form
                .iter()
                .find(|(field, _)| field == name.as_str())
                .map(|(_, value)| value.trim())
                .filter(|value| !value.is_empty());
            if let Some(value) = value {
                write_secret(&self.inner.secrets_dir, name, value)
                    .await
                    .map_err(|source| SecretLinkError::Write {
                        name: name.clone(),
                        source,
                    })?;
                saved.push(name.clone());
            }
        }
        if !saved.is_empty() {
            let note = format!("Saved {} on the private page.", joined(&saved));
            tracing::info!(names = %joined(&saved), "secrets saved through a link");
            self.inner
                .messages
                .append(
                    NewMessage {
                        room: RoomId::MAIN,
                        author: Author::Event,
                        text: note,
                        reply_to: None,
                    },
                    self.inner.clock.now(),
                )
                .await?;
        }
        Ok(Saved::Saved(saved))
    }
}

/// The names, joined with ", ".
fn joined(names: &[SecretName]) -> String {
    names
        .iter()
        .map(SecretName::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Writes `value` to `name`'s file in `dir`, making the directory (for the server's user only)
/// when it isn't there. The value goes to a new temporary file beside it first (made with
/// `O_EXCL`, so a link already at that name is never followed, and readable by the server's
/// user only, whatever was there before), which is synced and then renamed over the name: the
/// file is replaced whole or not at all, and a link at the name is replaced, not written
/// through. A temporary name starts with a dot, which no secret's name has.
async fn write_secret(dir: &Path, name: &SecretName, value: &str) -> std::io::Result<()> {
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(dir).await?;
    let dir = dir.to_owned();
    let name = name.as_str().to_owned();
    let value = value.to_owned();
    tokio::task::spawn_blocking(move || {
        let prefix = format!(".{name}.");
        let mut temporary = tempfile::Builder::new();
        temporary.prefix(&prefix).suffix(".tmp");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            temporary.permissions(std::fs::Permissions::from_mode(0o600));
        }
        let mut file = temporary.tempfile_in(&dir)?;
        std::io::Write::write_all(&mut file, value.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(dir.join(&name)).map_err(|error| error.error)?;
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}
