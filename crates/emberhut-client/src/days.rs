//! The pages about the person's days: everything scheduled, the calendar for a range of days,
//! and the diary's quick notes.

use emberhut_api::calendar::{CalendarPage, CalendarQuery};
use emberhut_api::notes::{NoteSave, NoteSaved, NotesPage};
use emberhut_api::paths;
use emberhut_api::scheduled::ScheduledPage;

use crate::exchange;
use crate::{Client, Error};

impl Client {
    /// Everything the assistant has with a next run, soonest first (`GET /api/scheduled`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 from a server without the list; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn scheduled(&self) -> Result<ScheduledPage, Error> {
        exchange::json(self.get(paths::SCHEDULED, "")?)
    }

    /// The calendar for the days `query` names (`GET /api/calendar`): the events and what the
    /// person has to work on.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason for a bad range; [`Error::SignInRequired`],
    /// the transport's and decoding errors.
    pub fn calendar(&self, query: &CalendarQuery) -> Result<CalendarPage, Error> {
        exchange::json(self.get(paths::CALENDAR, &query.to_query())?)
    }

    /// Every quick note, newest first (`GET /api/notes`).
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn notes(&self) -> Result<NotesPage, Error> {
        exchange::json(self.get(paths::NOTES, "")?)
    }

    /// Saves a quick note (`POST /api/notes`): a new one, an edit, or (empty text) a deletion.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason when it won't take the note (it never
    /// will: a bad uid, a text too long); [`Error::SignInRequired`], the transport's and
    /// decoding errors.
    pub fn save_note(&self, note: &NoteSave) -> Result<NoteSaved, Error> {
        exchange::json(self.post(paths::NOTES, note)?)
    }
}
