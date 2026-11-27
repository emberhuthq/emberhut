//! What the sidebar shows of the browser: a Browser row with `needs you` among the top rows
//! while the assistant has handed it over and waits, else the row under More, with who drives
//! beside it.

use crate::store::Store;
use crate::store::browser::shown;

/// Whether the assistant handed the browser over and waits for the person: its row sits among
/// the top rows until they take it.
#[must_use]
pub(crate) fn is_handed_over(store: &Store) -> bool {
    store.browser().is_some_and(|info| info.handoff.is_some())
}

/// What the row under More says beside it while someone drives.
#[must_use]
pub(crate) fn driver_note(store: &Store) -> Option<&'static str> {
    store.browser().and_then(shown::driver_note)
}
