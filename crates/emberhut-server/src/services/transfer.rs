//! Files between this server and its runners' machines, over the runners' own API:
//!
//! - [`TransferService::push`] puts a file of this machine into a runner session's inbox on its
//!   machine (`PUT /sessions/{id}/files`); the session hears of it on its next turn. Only files
//!   in the uploads folder, the home folder or `/tmp` go, never the server's secrets (its
//!   secrets folder, its database and journals, the machine's key, the configured token and key
//!   files) or the user's keys (`~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.config`), and 200 MB at
//!   most. What never goes is the server's one set ([`PrivatePaths`]), which `send_file`
//!   refuses too. Every folder and file is resolved to its real path once, when the server
//!   starts, and a file is compared by its real path, so a link never takes a private file
//!   out.
//! - [`TransferService::pull`] brings a file from a runner's machine (`GET /files`) into the
//!   uploads folder, in a folder of its own (`<machine>-<8 hex>/<name>`, served at
//!   `/uploads/<machine>-<8 hex>%2F<name>`), so it keeps its name. The same file, unchanged, is
//!   brought once (the `pulls` table).
//!
//! The runner decides what may leave its machine; the server logs every transfer it makes.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use emberhut_core::upload::url_of;
use emberhut_core::{Clock, RandomSource};
use emberhut_db::{DbError, PullStore, RemoteFile};
use serde_json::Value;

use crate::services::activity::ActivityLog;
use crate::services::private::{PrivatePaths, forms};
use crate::services::runner_api::{RunnerApi, RunnerError, Verb};
use crate::services::runners::{SessionId, form, text};

/// The largest file sent or brought, in bytes (the runners' own limit is the one that counts;
/// this one guards the server's disk).
pub const MAX_TRANSFER: u64 = 200 * 1024 * 1024;

/// Why a file wasn't sent or brought.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransferError {
    /// The file isn't one to send, or isn't there; in words.
    #[error("{0}")]
    Refused(String),
    /// The runner refused or didn't answer.
    #[error(transparent)]
    Runner(#[from] RunnerError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// Where a pushed file landed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pushed {
    /// Its path on the runner's machine.
    pub path: String,
    /// Its size, in bytes.
    pub size: u64,
    /// The machine.
    pub machine: String,
}

/// A file brought from a runner's machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pulled {
    /// Where it is here, relative to the uploads folder (`<machine>-<8 hex>/<name>`).
    pub local: String,
    /// Its name.
    pub name: String,
    /// Its size, in bytes.
    pub size: u64,
    /// Its path on the runner's machine.
    pub from: String,
}

impl Pulled {
    /// The address it is served at.
    #[must_use]
    pub fn url(&self) -> String {
        url_of(&self.local)
    }
}

/// The folders a pushed file may come from, and what it never may be.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roots {
    /// The uploads folder.
    pub uploads: PathBuf,
    /// The home folder of the server's user, if it has one.
    pub home: Option<PathBuf>,
    /// What never leaves the server, whichever folder it is in: the secrets folder, the
    /// database and its journals, the machine's key (see [`beside_database`]), the configured
    /// token and key files and the home's key folders. The chat's `send_file` refuses the same.
    pub private: PrivatePaths,
}

/// The folders of [`Roots`] resolved to their real paths, once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Resolved {
    /// Where a pushed file may come from, each as given and as its real path.
    allowed: Vec<PathBuf>,
    /// What a pushed file may never be, or be in.
    private: PrivatePaths,
}

/// The machine's key file beside the database (ADR 0007).
pub const MACHINE_KEY: &str = "machine.key";

/// The files the server keeps beside its database `database` that are never sent: the
/// database itself, its journals (`-wal`, `-shm`, `-journal`) and the machine's key
/// (`machine.key`, ADR 0007).
#[must_use]
pub fn beside_database(database: &Path) -> Vec<PathBuf> {
    if database.as_os_str().is_empty() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = ["", "-wal", "-shm", "-journal"]
        .iter()
        .map(|suffix| {
            let mut name = database.as_os_str().to_owned();
            name.push(suffix);
            PathBuf::from(name)
        })
        .collect();
    files.push(
        database
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(MACHINE_KEY),
    );
    files
}

impl Resolved {
    /// `roots`, resolved.
    fn of(roots: &Roots) -> Self {
        let mut allowed = vec![roots.uploads.clone(), PathBuf::from("/tmp")];
        if let Some(home) = &roots.home {
            allowed.push(home.clone());
        }
        Self {
            allowed: allowed.iter().flat_map(|path| forms(path)).collect(),
            private: roots.private.clone(),
        }
    }

    /// Whether the real path `real` may be sent.
    fn sendable(&self, real: &Path) -> bool {
        self.allowed.iter().any(|dir| real.starts_with(dir)) && !self.private.holds(real)
    }
}

/// The parts [`TransferService`] is built from.
#[derive(Clone, Debug)]
pub struct TransferParts {
    /// The runners.
    pub api: Arc<dyn RunnerApi>,
    /// What was brought already.
    pub pulls: PullStore,
    /// The activity log.
    pub activity: ActivityLog,
    /// Where files may come from and go.
    pub roots: Roots,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// The randomness of a brought file's folder name.
    pub random: Arc<dyn RandomSource>,
}

/// Sends and brings files.
#[derive(Clone, Debug)]
pub struct TransferService {
    parts: TransferParts,
    resolved: Arc<Resolved>,
}

/// `n` bytes in words: `1.5 MB`, or `12 KB` under a megabyte (1 KB at least).
#[must_use]
pub fn mb(n: u64) -> String {
    #[expect(clippy::cast_precision_loss, reason = "a size in words")]
    let n = n as f64;
    if n >= 1024.0 * 1024.0 {
        format!("{:.1} MB", n / 1024.0 / 1024.0)
    } else {
        emberhut_core::machine::num((n / 1024.0).round().max(1.0)) + " KB"
    }
}

/// `name` made safe to keep: only word characters, `.`, `-` and spaces (others `_`), its last
/// 120 characters, `file` when nothing is left.
#[must_use]
pub fn safe_name(name: &str) -> String {
    let mut safe = String::with_capacity(name.len());
    let mut replaced = false;
    for c in name.chars() {
        if c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | ' ') {
            safe.push(c);
            replaced = false;
        } else if !replaced {
            safe.push('_');
            replaced = true;
        }
    }
    let count = safe.chars().count();
    let safe: String = safe.chars().skip(count.saturating_sub(120)).collect();
    if safe.is_empty() || safe == "." || safe == ".." {
        "file".to_owned()
    } else {
        safe
    }
}

impl TransferService {
    /// The service over `parts`, its folders and files resolved to their real paths now.
    #[must_use]
    pub fn new(parts: TransferParts) -> Self {
        let resolved = Arc::new(Resolved::of(&parts.roots));
        Self { parts, resolved }
    }

    /// Puts the file at `path` (`~/` for the home folder) into the session `session`'s inbox on
    /// `machine`, sent `by` someone. Where it landed.
    ///
    /// # Errors
    ///
    /// [`TransferError::Refused`] for a path that isn't one to send (outside the uploads, home
    /// and `/tmp` folders, in a private folder, with `..`, not a file, over the limit);
    /// [`TransferError::Runner`].
    pub async fn push(
        &self,
        machine: &str,
        session: &SessionId,
        path: &str,
        by: &str,
    ) -> Result<Pushed, TransferError> {
        let real = self.sendable(path).await?;
        let meta = tokio::fs::metadata(&real)
            .await
            .map_err(|_| TransferError::Refused(format!("no file at {path}")))?;
        if !meta.is_file() {
            return Err(TransferError::Refused(format!("{path} is not a file")));
        }
        if meta.len() > MAX_TRANSFER {
            return Err(TransferError::Refused(format!(
                "{path} is {}, over the {} limit",
                mb(meta.len()),
                mb(MAX_TRANSFER)
            )));
        }
        let file_name = real
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file");
        let name = without_upload_prefix(file_name);
        let size = meta.len().to_string();
        let query = form(&[("name", name), ("by", by), ("size", &size)]);
        let answer = self
            .parts
            .api
            .upload(
                machine,
                &format!("/sessions/{session}/files?{query}"),
                &real,
            )
            .await?;
        let landed = text(&answer, "path").unwrap_or_else(|| "undefined".to_owned());
        let sent = answer.get("size").and_then(Value::as_u64).unwrap_or(0);
        let place = self.parts.api.place(machine);
        self.note(format!(
            "Sent {name} ({}) into session {session} on the {place}: {landed}, by {by}",
            mb(sent)
        ))
        .await;
        Ok(Pushed {
            path: landed,
            size: sent,
            machine: machine.to_owned(),
        })
    }

    /// The file `path` resolved, when it may be sent.
    async fn sendable(&self, path: &str) -> Result<PathBuf, TransferError> {
        let roots = &self.parts.roots;
        let trimmed = path.trim();
        let given = match (trimmed.strip_prefix("~/"), &roots.home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(trimmed),
        };
        if given.components().any(|part| part == Component::ParentDir) {
            return Err(TransferError::Refused("no .. in paths".to_owned()));
        }
        let real = tokio::fs::canonicalize(&given)
            .await
            .map_err(|_| TransferError::Refused(format!("no file at {}", given.display())))?;
        if !self.resolved.sendable(&real) {
            return Err(TransferError::Refused(format!(
                "{} isn't one to send (the uploads, home or /tmp only, never secrets)",
                given.display()
            )));
        }
        Ok(real)
    }

    /// Brings the file `path` from `machine` (looked for in the session `session`'s folders,
    /// when one is given) into the uploads folder, asked `by` someone `why`.
    ///
    /// # Errors
    ///
    /// [`TransferError::Runner`] when the runner refuses or doesn't answer;
    /// [`TransferError::Refused`] over the limit or when it didn't arrive whole.
    pub async fn pull(
        &self,
        machine: &str,
        path: &str,
        by: &str,
        why: &str,
        session: Option<&str>,
    ) -> Result<Pulled, TransferError> {
        let mut pairs = vec![("path", path), ("by", by)];
        if let Some(session) = session.filter(|session| !session.is_empty()) {
            pairs.push(("session", session));
        }
        let query = form(&pairs);
        let stat = self
            .parts
            .api
            .call(machine, Verb::Get, &format!("/files?{query}&stat=1"), None)
            .await?;
        let from = text(&stat, "path").unwrap_or_else(|| "undefined".to_owned());
        let size = stat.get("size").and_then(Value::as_u64).unwrap_or(0);
        let mtime = text(&stat, "mtime").unwrap_or_default();
        let base = from.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        let name = safe_name(base);
        let remote = RemoteFile {
            machine: machine.to_owned(),
            path: from.clone(),
            size: i64::try_from(size).unwrap_or(i64::MAX),
            mtime,
        };
        let uploads = &self.parts.roots.uploads;
        if let Some(local) = self.parts.pulls.find(&remote).await?
            && tokio::fs::metadata(uploads.join(&local))
                .await
                .is_ok_and(|meta| meta.is_file())
        {
            return Ok(Pulled {
                local,
                name,
                size,
                from,
            });
        }
        if size > MAX_TRANSFER {
            return Err(TransferError::Refused(format!(
                "{from} is {}, over the {} limit",
                mb(size),
                mb(MAX_TRANSFER)
            )));
        }
        let mut hex = [0_u8; 4];
        self.parts
            .random
            .fill(&mut hex)
            .map_err(|error| TransferError::Refused(error.to_string()))?;
        let [a, b, c, d] = hex;
        let folder = format!("{machine}-{a:02x}{b:02x}{c:02x}{d:02x}");
        let dir = uploads.join(&folder);
        tokio::fs::create_dir_all(&dir).await.map_err(|error| {
            TransferError::Refused(format!("{from} didn't arrive whole: {error}"))
        })?;
        let target = dir.join(&name);
        let got = match self
            .parts
            .api
            .download(machine, &format!("/files?{query}"), &target, MAX_TRANSFER)
            .await
        {
            Ok(got) => got,
            Err(error) => {
                if let Err(cleanup) = tokio::fs::remove_dir_all(&dir).await {
                    tracing::warn!(error = %cleanup, "a part of a brought file could not be removed");
                }
                return Err(TransferError::Refused(format!(
                    "{from} didn't arrive whole: {error}"
                )));
            }
        };
        let local = format!("{folder}/{name}");
        self.parts
            .pulls
            .record(&remote, &local, self.parts.clock.now())
            .await?;
        let place = self.parts.api.place(machine);
        let session = session
            .filter(|session| !session.is_empty())
            .map_or_default(|session| format!(" (session {session})"));
        self.note(format!(
            "Brought {name} ({}) from the {place}: {from}{session}, by {by} for {why}",
            mb(got)
        ))
        .await;
        Ok(Pulled {
            local,
            name,
            size: got,
            from,
        })
    }

    async fn note(&self, text: String) {
        if let Err(error) = self.parts.activity.note(text).await {
            tracing::warn!(%error, "an activity line could not be written");
        }
    }
}

/// A stored upload's name without its own 8-hex prefix.
fn without_upload_prefix(name: &str) -> &str {
    let bytes = name.as_bytes();
    if bytes.len() > 9
        && bytes[8] == b'-'
        && bytes[..8]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        &name[9..]
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_names_are_written_short() {
        assert_eq!(mb(3 * 1024 * 1024 / 2), "1.5 MB");
        assert_eq!(mb(10), "1 KB");
        assert_eq!(mb(12 * 1024), "12 KB");
        assert_eq!(safe_name("a/b?c d.png"), "a_b_c d.png");
        assert_eq!(safe_name(""), "file");
        assert_eq!(without_upload_prefix("1a2b3c4d-photo.png"), "photo.png");
        assert_eq!(without_upload_prefix("photo.png"), "photo.png");
    }

    #[test]
    fn the_database_its_journals_and_the_machine_key_are_kept_beside_it() {
        let files = beside_database(Path::new("/data/emberhut.db"));
        let files: Vec<&str> = files.iter().filter_map(|file| file.to_str()).collect();
        assert_eq!(
            files,
            [
                "/data/emberhut.db",
                "/data/emberhut.db-wal",
                "/data/emberhut.db-shm",
                "/data/emberhut.db-journal",
                "/data/machine.key"
            ]
        );
        assert_eq!(beside_database(Path::new("")), Vec::<PathBuf>::new());
    }
}
