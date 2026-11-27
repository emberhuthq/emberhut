//! What a configured agent may reach, decided in one place: whose a task is (the owner's, a
//! configured agent's, or nobody's now), and the folders the agent's files may come from.
//!
//! - A task is a configured agent's when it was made in that agent's room (the room is written
//!   with the task, in one insert). A task made in a room no agent owns any more is
//!   [`TaskOwner::Orphaned`]: it is no one's, and everything about it fails closed (no tools,
//!   no steps); it never becomes the owner's.
//! - An agent's roots are its own folder (`<data dir>/agents/<id>`, made for the server's user
//!   only on first use, its room session's working folder) and the work folders of the tasks
//!   made in its room. A file it sends, attaches or touches must resolve, as a real path, under
//!   one of them.

use std::io;
use std::path::{Path, PathBuf};

use emberhut_core::agents::{Agent, AgentId};
use emberhut_core::task::TaskId;
use emberhut_db::{AgentStore, DbError, TaskStore};

/// The folder of agents' own folders, in the data folder.
const AGENTS_FOLDER: &str = "agents";

/// The folder of the tasks' work folders, in the data folder.
pub const WORK_FOLDER: &str = "work";

/// The most of an agent's tasks whose work folders are its roots.
const ROOT_TASKS: u64 = 500;

/// Whose a task is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskOwner {
    /// The owner's (the default agent's): made in no configured agent's room.
    Owner,
    /// A configured agent's: made in its room.
    Agent(Box<Agent>),
    /// Made in a room no agent owns now (its agent was removed): no one's.
    Orphaned,
}

/// Whose the task `task` is.
///
/// # Errors
///
/// The database's.
pub async fn task_owner(
    tasks: &TaskStore,
    agents: &AgentStore,
    task: TaskId,
) -> Result<TaskOwner, DbError> {
    let Some(room) = tasks.room_of(task).await? else {
        return Ok(TaskOwner::Owner);
    };
    let agent = agents.by_room(room).await?;
    Ok(if agent.id.is_main() {
        TaskOwner::Orphaned
    } else {
        TaskOwner::Agent(Box::new(agent))
    })
}

/// The agent `id`'s own folder in `data_dir`.
#[must_use]
pub fn agent_folder(data_dir: &Path, id: &AgentId) -> PathBuf {
    data_dir.join(AGENTS_FOLDER).join(id.as_str())
}

/// Makes `folder` (and its parents) for the server's user only, when it isn't there.
///
/// # Errors
///
/// The file system's.
pub async fn make_private_folder(folder: &Path) -> io::Result<()> {
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(folder).await
}

/// The folders an agent's files may come from. Cheap to clone.
#[derive(Clone, Debug)]
pub struct AgentRoots {
    data_dir: PathBuf,
    work_dir: PathBuf,
    tasks: TaskStore,
}

impl AgentRoots {
    /// Roots under `data_dir` (agents' own folders) and `work_dir` (tasks' work folders), the
    /// tasks read from `tasks`.
    #[must_use]
    pub fn new(data_dir: PathBuf, work_dir: PathBuf, tasks: TaskStore) -> Self {
        Self {
            data_dir,
            work_dir,
            tasks,
        }
    }

    /// The agent `id`'s own folder.
    #[must_use]
    pub fn folder(&self, id: &AgentId) -> PathBuf {
        agent_folder(&self.data_dir, id)
    }

    /// `agent`'s roots: its own folder (made now when it isn't there) and its tasks' work
    /// folders.
    ///
    /// # Errors
    ///
    /// The database's; a folder that can't be made is logged and left out.
    pub async fn of(&self, agent: &Agent) -> Result<Vec<PathBuf>, DbError> {
        let folder = self.folder(&agent.id);
        if let Err(error) = make_private_folder(&folder).await {
            tracing::warn!(agent = %agent.id, %error, "an agent's folder couldn't be made");
        }
        let mut roots = vec![folder];
        roots.extend(
            self.tasks
                .in_room(agent.room, ROOT_TASKS)
                .await?
                .iter()
                .map(|task| self.work_dir.join(task.key())),
        );
        Ok(roots)
    }
}

/// Whether `path` (relative ones from `cwd`) resolves, as a real path, under one of `roots`:
/// links followed as far as the path exists, and a part that doesn't exist yet (a file about
/// to be written) taken as written, with no `..` in it. Anything at a part that won't resolve
/// (a dangling link, a part that can't be read) is refused, since writing there could land
/// anywhere. A root that isn't there holds nothing.
#[must_use]
pub fn within(path: &Path, cwd: &Path, roots: &[PathBuf]) -> bool {
    let Some(real) = real_path(&cwd.join(path)) else {
        return false;
    };
    roots
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .any(|root| real.starts_with(root))
}

/// The characters that make a part of a glob pattern match more than its own name.
const WILDCARDS: [char; 5] = ['*', '?', '[', '{', '}'];

/// Whether a glob `pattern` searched from `base` (relative ones from `cwd`) can reach only
/// files under one of `roots`. A file tool searches from an absolute pattern's own fixed start,
/// whatever folder it was given, and from `base` for a relative one, so the folder its fixed
/// part names (the parts before the first wildcard) must be [`within`] the roots. Refused when
/// in doubt: a part that steps up (`..`) or starts at a home folder (`~`), in any of its
/// `{a,b}` choices; a choice that starts at `/` anywhere but at the very start; an escape
/// (`\`), which could hide either.
#[must_use]
pub fn glob_within(pattern: &str, base: &Path, cwd: &Path, roots: &[PathBuf]) -> bool {
    if pattern.starts_with('~') || pattern.contains('\\') {
        return false;
    }
    let mut parts = pattern.split(['/', '{', '}', ',']);
    if parts.any(|part| part == ".." || part.starts_with('~')) {
        return false;
    }
    let choices_start_absolute = pattern
        .match_indices(['{', ','])
        .any(|(at, _)| pattern[at + 1..].starts_with('/'));
    if choices_start_absolute {
        return false;
    }
    let fixed_end = pattern.find(WILDCARDS).unwrap_or(pattern.len());
    let folder = match pattern[..fixed_end].rfind('/') {
        Some(0) => "/",
        Some(slash) => &pattern[..slash],
        None => "",
    };
    within(&base.join(folder), cwd, roots)
}

/// The real path of `path`: its longest part that exists, canonical, then the rest as written;
/// none when the rest steps up (`..`), when something is at a part that won't resolve (a
/// dangling link, or a part that can't be read), or nothing of it exists.
fn real_path(path: &Path) -> Option<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    loop {
        if let Ok(mut real) = std::fs::canonicalize(&existing) {
            for part in rest.iter().rev() {
                real.push(part);
            }
            return Some(real);
        }
        // Only a part that isn't there at all may be written as it reads: a link there (even
        // one to nowhere) is followed by whoever writes, and a part that can't be read may be
        // anything.
        match std::fs::symlink_metadata(&existing) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return None,
        }
        // `..` has no file name: a part not there yet never steps up.
        let name = existing.file_name()?.to_owned();
        if !existing.pop() {
            return None;
        }
        rest.push(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_within_only_as_its_real_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("agents").join("client-a");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("brief.pdf"), b"x").unwrap();
        let outside = dir.path().join("owner.txt");
        std::fs::write(&outside, b"private").unwrap();
        let roots = vec![root.clone()];
        assert!(within(&root.join("brief.pdf"), dir.path(), &roots));
        assert!(
            within(Path::new("brief.pdf"), &root, &roots),
            "relative to its cwd"
        );
        assert!(
            within(&root.join("new/draft.md"), dir.path(), &roots),
            "not written yet"
        );
        assert!(!within(&outside, dir.path(), &roots));
        assert!(!within(&root.join("../../owner.txt"), dir.path(), &roots));
        assert!(!within(
            &root.join("missing/../../../owner.txt"),
            dir.path(),
            &roots
        ));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("link.txt")).unwrap();
            assert!(
                !within(&root.join("link.txt"), dir.path(), &roots),
                "a link out"
            );
        }
        assert!(!within(&outside, dir.path(), &[dir.path().join("gone")]));
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_link_in_a_root_is_not_a_file_to_write() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("agents").join("client-a");
        std::fs::create_dir_all(&root).unwrap();
        let roots = vec![root.clone()];
        let outside = dir.path().join("new.txt");
        std::os::unix::fs::symlink(&outside, root.join("out.txt")).unwrap();
        assert!(!within(&root.join("out.txt"), dir.path(), &roots));
        std::os::unix::fs::symlink(dir.path().join("gone"), root.join("away")).unwrap();
        assert!(
            !within(&root.join("away/new.txt"), dir.path(), &roots),
            "a dangling link as a folder"
        );
        assert!(within(&root.join("fresh/new.txt"), dir.path(), &roots));
    }

    #[test]
    fn a_glob_reaches_only_what_its_fixed_folder_is_in() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("agents").join("client-a");
        std::fs::create_dir_all(root.join("notes")).unwrap();
        let roots = vec![root.clone()];
        let elsewhere = dir.path();
        for pattern in ["*.md", "**/*.md", "notes/*.md", "notes/{a,b}.md", "new/**"] {
            assert!(glob_within(pattern, &root, elsewhere, &roots), "{pattern}");
        }
        let mine = format!("{}/**/*.pdf", root.display());
        assert!(glob_within(&mine, Path::new("/"), elsewhere, &roots));
        let theirs = format!("{}/**/*.pdf", dir.path().display());
        for pattern in [
            "/etc/*",
            "/*",
            "/**/*.pdf",
            theirs.as_str(),
            "../../**",
            "notes/../../x",
            "{..,notes}/*",
            "{notes,/etc}/*",
            "notes/{a,/b}",
            "~/Documents/*",
            "{~,notes}/*",
            "\\.\\./x",
        ] {
            assert!(!glob_within(pattern, &root, elsewhere, &roots), "{pattern}");
        }
        assert!(
            glob_within("*.md", Path::new("."), &root, &roots),
            "a relative folder, from the cwd"
        );
        assert!(!glob_within("*.md", Path::new("/"), &root, &roots));
    }
}
