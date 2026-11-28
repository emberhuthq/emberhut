//! Outward steps (push, merge, PR, release, deploy, a service restart) of workers and ultra
//! runs' leads. One goes through when the owner's request for the task (or its parent) names
//! it, when their OK for the task's outward steps was given (relayed by the assistant, or their
//! Go ahead on the outward card), or when every repository it touches is one the installation
//! lets through ([`super::GuardConfig::always_merge`]). Otherwise the server asks the owner
//! itself; the lead is told to wait ([`emberhut_core::guard::outward::WAITS`]).
//!
//! A command the rules can't be sure of (inline code, a script, a command on another machine:
//! [`rules::Reading::Unsure`]) goes to the assistant first: its judge model reads the command
//! ([`rules::READ_ONLY_RULES`]), and one it reads as only reading runs. Only what it doesn't
//! (or a judge that is off, fails or takes longer than [`JUDGE_WAIT`]) asks the owner. Its
//! last verdicts are kept in memory, so a command run again isn't read again.
//!
//! The OKs are kept in the settings (`outwardOks`: task to the
//! line it says) until the task is closed after them ([`Guard::outward_ended`]).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::PoisonError;
use std::time::Duration;

use emberhut_core::MessageId;
use emberhut_core::guard::outward::{self as rules, Remotes};
use emberhut_core::message::Author;
use emberhut_core::task::{Status, TaskId, Who};
use emberhut_db::{DbError, SettingKey};

use super::{Guard, GuardError};

pub(crate) const OKS: SettingKey<BTreeMap<String, String>> = SettingKey::new("outwardOks");

/// How long the assistant's judge may take over one command: the gate answers the worker's
/// hook well inside its own time limit.
pub const JUDGE_WAIT: Duration = Duration::from_secs(25);

/// How many of the judge's verdicts are kept.
const VERDICTS_KEPT: usize = 200;

/// A folder's git remotes, as `owner/name` where they are GitHub's.
pub trait RepoRemotes: fmt::Debug + Send + Sync {
    /// The remotes of the repository at `dir`; none when it isn't one, or they can't be read.
    fn remotes(&self, dir: &str) -> Option<Vec<String>>;
}

/// `git -C <dir> remote -v`, given five seconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct GitRemotes;

impl RepoRemotes for GitRemotes {
    fn remotes(&self, dir: &str) -> Option<Vec<String>> {
        if !std::path::Path::new(dir).is_dir() {
            return None;
        }
        let mut child = std::process::Command::new("git")
            .args(["-C", dir, "remote", "-v"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break,
                Ok(Some(_)) | Err(_) => return None,
                Ok(None) if std::time::Instant::now() > deadline => {
                    if let Err(error) = child.kill() {
                        tracing::debug!(%error, "git didn't stop");
                    }
                    return None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        let output = child.wait_with_output().ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let mut urls: Vec<String> = Vec::new();
        for line in text.lines() {
            if let Some(url) = line.split_whitespace().nth(1) {
                let name = rules::repo_name(url).unwrap_or_else(|| url.to_owned());
                if !urls.contains(&name) {
                    urls.push(name);
                }
            }
        }
        (!urls.is_empty()).then_some(urls)
    }
}

/// [`Remotes`] for the rules: the home folder and a [`RepoRemotes`].
pub(super) struct Reader<'a> {
    pub(super) home: String,
    pub(super) remotes: &'a dyn RepoRemotes,
}

impl Remotes for Reader<'_> {
    fn home(&self) -> &str {
        &self.home
    }

    fn remotes(&self, dir: &str) -> Option<Vec<String>> {
        self.remotes.remotes(dir)
    }
}

impl Guard {
    /// The owner's request for the task or one of its parents, when it names an outward step:
    /// the line that says it covers it.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn request_names(&self, task: TaskId) -> Result<Option<String>, DbError> {
        let mut id = Some(task);
        for _ in 0..8 {
            let Some(at) = id else { break };
            let Some(t) = self.task(at).await? else { break };
            if let Some(r) = self.requested(at).await?
                && let Some(msg) = r.msg
                && let Some(said) = self.inner.messages.get(MessageId::new(msg)).await?
                && said.author == Author::User
                && let Some(words) = rules::named(&said.text)
            {
                return Ok(Some(format!(
                    "their request for {} at {} (\"{words}\")",
                    at.key(),
                    self.hhmm(said.created_at)
                )));
            }
            id = t.parent;
        }
        Ok(None)
    }

    /// Keeps the owner's OK for the task's outward steps, `how` it was given.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn outward_ok(&self, task: TaskId, how: &str) -> Result<(), DbError> {
        let how = how.to_owned();
        self.inner
            .settings
            .update(&OKS, BTreeMap::new(), |all| {
                all.insert(task.get().to_string(), how);
            })
            .await
    }

    /// The task was closed (done or canceled, by the owner or its worker): the owner's OK for
    /// its outward steps ends with that job, and a reopened task asks again.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn outward_ended(&self, task: TaskId) -> Result<(), GuardError> {
        let removed = self
            .inner
            .settings
            .update(&OKS, BTreeMap::new(), |all| {
                all.remove(&task.get().to_string()).is_some()
            })
            .await?;
        if removed {
            self.inner
                .tasks
                .note(
                    task,
                    Who::Assistant,
                    "The owner's OK for its push, merge and deploy steps ended with the task; reopened, it asks again",
                    false,
                )
                .await?;
        }
        Ok(())
    }

    /// What covers the task's outward steps in the owner's own words, or none: their OK on the
    /// task or a parent (held until that task is closed after it), else their request naming
    /// it while the job has its grant. A canceled task holds none.
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn covered_by(&self, task: TaskId) -> Result<Option<String>, GuardError> {
        // A configured agent's task (or no one's): no OK of the owner's covers it, so its
        // workers always have the gate and its policies decide each step.
        if self.not_the_owners(task).await? {
            return Ok(None);
        }
        match self.task(task).await? {
            None => return Ok(None),
            Some(t) if t.status == Status::Canceled => return Ok(None),
            Some(_) => {}
        }
        let oks = self.inner.settings.get_or(&OKS, BTreeMap::new()).await?;
        let mut id = Some(task);
        for _ in 0..8 {
            let Some(at) = id else { break };
            if let Some(ok) = oks.get(&at.get().to_string()) {
                return Ok(Some(ok.clone()));
            }
            id = self.task(at).await?.and_then(|t| t.parent);
        }
        if self.job_grant(task).await?.is_some() {
            return Ok(self.request_names(task).await?);
        }
        Ok(None)
    }

    /// Whether `repo` is one the installation always lets through.
    fn always(&self, repo: &str) -> bool {
        let repo = repo.to_lowercase();
        self.inner.config.always_merge.iter().any(|allowed| {
            let allowed = allowed.trim().to_lowercase();
            match allowed.strip_prefix("*/") {
                Some(name) => repo.rsplit('/').next() == Some(name),
                None => repo == allowed,
            }
        })
    }

    /// Whether `command` has an outward step at all.
    #[must_use]
    pub fn is_outward(&self, command: &str) -> bool {
        rules::is_outward(command, &self.home())
    }

    /// Whether every outward step of `command`, run in `cwd`, is in a repository always let
    /// through. Reads the folders' git remotes, off the async threads.
    pub async fn always_ok(&self, command: &str, cwd: &str) -> bool {
        if self.inner.config.always_merge.is_empty() {
            return false;
        }
        let guard = self.clone();
        let (command, cwd) = (command.to_owned(), cwd.to_owned());
        tokio::task::spawn_blocking(move || {
            let reader = Reader {
                home: guard.home(),
                remotes: guard.inner.remotes.as_ref(),
            };
            rules::always_ok(&command, &cwd, &reader, &|repo| guard.always(repo))
        })
        .await
        .unwrap_or(false)
    }

    /// A lead's shell command before it runs: none lets it through; otherwise what the lead is
    /// told, and the owner is asked (once, while that card is open) by `ask`. A command it
    /// can't be sure of is read by the assistant first ([`Self::assistant_reads_only`]).
    ///
    /// # Errors
    ///
    /// [`GuardError`].
    pub async fn hold_outward(
        &self,
        task: TaskId,
        command: &str,
        cwd: &str,
    ) -> Result<Option<&'static str>, GuardError> {
        let unsure = match rules::reading(command, &self.home()) {
            rules::Reading::Clear => return Ok(None),
            rules::Reading::Unsure(steps) => Some(steps),
            rules::Reading::Outward(_) => None,
        };
        if self.covered_by(task).await?.is_some() || self.always_ok(command, cwd).await {
            return Ok(None);
        }
        if let Some(steps) = unsure
            && self.assistant_reads_only(command, cwd, &steps).await
        {
            tracing::info!(
                task = %task.key(),
                command = %emberhut_core::guard::key_mask::mask(
                    &command.chars().take(160).collect::<String>(),
                    &[],
                    None
                ),
                "the assistant read a command the rules couldn't as only reading"
            );
            return Ok(None);
        }
        Ok(Some(rules::WAITS))
    }

    /// Whether the assistant's judge reads `command` (run in `cwd`, held as `held`) as only
    /// reading. No judge, no answer within [`JUDGE_WAIT`] or an answer that isn't a clear yes:
    /// no. Its verdicts are kept, so the same command isn't read twice.
    pub async fn assistant_reads_only(
        &self,
        command: &str,
        cwd: &str,
        held: &[rules::Step],
    ) -> bool {
        if !self.inner.judge.on() {
            return false;
        }
        let key = format!("{cwd}\n{command}");
        if let Some(seen) = self
            .inner
            .read_only
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, verdict)| *verdict)
        {
            return seen;
        }
        let prompt = rules::read_only_prompt(command, cwd, held);
        let said = tokio::time::timeout(
            JUDGE_WAIT,
            self.inner.judge.judge(rules::READ_ONLY_RULES, &prompt),
        )
        .await;
        let verdict = match said {
            Ok(Some(text)) => read_verdict(&text),
            // No answer (or none in time) isn't kept: the next try asks again.
            Ok(None) | Err(_) => return false,
        };
        let mut kept = self
            .inner
            .read_only
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        kept.push((key, verdict));
        let extra = kept.len().saturating_sub(VERDICTS_KEPT);
        kept.drain(..extra);
        verdict
    }
}

/// The judge's verdict: `{"reads_only": true}` alone is a yes.
fn read_verdict(said: &str) -> bool {
    let (Some(start), Some(end)) = (said.find('{'), said.rfind('}')) else {
        return false;
    };
    said.get(start..=end)
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| value.get("reads_only").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::read_verdict;

    #[test]
    fn only_a_clear_yes_reads_as_read_only() {
        assert!(read_verdict(
            r#"{"reads_only": true, "why": "it prints a file"}"#
        ));
        assert!(read_verdict("Here it is:\n{\"reads_only\": true}"));
        assert!(!read_verdict(r#"{"reads_only": false, "why": "it posts"}"#));
        assert!(!read_verdict(r#"{"reads_only": "true"}"#));
        assert!(!read_verdict("yes, it only reads"));
        assert!(!read_verdict(""));
    }
}
