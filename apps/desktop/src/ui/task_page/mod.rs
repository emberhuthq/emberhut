//! A task's page (the case file) as the window's column shows it, its actions pinned at the
//! foot; the column's bar has Back, its key and Close. The page is asked for by the store; this
//! view keeps what is the page's own to keep: which parts are open, the reply and the notes
//! being written, the date pickers. It shows the task the column shows.

mod actions;
mod activity;
mod agents;
mod decision;
mod details;
#[cfg(test)]
mod media_tests;
mod plan;
mod reply;
mod results;
mod text;
mod thumb;
mod top;
mod width;
mod work;

use std::collections::HashSet;

use emberhut_api::ids::TaskId;
use emberhut_api::task_actions::{Edit, TaskChange};
use emberhut_api::task_page::TaskPage;
use emberhut_api::tasks::Issue;
use emberhut_api::time::Day;
use gpui_kit::assets::IconName;
use gpui_kit::component::date_picker::{DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, Entity, FontWeight, ScrollHandle, Window, div, point, px};
use jiff::ZonedDateTime;
use jiff::civil::Date;

pub(crate) use self::width::column_width;
use super::column::Column;
use super::palette::{Pal, pal};
use super::tasks::loading;
use super::tasks::marks::glyph;
use super::tasks::swipe::{Swipe, Swiped};
use crate::store::lookup;
use crate::store::tasks::pages::PageStatus;
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};

/// The keys of the page's folded parts among what is open.
pub(super) mod fold {
    /// The worker's brief.
    pub(crate) const BRIEF: &str = "fold:brief";
    /// Details.
    pub(crate) const DETAILS: &str = "fold:details";
    /// History.
    pub(crate) const HISTORY: &str = "fold:history";
    /// Since the ask.
    pub(crate) const SINCE: &str = "fold:since";
    /// A scan's findings.
    pub(crate) const FINDINGS: &str = "fold:findings";
    /// Every run, not only the latest.
    pub(crate) const ALL_RUNS: &str = "fold:all-runs";
    /// Files and results under an ask.
    pub(crate) const RESULTS: &str = "fold:results";
    /// The plan's steps folded before the current one.
    pub(crate) const PLAN_EARLIER: &str = "fold:plan-earlier";
    /// The plan's steps folded after the next ones.
    pub(crate) const PLAN_LATER: &str = "fold:plan-later";
}

/// How often a page with running agents redraws, for their timers.
const TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// How long a pressed action's tick shows before the page goes back.
const LEAVE_AFTER: std::time::Duration = std::time::Duration::from_millis(350);

/// A task's page.
pub(crate) struct TaskView {
    store: Entity<Store>,
    /// The window's column, whose task it shows.
    column: Entity<Column>,
    /// The task the page's own state is for.
    shown: Option<TaskId>,
    reply: Entity<TextareaState>,
    notes: Entity<TextareaState>,
    /// What the notes field was last filled from (the task's body then).
    notes_from: String,
    /// Edit was pressed: the notes field is up.
    writing: bool,
    /// What is open: the long texts opened with Show more and the folded parts, by their key.
    open_texts: HashSet<String>,
    /// The text being read whole, and its title.
    reading: Option<(String, String)>,
    /// Stop the worker was pressed: it asks first.
    stopping: bool,
    /// The ultra switch as the user set it, until the page has it.
    ultra: Option<bool>,
    /// The action pressed: its tick shows and the buttons wait, until the page goes back.
    tapped: Option<String>,
    /// The reply box's sent count when the field was last emptied.
    replies_seen: u64,
    deadline_picker: Entity<DatePickerState>,
    /// The task's own deadline the deadline picker was last set from.
    deadline_seen: Option<Date>,
    decide_picker: Entity<DatePickerState>,
    /// The task's decide-by day the decide-by picker was last set from.
    decide_seen: Option<Date>,
    scroll: ScrollHandle,
    /// A two-finger swipe over the page: right goes back.
    swipe: Swipe,
    /// What the activity shows (on a server that keeps plans).
    feed: crate::store::tasks::activity::Filter,
    /// How far the running marks have turned: a quarter a second while the task runs.
    turn: u8,
}

impl TaskView {
    /// The page of the task `column` shows, over `store`.
    pub(crate) fn new(
        store: Entity<Store>,
        column: Entity<Column>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let reply = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .submit_on_enter(true)
        });
        cx.subscribe_in(&reply, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { shift: false, .. } = event {
                this.send_reply(window, cx);
            }
        })
        .detach();
        let notes = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 12));
        cx.subscribe_in(&notes, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Blur) {
                this.save_notes(cx);
                this.writing = false;
                cx.notify();
            }
        })
        .detach();
        let deadline_picker = cx.new(|cx| DatePickerState::new(window, cx));
        cx.subscribe_in(
            &deadline_picker,
            window,
            |this, _, event: &DatePickerEvent, _, cx| {
                let DatePickerEvent::Change(value) = event;
                if let (Some(task), Some(date)) = (this.shown, value.start()) {
                    let hard = this.store.read(cx).task_of(task).is_some_and(|t| t.hard);
                    let change = TaskChange {
                        deadline: Edit::Set(day(date.date())),
                        hard: Some(hard),
                        ..TaskChange::of(task)
                    };
                    this.store
                        .update(cx, |store, cx| store.change_task(change, cx));
                }
            },
        )
        .detach();
        let decide_picker = cx.new(|cx| DatePickerState::new(window, cx));
        cx.subscribe_in(
            &decide_picker,
            window,
            |this, _, event: &DatePickerEvent, _, cx| {
                let DatePickerEvent::Change(value) = event;
                if let (Some(task), Some(date)) = (this.shown, value.start()) {
                    let change = TaskChange {
                        decide_by: Edit::Set(day(date.date())),
                        ..TaskChange::of(task)
                    };
                    this.store
                        .update(cx, |store, cx| store.change_task(change, cx));
                }
            },
        )
        .detach();
        cx.observe_in(&store, window, |this, _, window, cx| this.sync(window, cx))
            .detach();
        cx.observe_in(&column, window, |this, _, window, cx| this.sync(window, cx))
            .detach();
        Self {
            store,
            column,
            shown: None,
            reply,
            notes,
            notes_from: String::new(),
            writing: false,
            open_texts: HashSet::new(),
            reading: None,
            stopping: false,
            ultra: None,
            tapped: None,
            replies_seen: 0,
            deadline_picker,
            deadline_seen: None,
            decide_picker,
            decide_seen: None,
            scroll: ScrollHandle::new(),
            swipe: Swipe::default(),
            feed: crate::store::tasks::activity::Filter::default(),
            turn: 0,
        }
    }

    /// Keeps the page's own state in step with the store: another task shown starts it over
    /// (notes being written are saved first), a reply that went empties the field, the notes
    /// follow the task while they aren't being written, and the buttons come back when what
    /// was pressed was undone or refused.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let current = self.current(cx);
        if current != self.shown {
            self.save_notes(cx);
            self.start_over(current, window, cx);
            return;
        }
        let Some(task) = current else {
            return;
        };
        let store = self.store.read(cx);
        let sent = store.tasks().reply(task).sent;
        if sent != self.replies_seen {
            self.replies_seen = sent;
            self.reply
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let store = self.store.read(cx);
        let body = store
            .task_of(task)
            .and_then(|t| t.body.clone())
            .unwrap_or_default();
        let gone = store.is_gone(task);
        if !self.writing && body != self.notes_from {
            self.notes_from.clone_from(&body);
            self.notes
                .update(cx, |input, cx| input.set_value(body, window, cx));
        }
        if !gone && self.tapped.is_some() {
            self.tapped = None;
        }
        self.sync_pickers(window, cx);
        cx.notify();
    }

    /// Sets each date picker to the task's day when that day changed: a switch turned on (the
    /// day it picked), a date set elsewhere, or another task shown. Setting one sends nothing,
    /// and a day the user just picked isn't set back while its change is on its way.
    fn sync_pickers(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let store = self.store.read(cx);
        let issue = self.shown.and_then(|task| store.task_of(task));
        let own = issue.and_then(|t| t.due.as_ref()).and_then(civil);
        let by = issue.and_then(|t| t.decide_by.as_ref()).and_then(civil);
        if let Some(date) = reseed(&mut self.deadline_seen, own) {
            self.deadline_picker
                .update(cx, |picker, cx| picker.set_date(date, window, cx));
        }
        if let Some(date) = reseed(&mut self.decide_seen, by) {
            self.decide_picker
                .update(cx, |picker, cx| picker.set_date(date, window, cx));
        }
    }

    /// The task the column shows, if it shows one.
    fn current(&self, cx: &gpui_kit::App) -> Option<TaskId> {
        match self.column.read(cx).current() {
            Some(Page::Task(task)) => Some(task),
            _ => None,
        }
    }

    /// Back in the column: the page under this one, or the chat.
    fn back(&self, cx: &mut Context<'_, Self>) {
        self.column.update(cx, Column::back);
    }

    /// Starts the page's own state over for `task`.
    fn start_over(
        &mut self,
        task: Option<TaskId>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.shown = task;
        self.writing = false;
        self.open_texts.clear();
        self.reading = None;
        self.stopping = false;
        self.ultra = None;
        self.tapped = None;
        self.feed = crate::store::tasks::activity::Filter::default();
        self.scroll.set_offset(point(px(0.), px(0.)));
        if let Some(task) = task {
            Self::tick_while_live(task, cx);
        }
        let store = self.store.read(cx);
        let issue = task.and_then(|task| store.task_of(task)).cloned();
        self.replies_seen = task.map_or(0, |task| store.tasks().reply(task).sent);
        self.notes_from = issue
            .as_ref()
            .and_then(|t| t.body.clone())
            .unwrap_or_default();
        let (notes, placeholder) = (self.notes_from.clone(), notes_placeholder(issue.as_ref()));
        self.notes.update(cx, |input, cx| {
            input.set_value(notes, window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        let asks = issue
            .as_ref()
            .is_some_and(|t| t.status == emberhut_api::tasks::TaskStatus::Waiting);
        let reply_placeholder = if asks {
            "Answer in your own words…".to_owned()
        } else {
            format!("Message {} about this task…", crate::brand::APP_NAME)
        };
        self.reply.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder(reply_placeholder, window, cx);
        });
        self.deadline_seen = None;
        self.decide_seen = None;
        self.sync_pickers(window, cx);
        cx.notify();
    }

    /// While `task` shows, redraws the page every second its agents run, so their timers tick.
    fn tick_while_live(task: TaskId, cx: &mut Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let going = this.update(cx, |this, cx| {
                    if this.shown != Some(task) {
                        return false;
                    }
                    let store = this.store.read(cx);
                    let runs = store.keeps_plans()
                        && store.task_of(task).is_some_and(|task| {
                            crate::store::tasks::words::doing(task)
                                == crate::store::tasks::words::Doing::Running
                        });
                    if runs && !crate::motion::reduced(cx) {
                        this.turn = this.turn.wrapping_add(1);
                        cx.notify();
                    } else if store.tasks().is_live(task) {
                        cx.notify();
                    }
                    true
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Saves the notes if they changed: when the field is left, or the page.
    fn save_notes(&mut self, cx: &mut Context<'_, Self>) {
        let Some(task) = self.shown else {
            return;
        };
        let written = self.notes.read(cx).value().to_string();
        if written != self.notes_from {
            self.notes_from.clone_from(&written);
            let change = TaskChange {
                body: Some(written),
                ..TaskChange::of(task)
            };
            self.store
                .update(cx, |store, cx| store.change_task(change, cx));
        }
    }

    /// Sends what the reply box holds.
    fn send_reply(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(task) = self.shown else {
            return;
        };
        let text = self.reply.read(cx).value().to_string();
        self.store
            .update(cx, |store, cx| store.send_task_message(task, &text, cx));
    }

    /// Whether the long text `key` was opened with Show more.
    fn is_open(&self, key: &str) -> bool {
        self.open_texts.contains(key)
    }

    fn toggle_open(&mut self, key: &str) {
        if !self.open_texts.remove(key) {
            self.open_texts.insert(key.to_owned());
        }
    }

    /// Opens `key` (a folded part).
    fn set_open(&mut self, key: &str) {
        self.open_texts.insert(key.to_owned());
    }

    fn read_full(&mut self, title: String, text: String) {
        self.reading = Some((title, text));
    }

    fn close_reading(&mut self) {
        self.reading = None;
    }

    /// The page shown after an action: a beat for its tick, then back.
    fn leave(&mut self, name: &str, cx: &mut Context<'_, Self>) {
        self.tapped = Some(name.to_owned());
        let task = self.shown;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LEAVE_AFTER).await;
            if let Err(error) = this.update(cx, |this, cx| {
                if this.shown == task {
                    this.back(cx);
                }
            }) {
                log::debug!("the task's page is gone: {error}");
            }
        })
        .detach();
        cx.notify();
    }
}

/// The notes field's placeholder: the user's details, or the worker's brief.
fn notes_placeholder(task: Option<&Issue>) -> String {
    if task.is_some_and(lookup::is_assistants) {
        format!("What {}'s worker should do", crate::brand::APP_NAME)
    } else {
        "Add details, links, notes…".to_owned()
    }
}

/// The day to set a picker to: the task's `now`, when it isn't the day it was last set from
/// (`seen`, then kept); none when it is, or the task has none (the picker is hidden then).
fn reseed(seen: &mut Option<Date>, now: Option<Date>) -> Option<Date> {
    if *seen == now {
        return None;
    }
    *seen = now;
    now
}

/// A contract day as a date.
fn civil(day: &Day) -> Option<Date> {
    lookup::civil_day(day.as_str())
}

/// A date as a contract day.
fn day(date: Date) -> Day {
    Day::new(date.strftime("%Y-%m-%d").to_string())
}

/// What a section of the page is drawn with.
pub(super) struct Part<'a> {
    pub(super) p: &'a Pal,
    pub(super) store: &'a Entity<Store>,
    pub(super) read: &'a Store,
    /// The task: the state's (its live fields are newest), else the page's.
    pub(super) task: &'a Issue,
    /// Its fetched page, once it came.
    pub(super) page: Option<&'a TaskPage>,
    pub(super) now: &'a ZonedDateTime,
}

/// A section's title.
pub(super) fn section(p: &Pal, title: impl Into<gpui_kit::SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.faint)
        .pt(px(16.))
        .pb(px(6.))
        .child(title.into())
}

/// A folded part's row: its title and a chevron.
pub(super) fn fold_row(
    p: &Pal,
    id: &'static str,
    title: String,
    open: bool,
) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        // Read as its title, open or folded.
        .button_named(title.clone())
        .aria_expanded(open)
        .aria_value(a11y::folded(open))
        .flex()
        .items_center()
        .gap(px(6.))
        .mt(px(12.))
        .py(px(6.))
        .cursor_pointer()
        .text_size(px(13.))
        .child(div().flex_1().child(title))
        .child(glyph(
            if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            },
            p.faint,
            13.,
        ))
}

impl Render for TaskView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let store_entity = self.store.clone();
        let Some(id) = self.current(cx) else {
            return div().into_any_element();
        };
        let store = self.store.read(cx);
        let now = ZonedDateTime::now();
        let status = store.tasks().page_status(id);
        let page = match status {
            PageStatus::Ready(page) => Some(page),
            _ => None,
        };
        let task = store.task_of(id);
        let column = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .on_scroll(cx.listener(|this, event, _, cx| {
                if this.swipe.feed_event(event) == Some(Swiped::Right) {
                    this.back(cx);
                }
            }));
        let Some(task) = task else {
            let body = match status {
                PageStatus::NotFound => empty(&p, "Task not found"),
                PageStatus::Failed(reason) => empty(&p, reason),
                PageStatus::Loading | PageStatus::Ready(_) => loading(&p, "Loading…"),
            };
            return column.child(body).into_any_element();
        };
        let part = Part {
            p: &p,
            store: &store_entity,
            read: store,
            task,
            page,
            now: &now,
        };
        let body = self.body(&part, cx);
        let bar = actions::bar(self, &part, cx);
        let reading = self
            .reading
            .as_ref()
            .map(|(title, text)| text::read_full_sheet(&p, &store_entity, title, text, cx));
        column
            .child(
                div()
                    .id("task-page")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(18.))
                    .pb(px(24.))
                    .child(body),
            )
            .child(bar)
            .children(reading)
            .into_any_element()
    }
}

impl TaskView {
    /// The page's sections, top to bottom: what the user has to do first (the state, the
    /// ask, the reply box), then the work (where it stands, results, subtasks, findings,
    /// agents, runs, sessions), then the rest folded (since the ask, details, notes,
    /// history) and Stop on a working task.
    fn body(&self, part: &Part<'_>, cx: &Context<'_, Self>) -> AnyElement {
        if part.read.keeps_plans() {
            return self.planned_body(part, cx);
        }
        let mut body = div().flex().flex_col();
        body = top::title_and_state(body, part, self, cx);
        body = reply::section(body, part, self, cx);
        body = top::standing(body, part, self, cx);
        body = results::section(body, part, self, cx);
        body = work::subtasks(body, part, cx);
        body = work::finding(body, part, self, cx);
        body = agents::section(body, part);
        body = work::runs(body, part, self, cx);
        body = work::sessions(body, part);
        body = details::since_asked(body, part, self, cx);
        body = details::details(body, part, self, cx);
        body = details::notes(body, part, self, cx);
        body = details::history(body, part, self, cx);
        body = details::stop(body, part, self, cx);
        body.into_any_element()
    }
}

impl TaskView {
    /// The page's sections on a server that keeps plans, top to bottom: the title and its head
    /// (state, now, needs you or blocked, ETA, the session at work), the plan, what needs the
    /// user (the ask in its card, the reply box), then the activity (the turns of work with
    /// the plan's changes, the files and the history, under one filter), the files and results,
    /// subtasks, findings, agents, sessions, then the rest folded (since the ask, details,
    /// notes) and Stop on a working task.
    fn planned_body(&self, part: &Part<'_>, cx: &Context<'_, Self>) -> AnyElement {
        let mut body = div().flex().flex_col();
        body = plan::head(body, part, self);
        body = plan::section_of(body, part, self, cx);
        body = plan::needs_you(body, part, self, cx);
        body = reply::section(body, part, self, cx);
        body = activity::section_of(body, part, self, cx);
        body = results::section(body, part, self, cx);
        body = work::subtasks(body, part, cx);
        body = work::finding(body, part, self, cx);
        body = agents::section(body, part);
        body = work::sessions(body, part);
        body = details::since_asked(body, part, self, cx);
        body = details::details(body, part, self, cx);
        body = details::notes(body, part, self, cx);
        body = details::stop(body, part, self, cx);
        body.into_any_element()
    }
}

/// A grey line where the page would be.
fn empty(p: &Pal, text: impl Into<gpui_kit::SharedString>) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(14.))
        .text_color(p.muted)
        .child(text.into())
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::reseed;

    #[test]
    fn a_picker_is_set_only_when_the_tasks_day_changes() {
        let mut seen = None;
        assert_eq!(reseed(&mut seen, None), None, "no day: nothing to set");
        let first = date(2026, 12, 2);
        assert_eq!(
            reseed(&mut seen, Some(first)),
            Some(first),
            "the switch turned on"
        );
        assert_eq!(reseed(&mut seen, Some(first)), None, "the same day again");
        let picked = date(2026, 12, 9);
        assert_eq!(
            reseed(&mut seen, Some(picked)),
            Some(picked),
            "a day set elsewhere"
        );
        assert_eq!(reseed(&mut seen, None), None, "turned off: hidden");
        assert_eq!(
            reseed(&mut seen, Some(first)),
            Some(first),
            "on again: set again"
        );
    }
}
