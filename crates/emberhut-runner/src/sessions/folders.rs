//! Folders: where to start, what is in one, a repository's branches, worktrees.
//!
//! The owner's new-session sheet lists where sessions started lately and the folders inside one
//! (inside the user's home folder only), and a repository's branches; a session may get a git
//! worktree of its own. git runs as a command (`tokio::process`), each call with a time limit.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use crate::store::Store;
use crate::text::clip;
use crate::wire::Refused;

/// How long one git command may take.
const GIT_WAIT: Duration = Duration::from_secs(60);
/// How many sessions the recent folders are looked for in.
const RECENT_FROM: u64 = 200;
/// How many recent folders the sheet shows.
const RECENT: usize = 12;
/// How many folders a listing has at most.
const LISTED: usize = 300;
/// How many branches a repository's list has at most.
const BRANCHES: usize = 100;

/// A folder in a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    /// Its absolute path.
    pub path: String,
    /// Its own name.
    pub name: String,
    /// Whether it is a git repository's top folder.
    pub git: bool,
    /// When a session there last changed (recent folders only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<String>,
}

/// The user's home folder, from the environment.
fn user_home() -> Option<PathBuf> {
    crate::config::user_home(&|k| std::env::var(k).ok())
}

/// `p` made absolute against `base` (or the process's folder), with `.` and `..` worked out
/// without touching the disk.
fn resolve(base: &Path, p: &Path) -> PathBuf {
    let joined = base.join(p);
    let absolute = if joined.is_absolute() {
        joined
    } else {
        std::env::current_dir().unwrap_or_default().join(joined)
    };
    let mut out = PathBuf::new();
    for c in absolute.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// A path as given (`~` for the home folder), made absolute.
fn untilde_path(p: &str, home: Option<&Path>) -> PathBuf {
    let p = match home {
        Some(h) => crate::config::untilde(p, h),
        None => PathBuf::from(p),
    };
    resolve(Path::new(""), &p)
}

/// Whether something is at `p` (a link: whether what it points to is).
async fn exists(p: &Path) -> bool {
    tokio::fs::try_exists(p).await.unwrap_or(false)
}

/// Whether `d` is a git repository's top folder.
async fn is_repo(d: &Path) -> bool {
    exists(&d.join(".git")).await
}

/// A path's last part, as text.
fn name_of(p: &Path) -> String {
    p.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// Runs git in `cwd`; its output trimmed, or its words when it fails.
async fn git<I, S>(cwd: &Path, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = match tokio::time::timeout(GIT_WAIT, cmd.output()).await {
        Err(_) => return Err(format!("git took over {} s", GIT_WAIT.as_secs())),
        Ok(Err(e)) => return Err(format!("git didn't start: {e}")),
        Ok(Ok(out)) => out,
    };
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        let words = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if words.is_empty() {
            format!("git failed ({})", out.status)
        } else {
            clip(&words, 300)
        })
    }
}

/// The start of the sheet (`GET /folders`): where sessions here started lately (12, newest
/// first), and the documents folder (the home folder when there is none).
pub async fn start_folders(store: &Store, machine: &str, home: &Path) -> Value {
    let sessions = store.sessions(RECENT_FROM).await.unwrap_or_else(|e| {
        tracing::warn!("the recent folders: {e}");
        Vec::new()
    });
    let mut seen = HashSet::new();
    let mut recent = Vec::new();
    for s in sessions {
        let d = s.origin.unwrap_or(s.cwd);
        let path = PathBuf::from(&d);
        if seen.contains(&d) || !exists(&path).await {
            continue;
        }
        recent.push(Folder {
            name: name_of(&path),
            git: is_repo(&path).await,
            path: d.clone(),
            used: s.updated,
        });
        seen.insert(d);
        if recent.len() >= RECENT {
            break;
        }
    }
    let documents = home.join("Documents");
    let documents = if exists(&documents).await {
        documents
    } else {
        home.to_path_buf()
    };
    json!({
        "machine": machine,
        "home": home.to_string_lossy(),
        "recent": recent,
        "documents": documents.to_string_lossy(),
    })
}

/// The folders inside one (`GET /folders?path=`; `~` for the home folder): inside the home folder
/// only, no hidden ones and no dependency folders, by name, 300 at most.
///
/// # Errors
/// 404 when `path` isn't a folder inside the home folder; 400 when it can't be read.
pub async fn folders_in(path: &str, machine: &str, home: &Path) -> Result<Value, Refused> {
    let dir = untilde_path(path, Some(home));
    let is_dir = tokio::fs::metadata(&dir).await.is_ok_and(|m| m.is_dir());
    if !dir.starts_with(home) || !is_dir {
        return Err(Refused::missing(format!("no folder {path} on {machine}")));
    }
    let mut entries = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| Refused::bad(format!("can't read {path}: {e}")))?;
    let mut names = Vec::new();
    while let Ok(Some(e)) = entries.next_entry().await {
        let name = e.file_name().to_string_lossy().into_owned();
        let folder = e.file_type().await.is_ok_and(|t| t.is_dir());
        if folder && !name.starts_with('.') && name != "node_modules" {
            names.push(name);
        }
    }
    names.sort_by_cached_key(|n| (n.to_lowercase(), n.clone()));
    names.truncate(LISTED);
    let mut folders = Vec::with_capacity(names.len());
    for name in names {
        let p = dir.join(&name);
        folders.push(Folder {
            path: p.to_string_lossy().into_owned(),
            git: is_repo(&p).await,
            name,
            used: None,
        });
    }
    Ok(json!({
        "machine": machine,
        "home": home.to_string_lossy(),
        "path": dir.to_string_lossy(),
        "git": is_repo(&dir).await,
        "folders": folders,
    }))
}

/// A repository's branches (`GET /branches?cwd=`), local and remote, newest first (100 at
/// most), and the one it is on.
///
/// # Errors
/// 400 `not a git folder` when `cwd` isn't in a repository, or git's words when it fails.
pub async fn branches(cwd: &str) -> Result<Value, Refused> {
    let dir = untilde_path(
        if cwd.is_empty() { "." } else { cwd },
        user_home().as_deref(),
    );
    let top = git(&dir, ["rev-parse", "--show-toplevel"])
        .await
        .map_err(|_| Refused::bad("not a git folder"))?;
    let top_dir = Path::new(&top);
    let list = git(
        top_dir,
        [
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            "refs/heads",
            "refs/remotes/origin",
        ],
    )
    .await
    .map_err(Refused::bad)?;
    let mut seen = HashSet::new();
    let names: Vec<&str> = list
        .lines()
        .filter(|b| !b.is_empty() && *b != "origin/HEAD" && *b != "origin")
        .map(|b| b.strip_prefix("origin/").unwrap_or(b))
        .filter(|b| seen.insert(*b))
        .take(BRANCHES)
        .collect();
    let current = git(top_dir, ["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .map_err(Refused::bad)?;
    Ok(json!({ "repo": top, "current": current, "branches": names }))
}

/// What a session asked for as its worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeAsk {
    /// One on a new branch named after the session, from the repository's `HEAD`.
    Default,
    /// One as described.
    Spec {
        /// The repository, relative to the session's folder (`None`: the folder's own).
        repo: Option<String>,
        /// The branch (`None`: a new one named after the session).
        branch: Option<String>,
        /// The commit a new branch starts from (`None`: `HEAD`; given: always a new branch).
        base: Option<String>,
    },
}

impl WorktreeAsk {
    /// The ask in an open request's `worktree`: an object describes one, any other true value
    /// (`true`, a string) asks for the default; `None` for none (`null`, `false`, `0`, `""`).
    #[must_use]
    pub fn from_json(v: &Value) -> Option<Self> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        match v {
            Value::Null | Value::Bool(false) => None,
            Value::String(s) if s.is_empty() => None,
            Value::Number(n) if n.as_f64().is_some_and(|f| f.abs() < f64::EPSILON) => None,
            Value::Object(_) => Some(Self::Spec {
                repo: text("repo"),
                branch: text("branch"),
                base: text("base"),
            }),
            _ => Some(Self::Default),
        }
    }
}

/// A session's worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// Its folder.
    pub dir: PathBuf,
    /// Its branch.
    pub branch: String,
    /// The repository's top folder it is a worktree of.
    pub top: PathBuf,
}

/// `s` with every run of characters `keep` turns down replaced by `with`.
fn replace_runs(s: &str, keep: impl Fn(char) -> bool, with: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_run = false;
    for c in s.chars() {
        if keep(c) {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push_str(with);
            in_run = true;
        }
    }
    out
}

/// A word character of a branch name: ASCII letters, digits and `_`.
fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// A title as a branch name's part: lowercase letters and digits between dashes, 40 at most.
fn slug(title: &str) -> String {
    let lower = title.to_lowercase();
    let s = replace_runs(
        &lower,
        |c| c.is_ascii_lowercase() || c.is_ascii_digit(),
        "-",
    );
    let s = s.strip_prefix('-').unwrap_or(&s);
    let s = s.strip_suffix('-').unwrap_or(s);
    let s: String = s.chars().take(40).collect();
    if s.is_empty() {
        "session".to_owned()
    } else {
        s
    }
}

/// Whether git and a shell take `b` as a branch name as it is.
fn is_branch_name(b: &str) -> bool {
    !b.is_empty()
        && b.chars().all(|c| word(c) || matches!(c, '.' | '/' | '-'))
        && !b.contains("..")
        && !b.starts_with('-')
}

/// Whether `top`'s repository has `r` (a full ref name).
async fn has_ref(top: &Path, r: &str) -> bool {
    git(top, ["rev-parse", "--verify", "--quiet", r])
        .await
        .is_ok()
}

/// Four random hex digits, so two sessions with one title get two branches.
fn hex4() -> Result<String, Refused> {
    let mut b = [0u8; 2];
    getrandom::getrandom(&mut b)
        .map_err(|e| Refused::new(500, format!("no random numbers: {e}")))?;
    Ok(format!("{:02x}{:02x}", b[0], b[1]))
}

/// A git worktree for a session, under `<worktrees_dir>/<repo>/<branch>`: on a new branch
/// (`<prefix><title>-<4 hex>` when none is named), on a local branch that exists, or tracking the
/// remote's.
///
/// # Errors
/// 400 when `cwd` (or the asked repository) isn't in a repository, the branch name isn't one,
/// or git can't make the worktree (its words).
pub async fn make_worktree(
    cwd: &Path,
    ask: &WorktreeAsk,
    title: &str,
    worktrees_dir: &Path,
    branch_prefix: &str,
) -> Result<Worktree, Refused> {
    let (repo, named, base) = match ask {
        WorktreeAsk::Default => (None, None, None),
        WorktreeAsk::Spec { repo, branch, base } => {
            (repo.as_deref(), branch.as_deref(), base.as_deref())
        }
    };
    let start = resolve(cwd, Path::new(repo.unwrap_or(".")));
    let top = git(&start, ["rev-parse", "--show-toplevel"])
        .await
        .map_err(|_| {
            Refused::bad(format!(
                "{} is not in a git repository, so it can't have a worktree",
                cwd.display()
            ))
        })?;
    let top = PathBuf::from(top);
    let branch = match named.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => b.to_owned(),
        None => format!("{branch_prefix}{}-{}", slug(title), hex4()?),
    };
    if !is_branch_name(&branch) {
        return Err(Refused::bad(format!("not a branch name: {branch}")));
    }
    let unprefixed = if branch_prefix.is_empty() {
        &branch[..]
    } else {
        branch.strip_prefix(branch_prefix).unwrap_or(&branch)
    };
    let leaf = replace_runs(unprefixed, |c| word(c) || matches!(c, '.' | '-'), "-");
    let repo_name = top
        .file_name()
        .map_or_else(|| "repo".into(), |n| n.to_string_lossy());
    let first = worktrees_dir.join(&*repo_name).join(&leaf);
    let mut dir = first.clone();
    for i in 2.. {
        if !exists(&dir).await {
            break;
        }
        dir = first.with_file_name(format!("{leaf}-{i}"));
    }
    let d = dir.as_os_str();
    let added = if has_ref(&top, &format!("refs/heads/{branch}")).await {
        git(
            &top,
            [
                OsStr::new("worktree"),
                OsStr::new("add"),
                d,
                OsStr::new(&branch),
            ],
        )
        .await
    } else if base.is_none() && has_ref(&top, &format!("refs/remotes/origin/{branch}")).await {
        let remote = format!("origin/{branch}");
        let args = ["worktree", "add", "--track", "-b", &branch].map(OsStr::new);
        git(&top, args.into_iter().chain([d, OsStr::new(&remote)])).await
    } else {
        let from = base.unwrap_or("HEAD");
        let args = [OsStr::new("worktree"), OsStr::new("add"), d];
        git(
            &top,
            args.into_iter()
                .chain(["-b", &branch, from].map(OsStr::new)),
        )
        .await
    };
    added.map_err(|why| Refused::bad(format!("the worktree can't be made: {why}")))?;
    Ok(Worktree { dir, branch, top })
}

/// The instruction files the agent loads for a folder: its own and those of the folders above
/// it, inside the user's home folder, outermost first.
#[must_use]
pub fn instruction_files(dir: &Path) -> Vec<String> {
    user_home().map_or_else(Vec::new, |home| instruction_files_under(dir, &home))
}

/// [`instruction_files`] with the home folder given.
fn instruction_files_under(dir: &Path, home: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut d = resolve(Path::new(""), dir);
    while d.starts_with(home) && d != home {
        for f in [d.join("CLAUDE.md"), d.join(".claude").join("CLAUDE.md")] {
            if f.exists() {
                out.insert(0, f.to_string_lossy().into_owned());
            }
        }
        if !d.pop() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::store::{now, session};

    /// A repository at `dir` on `main` with one commit.
    fn a_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.name", "T"]);
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "commit.gpgsign", "false"]);
        run(&["commit", "-q", "--allow-empty", "-m", "first"]);
    }

    fn git_out(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn a_session(id: &str, cwd: &Path, origin: Option<&Path>, updated: &str) -> session::Model {
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
            updated: Some(updated.to_owned()),
            background: 0,
            origin: origin.map(|o| o.to_string_lossy().into_owned()),
        }
    }

    #[tokio::test]
    async fn the_start_lists_where_sessions_started_newest_first_once_each() {
        let home = tempfile::tempdir().unwrap();
        let repo = home.path().join("work/repo");
        a_repo(&repo);
        let plain = home.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let wt = home.path().join("worktrees/repo/x");
        let store = Store::in_memory().await.unwrap();
        let sessions = [
            a_session("r-00000001", &plain, None, "2026-11-01T00:00:00.000Z"),
            a_session("r-00000002", &wt, Some(&repo), "2026-11-03T00:00:00.000Z"),
            a_session("r-00000003", &repo, None, "2026-11-02T00:00:00.000Z"),
            a_session(
                "r-00000004",
                &home.path().join("gone"),
                None,
                "2026-11-04T00:00:00.000Z",
            ),
        ];
        for s in sessions {
            store.add_session(s).await.unwrap();
        }
        let v = start_folders(&store, "box", home.path()).await;
        assert_eq!(v["machine"], "box");
        assert_eq!(v["home"], home.path().to_string_lossy().as_ref());
        assert_eq!(v["documents"], v["home"], "no Documents: the home folder");
        let recent = v["recent"].as_array().unwrap();
        let paths: Vec<&str> = recent.iter().map(|f| f["path"].as_str().unwrap()).collect();
        assert_eq!(paths, [repo.to_str().unwrap(), plain.to_str().unwrap()]);
        assert_eq!(recent[0]["name"], "repo");
        assert_eq!(recent[0]["git"], true);
        assert_eq!(recent[0]["used"], "2026-11-03T00:00:00.000Z");
        assert_eq!(recent[1]["git"], false);
        std::fs::create_dir(home.path().join("Documents")).unwrap();
        let v = start_folders(&store, "box", home.path()).await;
        assert_eq!(
            v["documents"],
            home.path().join("Documents").to_string_lossy().as_ref()
        );
    }

    #[tokio::test]
    async fn a_folders_listing_has_its_visible_folders_by_name() {
        let home = tempfile::tempdir().unwrap();
        let docs = home.path().join("Documents");
        for d in ["notes", "Apps", "b", ".hidden", "node_modules"] {
            std::fs::create_dir_all(docs.join(d)).unwrap();
        }
        a_repo(&docs.join("repo"));
        std::fs::write(docs.join("file.txt"), "x").unwrap();
        let v = folders_in(docs.to_str().unwrap(), "box", home.path())
            .await
            .unwrap();
        assert_eq!(v["path"], docs.to_string_lossy().as_ref());
        assert_eq!(v["git"], false);
        let folders = v["folders"].as_array().unwrap();
        let names: Vec<&str> = folders
            .iter()
            .map(|f| f["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Apps", "b", "notes", "repo"]);
        assert_eq!(folders[3]["git"], true);
        assert!(folders[0].get("used").is_none());
        assert_eq!(
            folders[2]["path"],
            docs.join("notes").to_string_lossy().as_ref()
        );
        let v = folders_in("~/Documents/repo", "box", home.path())
            .await
            .unwrap();
        assert_eq!(v["git"], true);
    }

    #[tokio::test]
    async fn a_folder_outside_the_home_folder_or_missing_is_not_there() {
        let home = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        for p in [
            other.path().to_str().unwrap().to_owned(),
            home.path().join("nope").to_string_lossy().into_owned(),
            format!("{}/..", home.path().display()),
        ] {
            let e = folders_in(&p, "box", home.path()).await.unwrap_err();
            assert_eq!(e.status, 404, "{p}");
            assert_eq!(e.message, format!("no folder {p} on box"));
        }
        std::fs::write(home.path().join("f"), "").unwrap();
        let p = home.path().join("f");
        let e = folders_in(p.to_str().unwrap(), "box", home.path())
            .await
            .unwrap_err();
        assert_eq!(e.status, 404);
    }

    #[tokio::test]
    async fn a_repositorys_branches_and_the_one_it_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        a_repo(&repo);
        git_out(&repo, &["branch", "example-fix"]);
        std::fs::create_dir(repo.join("src")).unwrap();
        let v = branches(repo.join("src").to_str().unwrap()).await.unwrap();
        assert_eq!(v["current"], "main");
        assert_eq!(
            v["repo"],
            git_out(&repo, &["rev-parse", "--show-toplevel"]).as_str()
        );
        let mut names: Vec<&str> = v["branches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_str().unwrap())
            .collect();
        names.sort_unstable();
        assert_eq!(names, ["example-fix", "main"]);
        let e = branches(dir.path().to_str().unwrap()).await.unwrap_err();
        assert_eq!((e.status, e.message.as_str()), (400, "not a git folder"));
    }

    #[test]
    fn the_worktree_ask_reads_like_the_request_means_it() {
        assert_eq!(
            WorktreeAsk::from_json(&json!(true)),
            Some(WorktreeAsk::Default)
        );
        assert_eq!(
            WorktreeAsk::from_json(&json!("yes")),
            Some(WorktreeAsk::Default)
        );
        for none in [json!(null), json!(false), json!(""), json!(0)] {
            assert_eq!(WorktreeAsk::from_json(&none), None, "{none}");
        }
        assert_eq!(
            WorktreeAsk::from_json(&json!({"branch": "tidy", "base": "main", "repo": 3})),
            Some(WorktreeAsk::Spec {
                repo: None,
                branch: Some("tidy".to_owned()),
                base: Some("main".to_owned())
            })
        );
    }

    #[tokio::test]
    async fn a_worktree_on_a_named_new_branch() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        a_repo(&repo);
        let root = dir.path().join("worktrees");
        let ask = WorktreeAsk::Spec {
            repo: None,
            branch: Some(" tidy ".to_owned()),
            base: None,
        };
        let w = make_worktree(&repo, &ask, "Tidy", &root, "emberhut/")
            .await
            .unwrap();
        assert_eq!(w.branch, "tidy");
        assert_eq!(w.dir, root.join("repo").join("tidy"));
        assert!(w.dir.join(".git").exists());
        assert_eq!(git_out(&w.dir, &["branch", "--show-current"]), "tidy");
        assert_eq!(
            w.top,
            PathBuf::from(git_out(&repo, &["rev-parse", "--show-toplevel"]))
        );
    }

    #[tokio::test]
    async fn a_default_worktree_is_named_after_the_title_and_never_reuses_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        a_repo(&repo);
        let root = dir.path().join("worktrees");
        let w = make_worktree(
            &repo,
            &WorktreeAsk::Default,
            "Fix the Build!",
            &root,
            "emberhut/",
        )
        .await
        .unwrap();
        let hex = w.branch.strip_prefix("emberhut/fix-the-build-").unwrap();
        assert!(hex.len() == 4 && hex.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(
            w.dir,
            root.join("repo").join(format!("fix-the-build-{hex}"))
        );
        // The folder is taken: the next one gets a number.
        std::fs::create_dir_all(root.join("repo/again")).unwrap();
        let ask = WorktreeAsk::Spec {
            repo: None,
            branch: Some("again".to_owned()),
            base: Some("main".to_owned()),
        };
        let w = make_worktree(&repo, &ask, "", &root, "").await.unwrap();
        assert_eq!(w.dir, root.join("repo/again-2"));
    }

    #[tokio::test]
    async fn a_worktree_on_a_local_branch_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        a_repo(&repo);
        git_out(&repo, &["branch", "example-fix"]);
        let ask = WorktreeAsk::Spec {
            repo: None,
            branch: Some("example-fix".to_owned()),
            base: None,
        };
        let w = make_worktree(&repo, &ask, "x", &dir.path().join("w"), "emberhut/")
            .await
            .unwrap();
        assert_eq!(
            git_out(&w.dir, &["branch", "--show-current"]),
            "example-fix"
        );
    }

    #[tokio::test]
    async fn a_worktree_is_refused_outside_a_repository_or_for_a_bad_branch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("w");
        let e = make_worktree(dir.path(), &WorktreeAsk::Default, "x", &root, "p/")
            .await
            .unwrap_err();
        assert_eq!(e.status, 400);
        assert!(
            e.message
                .ends_with("is not in a git repository, so it can't have a worktree")
        );
        let repo = dir.path().join("repo");
        a_repo(&repo);
        for bad in ["-x", "a..b", "a b", "a;b"] {
            let ask = WorktreeAsk::Spec {
                repo: None,
                branch: Some(bad.to_owned()),
                base: None,
            };
            let e = make_worktree(&repo, &ask, "x", &root, "p/")
                .await
                .unwrap_err();
            assert_eq!(e.message, format!("not a branch name: {bad}"));
        }
        let ask = WorktreeAsk::Spec {
            repo: None,
            branch: None,
            base: Some("no-such-commit".to_owned()),
        };
        let e = make_worktree(&repo, &ask, "x", &root, "p/")
            .await
            .unwrap_err();
        assert!(e.message.starts_with("the worktree can't be made: "), "{e}");
    }

    #[test]
    fn slugs_and_branch_names() {
        assert_eq!(slug("  Fix: the build -- now "), "fix-the-build-now");
        assert_eq!(slug("!!!"), "session");
        assert_eq!(slug(&"a".repeat(50)).len(), 40);
        assert!(is_branch_name("emberhut/fix-1.2_x"));
        assert!(!is_branch_name(""));
    }

    #[test]
    fn instruction_files_come_from_the_folder_and_those_above_it_in_the_home_folder() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path();
        let app = h.join("work/app");
        std::fs::create_dir_all(app.join(".claude")).unwrap();
        std::fs::write(h.join("CLAUDE.md"), "").unwrap();
        std::fs::write(h.join("work/CLAUDE.md"), "").unwrap();
        std::fs::write(app.join("CLAUDE.md"), "").unwrap();
        std::fs::write(app.join(".claude/CLAUDE.md"), "").unwrap();
        let s = |p: PathBuf| p.to_string_lossy().into_owned();
        assert_eq!(
            instruction_files_under(&app, h),
            [
                s(h.join("work/CLAUDE.md")),
                s(app.join(".claude/CLAUDE.md")),
                s(app.join("CLAUDE.md")),
            ],
            "the home folder's own file is the agent's user file, not the project's"
        );
        assert_eq!(
            instruction_files_under(Path::new("/elsewhere"), h),
            Vec::<String>::new()
        );
    }

    #[test]
    fn resolve_works_out_dots_without_the_disk() {
        assert_eq!(
            resolve(Path::new("/a/b"), Path::new("../c/./d")),
            PathBuf::from("/a/c/d")
        );
        assert_eq!(
            resolve(Path::new("/a"), Path::new("/x/..")),
            PathBuf::from("/")
        );
    }
}
