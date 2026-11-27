//! The fake's answers to the pages about the user's days: what is scheduled, the calendar and
//! the diary's notes, queued per route, with a plain default once a queue is empty.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use emberhut_api::calendar::{CalendarPage, CalendarQuery};
use emberhut_api::notes::{Note, NoteAuthor, NoteSave, NoteSaved, NotesPage};
use emberhut_api::scheduled::ScheduledPage;
use emberhut_api::time::{Day, Timestamp};

use crate::backend::Error;

/// The queued answers.
#[derive(Debug, Default)]
pub(crate) struct DaysReplies {
    scheduled: Mutex<VecDeque<Result<ScheduledPage, Error>>>,
    calendar: Mutex<VecDeque<Result<CalendarPage, Error>>>,
    notes: Mutex<VecDeque<Result<NotesPage, Error>>>,
    saves: Mutex<VecDeque<Result<NoteSaved, Error>>>,
}

fn push<T>(queue: &Mutex<VecDeque<T>>, reply: T) {
    queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push_back(reply);
}

fn pop<T>(queue: &Mutex<VecDeque<T>>) -> Option<T> {
    queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
}

impl DaysReplies {
    /// Queues the answer to the next read of what is scheduled (by default nothing is).
    pub(crate) fn push_scheduled(&self, reply: Result<ScheduledPage, Error>) {
        push(&self.scheduled, reply);
    }

    /// Queues the answer to the next read of the calendar (by default the days asked, empty).
    pub(crate) fn push_calendar(&self, reply: Result<CalendarPage, Error>) {
        push(&self.calendar, reply);
    }

    /// Queues the answer to the next read of the notes (by default there are none).
    pub(crate) fn push_notes(&self, reply: Result<NotesPage, Error>) {
        push(&self.notes, reply);
    }

    /// Queues the answer to the next note saved (by default the server keeps it as sent).
    pub(crate) fn push_save(&self, reply: Result<NoteSaved, Error>) {
        push(&self.saves, reply);
    }

    pub(super) fn scheduled(&self) -> Result<ScheduledPage, Error> {
        pop(&self.scheduled).unwrap_or_else(|| Ok(ScheduledPage { items: Vec::new() }))
    }

    pub(super) fn calendar(&self, query: &CalendarQuery) -> Result<CalendarPage, Error> {
        pop(&self.calendar).unwrap_or_else(|| {
            Ok(CalendarPage {
                from: query.from.clone(),
                to: query.to.clone(),
                events: Vec::new(),
                items: Vec::new(),
                note: None,
                at: Timestamp::new("2026-01-01T00:00:00.000Z"),
            })
        })
    }

    pub(super) fn notes(&self) -> Result<NotesPage, Error> {
        pop(&self.notes).unwrap_or_else(|| Ok(NotesPage { notes: Vec::new() }))
    }

    pub(super) fn save(&self, note: &NoteSave) -> Result<NoteSaved, Error> {
        pop(&self.saves).unwrap_or_else(|| {
            let at = note
                .at
                .clone()
                .unwrap_or_else(|| Timestamp::new("2026-01-01T00:00:00.000Z"));
            let kept = (!note.text.trim().is_empty()).then(|| Note {
                uid: note.uid.clone(),
                who: NoteAuthor::Person,
                day: Day::new(at.as_str().get(..10).unwrap_or("2026-01-01")),
                at: at.clone(),
                text: note.text.trim().to_owned(),
                created: at.clone(),
                updated: at,
            });
            Ok(NoteSaved {
                ok: true,
                note: kept,
            })
        })
    }
}
