//! Where the browser tools act: the server's own browser, or one of the owner's Chrome profiles
//! through the extension. Each is a [`Surface`]: an engine, the state of its tab and who drives
//! it, and its own hand-off and take-back. [`Kit`] is what both kinds share: the tasks, the
//! gate, pushes, the activity log, and each task's browser log.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use emberhut_core::Clock;
use emberhut_core::browser::mask;
use emberhut_core::push::Alert;
use futures_util::future::BoxFuture;
use jiff::Timestamp;

use super::desk::{NoteBy, StepGate, TaskDesk};
use super::engine::{BrowserEngine, TabId};
use crate::services::activity::ActivityLog;
use crate::services::push::PushService;

/// Where a surface is and who drives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SurfaceState {
    /// The tab's address.
    pub url: String,
    /// The tab's title.
    pub title: String,
    /// Who drives or drove it last.
    pub driver: Option<String>,
    /// The task the browsing is for.
    pub task: Option<i64>,
    /// When it last changed hands or was used.
    pub at: Option<Timestamp>,
}

/// One of the assistant's tabs there that closed, and by whom: `page` (it closed itself),
/// `assistant` (the assistant), `crash`, `window`, `chrome` (the owner or another extension), or only
/// its link cut, the tab still open: `frame`, then `relinked` once the assistant has it again.
/// `by` keeps the extension's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabClose {
    /// When, in ms since the epoch.
    pub at: i64,
    /// By whom (see above).
    pub by: String,
    /// Its address.
    pub url: String,
    /// What the agent is told.
    pub text: String,
    /// The tab, by the extension's id (0: unknown).
    pub tab: i64,
}

/// How long a close is remembered: the same page isn't opened again within it.
pub const CLOSE_KEPT_MS: i64 = 2 * 60_000;

/// Who is browsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Who {
    /// How the browser's state names the driver.
    pub driver: String,
    /// The task the browsing is for.
    pub task: Option<i64>,
}

/// How the state names the assistant when it drives.
pub const ASSISTANT: &str = "the assistant";

impl Who {
    /// The assistant, browsing for `task`.
    #[must_use]
    pub fn assistant(task: Option<i64>) -> Self {
        Self {
            driver: ASSISTANT.to_owned(),
            task: task.filter(|t| *t != 0),
        }
    }

    /// Who a task note is from.
    #[must_use]
    pub fn note_by(&self) -> NoteBy {
        if self.driver.starts_with("the assistant's worker") {
            NoteBy::Worker
        } else {
            NoteBy::Assistant
        }
    }
}

/// A browser the tools act in.
pub trait Surface: fmt::Debug + Send + Sync {
    /// Said in the owner's asks and the task's log: `""` here, `" in your Work Chrome"` there.
    fn place(&self) -> String;

    /// Its engine.
    fn engine(&self) -> Arc<dyn BrowserEngine>;

    /// Its state now.
    fn state(&self) -> SurfaceState;

    /// Changes its state.
    fn update(&self, change: &mut dyn FnMut(&mut SurfaceState));

    /// The tab the tools act in, when there is one.
    fn current(&self) -> Option<TabId>;

    /// Acts in `tab` from now on (none: pick one at the next step).
    fn use_tab(&self, tab: Option<TabId>);

    /// Why agents must wait now (the owner drives), or none.
    fn busy(&self) -> Option<String>;

    /// Hands it to the owner for `reason`, with a picture of where it is.
    fn handoff(
        &self,
        reason: String,
        task: Option<i64>,
        shot: Option<Vec<u8>>,
    ) -> BoxFuture<'_, String>;

    /// Ends the owner's turn at once.
    fn takeback(&self, reason: String, task: Option<i64>) -> BoxFuture<'_, String>;

    /// A password manager can be asked to fill here.
    fn autofill(&self) -> bool;

    /// Tells that browser too (the extension's popup lists the steps).
    fn step(&self, line: &str) {
        let _ = line;
    }

    /// The assistant's tabs there that closed lately, and by whom; none: not told.
    fn closes(&self) -> Option<Vec<TabClose>> {
        None
    }
}

/// What both kinds of surface share.
#[derive(Clone)]
pub struct Kit {
    /// The tasks and the assistant's events.
    pub desk: Arc<dyn TaskDesk>,
    /// The owner's OK on final steps.
    pub gate: Arc<dyn StepGate>,
    /// Pushes to the owner's phones.
    pub push: Option<PushService>,
    /// The activity log.
    pub activity: ActivityLog,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// Each task's browser log.
    pub logs: BrowserLog,
}

impl fmt::Debug for Kit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Kit")
            .field("logs", &self.logs)
            .finish_non_exhaustive()
    }
}

impl Kit {
    /// A line in the activity log (keys masked); a failure is logged, never raised.
    pub async fn log(&self, line: impl Into<String>) {
        self.log_kept(line).await;
    }

    /// [`Self::log`], saying whether the line was kept.
    pub async fn log_kept(&self, line: impl Into<String>) -> bool {
        let line = mask(&line.into());
        match self.activity.note(line).await {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, "an activity line was not kept");
                false
            }
        }
    }

    /// A push to the owner's phones.
    pub async fn push(&self, title: &str, body: &str) {
        if let Some(push) = &self.push {
            push.push(&Alert::new(title, &mask(body), None)).await;
        }
    }

    /// A line in task `task`'s browser log (and a picture), attached to the task.
    pub async fn note(&self, task: i64, line: &str, shot: Option<Vec<u8>>, url: &str) {
        let now = self.clock.now();
        match self.logs.write(task, line, shot.as_deref(), url, now) {
            Ok(made) => {
                for (path, title) in made {
                    self.desk
                        .attach(task, path.display().to_string(), title)
                        .await;
                }
            }
            Err(error) => tracing::warn!(%error, task, "the task's browser log was not written"),
        }
    }

    /// The assistant took a browser back from the owner: the owner is told in one line, and
    /// the tasks (the one it's for, the one it was handed over for) log it and leave "Needs you"
    /// (`action`: the hand-off's own line, cleared only if still there).
    pub async fn took_back(
        &self,
        what: &str,
        reason: &str,
        tasks: [Option<i64>; 2],
        action: &str,
        url: &str,
    ) {
        let line = format!("The assistant took {what} back: {reason}");
        self.log(line.clone()).await;
        self.push(&format!("The assistant took {what} back"), reason)
            .await;
        let mut done = Vec::new();
        for task in tasks.into_iter().flatten().filter(|t| *t != 0) {
            if done.contains(&task) || !self.desk.exists(task).await {
                continue;
            }
            done.push(task);
            self.desk.note(task, NoteBy::Assistant, line.clone()).await;
            self.note(task, &line, None, url).await;
            self.desk.clear_action(task, action.to_owned()).await;
        }
    }
}

/// Each task's browser log: `browser/logs/task-<id>.md` in the data folder, a line per step
/// (keys masked), and its pictures beside it. Owner-only files.
#[derive(Clone, Debug)]
pub struct BrowserLog {
    dir: PathBuf,
    heading: &'static str,
}

impl BrowserLog {
    /// The logs in `data_dir`'s `browser/logs`.
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join("browser").join("logs"),
            heading: "browser log",
        }
    }

    /// The logs of the assistant's steps on the owner's Mac: `mac/logs` in the data folder.
    #[must_use]
    pub fn mac(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join("mac").join("logs"),
            heading: "the assistant on the Mac",
        }
    }

    /// The log's folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes a line; returns the files made (the log, the first time, and the picture), each
    /// with its title.
    pub(crate) fn write(
        &self,
        task: i64,
        line: &str,
        shot: Option<&[u8]>,
        url: &str,
        now: Timestamp,
    ) -> std::io::Result<Vec<(PathBuf, String)>> {
        use std::io::Write as _;
        make_private_dir(&self.dir)?;
        let line = mask(line);
        let url = mask(url);
        let file = self.dir.join(format!("task-{task}.md"));
        let mut made = Vec::new();
        if !file.exists() {
            write_private(
                &file,
                format!("# Task {task}: {}\n\n", self.heading).as_bytes(),
            )?;
            let mut title = self.heading.to_owned();
            if let Some(first) = title.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            made.push((file.clone(), title));
        }
        let time = now.to_string();
        let hhmm = time.get(11..16).unwrap_or_default();
        let at = if url.is_empty() {
            String::new()
        } else {
            format!(" ({url})")
        };
        let mut log = std::fs::OpenOptions::new().append(true).open(&file)?;
        writeln!(log, "- {hhmm} {line}{at}")?;
        if let Some(shot) = shot {
            let picture = self
                .dir
                .join(format!("task-{task}-{}.jpg", now.as_millisecond()));
            write_private(&picture, shot)?;
            made.push((picture, line.chars().take(60).collect()));
        }
        Ok(made)
    }
}

/// Makes `dir` (and its parents), owner-only.
pub(crate) fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Writes `bytes` to `path` (made or emptied), owner-only.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
