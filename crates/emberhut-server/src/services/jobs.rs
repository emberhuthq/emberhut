//! Background jobs: every job running now on the runners' machines, for the app's Running tab:
//! the runners' sessions with a turn on, and the runs and scheduled jobs each runner lists of its
//! machine (`GET /jobs`: `{ procs: [{ pid, kind, args, cwd, age, cpuTime, rss, last, file }],
//! launchd: [{ label, pid, status }] }`). A look is kept 4 seconds; a job's CPU now comes from
//! its CPU time between two looks. Stopping a session stops it on its runner; stopping a run
//! asks its runner to (`POST /jobs/stop { pid }`).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use emberhut_api as api;
use emberhut_core::Clock;
use emberhut_db::DbError;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};

use crate::convert::timestamp_to_api;
use crate::services::activity::ActivityLog;
use crate::services::runner_api::Verb;
use crate::services::runners::{Filter, RunnerService, SessionError, SessionId, text};

/// How long a look is kept.
const KEEP: SignedDuration = SignedDuration::from_secs(4);

/// Why a job wasn't stopped.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum JobError {
    /// It can't be stopped, or is gone, in words.
    #[error("{0}")]
    Refused(String),
    /// Its runner failed.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// The jobs.
#[derive(Clone, Debug)]
pub struct JobService {
    runners: RunnerService,
    activity: ActivityLog,
    clock: Arc<dyn Clock>,
    kept: Arc<Mutex<Option<(Timestamp, api::jobs::JobsPage)>>>,
    cpu: Arc<Mutex<BTreeMap<String, (f64, Timestamp)>>>,
}

impl JobService {
    /// The service over the runners.
    #[must_use]
    pub fn new(runners: RunnerService, activity: ActivityLog, clock: Arc<dyn Clock>) -> Self {
        Self {
            runners,
            activity,
            clock,
            kept: Arc::default(),
            cpu: Arc::default(),
        }
    }

    /// Every job now: the last look when it is under 4 seconds old, unless `fresh`.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn now(&self, fresh: bool) -> Result<api::jobs::JobsPage, DbError> {
        let now = self.clock.now();
        if !fresh
            && let Some((at, list)) = self
                .kept
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
            && now.duration_since(*at) < KEEP
        {
            return Ok(list.clone());
        }
        let list = self.look().await?;
        *self.kept.lock().unwrap_or_else(PoisonError::into_inner) = Some((now, list.clone()));
        Ok(list)
    }

    async fn look(&self) -> Result<api::jobs::JobsPage, DbError> {
        let mut jobs = Vec::new();
        for session in self.runners.sessions(Filter::default()).await? {
            if !matches!(session.status.as_str(), "running" | "queued") {
                continue;
            }
            let said = session
                .doing
                .clone()
                .or_else(|| session.status_line.clone());
            jobs.push(api::jobs::Job {
                id: api::ids::MachineJobId::new(format!("session:{}", session.id.as_str())),
                kind: api::jobs::JobKind::Session,
                place: session.place.clone(),
                name: session.id.as_str().to_owned(),
                title: session.title.clone(),
                what: match &session.account_name {
                    Some(name) if !name.is_empty() => format!("Runner session · {name}"),
                    _ => "Runner session".to_owned(),
                },
                task: None,
                tasks: Vec::new(),
                linked: None,
                state: if session.status == "queued" {
                    api::jobs::JobState::Starting
                } else {
                    api::jobs::JobState::Running
                },
                since: session.since.clone(),
                ended: None,
                exit: None,
                cpu: None,
                cpu_time: None,
                memory: None,
                last: said.map(|text| api::jobs::JobActivity {
                    text,
                    at: Some(session.updated_at.clone()),
                    from: "agent".to_owned(),
                }),
                file: None,
                active: Some(session.updated_at.clone()),
                cwd: session.cwd.clone(),
                command: None,
                ports: Vec::new(),
                stop: true,
                clear: false,
            });
        }
        let mut runners = BTreeMap::new();
        for machine in self.runners.machines() {
            match self
                .runners
                .api()
                .call(&machine, Verb::Get, "/jobs", None)
                .await
            {
                Ok(answer) => {
                    jobs.extend(self.runner_jobs(&machine, &answer));
                    runners.insert(
                        machine,
                        api::jobs::JobsMac {
                            ok: true,
                            error: None,
                        },
                    );
                }
                Err(error) => {
                    runners.insert(
                        machine,
                        api::jobs::JobsMac {
                            ok: false,
                            error: Some(error.to_string()),
                        },
                    );
                }
            }
        }
        let rank = |state: &api::jobs::JobState| match state.as_str() {
            "failed" => 0,
            "running" | "starting" => 1,
            _ => 2,
        };
        jobs.sort_by(|a, b| {
            rank(&a.state)
                .cmp(&rank(&b.state))
                .then_with(|| b.since.cmp(&a.since))
        });
        // Whether the runners answered: every one did, or the first one's reason.
        let mac = api::jobs::JobsMac {
            ok: runners.values().all(|r: &api::jobs::JobsMac| r.ok),
            error: runners.values().find_map(|r| r.error.clone()),
        };
        Ok(api::jobs::JobsPage {
            at: timestamp_to_api(self.clock.now()),
            jobs,
            mac,
        })
    }

    /// The jobs `machine`'s runner listed.
    fn runner_jobs(&self, machine: &str, answer: &Value) -> Vec<api::jobs::Job> {
        let place = self.runners.place(machine);
        let now = self.clock.now();
        let list = |key: &str| {
            answer
                .get(key)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let mut jobs = Vec::new();
        for proc in list("procs") {
            let Some(pid) = proc.get("pid").and_then(Value::as_i64) else {
                continue;
            };
            let id = format!("{machine}:pid:{pid}");
            let codex = text(&proc, "kind").as_deref() == Some("codex");
            let name = if codex { "codex exec" } else { "agent run" };
            let title = if codex { "Codex run" } else { "Agent run" };
            let age = proc.get("age").and_then(Value::as_f64).unwrap_or(0.0);
            let cpu_time = proc.get("cpuTime").and_then(Value::as_f64);
            let since = now
                .checked_sub(SignedDuration::from_secs_f64(age.max(0.0)))
                .ok()
                .map(timestamp_to_api);
            let said = proc.get("last").filter(|said| said.is_object()).cloned();
            let file = proc.get("file").filter(|file| file.is_object()).cloned();
            let active = [&said, &file]
                .into_iter()
                .filter_map(|said| said.as_ref().and_then(|said| text(said, "at")))
                .max()
                .map(api::time::Timestamp::new);
            jobs.push(api::jobs::Job {
                cpu: cpu_time.and_then(|cpu_time| self.cpu_now(&id, cpu_time, now)),
                id: api::ids::MachineJobId::new(id),
                kind: api::jobs::JobKind::Process,
                place: place.clone(),
                name: name.to_owned(),
                title: title.to_owned(),
                what: title.to_owned(),
                task: None,
                tasks: Vec::new(),
                linked: None,
                state: api::jobs::JobState::Running,
                since,
                ended: None,
                exit: None,
                cpu_time,
                memory: proc.get("rss").and_then(Value::as_u64),
                last: said.and_then(|said| serde_json::from_value(said).ok()),
                file: file.and_then(|file| serde_json::from_value(file).ok()),
                active,
                cwd: text(&proc, "cwd"),
                command: text(&proc, "args").map(|args| args.chars().take(400).collect()),
                ports: Vec::new(),
                stop: true,
                clear: false,
            });
        }
        for job in list("launchd") {
            let Some(label) = text(&job, "label") else {
                continue;
            };
            let on = job
                .get("pid")
                .and_then(Value::as_i64)
                .is_some_and(|pid| pid != 0);
            let status = job.get("status").map_or_else(String::new, Value::to_string);
            jobs.push(api::jobs::Job {
                id: api::ids::MachineJobId::new(format!("{machine}:launchd:{label}")),
                kind: api::jobs::JobKind::Launchd,
                place: place.clone(),
                name: label.clone(),
                title: label,
                what: "launchd job".to_owned(),
                task: None,
                tasks: Vec::new(),
                linked: None,
                state: if on {
                    api::jobs::JobState::Running
                } else {
                    api::jobs::JobState::Failed
                },
                since: None,
                ended: None,
                exit: (!on).then(|| format!("exit {status}")),
                cpu: None,
                cpu_time: None,
                memory: None,
                last: None,
                file: None,
                active: None,
                cwd: None,
                command: None,
                ports: Vec::new(),
                stop: false,
                clear: false,
            });
        }
        jobs
    }

    /// The CPU `id` used since its last look, in % of one core.
    fn cpu_now(&self, id: &str, cpu_time: f64, now: Timestamp) -> Option<f64> {
        let mut seen = self.cpu.lock().unwrap_or_else(PoisonError::into_inner);
        let was = seen.insert(id.to_owned(), (cpu_time, now));
        let (before, at) = was?;
        let seconds = now.duration_since(at).as_secs_f64();
        (seconds > 0.0).then(|| ((cpu_time - before) / seconds * 100.0).max(0.0).round())
    }

    /// Stops the job `id` (the app asked the person first).
    ///
    /// # Errors
    ///
    /// [`JobError::Refused`] when it is gone or can't be stopped from here;
    /// [`JobError::Session`] when its runner fails.
    pub async fn stop(&self, id: &str) -> Result<(), JobError> {
        let list = self.now(true).await?;
        let Some(job) = list.jobs.iter().find(|job| job.id.as_str() == id) else {
            return Err(JobError::Refused(
                "that job isn't running anymore".to_owned(),
            ));
        };
        if !job.stop {
            return Err(JobError::Refused(
                "this one can't be stopped from here".to_owned(),
            ));
        }
        if let Some(session) = id.strip_prefix("session:") {
            self.runners.halt(&SessionId::parse(session)?, true).await?;
        } else if let Some((machine, pid)) = id.split_once(":pid:") {
            let pid: i64 = pid
                .parse()
                .map_err(|_| JobError::Refused("not a job's id".to_owned()))?;
            self.runners
                .api()
                .call(
                    machine,
                    Verb::Post,
                    "/jobs/stop",
                    Some(json!({ "pid": pid })),
                )
                .await
                .map_err(SessionError::from)?;
        } else {
            return Err(JobError::Refused(
                "this one can't be stopped from here".to_owned(),
            ));
        }
        *self.kept.lock().unwrap_or_else(PoisonError::into_inner) = None;
        if let Err(error) = self
            .activity
            .note(format!(
                "Stopped the background job {} from the app",
                job.name
            ))
            .await
        {
            tracing::warn!(%error, "an activity line could not be written");
        }
        Ok(())
    }
}
