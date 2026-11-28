//! Files in and out: a session's inbox, and what may leave the machine.
//!
//! The server pushes files into a session's inbox (a design, a spreadsheet) and may pull a
//! session's results. A file may leave only from a session's own folders (where it was asked to
//! start, its worktree, its inbox) and the temporary folder; never keys, logins or the runner's
//! own data. Links are resolved first, `..` is refused, and the size is capped both ways. Every
//! transfer and every refusal is kept in the store and logged.

use std::collections::HashSet;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use axum::body::Bytes;
use futures_util::{Stream, StreamExt as _};
use regex::Regex;
use serde::Serialize;
use tokio::io::AsyncWriteExt as _;

use crate::store::{Store, now, session, transfer};
use crate::text::{clip, mb};
use crate::wire::Refused;

/// Folders that never leave the machine, even inside a project.
static SECRET_DIRS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(^|[/\\])(\.ssh|\.gnupg|\.aws|\.kube|\.docker|\.password-store|Keychains)([/\\]|$)",
    )
    .ok()
});

/// Files that never leave the machine: environment files, logins, keys and certificates.
static SECRET_FILES: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(^|[/\\])(\.env(\.[A-Za-z0-9_.-]*)?|\.netrc|\.npmrc|\.pypirc|\.credentials\.json|id_(rsa|ed25519|ecdsa|dsa)(\.pub)?|[^/\\]+\.(pem|p12|pfx|key|mobileprovision))$",
    )
    .ok()
});

/// Every session, when the folders of all of them count.
const EVERY_SESSION: u64 = 1 << 32;

/// Who asked, when the request doesn't say.
const DEFAULT_BY: &str = "server";

/// A file that may leave the machine, as it is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    /// Its real path (links resolved).
    pub path: String,
    /// Its size in bytes.
    pub size: u64,
    /// When it last changed (RFC 3339, UTC, milliseconds).
    pub mtime: String,
}

/// A file that landed in a session's inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Received {
    /// Where it landed.
    pub path: String,
    /// Its size in bytes.
    pub size: u64,
    /// The session's inbox.
    pub inbox: String,
    /// This runner's name.
    pub machine: String,
}

/// A file that arrived for a session since it was last told.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Arrived {
    /// Where it landed.
    pub path: String,
    /// Its size in bytes.
    pub size: u64,
    /// Who sent it.
    pub by: String,
}

/// The file rules of one runner.
#[derive(Debug, Clone)]
pub struct Files {
    store: Store,
    home: PathBuf,
    machine: String,
    max: u64,
    tmp: PathBuf,
}

/// The real path of `p` (links resolved), when something is there.
async fn real(p: &Path) -> Option<PathBuf> {
    tokio::fs::canonicalize(p).await.ok()
}

/// Whether the regular expression matches; a missing one matches everything, so a check that
/// can't run refuses.
fn hits(re: Option<&Regex>, s: &str) -> bool {
    re.is_none_or(|re| re.is_match(s))
}

/// Who asked, as recorded: `server` when nobody is named, 40 characters at most.
fn by_of(by: &str) -> String {
    clip(if by.is_empty() { DEFAULT_BY } else { by }, 40)
}

/// A file name that is safe in an inbox: its last part only, letters, digits, `_`, `.`, `-` and
/// spaces (anything else becomes `_`), no leading dots or spaces, its last 120 characters.
fn safe_name(raw: &str) -> String {
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let last = raw
        .trim_end_matches(seps)
        .rsplit(seps)
        .next()
        .unwrap_or_default();
    let mut name = String::with_capacity(last.len());
    let mut in_run = false;
    for c in last.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ' ') {
            name.push(c);
            in_run = false;
        } else if !in_run {
            name.push('_');
            in_run = true;
        }
    }
    let name = name.trim_start_matches(['.', ' ']);
    // Only ASCII is left, so bytes are characters.
    let name = &name[name.len().saturating_sub(120)..];
    if name.is_empty() {
        "file".to_owned()
    } else {
        name.to_owned()
    }
}

/// A size as the store keeps it.
fn stored_size(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

impl Files {
    /// The file rules for a runner with this home (its own data), name, size limit in bytes and
    /// temporary folder files may come from.
    #[must_use]
    pub fn new(store: Store, home: PathBuf, machine: String, max: u64, tmp: PathBuf) -> Self {
        Self {
            store,
            home,
            machine,
            max,
            tmp,
        }
    }

    /// A session's inbox, where files sent to it land.
    #[must_use]
    pub fn inbox_of(&self, session: &str) -> PathBuf {
        self.home.join("inbox").join(session)
    }

    /// Whether a real path is private: a key or login folder or file, or the runner's own data
    /// (but the inboxes).
    async fn secret(&self, r: &Path) -> bool {
        let s = r.to_string_lossy();
        if hits(SECRET_DIRS.as_ref(), &s) || hits(SECRET_FILES.as_ref(), &s) {
            return true;
        }
        let Some(own) = real(&self.home).await else {
            return false;
        };
        let inside = |root: &Path| r != root && r.starts_with(root);
        let inbox = real(&self.home.join("inbox")).await;
        inside(&own) && !inbox.as_deref().is_some_and(inside)
    }

    /// The folders a path may be in: the temporary folder and the session's (or every
    /// session's), never a file system's root or the user's home folder itself.
    async fn roots(&self, s: Option<&session::Model>) -> Result<Vec<PathBuf>, Refused> {
        let rows = match s {
            Some(s) => vec![s.clone()],
            None => self
                .store
                .sessions(EVERY_SESSION)
                .await
                .map_err(|e| Refused::new(500, e.to_string()))?,
        };
        let mut candidates = vec![self.tmp.clone()];
        for x in &rows {
            candidates.push(PathBuf::from(&x.cwd));
            candidates.extend(x.origin.iter().chain(&x.worktree).map(PathBuf::from));
            candidates.push(self.inbox_of(&x.id));
        }
        let user = match crate::config::user_home(&|k| std::env::var(k).ok()) {
            Some(h) => real(&h).await,
            None => None,
        };
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for c in candidates {
            let Some(r) = real(&c).await else { continue };
            if r.parent().is_some() && Some(&r) != user.as_ref() && seen.insert(r.clone()) {
                out.push(r);
            }
        }
        Ok(out)
    }

    /// Checks a path that may leave the machine: absolute, no `..`, a file in the allowed
    /// folders (`session`'s, or every session's), not private, not over the limit.
    ///
    /// # Errors
    /// 403 for a path that isn't allowed, 404 for no file there, 413 for one over the limit.
    pub async fn check(
        &self,
        raw: &str,
        session: Option<&session::Model>,
    ) -> Result<FileInfo, Refused> {
        let trimmed = raw.trim();
        let p = match (
            trimmed.strip_prefix('~'),
            crate::config::user_home(&|k| std::env::var(k).ok()),
        ) {
            (Some(rest), Some(home)) if rest.starts_with(['/', '\\']) => {
                format!("{}{rest}", home.display())
            }
            _ => trimmed.to_owned(),
        };
        let b = p.as_bytes();
        let drive = b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'/' | b'\\');
        if !p.starts_with('/') && !drive {
            return Err(Refused::forbidden(format!(
                "not an absolute path: {}",
                clip(&p, 200)
            )));
        }
        if p.split(['/', '\\']).any(|part| part == "..") {
            return Err(Refused::forbidden(format!(
                "no .. in paths: {}",
                clip(&p, 200)
            )));
        }
        let no_file = || Refused::missing(format!("no file at {p} on {}", self.machine));
        let r = real(Path::new(&p)).await.ok_or_else(no_file)?;
        if !self.roots(session).await?.iter().any(|x| r.starts_with(x)) {
            let whose = session.map_or_else(
                || "the sessions' folders".to_owned(),
                |s| format!("session {}'s folders", s.id),
            );
            return Err(Refused::forbidden(format!(
                "{p} is outside {whose} and {} on {}",
                self.tmp.display(),
                self.machine
            )));
        }
        if self.secret(&r).await {
            return Err(Refused::forbidden(format!("{p} is private")));
        }
        let meta = tokio::fs::metadata(&r).await.map_err(|_| no_file())?;
        if !meta.is_file() {
            return Err(Refused::forbidden(format!("{p} is not a file")));
        }
        if meta.len() > self.max {
            return Err(Refused::new(
                413,
                format!("{p} is {}, over the {} limit", mb(meta.len()), mb(self.max)),
            ));
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| jiff::Timestamp::try_from(t).ok())
            .map_or_else(now, |t| t.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string());
        Ok(FileInfo {
            path: r.to_string_lossy().into_owned(),
            size: meta.len(),
            mtime,
        })
    }

    /// Keeps a transfer or a refusal in the store, and logs it.
    #[expect(
        clippy::too_many_arguments,
        reason = "one row of the transfers table, column by column"
    )]
    async fn record(
        &self,
        dir: &str,
        session: Option<&str>,
        path: &str,
        size: Option<u64>,
        by: &str,
        ok: bool,
        why: Option<&str>,
    ) {
        let size_words = size.map_or_else(String::new, |n| format!(" ({})", mb(n)));
        let session_words = session.map_or_else(String::new, |s| format!(" session {s}"));
        let why_words = why.map_or_else(String::new, |w| format!(": {w}"));
        let refused = if ok { "" } else { " refused" };
        tracing::info!("file {dir}{refused}: {path}{size_words}{session_words} by {by}{why_words}");
        let row = transfer::Model {
            id: 0,
            at: Some(now()),
            dir: Some(dir.to_owned()),
            session: session.map(str::to_owned),
            path: Some(path.to_owned()),
            size: size.map(stored_size),
            by: Some(by.to_owned()),
            ok: Some(i64::from(ok)),
            why: why.map(str::to_owned),
            told: 0,
        };
        if let Err(e) = self.store.record_transfer(row).await {
            tracing::warn!("a file transfer wasn't kept: {e}");
        }
    }

    /// [`Files::check`] for `GET /files?stat=1`, the refusal kept.
    ///
    /// # Errors
    /// As [`Files::check`].
    pub async fn stat(
        &self,
        raw: &str,
        session: Option<&session::Model>,
        by: &str,
    ) -> Result<FileInfo, Refused> {
        match self.check(raw, session).await {
            Ok(f) => Ok(f),
            Err(e) => Err(self.refused_out(raw, session, by, e).await),
        }
    }

    /// Keeps a refused `GET /files` and gives the refusal back (for the HTTP layer's own
    /// refusals, such as a session that isn't here).
    pub async fn refused_out(
        &self,
        raw: &str,
        session: Option<&session::Model>,
        by: &str,
        refused: Refused,
    ) -> Refused {
        let sid = session.map(|s| s.id.as_str());
        let path = clip(raw, 400);
        self.record(
            "out",
            sid,
            &path,
            None,
            &by_of(by),
            false,
            Some(&refused.message),
        )
        .await;
        refused
    }

    /// A file to send (`GET /files`), checked and opened; the transfer is kept as sent, a refusal
    /// as refused.
    ///
    /// # Errors
    /// As [`Files::check`], and 400 when the file can't be opened.
    pub async fn open_for_send(
        &self,
        raw: &str,
        session: Option<&session::Model>,
        by: &str,
    ) -> Result<(FileInfo, tokio::fs::File), Refused> {
        let f = self.stat(raw, session, by).await?;
        let file = match tokio::fs::File::open(&f.path).await {
            Ok(file) => file,
            Err(e) => {
                let e = Refused::bad(format!("{} can't be read: {e}", f.path));
                return Err(self.refused_out(raw, session, by, e).await);
            }
        };
        let sid = session.map(|s| s.id.as_str());
        self.record("out", sid, &f.path, Some(f.size), &by_of(by), true, None)
            .await;
        Ok((f, file))
    }

    /// [`Files::receive_sized`] without a size said beforehand.
    ///
    /// # Errors
    /// As [`Files::receive_sized`].
    pub async fn receive<S, E>(
        &self,
        session: &session::Model,
        name: &str,
        by: &str,
        body: S,
    ) -> Result<Received, Refused>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: Display,
    {
        self.receive_sized(session, name, by, None, body).await
    }

    /// A file into a session's inbox (`PUT /sessions/{id}/files`), under a free name: a name
    /// that is taken gets `-2`, `-3`… before its extension, and nothing is overwritten. `said` is
    /// the size the request gave (its length or `size`): over the limit, nothing is read. The
    /// limit holds while the body streams too; a file that doesn't arrive whole is removed.
    ///
    /// # Errors
    /// 413 over the limit; 400 when the body breaks off or the file can't be written.
    pub async fn receive_sized<S, E>(
        &self,
        session: &session::Model,
        name: &str,
        by: &str,
        said: Option<u64>,
        mut body: S,
    ) -> Result<Received, Refused>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
        E: Display,
    {
        let by = by_of(by);
        let name = safe_name(name);
        let dir = self.inbox_of(&session.id);
        let in_session = Some(session.id.as_str());
        let limit = mb(self.max);
        if let Some(declared) = said.filter(|n| *n > self.max) {
            let at = dir.join(&name).to_string_lossy().into_owned();
            let why = format!("over the {limit} limit");
            self.record(
                "in",
                in_session,
                &at,
                Some(declared),
                &by,
                false,
                Some(&why),
            )
            .await;
            return Err(Refused::new(
                413,
                format!("{name} is {}, over the {limit} limit", mb(declared)),
            ));
        }
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| Refused::bad(format!("the inbox can't be made: {e}")))?;
        let (stem, ext) = match name.rfind('.') {
            Some(dot) if dot > 0 => name.split_at(dot),
            _ => (name.as_str(), ""),
        };
        let mut n = 1_u64;
        let (path, mut file) = loop {
            let file_name = if n == 1 {
                name.clone()
            } else {
                format!("{stem}-{n}{ext}")
            };
            let path = dir.join(file_name);
            match tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .await
            {
                Ok(file) => break (path, file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => {
                    return Err(Refused::bad(format!("{name} can't be written: {e}")));
                }
            }
        };
        let shown = path.to_string_lossy().into_owned();
        let mut got = 0_u64;
        let failed = loop {
            let chunk = match body.next().await {
                None => break file.flush().await.err().map(|e| (e.to_string(), None)),
                Some(Err(e)) => break Some((e.to_string(), None)),
                Some(Ok(chunk)) => chunk,
            };
            got = got.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
            if got > self.max {
                let r = Refused::new(413, format!("{name} is over the {limit} limit"));
                break Some((r.message.clone(), Some(r)));
            }
            if let Err(e) = file.write_all(&chunk).await {
                break Some((e.to_string(), None));
            }
        };
        if let Some((why, refused)) = failed {
            drop(file);
            if let Err(e) = tokio::fs::remove_file(&path).await {
                tracing::warn!("{shown} wasn't removed: {e}");
            }
            self.record("in", in_session, &shown, Some(got), &by, false, Some(&why))
                .await;
            return Err(refused
                .unwrap_or_else(|| Refused::bad(format!("the file didn't arrive whole: {why}"))));
        }
        self.record("in", in_session, &shown, Some(got), &by, true, None)
            .await;
        Ok(Received {
            path: shown,
            size: got,
            inbox: dir.to_string_lossy().into_owned(),
            machine: self.machine.clone(),
        })
    }

    /// The files that arrived for a session since it was last told, marked told.
    pub async fn arrived(&self, session: &str) -> Vec<Arrived> {
        match self.store.take_arrived(session).await {
            Ok(rows) => rows
                .into_iter()
                .map(|r| Arrived {
                    path: r.path.unwrap_or_default(),
                    size: r.size.and_then(|n| u64::try_from(n).ok()).unwrap_or(0),
                    by: r.by.unwrap_or_default(),
                })
                .collect(),
            Err(e) => {
                tracing::warn!("the files that arrived for {session}: {e}");
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use futures_util::stream;
    use tokio::io::AsyncReadExt as _;

    use super::*;

    struct Setup {
        files: Files,
        store: Store,
        work: tempfile::TempDir,
        home: tempfile::TempDir,
        tmp: tempfile::TempDir,
    }

    async fn setup(max: u64) -> Setup {
        let store = Store::in_memory().await.unwrap();
        let (work, home, tmp) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let files = Files::new(
            store.clone(),
            home.path().to_path_buf(),
            "box".to_owned(),
            max,
            tmp.path().to_path_buf(),
        );
        Setup {
            files,
            store,
            work,
            home,
            tmp,
        }
    }

    fn a_session(id: &str, cwd: &Path) -> session::Model {
        session::Model {
            id: id.to_owned(),
            title: None,
            cwd: cwd.to_string_lossy().into_owned(),
            project: None,
            model: None,
            effort: None,
            account: None,
            claude: None,
            materialized: 0,
            status: "idle".to_owned(),
            worktree: None,
            branch: None,
            meta: None,
            seq: 0,
            resume_at: None,
            created: Some(now()),
            updated: Some(now()),
            background: 0,
            origin: None,
        }
    }

    fn body(parts: &[&str]) -> impl Stream<Item = Result<Bytes, Infallible>> + Unpin {
        let chunks: Vec<_> = parts
            .iter()
            .map(|p| Ok(Bytes::from(p.as_bytes().to_vec())))
            .collect();
        stream::iter(chunks)
    }

    fn real_of(p: &Path) -> String {
        std::fs::canonicalize(p)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[tokio::test]
    async fn a_file_lands_in_the_inbox_under_a_free_name_and_is_named_once() {
        let t = setup(1024).await;
        let s = a_session("r-00000001", t.work.path());
        let got = t
            .files
            .receive(&s, "plan.txt", "owner", body(&["the ", "plan"]))
            .await
            .unwrap();
        let inbox = t.files.inbox_of("r-00000001");
        assert_eq!(got.size, 8);
        assert_eq!(got.machine, "box");
        assert_eq!(got.inbox, inbox.to_string_lossy());
        assert_eq!(got.path, inbox.join("plan.txt").to_string_lossy());
        assert_eq!(std::fs::read_to_string(&got.path).unwrap(), "the plan");
        let again = t
            .files
            .receive(&s, "plan.txt", "", body(&["v2"]))
            .await
            .unwrap();
        assert_eq!(again.path, inbox.join("plan-2.txt").to_string_lossy());
        assert_eq!(
            std::fs::read_to_string(&got.path).unwrap(),
            "the plan",
            "nothing is overwritten"
        );
        let arrived = t.files.arrived("r-00000001").await;
        assert_eq!(
            arrived,
            [
                Arrived {
                    path: got.path,
                    size: 8,
                    by: "owner".to_owned()
                },
                Arrived {
                    path: again.path,
                    size: 2,
                    by: "server".to_owned()
                },
            ]
        );
        assert_eq!(t.files.arrived("r-00000001").await, []);
    }

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(safe_name("../../etc/pass wd?*.txt"), "pass wd_.txt");
        assert_eq!(safe_name("..hidden"), "hidden");
        assert_eq!(safe_name("a/"), "a");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name(".."), "file");
        assert_eq!(safe_name("résumé.pdf"), "r_sum_.pdf");
        let long = format!("{}.bin", "x".repeat(200));
        assert_eq!(safe_name(&long).len(), 120);
        assert!(
            Path::new(&safe_name(&long))
                .extension()
                .is_some_and(|e| e == "bin")
        );
    }

    #[tokio::test]
    async fn a_file_over_the_limit_is_refused_said_or_streamed_and_nothing_is_left() {
        let t = setup(4).await;
        let s = a_session("r-00000001", t.work.path());
        let e = t
            .files
            .receive_sized(&s, "big.bin", "owner", Some(10), body(&["0123456789"]))
            .await
            .unwrap_err();
        assert_eq!(e.status, 413);
        assert_eq!(e.message, "big.bin is 1 KB, over the 1 KB limit");
        assert!(!t.files.inbox_of("r-00000001").exists());
        let e = t
            .files
            .receive(&s, "big.bin", "owner", body(&["012", "345"]))
            .await
            .unwrap_err();
        assert_eq!(e.status, 413);
        assert_eq!(e.message, "big.bin is over the 1 KB limit");
        let inbox = t.files.inbox_of("r-00000001");
        assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 0);
        assert_eq!(t.files.arrived("r-00000001").await, []);
    }

    #[tokio::test]
    async fn a_body_that_breaks_off_leaves_no_file() {
        let t = setup(1024).await;
        let s = a_session("r-00000001", t.work.path());
        let parts: Vec<Result<Bytes, String>> =
            vec![Ok(Bytes::from_static(b"half")), Err("reset".to_owned())];
        let e = t
            .files
            .receive(&s, "a.txt", "owner", stream::iter(parts))
            .await
            .unwrap_err();
        assert_eq!(
            (e.status, e.message.as_str()),
            (400, "the file didn't arrive whole: reset")
        );
        assert!(!t.files.inbox_of("r-00000001").join("a.txt").exists());
    }

    #[tokio::test]
    async fn a_sessions_file_may_leave_and_says_what_it_is() {
        let t = setup(1024).await;
        let s = a_session("r-00000001", t.work.path());
        let out = t.work.path().join("out.txt");
        std::fs::write(&out, "ok").unwrap();
        let f = t
            .files
            .check(out.to_str().unwrap(), Some(&s))
            .await
            .unwrap();
        assert_eq!(f.path, real_of(&out));
        assert_eq!(f.size, 2);
        assert!(
            f.mtime.parse::<jiff::Timestamp>().is_ok() && f.mtime.ends_with('Z'),
            "{}",
            f.mtime
        );
        let (f, mut file) = t
            .files
            .open_for_send(&format!("  {}  ", out.display()), Some(&s), "owner")
            .await
            .unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).await.unwrap();
        assert_eq!((f.size, text.as_str()), (2, "ok"));
        // With no session named, every session's folders count.
        t.store.add_session(s).await.unwrap();
        assert!(t.files.check(out.to_str().unwrap(), None).await.is_ok());
        // The temporary folder, and a session's inbox, count too.
        let tmp = t.tmp.path().join("shot.png");
        std::fs::write(&tmp, "png").unwrap();
        assert!(t.files.check(tmp.to_str().unwrap(), None).await.is_ok());
    }

    #[tokio::test]
    async fn paths_that_may_not_leave_are_refused_with_their_reason() {
        let t = setup(4).await;
        let s = a_session("r-00000001", t.work.path());
        let work = t.work.path();
        std::fs::write(work.join(".env"), "SECRET=1").unwrap();
        std::fs::create_dir_all(work.join(".ssh")).unwrap();
        std::fs::write(work.join(".ssh/config"), "").unwrap();
        std::fs::write(work.join("id_ed25519"), "").unwrap();
        std::fs::write(work.join("server.KEY"), "").unwrap();
        std::fs::write(work.join("big.bin"), "0123456789").unwrap();
        std::fs::create_dir(work.join("dir")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x"), "").unwrap();
        let at = |rel: &str| work.join(rel).to_string_lossy().into_owned();
        let cases = [
            (
                "relative/x".to_owned(),
                403,
                "not an absolute path: relative/x".to_owned(),
            ),
            (
                format!("{}/../x", work.display()),
                403,
                format!("no .. in paths: {}/../x", work.display()),
            ),
            (at("nope"), 404, format!("no file at {} on box", at("nope"))),
            (at(".env"), 403, format!("{} is private", at(".env"))),
            (
                at(".ssh/config"),
                403,
                format!("{} is private", at(".ssh/config")),
            ),
            (
                at("id_ed25519"),
                403,
                format!("{} is private", at("id_ed25519")),
            ),
            (
                at("server.KEY"),
                403,
                format!("{} is private", at("server.KEY")),
            ),
            (at("dir"), 403, format!("{} is not a file", at("dir"))),
            (
                at("big.bin"),
                413,
                format!("{} is 1 KB, over the 1 KB limit", at("big.bin")),
            ),
        ];
        for (raw, status, words) in cases {
            let e = t.files.check(&raw, Some(&s)).await.unwrap_err();
            assert_eq!((e.status, e.message), (status, words), "{raw}");
        }
        let elsewhere = outside.path().join("x");
        let e = t
            .files
            .check(elsewhere.to_str().unwrap(), Some(&s))
            .await
            .unwrap_err();
        assert_eq!(
            e.message,
            format!(
                "{} is outside session r-00000001's folders and {} on box",
                elsewhere.display(),
                t.tmp.path().display()
            )
        );
        let e = t
            .files
            .stat(elsewhere.to_str().unwrap(), None, "owner")
            .await
            .unwrap_err();
        assert!(e.message.contains("outside the sessions' folders"), "{e}");
    }

    #[tokio::test]
    async fn the_runners_own_data_stays_but_its_inboxes_may_leave() {
        let t = setup(1024).await;
        // A session started in the folder that holds the runner's home.
        let s = a_session("r-00000001", t.home.path().parent().unwrap());
        std::fs::write(t.home.path().join("token"), "t").unwrap();
        let token = t.home.path().join("token");
        let e = t
            .files
            .check(token.to_str().unwrap(), Some(&s))
            .await
            .unwrap_err();
        assert_eq!(e.message, format!("{} is private", token.display()));
        let got = t
            .files
            .receive(&s, "in.txt", "owner", body(&["x"]))
            .await
            .unwrap();
        assert!(t.files.check(&got.path, Some(&s)).await.is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_is_followed_before_the_rules() {
        let t = setup(1024).await;
        let s = a_session("r-00000001", t.work.path());
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x"), "").unwrap();
        let link = t.work.path().join("link");
        std::os::unix::fs::symlink(outside.path().join("x"), &link).unwrap();
        let e = t
            .files
            .check(link.to_str().unwrap(), Some(&s))
            .await
            .unwrap_err();
        assert!(e.message.contains("is outside session"), "{e}");
    }
}
