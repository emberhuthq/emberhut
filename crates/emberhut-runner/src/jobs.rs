//! The machine's background agent runs, for the server's Background jobs page (`GET /jobs`).
//!
//! A run is a `codex exec` or a `claude -p` process (not an interactive session, and not the
//! runner's own sessions, which the server lists itself). Each comes with its age, CPU time,
//! memory, folder and last activity: what Codex last said in its run's log (its rollout file),
//! else the newest file in its folder. One `ps` lists them, the same on Linux and macOS; the
//! folders come from `/proc` on Linux and one `lsof` on macOS. On macOS the launchd jobs whose
//! labels match the configured pattern are listed too.
//!
//! Read-only: stopping a run is `POST /jobs/stop`, and the runner stops only a process that
//! [`runs`] lists.

use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The longest command line kept, in characters.
const ARGS_KEPT: usize = 600;
/// How far back a log is read for its last words, in bytes.
const LOG_BUDGET: u64 = 64 << 20;
/// The longest log line decoded; a longer one (a picture as base64) is skipped.
const LONGEST_LINE: usize = 256 << 10;
/// How much of a log is read at once, from the end.
const CHUNK: u64 = 1 << 20;
/// The longest last words kept, in characters.
const SHORT: usize = 160;
/// How many folder entries the look for a run's newest file may see.
const FILE_BUDGET: usize = 20_000;
/// How deep that look goes below the run's folder.
const FILE_DEPTH: usize = 5;

/// Which agent a run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// `codex exec`.
    Codex,
    /// `claude -p`.
    Claude,
}

/// A process, and when it is a run, which kind.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proc {
    /// Its process id.
    pub pid: u32,
    /// Its parent's.
    pub ppid: u32,
    /// Seconds since it started.
    pub age: u64,
    /// The CPU seconds it used.
    pub cpu_time: f64,
    /// Its resident memory, in bytes.
    pub rss: u64,
    /// Its command line (the first 600 characters).
    pub args: String,
    /// The kind of run, or `None` for any other process.
    pub kind: Option<Kind>,
    /// The folder it works in.
    pub cwd: Option<String>,
    /// The systemd unit it lives in (Linux).
    pub unit: Option<String>,
    /// Codex's log of the run.
    pub rollout: Option<String>,
}

/// Where a run's last activity was seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Codex's own log of the run.
    Codex,
    /// The newest file in its folder.
    File,
}

/// What a run did last: its own words, or the file it last changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Last {
    /// The words, or the file's path inside the run's folder.
    pub text: String,
    /// When (RFC 3339), if known.
    pub at: Option<String>,
    /// Where it was seen.
    pub from: Source,
}

/// A launchd job (macOS): running when it has a pid, failed when its last exit isn't 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaunchdJob {
    /// Its label.
    pub label: String,
    /// Its process id while it runs.
    pub pid: Option<u32>,
    /// Its last exit status (negative: the signal that ended it).
    pub status: i64,
}

/// A run with its last activity, as `GET /jobs` lists it.
#[derive(Debug, Serialize)]
struct Job {
    /// The process.
    #[serde(flatten)]
    proc: Proc,
    /// What it last said.
    last: Option<Last>,
    /// The newest file in its folder.
    file: Option<Last>,
}

/// `GET /jobs`'s answer.
#[derive(Debug, Serialize)]
struct Jobs {
    /// When the look was taken.
    at: String,
    /// The runs.
    procs: Vec<Job>,
    /// The matching launchd jobs.
    launchd: Vec<LaunchdJob>,
}

/// A pattern known to compile; `None` only if it didn't, which the tests rule out.
fn re(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

/// `GET /jobs`: the runs with their last activity, and the launchd jobs whose label matches
/// `labels` (none without a pattern).
pub async fn scan(labels: Option<&Regex>) -> Value {
    let (found, launchd) = tokio::join!(runs(), launchd_jobs(labels));
    // Reading logs and walking folders is file work that may take a while: off the runtime.
    let look = tokio::task::spawn_blocking(move || {
        found
            .into_iter()
            .map(|proc| {
                let last = proc
                    .rollout
                    .as_deref()
                    .and_then(|f| codex_last(Path::new(f)));
                let file = proc.cwd.as_deref().and_then(|d| newest_file(Path::new(d)));
                Job { proc, last, file }
            })
            .collect::<Vec<_>>()
    });
    let procs = look.await.unwrap_or_default();
    let jobs = Jobs {
        at: crate::machine::now_text(),
        procs,
        launchd,
    };
    serde_json::to_value(jobs).unwrap_or_default()
}

/// The runs on this machine now, with the folder each works in and (Linux) its systemd unit.
/// The runner's own sessions (its child processes) are not among them: the server lists those
/// itself.
pub async fn runs() -> Vec<Proc> {
    let own = std::process::id();
    let list: Vec<Proc> = procs()
        .await
        .into_iter()
        .filter(|p| is_run(p, own))
        .collect();
    if list.is_empty() {
        return list;
    }
    details(list).await
}

/// Every process, from one `ps`.
#[cfg(unix)]
async fn procs() -> Vec<Proc> {
    let args: &[&str] = if cfg!(target_os = "macos") {
        &["-axww", "-o", "pid=,ppid=,etime=,time=,rss=,command="]
    } else {
        &["-eww", "-o", "pid=,ppid=,etime=,time=,rss=,args="]
    };
    match crate::machine::run("ps", args).await {
        Some(out) if out.status.success() => parse_ps(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

/// Every process: none listed where there is no `ps`.
#[cfg(not(unix))]
#[expect(
    clippy::unused_async,
    reason = "elsewhere ps is waited on for the same"
)]
async fn procs() -> Vec<Proc> {
    Vec::new()
}

/// Linux: each run's folder, unit and (Codex) rollout file from `/proc`.
#[cfg(target_os = "linux")]
#[expect(
    clippy::unused_async,
    reason = "macOS asks lsof, which is waited on, for the same"
)]
async fn details(mut list: Vec<Proc>) -> Vec<Proc> {
    for p in &mut list {
        let at = PathBuf::from(format!("/proc/{}", p.pid));
        p.cwd = std::fs::read_link(at.join("cwd"))
            .ok()
            .map(|d| d.to_string_lossy().into_owned());
        p.unit = std::fs::read_to_string(at.join("cgroup"))
            .ok()
            .as_deref()
            .and_then(crate::machine::parse::unit)
            .map(str::to_owned);
        if p.kind == Some(Kind::Codex) {
            p.rollout = std::fs::read_dir(at.join("fd")).ok().and_then(|fds| {
                fds.filter_map(|fd| std::fs::read_link(fd.ok()?.path()).ok())
                    .map(|f| f.to_string_lossy().into_owned())
                    .find(|f| is_rollout(f))
            });
        }
    }
    list
}

/// macOS: one `lsof` for all the runs, for each one's folder and Codex's rollout file.
#[cfg(target_os = "macos")]
async fn details(mut list: Vec<Proc>) -> Vec<Proc> {
    let pids = list
        .iter()
        .map(|p| p.pid.to_string())
        .collect::<Vec<_>>()
        .join(",");
    // lsof fails when one of the processes is gone by now; what it printed still counts.
    if let Some(out) = crate::machine::run("lsof", &["-a", "-p", &pids, "-Fpfn"]).await {
        parse_lsof(&String::from_utf8_lossy(&out.stdout), &mut list);
    }
    list
}

/// Elsewhere: the runs as `ps` lists them.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[expect(
    clippy::unused_async,
    reason = "macOS asks lsof, which is waited on, for the same"
)]
async fn details(list: Vec<Proc>) -> Vec<Proc> {
    list
}

/// macOS: the launchd jobs whose label matches `labels`, running or failed.
#[cfg(target_os = "macos")]
async fn launchd_jobs(labels: Option<&Regex>) -> Vec<LaunchdJob> {
    let Some(labels) = labels else {
        return Vec::new();
    };
    // launchctl may fail and still print the list.
    crate::machine::run("launchctl", &["list"])
        .await
        .map_or_default(|out| parse_launchctl(&String::from_utf8_lossy(&out.stdout), labels))
}

/// launchd jobs: only on macOS.
#[cfg(not(target_os = "macos"))]
#[expect(
    clippy::unused_async,
    reason = "macOS asks launchctl, which is waited on, for the same"
)]
async fn launchd_jobs(_labels: Option<&Regex>) -> Vec<LaunchdJob> {
    Vec::new()
}

/// `ps`'s times (`1-02:03:04`, `02:03:04`, `03:04`, `03:04.50`) in seconds.
#[must_use]
pub fn secs(text: &str) -> Option<f64> {
    let (days, rest) = match text.split_once('-') {
        Some((d, rest)) => (d.parse::<f64>().ok()?, rest),
        None => (0.0, text),
    };
    let mut total = 0.0;
    for part in rest.split(':') {
        total = total * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(days * 86_400.0 + total)
}

/// Which run a command line is: `codex exec …` (and `codex exec resume …`) or `claude -p …`.
#[must_use]
pub fn kind_of(args: &str) -> Option<Kind> {
    static CODEX: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"^(\S*/)?codex\s+exec\b"));
    static CLAUDE: LazyLock<Option<Regex>> =
        LazyLock::new(|| re(r"^(\S*/)?claude\s+(.*\s)?(-p|--print)(\s|$)"));
    let hit =
        |pattern: &LazyLock<Option<Regex>>| pattern.as_ref().is_some_and(|r| r.is_match(args));
    if hit(&CODEX) {
        Some(Kind::Codex)
    } else if hit(&CLAUDE) {
        Some(Kind::Claude)
    } else {
        None
    }
}

/// `ps -o pid=,ppid=,etime=,time=,rss=,args=`: every process.
#[must_use]
pub fn parse_ps(text: &str) -> Vec<Proc> {
    static LINE: LazyLock<Option<Regex>> =
        LazyLock::new(|| re(r"^(\d+)\s+(\d+)\s+(\S+)\s+(\S+)\s+(\d+)\s+(.*)$"));
    let Some(line_re) = LINE.as_ref() else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let m = line_re.captures(line.trim())?;
            let field = |i: usize| m.get(i).map_or("", |g| g.as_str());
            let args = field(6);
            Some(Proc {
                pid: field(1).parse().ok()?,
                ppid: field(2).parse().ok()?,
                age: whole_secs(secs(field(3)).unwrap_or(0.0)),
                cpu_time: secs(field(4)).unwrap_or(0.0),
                rss: field(5).parse::<u64>().ok()?.saturating_mul(1024),
                args: args.chars().take(ARGS_KEPT).collect(),
                kind: kind_of(args),
                cwd: None,
                unit: None,
                rollout: None,
            })
        })
        .collect()
}

/// Whether a process is a background run, not one of the runner's (`own`) sessions.
fn is_run(p: &Proc, own: u32) -> bool {
    p.kind.is_some() && p.ppid != own
}

/// Seconds as a whole number, rounded down.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a process's age in seconds, non-negative and far below u64's range"
)]
fn whole_secs(secs: f64) -> u64 {
    secs.max(0.0) as u64
}

/// Whether a path is Codex's log of a run (its rollout file).
fn is_rollout(path: &str) -> bool {
    static ROLLOUT: LazyLock<Option<Regex>> =
        LazyLock::new(|| re(r"/\.codex/sessions/(?:.*/)?rollout-[^/]*\.jsonl$"));
    ROLLOUT.as_ref().is_some_and(|r| r.is_match(path))
}

/// `lsof -Fpfn` for the runs in `list`: each one's folder (`cwd`) and Codex's rollout file.
pub fn parse_lsof(text: &str, list: &mut [Proc]) {
    let mut pid = None;
    let mut fd = "";
    for line in text.lines() {
        let Some(tag) = line.chars().next() else {
            continue;
        };
        let value = &line[tag.len_utf8()..];
        match tag {
            'p' => pid = value.parse::<u32>().ok(),
            'f' => fd = value,
            'n' => {
                let Some(p) = list.iter_mut().find(|p| Some(p.pid) == pid) else {
                    continue;
                };
                if fd == "cwd" {
                    p.cwd = Some(value.to_owned());
                } else if is_rollout(value) {
                    p.rollout = Some(value.to_owned());
                }
            }
            _ => {}
        }
    }
}

/// `launchctl list` (`PID\tStatus\tLabel` lines): the jobs whose label matches `labels`, running
/// (a pid) or failed (a last exit that isn't 0).
#[must_use]
pub fn parse_launchctl(text: &str, labels: &Regex) -> Vec<LaunchdJob> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let mut cols = line.split('\t');
            let (pid, status, label) = (cols.next()?, cols.next()?, cols.next()?);
            if cols.next().is_some() {
                return None;
            }
            Some(LaunchdJob {
                label: label.to_owned(),
                pid: pid.parse().ok().filter(|p| *p != 0),
                status: status.parse().unwrap_or(0),
            })
        })
        .filter(|j| labels.is_match(&j.label) && (j.pid.is_some() || j.status != 0))
        .collect()
}

/// A file's lines from the end, newest first, reading at most `budget` bytes back; a line
/// longer than `longest` bytes is skipped without being decoded. Empty lines are skipped.
#[derive(Debug)]
struct BackLines {
    /// The file.
    file: File,
    /// Where the part not read yet ends.
    pos: u64,
    /// Bytes read so far.
    read: u64,
    /// How many bytes may be read.
    budget: u64,
    /// The longest line decoded.
    longest: usize,
    /// The line being put together, its pieces last first; `None` while skipping a long one.
    cur: Option<Vec<Vec<u8>>>,
    /// The length of the line being put together.
    len: usize,
    /// Lines found in the last chunk, newest first.
    ready: std::collections::VecDeque<String>,
    /// Nothing more to read.
    done: bool,
}

impl BackLines {
    /// The lines of `path`, from its end.
    fn open(path: &Path, budget: u64, longest: usize) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let pos = file.metadata()?.len();
        Ok(Self {
            file,
            pos,
            read: 0,
            budget,
            longest,
            cur: Some(Vec::new()),
            len: 0,
            ready: std::collections::VecDeque::new(),
            done: false,
        })
    }

    /// The line put together so far, if any.
    fn finish(&mut self) -> Option<String> {
        let pieces = self.cur.replace(Vec::new())?;
        let len = std::mem::take(&mut self.len);
        (len > 0).then(|| {
            let line: Vec<u8> = pieces.into_iter().rev().flatten().collect();
            String::from_utf8_lossy(&line).into_owned()
        })
    }

    /// Reads the chunk before the part read; `false` at the start of the file, at the budget or
    /// on an error.
    fn chunk(&mut self) -> bool {
        if self.pos == 0 || self.read >= self.budget {
            return false;
        }
        let n = CHUNK.min(self.pos);
        self.pos -= n;
        self.read += n;
        let Ok(size) = usize::try_from(n) else {
            return false;
        };
        let mut buf = vec![0; size];
        if self.file.seek(SeekFrom::Start(self.pos)).is_err()
            || self.file.read_exact(&mut buf).is_err()
        {
            return false;
        }
        let mut end = buf.len();
        loop {
            let newline = buf[..end].iter().rposition(|b| *b == b'\n');
            let piece = &buf[newline.map_or(0, |j| j + 1)..end];
            if let Some(cur) = self.cur.as_mut() {
                self.len += piece.len();
                if self.len > self.longest {
                    self.cur = None;
                } else {
                    cur.push(piece.to_vec());
                }
            }
            let Some(j) = newline else {
                // The line goes on in the chunk before.
                break;
            };
            if let Some(line) = self.finish() {
                self.ready.push_back(line);
            }
            self.cur = Some(Vec::new());
            self.len = 0;
            end = j;
        }
        true
    }
}

impl Iterator for BackLines {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        loop {
            if let Some(line) = self.ready.pop_front() {
                return Some(line);
            }
            if self.done {
                return None;
            }
            if !self.chunk() {
                self.done = true;
                if self.pos == 0 {
                    return self.finish();
                }
            }
        }
    }
}

/// Words cut to one line of at most `SHORT` characters.
fn short(text: &str) -> String {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > SHORT {
        let mut cut: String = t.chars().take(SHORT - 1).collect();
        cut.push('…');
        cut
    } else {
        t
    }
}

/// The home folder, for writing paths under it as `~/…`.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
}

/// One line of a Codex rollout file, as far as it is read.
#[derive(Debug, Deserialize)]
struct Rollout {
    /// When it was written.
    timestamp: Option<String>,
    /// `response_item`, `event_msg`, …
    #[serde(rename = "type")]
    kind: Option<String>,
    /// What it says.
    payload: Option<Payload>,
}

/// A rollout line's payload.
#[derive(Debug, Deserialize)]
struct Payload {
    /// `message`, `agent_message`, …
    #[serde(rename = "type")]
    kind: Option<String>,
    /// Who wrote a message.
    role: Option<String>,
    /// An agent message's words.
    message: Option<String>,
    /// A message's parts.
    content: Option<Vec<Content>>,
    /// A command the run carried out.
    item: Option<Item>,
}

/// A part of a message.
#[derive(Debug, Deserialize)]
struct Content {
    /// Its words.
    text: Option<String>,
}

/// A command the run carried out.
#[derive(Debug, Deserialize)]
struct Item {
    /// `CommandExecution`, …
    #[serde(rename = "type")]
    kind: Option<String>,
    /// The command line, as its words.
    command: Option<Vec<String>>,
}

/// What Codex last said in its run (its rollout file): its last message, else what its last
/// command was, with the home folder written `~`.
#[must_use]
pub fn codex_last(file: &Path) -> Option<Last> {
    codex_last_in(
        BackLines::open(file, LOG_BUDGET, LONGEST_LINE).ok()?,
        home().as_deref(),
    )
}

/// [`codex_last`] over a log's lines, newest first.
fn codex_last_in(lines: impl Iterator<Item = String>, home: Option<&Path>) -> Option<Last> {
    let home = home.map(|h| format!("{}/", h.to_string_lossy().trim_end_matches('/')));
    let mut command = None;
    for line in lines {
        let Ok(entry) = serde_json::from_str::<Rollout>(&line) else {
            continue;
        };
        let Some(p) = entry.payload else {
            continue;
        };
        let at = entry.timestamp;
        let said = |text: &str| Last {
            text: short(text),
            at: at.clone(),
            from: Source::Codex,
        };
        match (entry.kind.as_deref(), p.kind.as_deref()) {
            (Some("response_item"), Some("message")) if p.role.as_deref() == Some("assistant") => {
                let text = p
                    .content
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|c| c.text)
                    .collect::<Vec<_>>()
                    .join(" ");
                if !text.trim().is_empty() {
                    return Some(said(&text));
                }
            }
            (Some("event_msg"), Some("agent_message"))
                if p.message.as_deref().is_some_and(|m| !m.is_empty()) =>
            {
                return p.message.as_deref().map(said);
            }
            (Some("event_msg"), _) if command.is_none() => {
                let last = p
                    .item
                    .filter(|i| i.kind.as_deref() == Some("CommandExecution"))
                    .and_then(|i| i.command?.pop());
                if let Some(last) = last {
                    let text = match &home {
                        Some(h) => last.replace(h.as_str(), "~/"),
                        None => last,
                    };
                    command = Some(said(&text));
                }
            }
            _ => {}
        }
    }
    command
}

/// A folder waiting to be looked in, the most recently changed first.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Folder {
    /// When it last changed (a new file changes its folder's time).
    at: SystemTime,
    /// How deep below the run's folder.
    depth: usize,
    /// Where.
    path: PathBuf,
}

/// The newest file under a run's folder, logs aside (a log's last line says more), in a
/// bounded look: the most recently changed folders first, 5 deep, 20000 entries at most, no
/// dot folders or dependency folders. Never the whole home folder or the root.
#[must_use]
pub fn newest_file(dir: &Path) -> Option<Last> {
    if dir == Path::new("/") || home().as_deref() == Some(dir) || !dir.is_dir() {
        return None;
    }
    newest_under(dir, FILE_BUDGET)
}

/// [`newest_file`], seeing at most `budget` entries.
fn newest_under(dir: &Path, budget: usize) -> Option<Last> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    let mut seen = 0;
    let mut queue = BinaryHeap::from([Folder {
        // The only folder in the queue at first: its time doesn't matter.
        at: SystemTime::UNIX_EPOCH,
        depth: 0,
        path: dir.to_owned(),
    }]);
    'look: while seen < budget {
        let Some(folder) = queue.pop() else {
            break;
        };
        let Ok(entries) = std::fs::read_dir(&folder.path) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > budget {
                break 'look;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "node_modules" || name == "__pycache__" {
                continue;
            }
            let (Ok(kind), Ok(at)) = (
                entry.file_type(),
                entry.metadata().and_then(|m| m.modified()),
            ) else {
                continue;
            };
            if kind.is_dir() {
                if folder.depth < FILE_DEPTH {
                    queue.push(Folder {
                        at,
                        depth: folder.depth + 1,
                        path: entry.path(),
                    });
                }
            } else if kind.is_file()
                && !name.ends_with(".log")
                && !name.ends_with(".jsonl")
                && best.as_ref().is_none_or(|(was, _)| at > *was)
            {
                best = Some((at, entry.path()));
            }
        }
    }
    let (at, path) = best?;
    Some(Last {
        text: path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned(),
        at: jiff::Timestamp::try_from(at)
            .ok()
            .map(|t| format!("{t:.3}")),
        from: Source::File,
    })
}

/// Asks a run to stop (`SIGTERM` on Unix; on Windows the process is ended).
///
/// # Errors
/// The system refused (no such process, or not ours).
pub fn stop(pid: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let pid = i32::try_from(pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .ok_or_else(|| std::io::Error::other("not a process id"))?;
        rustix::process::kill_process(pid, rustix::process::Signal::TERM)
            .map_err(std::io::Error::from)
    }
    #[cfg(not(unix))]
    {
        let status = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("taskkill said {status}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    const PS: &str = "    1     0 12-03:04:05 00:01:02    12000 /sbin/init splash
 4242     1    01:02:03 00:00:12.50 51200 codex exec --full-auto make the example pictures
 4243  4242       03:04 00:00:01   20480 /usr/local/bin/codex exec resume --last
 5000     1  1-00:00:00 01:00:00  307200 node /opt/example/bin/claude -p hello
 5001     1       00:10 00:00:00  102400 /home/example/.local/bin/claude --model opus --print summarise
 5002     1       00:10 00:00:00  102400 claude
 5003     1       00:10 00:00:00  102400 claude --resume abc
 5004     1       00:10 00:00:00  102400 codex
";

    #[test]
    fn ps_times() {
        assert_eq!(secs("1-02:03:04"), Some(93_784.0));
        assert_eq!(secs("02:03:04"), Some(7384.0));
        assert_eq!(secs("03:04"), Some(184.0));
        assert_eq!(secs("03:04.50"), Some(184.5));
        assert_eq!(secs("x"), None);
    }

    #[test]
    fn ps_lists_the_runs() {
        let all = parse_ps(PS);
        assert_eq!(all.len(), 8);
        let kinds: Vec<_> = all.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            [
                None,
                Some(Kind::Codex),
                Some(Kind::Codex),
                None,
                Some(Kind::Claude),
                None,
                None,
                None
            ]
        );
        let codex = &all[1];
        assert_eq!((codex.pid, codex.ppid, codex.age), (4242, 1, 3723));
        assert!((codex.cpu_time - 12.5).abs() < f64::EPSILON);
        assert_eq!(codex.rss, 51_200 * 1024);
        assert_eq!(
            codex.args,
            "codex exec --full-auto make the example pictures"
        );
        assert_eq!(all[0].age, 12 * 86_400 + 3 * 3600 + 4 * 60 + 5);
        let long = format!("9 1 00:01 00:00 1 codex exec {}", "x".repeat(1000));
        assert_eq!(parse_ps(&long)[0].args.chars().count(), ARGS_KEPT);
    }

    #[test]
    fn the_runners_own_sessions_are_not_runs() {
        let all = parse_ps(PS);
        let runs: Vec<u32> = all
            .iter()
            .filter(|p| is_run(p, 4242))
            .map(|p| p.pid)
            .collect();
        assert_eq!(runs, [4242, 5001], "4243 is a child of 4242");
    }

    #[test]
    fn claude_print_flags() {
        assert_eq!(kind_of("claude -p hi"), Some(Kind::Claude));
        assert_eq!(kind_of("claude --print"), Some(Kind::Claude));
        assert_eq!(
            kind_of("claude --permission-mode plan -p"),
            Some(Kind::Claude)
        );
        assert_eq!(kind_of("claude --plugin-dir x"), None);
        assert_eq!(kind_of("codex execute"), None);
        assert_eq!(kind_of("vim codex exec"), None);
    }

    #[test]
    fn lsof_gives_folders_and_rollouts() {
        let mut list = parse_ps(PS);
        let lsof = "p4242\nfcwd\nn/work/example\nftxt\nn/usr/local/bin/codex\nf12\nn/Users/example/.codex/sessions/2026/11/24/rollout-2026-11-24T10-00-00-0123.jsonl\np5001\nfcwd\nn/work/other app\nf3\nn/Users/example/.codex/sessions/other.jsonl\n";
        parse_lsof(lsof, &mut list);
        assert_eq!(list[1].cwd.as_deref(), Some("/work/example"));
        assert_eq!(
            list[1].rollout.as_deref(),
            Some(
                "/Users/example/.codex/sessions/2026/11/24/rollout-2026-11-24T10-00-00-0123.jsonl"
            )
        );
        assert_eq!(list[4].cwd.as_deref(), Some("/work/other app"));
        assert_eq!(list[4].rollout, None);
        assert_eq!(list[0].cwd, None);
    }

    #[test]
    fn launchctl_keeps_matching_running_or_failed_jobs() {
        let text = "PID\tStatus\tLabel\n-\t0\tcom.example.nightly\n812\t0\tcom.example.build\n-\t1\tcom.example.backup\n-\t-9\tcom.example.render\n-\t1\tcom.other.agent\n77\t0\n";
        let labels = regex::RegexBuilder::new(r"^com\.example\.")
            .case_insensitive(true)
            .build()
            .unwrap();
        let jobs = parse_launchctl(text, &labels);
        assert_eq!(
            jobs,
            [
                LaunchdJob {
                    label: "com.example.build".to_owned(),
                    pid: Some(812),
                    status: 0
                },
                LaunchdJob {
                    label: "com.example.backup".to_owned(),
                    pid: None,
                    status: 1
                },
                LaunchdJob {
                    label: "com.example.render".to_owned(),
                    pid: None,
                    status: -9
                },
            ]
        );
    }

    fn lines_of(text: &str, budget: u64, longest: usize) -> Vec<String> {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(text.as_bytes()).unwrap();
        BackLines::open(f.path(), budget, longest)
            .unwrap()
            .collect()
    }

    #[test]
    fn lines_from_the_end() {
        assert_eq!(
            lines_of("one\ntwo\n\nthree", 1 << 20, 100),
            ["three", "two", "one"]
        );
        assert_eq!(lines_of("one\ntwo\n", 1 << 20, 100), ["two", "one"]);
        assert_eq!(lines_of("", 1 << 20, 100).len(), 0);
        let long = format!("short\n{}\nlast\n", "x".repeat(500));
        assert_eq!(lines_of(&long, 1 << 20, 100), ["last", "short"]);
    }

    #[test]
    fn lines_across_chunks() {
        let big = "y".repeat(usize::try_from(CHUNK).unwrap() + 10);
        let text = format!("first\n{big}\nend\n");
        let lines = lines_of(&text, LOG_BUDGET, usize::MAX);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "end");
        assert_eq!(lines[1].len(), big.len());
        assert_eq!(lines[2], "first");
        let skipped = lines_of(&text, LOG_BUDGET, 100);
        assert_eq!(skipped, ["end", "first"]);
        let within = lines_of(&text, 1, usize::MAX);
        assert_eq!(within[0], "end", "the budget stops the look after a chunk");
    }

    fn rollout(lines: &[&str]) -> Vec<String> {
        lines.iter().rev().map(|l| (*l).to_owned()).collect()
    }

    #[test]
    fn codex_last_message() {
        let log = rollout(&[
            r#"{"timestamp":"2026-11-24T10:00:00.000Z","type":"session_meta","payload":{"id":"x"}}"#,
            r#"{"timestamp":"2026-11-24T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Wrote   a.rs\nand b.rs"}]}}"#,
            r#"{"timestamp":"2026-11-24T10:02:00.000Z","type":"event_msg","payload":{"type":"exec","item":{"type":"CommandExecution","command":["bash","-lc","cargo test --manifest-path /home/example/work/Cargo.toml"]}}}"#,
            "not json",
        ]);
        let last =
            codex_last_in(log.clone().into_iter(), Some(Path::new("/home/example"))).unwrap();
        assert_eq!(last.text, "Wrote a.rs and b.rs");
        assert_eq!(last.at.as_deref(), Some("2026-11-24T10:01:00.000Z"));
        assert_eq!(last.from, Source::Codex);

        let commands_only = log.into_iter().filter(|l| !l.contains("response_item"));
        let last = codex_last_in(commands_only, Some(Path::new("/home/example/"))).unwrap();
        assert_eq!(last.text, "cargo test --manifest-path ~/work/Cargo.toml");

        let agent = rollout(&[
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"All checks pass"}}"#,
        ]);
        let last = codex_last_in(agent.into_iter(), None).unwrap();
        assert_eq!((last.text.as_str(), last.at), ("All checks pass", None));
        assert!(codex_last_in(std::iter::empty(), None).is_none());
    }

    #[test]
    fn long_words_are_cut() {
        let cut = short(&"word ".repeat(100));
        assert_eq!(cut.chars().count(), SHORT);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn the_newest_file_in_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let at = |secs: u64| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        let put = |p: &str, secs: u64| {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "x").unwrap();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(at(secs))
                .unwrap();
        };
        put("README.md", 1_700_000_000);
        put("src/a.rs", 1_700_000_100);
        put("out/run.log", 1_800_000_000);
        put(".git/index", 1_800_000_000);
        put("node_modules/x/index.js", 1_800_000_000);
        let last = newest_file(dir.path()).unwrap();
        assert_eq!(last.text, Path::new("src").join("a.rs").to_string_lossy());
        assert_eq!(last.from, Source::File);
        assert_eq!(last.at.as_deref(), Some("2023-11-14T22:15:00.000Z"));
        assert!(newest_file(Path::new("/")).is_none());
        assert!(newest_file(&dir.path().join("missing")).is_none());
        assert!(
            newest_under(dir.path(), 1).is_none(),
            "one entry seen is a folder or a skipped name"
        );
    }

    #[test]
    fn the_answer_has_the_servers_shape() {
        let job = Job {
            proc: parse_ps(PS).swap_remove(1),
            last: Some(Last {
                text: "Wrote a.rs".to_owned(),
                at: None,
                from: Source::Codex,
            }),
            file: None,
        };
        let v = serde_json::to_value(Jobs {
            at: "2026-11-25T10:00:00.000Z".to_owned(),
            procs: vec![job],
            launchd: Vec::new(),
        })
        .unwrap();
        let p = &v["procs"][0];
        assert_eq!(p["pid"].as_u64(), Some(4242));
        assert_eq!(p["kind"], "codex");
        assert_eq!(p["cpuTime"], 12.5);
        assert_eq!(p["rss"].as_u64(), Some(51_200 * 1024));
        assert_eq!(p["age"].as_u64(), Some(3723));
        assert_eq!(p["last"]["from"], "codex");
        assert_eq!(p["file"], Value::Null);
        assert_eq!(v["launchd"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn a_look_at_this_machine() {
        let v = scan(None).await;
        assert!(v["procs"].is_array());
        assert_eq!(v["launchd"], serde_json::json!([]));
        for p in runs().await {
            assert!(p.kind.is_some());
        }
    }
}
