//! Party's data: the list ([`roster`]), a new party being put together ([`draft`]), and the
//! party open ([`room`]), followed live in the background while it shows: a snapshot,
//! then each change. The stream opens again 2 s after the server ends it; after a failure to
//! reach the server ("Reconnecting…") the wait doubles up to 30 s. A refusal (the server
//! doesn't have the party, or won't show it) is final: the room says so and the stream stops.
//!
//! The Party window shows them; what they hold and how they word it is pure and tested on its
//! own ([`roster`], [`draft`], [`room`], [`words`]).

pub(crate) mod draft;
pub(crate) mod room;
pub(crate) mod roster;
pub(crate) mod words;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use emberhut_api::ids::PartyId;
use emberhut_api::party::{PartyAction, PartySay, SayKind};
use gpui_kit::AppContext as _;
use gpui_kit::{BackgroundExecutor, Context};

use self::draft::Draft;
use self::room::{Heard, Room};
use self::roster::Roster;
use super::{Store, update_or_log};
use crate::backend::{Backend, Error};

/// How long to wait before following the stream again after it ended, or after its first
/// failure in a row.
const RECONNECT_AFTER: Duration = Duration::from_secs(2);

/// The longest wait between failures in a row: each one doubles the wait up to this.
const RECONNECT_AT_MOST: Duration = Duration::from_secs(30);

/// Party's data in the store.
#[derive(Debug, Default)]
pub(crate) struct PartyData {
    roster: Roster,
    draft: Option<Draft>,
    room: Option<Room>,
    /// Bumped whenever the party open changes: a stream whose number is no longer
    /// current stops.
    generation: Arc<AtomicU64>,
}

impl PartyData {
    /// The list.
    #[must_use]
    pub(crate) fn roster(&self) -> &Roster {
        &self.roster
    }

    /// The new party being put together, while its form is open.
    #[must_use]
    pub(crate) fn draft(&self) -> Option<&Draft> {
        self.draft.as_ref()
    }

    /// The party open.
    #[must_use]
    pub(crate) fn room(&self) -> Option<&Room> {
        self.room.as_ref()
    }
}

impl Drop for PartyData {
    /// A store that goes away stops its party's stream too (at the stream's next line).
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}

impl Store {
    /// Party's data.
    #[must_use]
    pub(crate) fn party(&self) -> &PartyData {
        &self.party
    }

    /// Reads the parties again.
    pub(crate) fn load_parties(&mut self, cx: &mut Context<'_, Self>) {
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let page = cx.background_spawn(async move { backend.parties() }).await;
            update_or_log(&this, cx, |store, cx| {
                if let Err(error) = &page {
                    log::warn!("couldn't read the parties: {error}");
                }
                store.party.roster.take(page);
                if let Some(draft) = &mut store.party.draft {
                    draft.start_from(store.party.roster.presets());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the new party's form (`true`), with the starting cast, or closes it.
    pub(crate) fn show_new_party(&mut self, open: bool, cx: &mut Context<'_, Self>) {
        self.party.draft = open.then(|| Draft::new(self.party.roster.presets()));
        if open && !self.party.roster.is_loaded() {
            self.load_parties(cx);
        }
        cx.notify();
    }

    /// Changes the new party's form.
    pub(crate) fn change_draft(
        &mut self,
        change: impl FnOnce(&mut Draft),
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(draft) = &mut self.party.draft {
            change(draft);
            cx.notify();
        }
    }

    /// Starts the party the form describes, about `topic`: once the server has made it, the
    /// form closes and the party opens; else the form says why.
    pub(crate) fn start_party(&mut self, topic: &str, cx: &mut Context<'_, Self>) {
        let Some(draft) = &mut self.party.draft else {
            return;
        };
        if !draft.can_start(topic) {
            return;
        }
        draft.busy = true;
        draft.problem = None;
        let request = draft.request(topic);
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let started = cx
                .background_spawn(async move { backend.start_party(&request) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                match started {
                    // The form is gone when the window closed while the server was making the
                    // party: it isn't opened then, as nothing would stop its stream.
                    Ok(party) => {
                        if store.party.draft.take().is_some() {
                            store.open_party(Some(party.id), cx);
                            store.load_parties(cx);
                        } else {
                            log::info!("party {} started after its window closed", party.id);
                        }
                    }
                    Err(error) => {
                        log::warn!("couldn't start the party: {error}");
                        if let Some(draft) = &mut store.party.draft {
                            draft.busy = false;
                            draft.problem = Some(refusal(&error, "Couldn't start it"));
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Deletes party `id`: off the list at once, then the list read again.
    pub(crate) fn delete_party(&mut self, id: PartyId, cx: &mut Context<'_, Self>) {
        self.party.roster.remove(id);
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let deleted = cx
                .background_spawn(async move { backend.act_on_party(id, &PartyAction::Delete) })
                .await;
            if let Err(error) = deleted {
                log::warn!("couldn't delete party {id}: {error}");
            }
            update_or_log(&this, cx, Store::load_parties);
        })
        .detach();
        cx.notify();
    }

    /// Opens party `id` (followed live while it shows), or goes back to the list (`None`),
    /// which is read again.
    pub(crate) fn open_party(&mut self, id: Option<PartyId>, cx: &mut Context<'_, Self>) {
        if self.party.room.as_ref().map(Room::id) == id {
            return;
        }
        let mine = self.party.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.party.room = id.map(Room::new);
        match id {
            Some(id) => self.follow_party(id, mine, cx),
            None => self.load_parties(cx),
        }
        cx.notify();
    }

    /// The Party window closed: the party open stops being followed, and the form goes.
    pub(crate) fn leave_parties(&mut self, cx: &mut Context<'_, Self>) {
        self.party.generation.fetch_add(1, Ordering::Relaxed);
        self.party.room = None;
        self.party.draft = None;
        cx.notify();
    }

    /// One of the user's controls on the party open.
    pub(crate) fn act_on_party(&mut self, action: PartyAction, cx: &mut Context<'_, Self>) {
        let Some(id) = self.party.room.as_ref().map(Room::id) else {
            return;
        };
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let reply = cx
                .background_spawn(async move { backend.act_on_party(id, &action) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                if let Some(room) = store.party.room.as_mut().filter(|room| room.id() == id) {
                    room.answered(reply.map_err(|error| {
                        log::warn!("party {id}'s control failed: {error}");
                        refusal(&error, "Couldn't do that now")
                    }));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The user's line in the party open: their words (`steer` false) or a direction. Blank
    /// says nothing.
    pub(crate) fn say_in_party(&mut self, text: &str, steer: bool, cx: &mut Context<'_, Self>) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let kind = if steer { SayKind::Steer } else { SayKind::Say };
        self.act_on_party(
            PartyAction::Say(PartySay {
                text: text.to_owned(),
                kind,
            }),
            cx,
        );
    }

    /// Follows party `id`'s stream in the background while `mine` is current. Each time the
    /// stream is open it blocks a thread of its own ([`smol::unblock`]), never one of the
    /// executor's few background threads, which the rest of the app shares.
    fn follow_party(&self, id: PartyId, mine: u64, cx: &mut Context<'_, Self>) {
        let (heard, hearing) = smol::channel::unbounded::<Heard>();
        let stream = Stream {
            backend: Arc::clone(&self.backend),
            id,
            generation: Arc::clone(&self.party.generation),
            mine,
        };
        let executor = cx.background_executor().clone();
        cx.background_spawn(async move { stream.follow(&heard, &executor).await })
            .detach();
        let generation = Arc::clone(&self.party.generation);
        cx.spawn(async move |this, cx| {
            while let Ok(heard) = hearing.recv().await {
                if generation.load(Ordering::Relaxed) != mine {
                    return;
                }
                let gone = update_or_log(&this, cx, |store, cx| {
                    if let Some(room) = &mut store.party.room
                        && room.hear(heard)
                    {
                        cx.notify();
                    }
                })
                .is_none();
                if gone {
                    return;
                }
            }
        })
        .detach();
    }
}

/// A party's stream, followed while its number is current.
struct Stream {
    backend: Arc<dyn Backend>,
    id: PartyId,
    generation: Arc<AtomicU64>,
    mine: u64,
}

impl Stream {
    fn current(&self) -> bool {
        is_current(&self.generation, self.mine)
    }

    /// Follows the stream while it is current, again after each end ([`next_wait`]); what it
    /// hears goes to `heard`. A refusal ends it for good, told as [`Heard::Gone`].
    async fn follow(&self, heard: &smol::channel::Sender<Heard>, executor: &BackgroundExecutor) {
        let id = self.id;
        let mut failures = 0;
        while self.current() {
            let (backend, generation, mine) = (
                Arc::clone(&self.backend),
                Arc::clone(&self.generation),
                self.mine,
            );
            let sender = heard.clone();
            let (outcome, open, any) = smol::unblock(move || {
                let (mut open, mut any) = (true, false);
                let outcome =
                    backend.follow_party(id, &|| !is_current(&generation, mine), &mut |event| {
                        any = true;
                        open = sender.try_send(Heard::Event(Box::new(event))).is_ok();
                    });
                (outcome, open, any)
            })
            .await;
            if !self.current() || !open {
                return;
            }
            if any {
                failures = 0;
            }
            match outcome {
                Ok(()) => failures = 0,
                Err(error) if is_final(&error) => {
                    log::info!("party {id}'s stream was refused: {error}");
                    if heard.try_send(Heard::Gone(gone_words(&error))).is_err() {
                        log::debug!("party {id} was refused after its window closed");
                    }
                    return;
                }
                Err(error) => {
                    log::debug!("party {id}'s stream broke off: {error}");
                    failures += 1;
                    if heard.try_send(Heard::Dropped).is_err() {
                        return;
                    }
                }
            }
            executor.timer(next_wait(failures)).await;
        }
    }
}

/// Whether the stream numbered `mine` is still the current one.
fn is_current(generation: &AtomicU64, mine: u64) -> bool {
    generation.load(Ordering::Relaxed) == mine
}

/// The wait before following again after `failures` failures in a row: [`RECONNECT_AFTER`]
/// after an end (none), then doubling each time up to [`RECONNECT_AT_MOST`].
fn next_wait(failures: u32) -> Duration {
    let doubled = RECONNECT_AFTER.saturating_mul(1 << failures.saturating_sub(1).min(5));
    doubled.min(RECONNECT_AT_MOST)
}

/// Whether following again can't help: the server refused the party (any 4xx: it doesn't have
/// it, or won't show it), or this device has to sign in first.
fn is_final(error: &Error) -> bool {
    match error {
        Error::Refused {
            status: Some(status),
            ..
        } => (400..500).contains(status),
        Error::SignInRequired | Error::WrongMachine => true,
        Error::Refused { status: None, .. }
        | Error::Unreachable(_)
        | Error::Unreadable(_)
        | Error::Local(_) => false,
    }
}

/// What the room says once its stream was refused for good.
fn gone_words(error: &Error) -> String {
    match error {
        Error::Refused {
            status: Some(404), ..
        } => "This party is gone.".to_owned(),
        Error::SignInRequired => "This device has to sign in first.".to_owned(),
        _ => error.server_message().map_or_else(
            || "The server won't show this party.".to_owned(),
            str::to_owned,
        ),
    }
}

/// The server's own words for a refusal, else `otherwise` (or that it can't be reached).
fn refusal(error: &Error, otherwise: &str) -> String {
    match error {
        Error::Unreachable(_) => format!("Can't reach {}", crate::brand::APP_NAME),
        _ => error
            .server_message()
            .map_or_else(|| otherwise.to_owned(), str::to_owned),
    }
}

#[cfg(test)]
pub(crate) mod tests;
