//! Tasks: the Tasks list (its sections, filter, search and archived list) and the pages of the
//! tasks the windows' columns hold (their fetch, their reply boxes, the actions and changes the
//! user makes).
//!
//! The submodules with no GPUI in them hold the decisions and are tested on their own; this one
//! runs them against the backend.

pub(crate) mod activity;
pub(crate) mod agents;
pub(crate) mod clock;
pub(crate) mod counts;
pub(crate) mod filter;
pub(crate) mod findings;
pub(crate) mod history;
pub(crate) mod list;
pub(crate) mod loose;
pub(crate) mod pages;
pub(crate) mod plan;
pub(crate) mod replies;
pub(crate) mod runs;
pub(crate) mod search;
#[cfg(test)]
mod tests;
pub(crate) mod tick;
pub(crate) mod words;

use std::collections::HashSet;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use emberhut_api::ids::TaskId;
use emberhut_api::task_actions::{TaskAction, TaskChange};
use emberhut_api::task_message::TaskMessageRequest;
use emberhut_api::task_search::TaskQuery;
use emberhut_api::tasks::Issue;
use emberhut_api::time::Timestamp;
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use jiff::ZonedDateTime;

use self::filter::TaskFilter;
use self::pages::{LIVE_EVERY, Missing, PageStatus, Seen, TaskPages};
use self::replies::{Replies, Reply};
use self::search::{
    ARCHIVED_LIMIT, SEARCH_LIMIT, StateTasks, TYPING_PAUSE, TaskLookup, TaskSearch, Ticket,
};
use super::{Page, Store, lookup, update_or_log};
use crate::backend::{Error, UploadSource};
use crate::config::AppFiles;
use crate::json_file::Writes;

/// How long a note about a change that didn't go through stays up.
const NOTICE_FOR: Duration = Duration::from_secs(6);

/// How long after a message on a task its page is asked for again, so what it started shows.
const AFTER_MESSAGE: Duration = Duration::from_secs(1);

/// Everything the store keeps for the Tasks list and the tasks' pages.
#[derive(Debug)]
pub(crate) struct TaskWork {
    pages: TaskPages,
    search: TaskSearch,
    archived: TaskLookup,
    archived_open: bool,
    /// The tasks the last state held: the search and the archived list are asked again when
    /// they change.
    state_tasks: StateTasks,
    filter: TaskFilter,
    /// The numbers of the writes of the kept filter, so the newest is the one on disk.
    filter_writes: Writes,
    replies: Replies,
    notice: Option<String>,
    notice_ticket: u64,
    subtasks_open: HashSet<TaskId>,
    composer_start: Option<String>,
    /// The generation of the loop that asks for a live page again: a new path starts a new one.
    live_poll: u64,
    /// The ticks whose check still shows.
    ticks: tick::Ticks,
}

impl TaskWork {
    /// The task work at launch, with the filter kept on this device.
    pub(crate) fn new(files: Option<&AppFiles>) -> Self {
        Self {
            pages: TaskPages::default(),
            search: TaskSearch::default(),
            archived: TaskLookup::default(),
            archived_open: false,
            state_tasks: StateTasks::default(),
            filter: filter::load(files),
            filter_writes: Writes::default(),
            replies: Replies::default(),
            notice: None,
            notice_ticket: 0,
            subtasks_open: HashSet::new(),
            composer_start: None,
            live_poll: 0,
            ticks: tick::Ticks::default(),
        }
    }

    /// Where `task`'s page stands.
    #[must_use]
    pub(crate) fn page_status(&self, task: TaskId) -> PageStatus<'_> {
        self.pages.status(task)
    }

    /// The search over every task.
    #[must_use]
    pub(crate) fn search(&self) -> &TaskSearch {
        &self.search
    }

    /// The archived list, when it is open.
    #[must_use]
    pub(crate) fn archived(&self) -> Option<&TaskLookup> {
        self.archived_open.then_some(&self.archived)
    }

    /// `task`'s reply box.
    #[must_use]
    pub(crate) fn reply(&self, task: TaskId) -> Reply {
        self.replies.get(task)
    }

    /// The note about a change that didn't go through, while it shows.
    #[must_use]
    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Whether `task`'s page has agents running now.
    #[must_use]
    pub(crate) fn is_live(&self, task: TaskId) -> bool {
        self.pages.is_live(task)
    }

    /// Whether `parent`'s subtasks show under it in the list.
    #[must_use]
    pub(crate) fn shows_subtasks(&self, parent: TaskId) -> bool {
        self.subtasks_open.contains(&parent)
    }
}

impl Store {
    /// The Tasks list and the tasks' pages.
    #[must_use]
    pub(crate) fn tasks(&self) -> &TaskWork {
        &self.tasks
    }

    /// Whether the projects are hidden (the server's switch; hidden until it says).
    #[must_use]
    pub(crate) fn hides_projects(&self) -> bool {
        self.state.as_ref().is_none_or(lookup::shows_tasks)
    }

    /// Whether the server keeps plans: tasks show their plans and the sessions move out of the
    /// main navigation ([`lookup::keeps_plans`]); off until a state says so.
    #[must_use]
    pub(crate) fn keeps_plans(&self) -> bool {
        self.state.as_ref().is_some_and(lookup::keeps_plans)
    }

    /// The filter that applies: a project pick is dropped while projects are hidden.
    #[must_use]
    pub(crate) fn task_filter(&self) -> TaskFilter {
        self.tasks.filter.shown(self.hides_projects())
    }

    /// Task `id` as the column shows it: the state's (its live fields are the newest), else its
    /// fetched page's (a task done weeks ago is only there).
    #[must_use]
    pub(crate) fn task_of(&self, id: TaskId) -> Option<&Issue> {
        self.state
            .as_ref()
            .and_then(|state| lookup::issue(state, id))
            .or_else(|| self.tasks.pages.page(id).map(|page| &page.task))
    }

    /// A task's key, for words that name it (`T-4`); its number when it isn't known here.
    #[must_use]
    pub(crate) fn task_key(&self, id: TaskId) -> String {
        self.task_of(id)
            .map_or_else(|| format!("#{id}"), |task| task.key.clone())
    }

    /// The tasks the windows' columns hold changed: pages off every column are dropped, the
    /// ones shown are asked for, and a new loop asks for them again while their agents run.
    pub(super) fn tasks_on_screen(&mut self, cx: &mut Context<'_, Self>) {
        let columns = &self.columns;
        self.tasks.pages.retain(|task| columns.holds_task(task));
        self.tasks.live_poll += 1;
        let shown = self.columns.shown_tasks();
        if shown.is_empty() {
            return;
        }
        for task in shown {
            self.fetch_task_page(task, false, cx);
        }
        Self::watch_live_page(self.tasks.live_poll, cx);
    }

    /// Asks for `task`'s page if [`TaskPages::begin`] says so (`again`: whatever it says of
    /// changes).
    fn fetch_task_page(&mut self, task: TaskId, again: bool, cx: &mut Context<'_, Self>) {
        let seen = self
            .state
            .as_ref()
            .and_then(|state| lookup::issue(state, task))
            .map(Seen::of);
        if !self.tasks.pages.begin(task, seen.as_ref(), again) {
            return;
        }
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let page = cx.background_spawn(async move { backend.task(task) }).await;
            update_or_log(&this, cx, |store, cx| {
                let answer = page.map_err(|error| match error {
                    Error::Refused {
                        status: Some(404), ..
                    } => Missing::NotFound,
                    other => {
                        log::warn!("couldn't fetch task {task}'s page: {other}");
                        Missing::Failed(super::status::problem(&other))
                    }
                });
                store.tasks.pages.finish(task, answer);
                store.fetch_task_pictures(task, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks for the pages shown again now: the user changed a task.
    pub(super) fn refetch_open_task(&mut self, cx: &mut Context<'_, Self>) {
        for task in self.columns.shown_tasks() {
            self.fetch_task_page(task, true, cx);
        }
    }

    /// A new state: the page shown is asked for again if its task changed in it; the tasks
    /// found show as the state has them, and the search and the archived list are asked again
    /// when its tasks changed (one unarchived is back in it, one archived or added is new).
    pub(super) fn tasks_follow_state(&mut self, cx: &mut Context<'_, Self>) {
        for task in self.columns.shown_tasks() {
            self.fetch_task_page(task, false, cx);
        }
        let Some(state) = self.state.as_ref() else {
            return;
        };
        self.tasks.search.lookup_mut().follow(&state.issues);
        self.tasks.archived.drop_listed(&state.issues);
        if !self.tasks.state_tasks.changed(&state.issues) {
            return;
        }
        if let Some(ticket) = self.tasks.search.lookup_mut().refresh() {
            Self::ask_search(ticket, Duration::ZERO, cx);
        }
        if let Some(ticket) = self.tasks.archived.refresh() {
            self.fetch_archived(ticket, cx);
        }
    }

    /// Every few seconds while the columns show the pages of this `generation`, asks for each
    /// again whose agents run.
    fn watch_live_page(generation: u64, cx: &mut Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(LIVE_EVERY).await;
                let going = update_or_log(&this, cx, |store, cx| {
                    if store.tasks.live_poll != generation {
                        return false;
                    }
                    for task in store.columns.shown_tasks() {
                        if store.tasks.pages.is_live(task) {
                            store.fetch_task_page(task, true, cx);
                        }
                    }
                    true
                });
                if going != Some(true) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Fetches the thumbnails of what `task` produced that aren't here yet.
    fn fetch_task_pictures(&mut self, task: TaskId, cx: &mut Context<'_, Self>) {
        let thumbs: Vec<String> = self
            .task_of(task)
            .into_iter()
            .chain(self.tasks.pages.page(task).map(|page| &page.task))
            .flat_map(|issue| &issue.artifacts)
            .filter_map(|artifact| artifact.thumb.clone())
            .filter(|thumb| crate::config::ServerUrl::is_path(thumb))
            .collect();
        let room = self.room;
        for thumb in thumbs {
            self.fetch_picture(room, thumb, cx);
        }
    }

    /// The user marks their task done: it leaves the lists at once and goes once its Undo
    /// window runs out.
    pub(crate) fn mark_done(&mut self, task: TaskId, cx: &mut Context<'_, Self>) {
        let held = self.answers.act(task, TaskAction::mark_done(task));
        self.send_when_due(task, held, cx);
    }

    /// Not now: the task is put off until a week from today, held with Undo.
    pub(crate) fn not_now(&mut self, task: TaskId, cx: &mut Context<'_, Self>) {
        let until = clock::next_week(&ZonedDateTime::now());
        let action = TaskAction::park(task, Timestamp::new(until.to_string()));
        let held = self.answers.act(task, action);
        self.send_when_due(task, held, cx);
    }

    /// Archive: the task is put away for good with its open subtasks, held with Undo.
    pub(crate) fn archive(&mut self, task: TaskId, cx: &mut Context<'_, Self>) {
        let held = self.answers.act(task, TaskAction::archive(task));
        self.send_when_due(task, held, cx);
    }

    /// Unarchive: the task is back in to do, held with Undo.
    pub(crate) fn unarchive(&mut self, task: TaskId, cx: &mut Context<'_, Self>) {
        let held = self.answers.act(task, TaskAction::unarchive(task));
        self.send_when_due(task, held, cx);
    }

    /// Whether `task` was acted on here and is off the lists for now.
    #[must_use]
    pub(crate) fn is_gone(&self, task: TaskId) -> bool {
        self.answers.gone().contains(&task)
    }

    /// Changes a task (its dates, size, notes, ultra mode); sent at once.
    pub(crate) fn change_task(&mut self, change: TaskChange, cx: &mut Context<'_, Self>) {
        self.act_now(TaskAction::from(change), "Task not saved", cx);
    }

    /// Stops the worker on `task`: its current run ends where it is.
    pub(crate) fn stop_worker(&mut self, task: TaskId, cx: &mut Context<'_, Self>) {
        self.act_now(TaskAction::stop_worker(task), "Worker not stopped", cx);
    }

    /// Adds a task of the user's own titled `title`; a blank title adds nothing (`false`).
    pub(crate) fn create_task(&mut self, title: &str, cx: &mut Context<'_, Self>) -> bool {
        let title = title.trim();
        if title.is_empty() {
            return false;
        }
        self.act_now(TaskAction::create(title), "Task not added", cx);
        true
    }

    /// Sends `action` now; a refusal shows as a note with the server's reason.
    fn act_now(&mut self, action: TaskAction, failure: &'static str, cx: &mut Context<'_, Self>) {
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        let sending = self.in_flight.start();
        let request = cx.background_spawn(async move {
            let sent = backend.act_on_tasks(&action);
            drop(sending);
            sent
        });
        cx.spawn(async move |this, cx| {
            let sent = request.await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                if let Err(error) = sent {
                    log::warn!("a task change wasn't sent: {error}");
                    store.show_notice(format!("{failure}: {}", reason(&error)), cx);
                }
                store.refetch_open_task(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Shows `text` as the note, for a few seconds.
    pub(crate) fn show_notice(&mut self, text: String, cx: &mut Context<'_, Self>) {
        self.tasks.notice_ticket += 1;
        let ticket = self.tasks.notice_ticket;
        self.tasks.notice = Some(text);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTICE_FOR).await;
            update_or_log(&this, cx, |store, cx| {
                if store.tasks.notice_ticket == ticket {
                    store.tasks.notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// The user typed `text` in the search field: it asks the server once the typing pauses.
    pub(crate) fn search_tasks(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        if self.tasks.search.text() == text {
            return;
        }
        if let Some(ticket) = self.tasks.search.set_text(text) {
            Self::ask_search(ticket, TYPING_PAUSE, cx);
        }
        cx.notify();
    }

    /// Asks the search with `ticket` after `pause` (the typing's), if it is still the latest.
    fn ask_search(ticket: Ticket, pause: Duration, cx: &mut Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            if !pause.is_zero() {
                cx.background_executor().timer(pause).await;
            }
            let asked = update_or_log(&this, cx, |store, _| {
                store.tasks.search.lookup().is_current(ticket).then(|| {
                    let query = TaskQuery {
                        limit: Some(SEARCH_LIMIT),
                        ..TaskQuery::search(store.tasks.search.text().trim())
                    };
                    (
                        Arc::clone(&store.backend),
                        store.task_filter().narrow(query),
                    )
                })
            })
            .flatten();
            let Some((backend, query)) = asked else {
                return;
            };
            let found = cx
                .background_spawn(async move { backend.find_tasks(&query) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store
                    .tasks
                    .search
                    .lookup_mut()
                    .finish(ticket, found.map_err(|error| reason(&error)));
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the archived list (it asks the server for them) or folds it.
    pub(crate) fn toggle_archived_tasks(&mut self, cx: &mut Context<'_, Self>) {
        self.tasks.archived_open = !self.tasks.archived_open;
        if self.tasks.archived_open {
            self.ask_archived(cx);
        } else {
            self.tasks.archived.stop();
        }
        cx.notify();
    }

    /// Asks for the archived list anew: nothing shows until it comes.
    fn ask_archived(&mut self, cx: &mut Context<'_, Self>) {
        let ticket = self.tasks.archived.ask();
        self.fetch_archived(ticket, cx);
    }

    /// Asks the server for the archived tasks, answering `ticket`.
    fn fetch_archived(&mut self, ticket: Ticket, cx: &mut Context<'_, Self>) {
        // The filter's project and owner narrow it; its status and day don't (they're all
        // archived, and have no day to decide).
        let filter = TaskFilter {
            status: None,
            when: None,
            ..self.task_filter()
        };
        let query = filter.narrow(TaskQuery {
            archived: true,
            limit: Some(ARCHIVED_LIMIT),
            ..TaskQuery::default()
        });
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { backend.find_tasks(&query) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store
                    .tasks
                    .archived
                    .finish(ticket, found.map_err(|error| reason(&error)));
                cx.notify();
            });
        })
        .detach();
    }

    /// The user picked `filter` on the Tasks list: it is kept on this device (written off the
    /// UI thread), and a search or archived list on is asked anew.
    pub(crate) fn set_task_filter(&mut self, filter: TaskFilter, cx: &mut Context<'_, Self>) {
        if self.tasks.filter == filter {
            return;
        }
        if let Some(files) = self.files.clone() {
            let (number, written) = self.tasks.filter_writes.next();
            let kept = filter.clone();
            cx.background_spawn(async move { filter::save(&files, &kept, number, &written) })
                .detach();
        }
        self.tasks.filter = filter;
        if let Some(ticket) = self.tasks.search.again() {
            Self::ask_search(ticket, TYPING_PAUSE, cx);
        }
        if self.tasks.archived_open {
            self.ask_archived(cx);
        }
        cx.notify();
    }

    /// Shows `parent`'s subtasks under it in the list, or folds them.
    pub(crate) fn toggle_subtasks(&mut self, parent: TaskId, cx: &mut Context<'_, Self>) {
        if !self.tasks.subtasks_open.remove(&parent) {
            self.tasks.subtasks_open.insert(parent);
        }
        cx.notify();
    }

    /// Adds the photos at `paths` to `task`'s reply box (as many as it still takes), each
    /// stored on the server as it is added.
    pub(crate) fn add_task_photos(
        &mut self,
        task: TaskId,
        paths: Vec<PathBuf>,
        cx: &mut Context<'_, Self>,
    ) {
        let room = self.tasks.replies.get(task).room_for();
        for path in paths.into_iter().take(room) {
            self.tasks.replies.of(task).uploading += 1;
            let name = path.file_name().map_or_else(
                || "photo".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            let backend = Arc::clone(&self.backend);
            cx.spawn(async move |this, cx| {
                let stored = cx
                    .background_spawn(async move {
                        backend.upload(&name, &UploadSource::File(path), &mut |_| {
                            ControlFlow::Continue(())
                        })
                    })
                    .await;
                update_or_log(&this, cx, |store, cx| {
                    let reply = store.tasks.replies.of(task);
                    reply.uploading = reply.uploading.saturating_sub(1);
                    match stored {
                        Ok(file) => reply.files.push(file),
                        Err(error) => {
                            log::warn!("a photo for task {task} didn't upload: {error}");
                            reply.note = Some("The photo didn't upload; try again".to_owned());
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    /// Takes the photo stored at `url` out of `task`'s reply box.
    pub(crate) fn remove_task_photo(
        &mut self,
        task: TaskId,
        url: &str,
        cx: &mut Context<'_, Self>,
    ) {
        self.tasks
            .replies
            .of(task)
            .files
            .retain(|file| file.url != url);
        cx.notify();
    }

    /// Sends `text` and the photos in `task`'s reply box to the task (its worker, its session
    /// or the assistant in its room; never Main). Whether it went out (then the reply box's
    /// `sent` count moves once it arrived, and the view empties its field).
    pub(crate) fn send_task_message(
        &mut self,
        task: TaskId,
        text: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let reply = self.tasks.replies.of(task);
        if !reply.can_send(text) {
            return false;
        }
        reply.sending = true;
        reply.note = None;
        let request = TaskMessageRequest {
            text: text.trim().to_owned(),
            files: reply.files.clone(),
        };
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        let sending = self.in_flight.start();
        cx.spawn(async move |this, cx| {
            let sent = cx
                .background_spawn(async move {
                    let sent = backend.message_task(task, &request);
                    drop(sending);
                    sent
                })
                .await;
            kicker.kick();
            let went = update_or_log(&this, cx, |store, cx| {
                let reply = store.tasks.replies.of(task);
                reply.sending = false;
                let went = match sent {
                    Ok(answer) => {
                        reply.files.clear();
                        reply.sent += 1;
                        reply.note = Some(history::sent_note(&answer));
                        true
                    }
                    Err(error) => {
                        log::warn!("a message on task {task} wasn't sent: {error}");
                        reply.note = Some(format!("Not sent: {}", reason(&error)));
                        false
                    }
                };
                cx.notify();
                went
            });
            if went == Some(true) {
                cx.background_executor().timer(AFTER_MESSAGE).await;
                update_or_log(&this, cx, |store, cx| {
                    if store.columns.shows(Page::Task(task)) {
                        store.fetch_task_page(task, true, cx);
                    }
                });
            }
        })
        .detach();
        cx.notify();
        true
    }

    /// Talk about `task`: its project's room opens (the open room when it has none) with the
    /// composer started on `About T-4 "Its title": `.
    pub(crate) fn talk_about_task(&mut self, task: TaskId, cx: &mut Context<'_, Self>) {
        let Some(issue) = self.task_of(task) else {
            return;
        };
        let opening = format!("About {} \"{}\": ", issue.key, issue.title);
        let room = issue
            .pid
            .and_then(|pid| {
                self.state
                    .as_ref()?
                    .rooms
                    .iter()
                    .find(|room| room.project == Some(pid))
                    .map(|room| room.id)
            })
            .unwrap_or(self.room);
        self.tasks.composer_start = Some(opening);
        // The chat beside the column: the task's page stays open.
        self.switch_room(room, cx);
    }

    /// What the composer starts with, once: taken by the chat as it shows it.
    pub(crate) fn take_composer_start(&mut self) -> Option<String> {
        self.tasks.composer_start.take()
    }
}

/// The reason to show for `error`: the server's own words, else ours.
fn reason(error: &Error) -> String {
    error
        .server_message()
        .map_or_else(|| super::status::problem(error), str::to_owned)
}
