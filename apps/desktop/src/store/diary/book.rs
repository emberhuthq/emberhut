//! The diary on this device: the server's notes with the user's changes it hasn't taken yet
//! on top, so a note written offline shows (and lasts across launches) until it is sent.
//!
//! The outbox holds each changed note as it should be, by its uid; empty text is a deletion.
//! A note is sent by its uid, so sending it twice is harmless. A note the server refused for
//! good (too long, or any other refusal it won't change its mind on) is kept too, with the
//! server's reason, so its words are never lost: it shows until it is changed and saved again.
//! The quick note's draft is kept here too, so closing the window without saving loses nothing.

use std::collections::BTreeMap;

use emberhut_api::ids::NoteUid;
use emberhut_api::notes::{MAX_NOTE_LEN, Note, NoteAuthor, NoteSave};
use emberhut_api::time::{Day, Timestamp as WireTime};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};

/// One day of the diary: its notes in the order they were written, the assistant's entry last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiaryDay<'a> {
    /// The day.
    pub(crate) day: &'a Day,
    /// Its notes.
    pub(crate) notes: Vec<&'a Note>,
}

/// The diary as this device holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Book {
    /// The notes as the server last sent them.
    #[serde(default)]
    notes: Vec<Note>,
    /// The changes not taken yet, by uid: each note as it should be (empty text: deleted).
    #[serde(default)]
    outbox: BTreeMap<NoteUid, Note>,
    /// What was being written in the quick note when it closed unsaved.
    #[serde(default)]
    draft: String,
    /// The notes the server refused for good, by uid, each as it was written and with the
    /// server's reason; none is sent again until it is changed.
    #[serde(default)]
    refused: BTreeMap<NoteUid, Refused>,
}

/// A note the server refused for good, kept so its words aren't lost.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Refused {
    /// The note as it was written.
    pub(crate) note: Note,
    /// Why, worded for the user (the server's own words when it gave them).
    pub(crate) why: String,
}

/// How much longer than the server takes `text` is, in UTF-16 units once trimmed (the server's
/// rule, [`MAX_NOTE_LEN`]); `None` when it fits.
#[must_use]
pub(crate) fn over_limit(text: &str) -> Option<usize> {
    let units = text.trim().encode_utf16().count();
    units.checked_sub(MAX_NOTE_LEN).filter(|over| *over > 0)
}

impl Book {
    /// The notes the diary shows: the server's, with the changes not sent yet applied, newest
    /// first.
    #[must_use]
    pub(crate) fn shown(&self) -> Vec<&Note> {
        let mut by_uid: BTreeMap<&NoteUid, &Note> =
            self.notes.iter().map(|note| (&note.uid, note)).collect();
        for (uid, refused) in &self.refused {
            by_uid.insert(uid, &refused.note);
        }
        for (uid, note) in &self.outbox {
            if note.text.is_empty() {
                by_uid.remove(uid);
            } else {
                by_uid.insert(uid, note);
            }
        }
        let mut shown: Vec<&Note> = by_uid.into_values().collect();
        shown.sort_by(|a, b| b.at.cmp(&a.at));
        shown
    }

    /// The days, newest first; in each the user's notes in the order written, the assistant's
    /// entry last.
    #[must_use]
    pub(crate) fn days(&self) -> Vec<DiaryDay<'_>> {
        let mut days: BTreeMap<&Day, Vec<&Note>> = BTreeMap::new();
        for note in self.shown() {
            days.entry(&note.day).or_default().push(note);
        }
        days.into_iter()
            .rev()
            .map(|(day, mut notes)| {
                notes.sort_by(|a, b| (a.is_assistants(), &a.at).cmp(&(b.is_assistants(), &b.at)));
                DiaryDay { day, notes }
            })
            .collect()
    }

    /// The user's own notes of `day`, newest first (the quick note lists today's under its
    /// field).
    #[must_use]
    pub(crate) fn own_notes_of(&self, day: &Day) -> Vec<&Note> {
        self.shown()
            .into_iter()
            .filter(|note| note.day == *day && !note.is_assistants())
            .collect()
    }

    /// A note shown, by its uid.
    #[must_use]
    pub(crate) fn note(&self, uid: &NoteUid) -> Option<&Note> {
        self.shown().into_iter().find(|note| note.uid == *uid)
    }

    /// Whether the server hasn't taken the latest change to `uid` yet.
    #[must_use]
    pub(crate) fn is_unsent(&self, uid: &NoteUid) -> bool {
        self.outbox.contains_key(uid)
    }

    /// Why the server refused the note `uid` as it shows now, if it did.
    #[must_use]
    pub(crate) fn refusal(&self, uid: &NoteUid) -> Option<&str> {
        if self.outbox.contains_key(uid) {
            return None;
        }
        self.refused.get(uid).map(|refused| refused.why.as_str())
    }

    /// Whether changes wait to be sent.
    #[must_use]
    pub(crate) fn has_unsent(&self) -> bool {
        !self.outbox.is_empty()
    }

    /// The quick note's draft.
    #[must_use]
    pub(crate) fn draft(&self) -> &str {
        &self.draft
    }

    /// Keeps `draft` for the next quick note; whether it changed.
    pub(crate) fn set_draft(&mut self, draft: String) -> bool {
        let changed = self.draft != draft;
        self.draft = draft;
        changed
    }

    /// Writes a new note (`editing` none) or changes one, which keeps its time and day; empty
    /// text deletes an edited note, and is no new note. A new note takes `uid`, `now` and its
    /// day in `zone`. A text longer than the server takes ([`over_limit`]) isn't saved. Whether
    /// anything changed.
    pub(crate) fn save(&mut self, text: &str, editing: Option<&NoteUid>, new: NewNote<'_>) -> bool {
        if over_limit(text).is_some() {
            return false;
        }
        let text = text.trim().to_owned();
        let note = match editing {
            Some(uid) => {
                let Some(note) = self.note(uid).cloned() else {
                    return false;
                };
                if note.text == text || note.is_assistants() {
                    return false;
                }
                Note { text, ..note }
            }
            None if text.is_empty() => return false,
            None => {
                let at = wire_time(new.now);
                Note {
                    uid: new.uid,
                    who: NoteAuthor::Person,
                    day: day_of(new.now, new.zone),
                    created: at.clone(),
                    updated: at.clone(),
                    at,
                    text,
                }
            }
        };
        // Changed again, a refused note is sent again as it is now.
        self.refused.remove(&note.uid);
        self.outbox.insert(note.uid.clone(), note);
        true
    }

    /// The changes to send, oldest first.
    #[must_use]
    pub(crate) fn to_send(&self) -> Vec<NoteSave> {
        let mut notes: Vec<&Note> = self.outbox.values().collect();
        notes.sort_by(|a, b| a.at.cmp(&b.at));
        notes
            .into_iter()
            .map(|note| NoteSave {
                uid: note.uid.clone(),
                text: note.text.clone(),
                at: Some(note.at.clone()),
            })
            .collect()
    }

    /// The server took `sent`, keeping `kept` (its copy; none for a deletion): it leaves the
    /// outbox, unless it changed again meanwhile, and the server's notes take it at once (the
    /// note added or replaced, or gone for empty text), so it doesn't vanish or come back until
    /// the diary is read again. Whether it left.
    pub(crate) fn sent(&mut self, sent: &NoteSave, kept: Option<Note>) -> bool {
        let Some(note) = self.leave_outbox(sent) else {
            return false;
        };
        let note = kept.filter(|kept| kept.uid == note.uid).unwrap_or(note);
        let at = self.notes.iter().position(|known| known.uid == note.uid);
        match at {
            Some(at) if note.text.is_empty() => {
                self.notes.remove(at);
            }
            Some(at) => self.notes[at] = note,
            None if note.text.is_empty() => {}
            None => self.notes.insert(0, note),
        }
        true
    }

    /// The server refused `sent` for good, saying `why`: it leaves the outbox, unless it
    /// changed again meanwhile, and is kept as written with the reason, so its words still show
    /// (a refused deletion just leaves the note as the server has it). Whether it left.
    pub(crate) fn refused(&mut self, sent: &NoteSave, why: String) -> bool {
        let Some(note) = self.leave_outbox(sent) else {
            return false;
        };
        if !note.text.is_empty() {
            self.refused.insert(note.uid.clone(), Refused { note, why });
        }
        true
    }

    /// Takes `sent`'s note out of the outbox, if it is still the one sent.
    fn leave_outbox(&mut self, sent: &NoteSave) -> Option<Note> {
        let same = self
            .outbox
            .get(&sent.uid)
            .is_some_and(|note| note.text == sent.text);
        if same {
            self.outbox.remove(&sent.uid)
        } else {
            None
        }
    }

    /// The server's notes, as it sent them now; whether they changed.
    pub(crate) fn take_notes(&mut self, notes: Vec<Note>) -> bool {
        let changed = self.notes != notes;
        self.notes = notes;
        changed
    }
}

/// Where "go to a day" lands in `days` (newest first): the first day on or before `pick`, else
/// the oldest; none in an empty diary.
#[must_use]
pub(crate) fn landing(days: &[DiaryDay<'_>], pick: &Day) -> Option<usize> {
    days.iter()
        .position(|day| day.day <= pick)
        .or_else(|| days.len().checked_sub(1))
}

/// What a new note takes: its uid, the time it is written, and the zone its day is in.
#[derive(Clone, Debug)]
pub(crate) struct NewNote<'a> {
    /// Its uid.
    pub(crate) uid: NoteUid,
    /// Now.
    pub(crate) now: Timestamp,
    /// The zone its day is taken in (this device's) until the server files it.
    pub(crate) zone: &'a TimeZone,
}

/// `at`'s day in `zone`, as the wire writes days.
#[must_use]
pub(crate) fn day_of(at: Timestamp, zone: &TimeZone) -> Day {
    Day::new(
        at.to_zoned_datetime(zone.clone())
            .strftime("%Y-%m-%d")
            .to_string(),
    )
}

/// `at` as the server writes instants: UTC, with milliseconds.
fn wire_time(at: Timestamp) -> WireTime {
    let utc = at.to_zoned_datetime(TimeZone::UTC);
    WireTime::new(format!(
        "{}.{:03}Z",
        utc.strftime("%Y-%m-%dT%H:%M:%S"),
        at.subsec_millisecond()
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn note(uid: &str, who: &str, day: &str, at: &str, text: &str) -> Note {
        serde_json::from_value(json!({
            "uid": uid, "who": who, "day": day, "at": at, "text": text,
            "created": at, "updated": at
        }))
        .unwrap()
    }

    fn server_notes() -> Vec<Note> {
        vec![
            note(
                "assistant:2026-11-24",
                "assistant",
                "2026-11-24",
                "2026-11-24T18:00:00.000Z",
                "A day.",
            ),
            note(
                "bbbbbbbb",
                "owner",
                "2026-11-24",
                "2026-11-24T09:00:00.000Z",
                "Second",
            ),
            note(
                "aaaaaaaa",
                "owner",
                "2026-11-24",
                "2026-11-24T07:00:00.000Z",
                "First",
            ),
            note(
                "cccccccc",
                "owner",
                "2026-11-22",
                "2026-11-22T07:00:00.000Z",
                "Older",
            ),
        ]
    }

    fn new_at<'a>(uid: &str, iso: &str, zone: &'a TimeZone) -> NewNote<'a> {
        NewNote {
            uid: NoteUid::new(uid),
            now: iso.parse().unwrap(),
            zone,
        }
    }

    #[test]
    fn days_are_newest_first_with_the_assistants_entry_last() {
        let mut book = Book::default();
        book.take_notes(server_notes());
        let days = book.days();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].day.as_str(), "2026-11-24");
        let texts: Vec<&str> = days[0].notes.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(texts, ["First", "Second", "A day."]);
        assert_eq!(days[1].notes[0].text, "Older");
        let today = book.own_notes_of(&Day::new("2026-11-24"));
        assert_eq!(
            today.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(),
            ["Second", "First"],
            "the user's own, newest first"
        );
    }

    #[test]
    fn going_to_a_day_lands_on_it_or_the_nearest_before() {
        let mut book = Book::default();
        book.take_notes(server_notes());
        let days = book.days();
        assert_eq!(landing(&days, &Day::new("2026-11-24")), Some(0));
        assert_eq!(landing(&days, &Day::new("2026-11-23")), Some(1), "the 22nd");
        assert_eq!(
            landing(&days, &Day::new("2026-01-01")),
            Some(1),
            "the oldest"
        );
        assert_eq!(landing(&[], &Day::new("2026-01-01")), None);
    }

    #[test]
    fn a_new_note_shows_at_once_unsent_until_the_server_takes_it() {
        let zone = TimeZone::fixed(jiff::tz::offset(3));
        let mut book = Book::default();
        assert!(!book.save(
            "   ",
            None,
            new_at("dddddddd", "2026-11-24T22:30:00.250Z", &zone)
        ));
        assert!(book.save(
            " Late thought ",
            None,
            new_at("dddddddd", "2026-11-24T22:30:00.250Z", &zone)
        ));
        let uid = NoteUid::new("dddddddd");
        let note = book.note(&uid).unwrap();
        assert_eq!(note.text, "Late thought");
        assert_eq!(
            note.day.as_str(),
            "2026-11-25",
            "its day in the zone given (the device's)"
        );
        assert_eq!(note.at.as_str(), "2026-11-24T22:30:00.250Z");
        assert!(book.is_unsent(&uid));
        let [send] = book.to_send().try_into().unwrap();
        assert_eq!(send.text, "Late thought");
        assert!(book.sent(&send, None));
        assert!(!book.has_unsent());
        assert!(!book.is_unsent(&uid));
        assert_eq!(
            book.note(&uid).map(|note| note.text.as_str()),
            Some("Late thought"),
            "it stays on screen, sent"
        );
    }

    #[test]
    fn a_sent_change_shows_at_once_as_the_server_keeps_it() {
        let zone = TimeZone::UTC;
        let mut book = Book::default();
        book.take_notes(server_notes());
        let unused = || new_at("eeeeeeee", "2026-11-25T00:00:00Z", &zone);
        let first = NoteUid::new("aaaaaaaa");
        book.save("First, edited", Some(&first), unused());
        let send = book.to_send().remove(0);
        let mut filed = book.note(&first).unwrap().clone();
        filed.day = Day::new("2026-11-23");
        assert!(book.sent(&send, Some(filed)));
        let shown = book.note(&first).unwrap();
        assert_eq!(
            (shown.text.as_str(), shown.day.as_str()),
            ("First, edited", "2026-11-23"),
            "the server's copy, with the day it filed it under"
        );
        let second = NoteUid::new("bbbbbbbb");
        book.save("", Some(&second), unused());
        let send = book.to_send().remove(0);
        assert!(book.sent(&send, None));
        assert!(book.note(&second).is_none(), "a deletion stays gone");
        assert!(!book.has_unsent());

        book.save(
            "Refused",
            None,
            new_at("ffffffff", "2026-11-25T08:00:00Z", &zone),
        );
        let send = book.to_send().remove(0);
        assert!(book.refused(&send, "Too long".to_owned()));
        let refused = NoteUid::new("ffffffff");
        assert_eq!(
            book.note(&refused).map(|note| note.text.as_str()),
            Some("Refused"),
            "the server never took it, and its words stay"
        );
        assert_eq!(book.refusal(&refused), Some("Too long"));
        assert!(!book.has_unsent(), "not sent again as it is");
        let kept: Book = serde_json::from_value(serde_json::to_value(&book).unwrap()).unwrap();
        assert_eq!(kept, book, "kept across launches");
        assert!(book.save("Shorter", Some(&refused), unused()));
        assert_eq!(book.refusal(&refused), None, "changed, it goes again");
        assert!(book.is_unsent(&refused));
        book.take_notes(server_notes());
        assert!(
            book.note(&refused).is_some(),
            "a read of the diary keeps it"
        );
    }

    #[test]
    fn a_refused_deletion_leaves_the_note_as_the_server_has_it() {
        let zone = TimeZone::UTC;
        let mut book = Book::default();
        book.take_notes(server_notes());
        let uid = NoteUid::new("aaaaaaaa");
        book.save(
            "",
            Some(&uid),
            new_at("eeeeeeee", "2026-11-25T00:00:00Z", &zone),
        );
        let send = book.to_send().remove(0);
        assert!(book.refused(&send, "No".to_owned()));
        assert_eq!(
            book.note(&uid).map(|note| note.text.as_str()),
            Some("First")
        );
        assert_eq!(book.refusal(&uid), None);
    }

    #[test]
    fn a_note_longer_than_the_server_takes_isnt_saved() {
        let zone = TimeZone::UTC;
        let mut book = Book::default();
        let most = "é".repeat(MAX_NOTE_LEN);
        assert_eq!(
            over_limit(&format!("  {most}\n")),
            None,
            "trimmed, as the server does"
        );
        assert_eq!(over_limit(&format!("{most}ab")), Some(2));
        // An emoji is two UTF-16 units.
        assert_eq!(
            over_limit(&format!("{}😀", "x".repeat(MAX_NOTE_LEN - 1))),
            Some(1)
        );
        let new = new_at("ffffffff", "2026-11-25T08:00:00Z", &zone);
        assert!(!book.save(&format!("{most}a"), None, new));
        assert!(!book.has_unsent());
    }

    #[test]
    fn an_edit_keeps_its_time_and_empty_text_deletes() {
        let zone = TimeZone::UTC;
        let mut book = Book::default();
        book.take_notes(server_notes());
        let uid = NoteUid::new("aaaaaaaa");
        let unused = || new_at("eeeeeeee", "2026-11-25T00:00:00Z", &zone);
        assert!(!book.save("First", Some(&uid), unused()), "nothing changed");
        assert!(book.save("First, edited", Some(&uid), unused()));
        let edited = book.note(&uid).unwrap();
        assert_eq!(edited.at.as_str(), "2026-11-24T07:00:00.000Z");
        assert_eq!(edited.text, "First, edited");
        assert!(book.save("", Some(&uid), unused()));
        assert!(book.note(&uid).is_none(), "deleted at once");
        assert_eq!(
            book.to_send()[0].text,
            "",
            "a deletion is sent as empty text"
        );
        let entry = NoteUid::new("assistant:2026-11-24");
        assert!(
            !book.save("Mine now", Some(&entry), unused()),
            "the assistant's entry is its own"
        );
    }

    #[test]
    fn a_change_made_while_sending_stays_to_be_sent() {
        let zone = TimeZone::UTC;
        let mut book = Book::default();
        book.save(
            "One",
            None,
            new_at("ffffffff", "2026-11-24T08:00:00Z", &zone),
        );
        let sending = book.to_send().remove(0);
        book.save(
            "One, again",
            Some(&NoteUid::new("ffffffff")),
            new_at("gggggggg", "2026-11-24T08:01:00Z", &zone),
        );
        assert!(!book.sent(&sending, None));
        assert!(!book.refused(&sending, "No".to_owned()));
        assert!(book.has_unsent());
    }

    #[test]
    fn the_book_and_its_draft_last_across_launches() {
        let zone = TimeZone::UTC;
        let mut book = Book::default();
        book.take_notes(server_notes());
        book.save(
            "Offline",
            None,
            new_at("hhhhhhhh", "2026-11-24T08:00:00Z", &zone),
        );
        assert!(book.set_draft("Half a tho".to_owned()));
        let kept: Book = serde_json::from_value(serde_json::to_value(&book).unwrap()).unwrap();
        assert_eq!(kept, book);
        assert_eq!(kept.draft(), "Half a tho");
        assert_eq!(serde_json::from_str::<Book>("{}").unwrap(), Book::default());
    }
}
