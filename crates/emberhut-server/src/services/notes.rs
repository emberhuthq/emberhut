//! Quick notes and the diary (`emberhut_core::memory::notes`): the app's saves, the notes tools,
//! and the owner's new notes riding along with what next wakes Main's assistant.

use std::sync::Arc;

use emberhut_core::Clock;
use emberhut_core::memory::notes::{
    Note, NoteRefusal, check_day, check_uid, checked_text, day_of, news, written_at,
};
use emberhut_db::{DbError, NoteStore, OwnerNote, SettingKey, SettingStore};
use jiff::Timestamp;
use jiff::tz::TimeZone;

/// When Main's assistant last took the owner's new notes.
const NOTES_SEEN: SettingKey<String> = SettingKey::new("notesSeen");

/// Why a note was not saved.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NoteError {
    /// The note is not one the diary takes.
    #[error(transparent)]
    Refused(#[from] NoteRefusal),
    /// The database failed.
    #[error(transparent)]
    Database(#[from] DbError),
}

/// Keeps the notes.
#[derive(Clone, Debug)]
pub struct NoteService {
    notes: NoteStore,
    settings: SettingStore,
    clock: Arc<dyn Clock>,
    zone: TimeZone,
}

impl NoteService {
    /// A service over `notes`, whose days are `zone`'s.
    #[must_use]
    pub fn new(
        notes: NoteStore,
        settings: SettingStore,
        clock: Arc<dyn Clock>,
        zone: TimeZone,
    ) -> Self {
        Self {
            notes,
            settings,
            clock,
            zone,
        }
    }

    /// The time zone notes' days are in.
    #[must_use]
    pub fn zone(&self) -> &TimeZone {
        &self.zone
    }

    /// Today, in the notes' time zone.
    #[must_use]
    pub fn today(&self) -> String {
        day_of(self.clock.now(), &self.zone)
    }

    /// The notes of the days `from` to `to` (none: no bound), newest first.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn list(&self, from: Option<&str>, to: Option<&str>) -> Result<Vec<Note>, DbError> {
        self.notes.list(from, to).await
    }

    /// The app's save: a new note or an edit (by `uid`); an empty `text` deletes it. The
    /// note as it is now; none when it was deleted (or there was nothing to delete).
    ///
    /// # Errors
    ///
    /// [`NoteError::Refused`] for an id or a text the diary doesn't take; [`NoteError::Database`].
    pub async fn save(
        &self,
        uid: &str,
        text: &str,
        at: Option<&str>,
    ) -> Result<Option<Note>, NoteError> {
        check_uid(uid)?;
        let text = checked_text(text)?;
        if text.is_empty() {
            self.notes.delete_owners(uid).await?;
            return Ok(None);
        }
        let now = self.clock.now();
        let at = written_at(at, now);
        let day = day_of(at, &self.zone);
        let note = OwnerNote {
            uid,
            text,
            at,
            day: &day,
        };
        Ok(Some(self.notes.save_owners(&note, now).await?))
    }

    /// The assistant's entry for `day` (none: today), in place of the one it had.
    ///
    /// # Errors
    ///
    /// [`NoteError::Refused`] for a day that isn't one, or is after today; [`NoteError::Database`].
    pub async fn write(&self, text: &str, day: Option<&str>) -> Result<Note, NoteError> {
        let today = self.today();
        let day = day.filter(|day| !day.is_empty()).unwrap_or(&today);
        check_day(day, &today)?;
        Ok(self
            .notes
            .write_assistants(day, text.trim(), self.clock.now())
            .await?)
    }

    /// The owner's notes new or edited since Main's assistant last took them, as a paragraph
    /// for what next wakes it (empty when there are none). Taken once: the next call has only
    /// what changes after this one.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn take_new(&self) -> Result<String, DbError> {
        let seen = self
            .settings
            .get(&NOTES_SEEN)
            .await?
            .and_then(|seen| seen.parse::<Timestamp>().ok());
        let fresh = self.notes.owners_changed_after(seen).await?;
        let Some(newest) = fresh.iter().map(|note| note.updated).max() else {
            return Ok(String::new());
        };
        self.settings
            .set(&NOTES_SEEN, &emberhut_core::memory::iso(newest))
            .await?;
        Ok(news(&fresh, seen, &self.today(), &self.zone))
    }
}
