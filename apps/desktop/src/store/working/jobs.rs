//! The background jobs on the machines: asked for every 10 s while their list or one of their
//! pages shows in the front window, and at once when one opens or the window comes back to the
//! front. A job's page keeps the job as it was last seen once it ends, so the page doesn't
//! vanish under the pointer. Stop and Clear ask first, go one at a time, and the list comes
//! back fresh after; a refusal says why on the job's page.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use emberhut_api::ids::MachineJobId;
use emberhut_api::jobs::{Job, JobAction, JobState, JobsPage};
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use jiff::Timestamp;

use super::keys::{Key, Keys};
use crate::backend::Error;
use crate::brand::APP_NAME;
use crate::store::poll::{self, Kicked, Kicker};
use crate::store::presence::Presence;
use crate::store::{Page, Store, update_or_log};

/// How often the jobs are asked for while they show.
const EVERY: Duration = Duration::from_secs(10);

/// A job's page key.
pub(crate) type JobKey = Key<MachineJobId>;

/// The jobs, as last fetched.
#[derive(Debug)]
pub(crate) struct JobsFeed {
    page: Option<JobsPage>,
    problem: Option<String>,
    /// The job whose Stop or Clear is on its way.
    busy: Option<MachineJobId>,
    /// Why the last Stop or Clear wasn't done, and on which job.
    refused: Option<(MachineJobId, String)>,
    /// The Stop or Clear waiting for the user's yes.
    confirming: Option<(MachineJobId, JobAction)>,
    /// The jobs whose pages are open, each as last seen, for its page once it is gone from
    /// the list; a job is let go with its last page.
    seen: HashMap<MachineJobId, Job>,
    /// Whether the services are unfolded.
    services_open: bool,
    keys: Keys<MachineJobId>,
    kicker: Kicker,
    kicked: Option<Kicked>,
}

impl Default for JobsFeed {
    fn default() -> Self {
        let (kicker, waiting) = poll::kicker();
        Self {
            page: None,
            problem: None,
            busy: None,
            refused: None,
            confirming: None,
            seen: HashMap::new(),
            services_open: false,
            keys: Keys::default(),
            kicker,
            kicked: Some(waiting),
        }
    }
}

/// A job as its page shows it: the one listed now, or the last seen of one that ended.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Shown<'a> {
    /// It is on the list.
    Listed(&'a Job),
    /// It ended: the last seen of it.
    Gone(&'a Job),
}

impl JobsFeed {
    /// The last list, if one came.
    #[must_use]
    pub(crate) fn page(&self) -> Option<&JobsPage> {
        self.page.as_ref()
    }

    /// Why the last fetch failed, worded for the user.
    #[must_use]
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// The job whose Stop or Clear is on its way.
    #[must_use]
    pub(crate) fn busy(&self) -> Option<&MachineJobId> {
        self.busy.as_ref()
    }

    /// Why the last Stop or Clear on `id` wasn't done.
    #[must_use]
    pub(crate) fn refused(&self, id: &MachineJobId) -> Option<&str> {
        self.refused
            .as_ref()
            .filter(|(on, _)| on == id)
            .map(|(_, why)| why.as_str())
    }

    /// The Stop or Clear on `id` waiting for the user's yes.
    #[must_use]
    pub(crate) fn confirming(&self, id: &MachineJobId) -> Option<JobAction> {
        self.confirming
            .as_ref()
            .filter(|(on, _)| on == id)
            .map(|(_, action)| *action)
    }

    /// Whether the services are unfolded.
    #[must_use]
    pub(crate) fn services_open(&self) -> bool {
        self.services_open
    }

    /// The job of `key`'s page, if it was ever seen.
    #[must_use]
    pub(crate) fn shown(&self, key: JobKey) -> Option<Shown<'_>> {
        let id = self.keys.get(key)?;
        let listed = self
            .page
            .as_ref()
            .and_then(|page| page.jobs.iter().find(|job| &job.id == id));
        match listed {
            Some(job) => Some(Shown::Listed(job)),
            None => self.seen.get(id).map(Shown::Gone),
        }
    }

    /// Asks for the jobs now if they are wanted, rather than at the next tick.
    pub(super) fn kick(&self) {
        self.kicker.kick();
    }

    /// Takes a fetch's outcome in; whether anything changed. A failure keeps the last list.
    /// Each job whose page is `open` is kept as last seen.
    fn take(&mut self, page: Result<JobsPage, Error>, open: &[MachineJobId]) -> bool {
        let (page, problem) = match page {
            Ok(page) => (Some(page), None),
            Err(error) => (self.page.take(), Some(read_problem(&error))),
        };
        // The last list first, so a job that left it between two fetches keeps its page.
        for job in self.page.iter().chain(page.iter()).flat_map(|p| &p.jobs) {
            if open.contains(&job.id) {
                self.seen.insert(job.id.clone(), job.clone());
            }
        }
        self.forget_all_but(open);
        let changed = self.page != page || self.problem != problem;
        (self.page, self.problem) = (page, problem);
        changed
    }

    /// Lets go of the jobs last seen whose pages are not `open`.
    fn forget_all_but(&mut self, open: &[MachineJobId]) {
        self.seen.retain(|id, _| open.contains(id));
    }
}

/// Why the jobs couldn't be read, as the reference client words it.
fn read_problem(error: &Error) -> String {
    match error {
        Error::Unreachable(_) => format!("Can't reach {APP_NAME}"),
        Error::Refused {
            status: Some(404), ..
        } => format!("{APP_NAME}'s server is on older code: the list comes with its next update"),
        other => other
            .server_message()
            .map_or_else(|| "Couldn't read the jobs".to_owned(), str::to_owned),
    }
}

/// The list's two parts: the work (failed first), then the services and file servers, folded.
#[must_use]
pub(crate) fn split(jobs: &[Job]) -> (Vec<&Job>, Vec<&Job>) {
    let (mut work, services): (Vec<&Job>, Vec<&Job>) = jobs
        .iter()
        .partition(|job| job.is_failed() || !job.is_quiet());
    work.sort_by_key(|job| !job.is_failed());
    (work, services)
}

/// A span of seconds, short: `45s`, `7m`, `1h 7m`, `3d 2h`.
#[must_use]
pub(crate) fn short_span(seconds: f64) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a job's span in seconds is far inside i64's range; NaN reads 0"
    )]
    let s = seconds.max(0.) as i64;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h {}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d {}h", s / 86_400, s % 86_400 / 3600),
    }
}

/// `text` as a time, if it reads as one.
fn time(text: Option<&emberhut_api::time::Timestamp>) -> Option<Timestamp> {
    text?.as_str().parse().ok()
}

/// How long it has run (or ran, once it ended).
#[must_use]
pub(crate) fn ran(job: &Job, now: Timestamp) -> Option<String> {
    let since = time(job.since.as_ref())?;
    let end = time(job.ended.as_ref()).unwrap_or(now);
    #[expect(
        clippy::cast_precision_loss,
        reason = "seconds of a job's run are far below f64's exact range"
    )]
    let seconds = (end.as_second() - since.as_second()) as f64;
    Some(short_span(seconds))
}

/// Its memory, as sizes read in the app.
#[must_use]
pub(crate) fn memory(job: &Job) -> Option<String> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a job's memory in bytes reads to a tenth of a GB"
    )]
    job.memory
        .map(|bytes| crate::format::gigabytes(Some(bytes as f64)))
}

/// Its CPU: now, else its CPU time.
#[must_use]
pub(crate) fn cpu(job: &Job) -> Option<String> {
    job.cpu
        .map(|cpu| format!("CPU {}%", crate::format::whole(cpu)))
        .or_else(|| job.cpu_time.map(|t| format!("CPU time {}", short_span(t))))
}

/// Its state in a word.
#[must_use]
pub(crate) fn state_word(state: &JobState) -> String {
    match state {
        JobState::Running => "Running".to_owned(),
        JobState::Starting => "Starting".to_owned(),
        JobState::Failed => "Failed".to_owned(),
        JobState::Exited => "Ended".to_owned(),
        other => crate::format::capitalized(other.as_str()),
    }
}

/// A row's facts: `Running 1h 7m · CPU 12% · 4.9 GB · Mac`, `Failed · exit-code · Mac`.
#[must_use]
pub(crate) fn facts(job: &Job, now: Timestamp) -> String {
    let mac = (job.place == "Mac").then(|| "Mac".to_owned());
    let parts: Vec<String> = if job.is_failed() {
        [Some("Failed".to_owned()), job.exit.clone(), mac]
            .into_iter()
            .flatten()
            .collect()
    } else {
        let ran = ran(job, now);
        let first = if job.is_live() {
            ran.map(|ran| format!("Running {ran}"))
        } else {
            Some(ran.map_or_else(|| "Ended".to_owned(), |ran| format!("Ended · ran {ran}")))
        };
        [first, cpu(job), memory(job), mac]
            .into_iter()
            .flatten()
            .collect()
    };
    parts.join(" · ")
}

/// When it was last active, for its row.
#[must_use]
pub(crate) fn last_active(job: &Job) -> Option<Timestamp> {
    time(job.active.as_ref()).or_else(|| time(job.last.as_ref()?.at.as_ref()))
}

/// The question before a Stop or Clear, and what it says.
#[must_use]
pub(crate) fn confirm_words(job: &Job, action: JobAction, key: Option<&str>) -> (String, String) {
    match action {
        JobAction::Clear => (
            format!("Clear {}?", job.name),
            "It failed and stays off; this only takes it off the list.".to_owned(),
        ),
        JobAction::Stop => {
            let task = key.map_or_default(|key| format!(". {key} stays open"));
            (
                format!("Stop {}?", job.name),
                format!(
                    "{} on the {} ends now{task}. This can't be undone.",
                    job.what, job.place
                ),
            )
        }
    }
}

/// Whether to ask for the jobs now: while their list or a job's page shows, in front.
#[must_use]
fn wanted(on_page: bool, presence: Presence) -> bool {
    on_page && presence == Presence::Front
}

impl Store {
    /// Whether the jobs are [`wanted`] now.
    fn jobs_wanted(&self) -> bool {
        let on_page = self
            .columns
            .shows_any(|page| matches!(page, Page::Jobs | Page::Job(_)));
        wanted(on_page, self.presence())
    }

    /// The jobs whose pages are in a column, shown or under another.
    fn open_jobs(&self) -> Vec<MachineJobId> {
        self.working
            .jobs
            .keys
            .iter()
            .filter(|(key, _)| self.columns.holds(Page::Job(*key)))
            .map(|(_, id)| id.clone())
            .collect()
    }

    /// Takes a fetch of the jobs in; whether anything changed.
    fn take_jobs(&mut self, page: Result<JobsPage, Error>) -> bool {
        let open = self.open_jobs();
        self.working.jobs.take(page, &open)
    }

    /// Lets go of the ended jobs whose pages are closed.
    pub(in crate::store) fn forget_closed_jobs(&mut self) {
        let open = self.open_jobs();
        self.working.jobs.forget_all_but(&open);
    }

    /// Starts the jobs' loop (once).
    pub(super) fn start_jobs(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(kicked) = self.working.jobs.kicked.take() {
            super::follow::follow(
                super::follow::Feed {
                    every: EVERY,
                    wanted: Self::jobs_wanted,
                    fetch: |backend| backend.jobs(false),
                    take: Self::take_jobs,
                },
                kicked,
                cx,
            );
        }
    }

    /// The jobs.
    #[must_use]
    pub(crate) fn jobs(&self) -> &JobsFeed {
        &self.working.jobs
    }

    /// The page key of job `id`, to open its page in a column.
    pub(crate) fn job_key(&mut self, id: MachineJobId) -> JobKey {
        self.working.jobs.keys.key(id)
    }

    /// Unfolds the services, or folds them again.
    pub(crate) fn toggle_services(&mut self, cx: &mut Context<'_, Self>) {
        self.working.jobs.services_open = !self.working.jobs.services_open;
        cx.notify();
    }

    /// Asks for the user's yes to `action` on `id` (`None`: they said no).
    pub(crate) fn confirm_job(
        &mut self,
        ask: Option<(MachineJobId, JobAction)>,
        cx: &mut Context<'_, Self>,
    ) {
        self.working.jobs.confirming = ask;
        cx.notify();
    }

    /// Stops job `id`, or clears it (the user said yes); one at a time. The list comes back
    /// fresh after.
    pub(crate) fn act_on_job(
        &mut self,
        id: MachineJobId,
        action: JobAction,
        cx: &mut Context<'_, Self>,
    ) {
        let feed = &mut self.working.jobs;
        if feed.busy.is_some() {
            return;
        }
        feed.confirming = None;
        feed.refused = None;
        feed.busy = Some(id.clone());
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let (done, page) = cx
                .background_spawn(async move {
                    let done = backend.act_on_job(action, &id).map_err(|e| (id, e));
                    (done, backend.jobs(true))
                })
                .await;
            update_or_log(&this, cx, |store, cx| {
                let feed = &mut store.working.jobs;
                feed.busy = None;
                if let Err((id, error)) = done {
                    let why = error
                        .server_message()
                        .map_or_else(|| format!("{APP_NAME} couldn't do that"), str::to_owned);
                    feed.refused = Some((id, why));
                }
                store.take_jobs(page);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn job(id: &str, more: &serde_json::Value) -> Job {
        let mut base = json!({"id": id, "kind": "unit", "place": "Linux PC", "name": id,
            "title": "A run", "what": "Codex run", "task": null, "state": "running",
            "since": "2026-11-24T20:53:00Z", "cpu": 12.4, "memory": 5_261_334_938_u64,
            "stop": true, "clear": false});
        if let (Some(base), Some(more)) = (base.as_object_mut(), more.as_object()) {
            base.extend(more.clone());
        }
        serde_json::from_value(base).unwrap()
    }

    fn now() -> Timestamp {
        "2026-11-24T22:00:00Z".parse().unwrap()
    }

    #[test]
    fn spans_read_short() {
        assert_eq!(short_span(45.), "45s");
        assert_eq!(short_span(420.), "7m");
        assert_eq!(short_span(4020.), "1h 7m");
        assert_eq!(short_span(266_400.), "3d 2h");
        assert_eq!(short_span(-5.), "0s");
    }

    #[test]
    fn a_rows_facts_say_how_it_runs() {
        assert_eq!(
            facts(&job("a", &json!({})), now()),
            "Running 1h 7m · CPU 12% · 4.9 GB"
        );
        assert_eq!(
            facts(
                &job(
                    "b",
                    &json!({"state": "failed", "exit": "exit-code (exit 1)", "place": "Mac"})
                ),
                now()
            ),
            "Failed · exit-code (exit 1) · Mac"
        );
        assert_eq!(
            facts(
                &job(
                    "c",
                    &json!({"state": "exited", "ended": "2026-11-24T21:00:00Z", "cpu": null,
                    "cpuTime": 30, "memory": null})
                ),
                now()
            ),
            "Ended · ran 7m · CPU time 30s"
        );
    }

    #[test]
    fn failed_work_comes_first_and_services_fold() {
        let jobs = [
            job("work", &json!({})),
            job("service", &json!({"kind": "service"})),
            job("failed", &json!({"state": "failed"})),
            job(
                "failed-service",
                &json!({"kind": "service", "state": "failed"}),
            ),
        ];
        let (work, services) = split(&jobs);
        let ids: Vec<&str> = work.iter().map(|job| job.id.as_str()).collect();
        assert_eq!(ids, ["failed", "failed-service", "work"]);
        assert_eq!(services.len(), 1);
    }

    #[test]
    fn a_job_that_ends_keeps_its_page() {
        let mut feed = JobsFeed::default();
        let key = feed.keys.key(MachineJobId::new("a"));
        let page = |jobs: Vec<Job>| JobsPage {
            at: emberhut_api::time::Timestamp::new("2026-11-24T22:00:00Z"),
            jobs,
            mac: emberhut_api::jobs::JobsMac::default(),
        };
        let open = [MachineJobId::new("a")];
        let both = || page(vec![job("a", &json!({})), job("b", &json!({}))]);
        assert!(feed.take(Ok(both()), &[]));
        assert!(feed.seen.is_empty(), "no page open: nothing kept");
        assert!(matches!(feed.shown(key), Some(Shown::Listed(_))));
        assert!(feed.take(Ok(page(Vec::new())), &open));
        assert!(
            matches!(feed.shown(key), Some(Shown::Gone(_))),
            "its page opened after the last list: still kept"
        );
        assert!(feed.take(
            Err(Error::Refused {
                status: Some(404),
                message: None
            }),
            &open
        ));
        assert!(matches!(feed.shown(key), Some(Shown::Gone(_))));
        feed.forget_all_but(&[]);
        assert_eq!(feed.shown(key).map(|_| ()), None, "let go with its page");
        assert_eq!(
            feed.problem(),
            Some("Emberhut's server is on older code: the list comes with its next update")
        );
    }

    #[test]
    fn stopping_names_the_task_that_stays_open() {
        let (title, words) = confirm_words(&job("a", &json!({})), JobAction::Stop, Some("EX-12"));
        assert_eq!(title, "Stop a?");
        assert_eq!(
            words,
            "Codex run on the Linux PC ends now. EX-12 stays open. This can't be undone."
        );
        let (title, _) = confirm_words(&job("a", &json!({})), JobAction::Clear, None);
        assert_eq!(title, "Clear a?");
    }
}
