//! A task's activity on a page that shows its plan: one feed, newest first, of its turns of work
//! and what happened between them. Each turn is one readable entry (what it was asked, the
//! first line of its answer or its STATUS line, the session it ran in), with what it ticked,
//! skipped, blocked and made as receipts under it, so nothing shows twice; the plan's other
//! changes, the files made outside a turn and the task's history stand on their own. A filter
//! narrows it to the turns, the plan's changes or the files. Whole answers stay in the
//! session's transcript.

use emberhut_api::ids::{RunId, SessionId};
use emberhut_api::task_page::{Run, TaskPage};
use emberhut_api::task_plan::StepState;
use jiff::{Timestamp, ZonedDateTime};

use super::clock;
use super::runs::{self, RunState, RunStatus};

/// What the feed shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Filter {
    /// Everything.
    #[default]
    All,
    /// The turns of work, with their receipts.
    Turns,
    /// The plan's changes: ticked, skipped, blocked.
    Plan,
    /// The files and results made.
    Files,
}

impl Filter {
    /// Every filter, in the order the page offers them.
    pub(crate) const ALL: [Self; 4] = [Self::All, Self::Turns, Self::Plan, Self::Files];

    /// Its name on the page.
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Turns => "Turns",
            Self::Plan => "Plan",
            Self::Files => "Files",
        }
    }
}

/// What an event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EventKind {
    /// A step ticked done.
    Ticked,
    /// A step skipped.
    Skipped,
    /// A step blocked.
    Blocked,
    /// A file or result made.
    File,
    /// A step of the task's history.
    Note,
}

impl EventKind {
    /// How its line starts: `Ticked`, `Skipped`, `Blocked`, `Made`; none for a history step.
    #[must_use]
    pub(crate) const fn word(self) -> Option<&'static str> {
        match self {
            Self::Ticked => Some("Ticked"),
            Self::Skipped => Some("Skipped"),
            Self::Blocked => Some("Blocked"),
            Self::File => Some("Made"),
            Self::Note => None,
        }
    }

    /// Whether it is one of the plan's changes.
    #[must_use]
    pub(crate) const fn of_plan(self) -> bool {
        matches!(self, Self::Ticked | Self::Skipped | Self::Blocked)
    }
}

/// Something that happened, at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Event {
    /// When.
    pub(crate) at: Timestamp,
    /// What kind.
    pub(crate) kind: EventKind,
    /// What: a step's title, a file's name, a history line.
    pub(crate) text: String,
}

/// One turn of work, as the feed reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Turn {
    /// The run's id.
    pub(crate) run: RunId,
    /// When it started (else was sent, else ended).
    pub(crate) at: Option<Timestamp>,
    /// When it ran and for how long, in words.
    pub(crate) when: String,
    /// What it was asked, its first line.
    pub(crate) asked: Option<String>,
    /// What it came to: its answer's first line, else its STATUS line's words; a turn still
    /// going says so.
    pub(crate) said: Option<String>,
    /// What its STATUS line says.
    pub(crate) state: Option<RunState>,
    /// It is still going.
    pub(crate) live: bool,
    /// The session it ran in, when the page lists it: its id and `<title> on <machine>`.
    pub(crate) session: Option<(SessionId, String)>,
    /// What it ticked, skipped, blocked and made, oldest first.
    pub(crate) receipts: Vec<Event>,
}

/// A line of the feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    /// A turn of work.
    Turn(Turn),
    /// Something that happened on its own.
    Event(Event),
}

impl Item {
    fn at(&self) -> Option<Timestamp> {
        match self {
            Self::Turn(turn) => turn.at,
            Self::Event(event) => Some(event.at),
        }
    }
}

/// The page's feed under `filter`, newest first.
#[must_use]
pub(crate) fn feed(page: Option<&TaskPage>, now: &ZonedDateTime, filter: Filter) -> Vec<Item> {
    let Some(page) = page else {
        return Vec::new();
    };
    let mut turns: Vec<(Turn, Option<Timestamp>)> = runs::timeline(Some(page))
        .iter()
        .map(|run| (turn(page, run, now), window_end(run, now)))
        .collect();
    let mut loose = Vec::new();
    for event in events(page) {
        let receipt =
            event.kind != EventKind::Note && matches!(filter, Filter::All | Filter::Turns);
        let during = turns.iter_mut().find(|(turn, end)| {
            turn.at.is_some_and(|start| start <= event.at) && end.is_some_and(|end| event.at <= end)
        });
        match during {
            Some((turn, _)) if receipt => turn.receipts.push(event),
            _ => loose.push(event),
        }
    }
    let mut items: Vec<Item> = Vec::new();
    if matches!(filter, Filter::All | Filter::Turns) {
        items.extend(turns.into_iter().map(|(mut turn, _)| {
            turn.receipts.sort_by_key(|event| event.at);
            Item::Turn(turn)
        }));
    }
    items.extend(
        loose
            .into_iter()
            .filter(|event| match filter {
                Filter::All => true,
                Filter::Turns => false,
                Filter::Plan => event.kind.of_plan(),
                Filter::Files => event.kind == EventKind::File,
            })
            .map(Item::Event),
    );
    // Newest first; a turn still to start has no time and goes on top.
    items.sort_by(|a, b| match (a.at(), b.at()) {
        (Some(a), Some(b)) => b.cmp(&a),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    items
}

/// When a turn's receipts stop: when it ended, or now while it runs.
fn window_end(run: &Run, now: &ZonedDateTime) -> Option<Timestamp> {
    match &run.ended {
        Some(ended) => clock::instant(ended),
        None => Some(now.timestamp()),
    }
}

/// A run as the feed reads it.
fn turn(page: &TaskPage, run: &Run, now: &ZonedDateTime) -> Turn {
    let status = run.status.as_deref().and_then(RunStatus::parse);
    let live = run.ended.is_none();
    let said = run
        .answer
        .as_deref()
        .and_then(first_line)
        .or_else(|| status.as_ref().map(|status| status.text.clone()))
        .or_else(|| live.then(|| going(run).to_owned()));
    let at = [&run.started, &run.requested, &run.ended]
        .into_iter()
        .flatten()
        .find_map(clock::instant);
    Turn {
        run: run.id,
        at,
        when: runs::run_when(run, now),
        asked: run.asked.as_deref().and_then(first_line),
        said,
        state: status.map(|status| status.state),
        live,
        session: page
            .sessions
            .iter()
            .find(|session| !run.thread.is_empty() && session.id.as_str() == run.thread)
            .map(|session| {
                let name = crate::store::sessions::words::short_title(&session.title);
                let label = if session.place.is_empty() {
                    name.to_owned()
                } else {
                    format!("{name} on {}", session.place)
                };
                (session.id.clone(), label)
            }),
        receipts: Vec::new(),
    }
}

/// What a turn still going says while it has no answer.
fn going(run: &Run) -> &'static str {
    if run.started.is_none() {
        "Not started yet"
    } else {
        "Working on it…"
    }
}

/// The page's events: the plan's ticks, skips and blocks, the files made, and the history's
/// steps (but the lines an older server's finished turns left, which are its turns).
fn events(page: &TaskPage) -> Vec<Event> {
    let mut events = Vec::new();
    if let Some(plan) = &page.plan {
        for step in &plan.steps {
            let (kind, at) = match step.state {
                StepState::Done => (EventKind::Ticked, step.finished.as_ref()),
                StepState::Skipped => (EventKind::Skipped, step.finished.as_ref()),
                StepState::Blocked => (EventKind::Blocked, Some(&step.updated)),
                _ => continue,
            };
            if let Some(at) = at.and_then(clock::instant) {
                events.push(Event {
                    at,
                    kind,
                    text: step.title.clone(),
                });
            }
        }
    }
    for artifact in &page.task.artifacts {
        if let Some(at) = clock::instant(&artifact.at) {
            events.push(Event {
                at,
                kind: EventKind::File,
                text: artifact.name.clone(),
            });
        }
    }
    for moment in &page.history {
        if moment.text.trim().is_empty() || moment.text.contains(" · STATUS:") {
            continue;
        }
        if let Some(at) = clock::instant(&moment.at) {
            events.push(Event {
                at,
                kind: EventKind::Note,
                text: moment.text.clone(),
            });
        }
    }
    events
}

/// A time in the feed's time column: the clock time today, else the short day (`Thu`, or
/// `3 Nov` past a week).
#[must_use]
pub(crate) fn time_label(at: Timestamp, now: &ZonedDateTime) -> String {
    let zoned = at.to_zoned_datetime(now.time_zone().clone());
    let days = crate::store::lookup::days_from(zoned.date(), now.date()).unwrap_or(i32::MAX);
    match days {
        0 => clock::clock_minutes(&zoned),
        1..=6 => zoned.strftime("%a").to_string(),
        _ => zoned.strftime("%-d %b").to_string(),
    }
}

/// How many sessions the turns ran in, for the feed's heading: `2 sessions`.
#[must_use]
pub(crate) fn sessions_note(items: &[Item]) -> Option<String> {
    let mut seen: Vec<&SessionId> = Vec::new();
    for item in items {
        if let Item::Turn(Turn {
            session: Some((id, _)),
            ..
        }) = item
            && !seen.contains(&id)
        {
            seen.push(id);
        }
    }
    match seen.len() {
        0 => None,
        1 => Some("1 session".to_owned()),
        n => Some(format!("{n} sessions")),
    }
}

/// A text's first line that says something, without markdown's marks at its start, `*` and
/// backticks; none when it is blank.
#[must_use]
pub(crate) fn first_line(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(|line| line.trim().trim_start_matches(['#', '-', '>', ' ']).trim())
        .find(|line| !line.is_empty() && !line.chars().all(|c| matches!(c, '-' | '=' | '*')))?;
    let line = line.replace(['*', '`'], "");
    (!line.trim().is_empty()).then(|| line.trim().to_owned())
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod tests;
