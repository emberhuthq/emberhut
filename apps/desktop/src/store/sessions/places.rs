//! What a session page can show, by a small key a [`Page`](crate::store::Page) carries (a page
//! is a plain value the column copies around; a session's id is text): a session or one helper
//! agent's steps in it, an external thread, or a new session about to open. A place asked for twice
//! has one key; each new session's draft has its own.

use emberhut_api::ids::{MachineId, WorkId};

use super::sheet::Setup;
use super::view::SessionRef;

/// A place's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SessionKey(u32);

/// What a session page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionPlace {
    /// A session, or one helper's steps in it.
    Session(SessionRef),
    /// An external thread, read only.
    Thread {
        /// The coding-thread app it runs in.
        machine: MachineId,
        /// Its id there.
        thread: String,
    },
    /// A new session of the user's, before its first message opens it.
    Draft(Setup),
    /// A row of work in flight that is a thread (Needs you's, the working rows): on a runner its
    /// session, else what it was asked and said, read only.
    Work(WorkId),
}

/// Every place asked for, by key.
#[derive(Debug, Default)]
pub(crate) struct Places(Vec<SessionPlace>);

impl Places {
    /// The key of `place`: its own when it was asked for before (a draft never is).
    pub(crate) fn key(&mut self, place: SessionPlace) -> SessionKey {
        let known = (!matches!(place, SessionPlace::Draft(_)))
            .then(|| self.0.iter().position(|p| *p == place))
            .flatten();
        let index = known.unwrap_or_else(|| {
            self.0.push(place);
            self.0.len() - 1
        });
        SessionKey(u32::try_from(index).unwrap_or(u32::MAX))
    }

    /// The place of `key`.
    #[must_use]
    pub(crate) fn get(&self, key: SessionKey) -> Option<&SessionPlace> {
        self.0.get(usize::try_from(key.0).ok()?)
    }
}

#[cfg(test)]
mod tests {
    use emberhut_api::ids::SessionId;

    use super::*;
    use crate::store::sessions::kept::SheetChoices;

    #[test]
    fn a_place_keeps_its_key_and_each_draft_gets_its_own() {
        let mut places = Places::default();
        let session = SessionPlace::Session(SessionRef::whole(SessionId::from("r-1")));
        let one = places.key(session.clone());
        assert_eq!(places.key(session.clone()), one);
        assert_eq!(places.get(one), Some(&session));
        let draft = SessionPlace::Draft(Setup::from_choices(&SheetChoices::default()));
        let first = places.key(draft.clone());
        assert_ne!(places.key(draft), first);
        assert_eq!(places.get(SessionKey(99)), None);
    }
}
