//! Parties: AI personas talking about the person's topic, turn by turn, while the person
//! watches (`emberhut-core`'s [`party`] rules; the talk goes through a [`ChatModel`]).
//!
//! One talk runs at a time per party, on its own task: each persona's reply streams into the
//! party's `speaking` and to the live stream as it is written, and is stored once the turn
//! ends. Pausing lets the turn being said finish; stopping cuts it, keeping what was said. At
//! start-up a party still marked running is paused: nothing runs a bill up on its own after a
//! restart. A shutdown cuts the turn in flight.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use emberhut_api::parties::PartyPage;
use emberhut_api::party as api;
use emberhut_core::party::{
    self, MessageKind, NewParty, NewPartyMessage, Next, Party, PartyMessage, PartyRefusal,
    PartyStatus, Persona,
};
use emberhut_core::{Clock, PartyId};
use emberhut_db::{DbError, PartyStore};
use serde_json::{Map, Value};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::convert::timestamp_to_api;
use crate::services::chat_model::{ChatLine, ChatModel, ChatModelError, ChatPiece, ChatRequest};

// The requests as read: leniently (the contract's typed
// forms are `emberhut_api::party::NewParty` and `PartySay`).

/// A persona the person casts.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewPersona {
    /// The name.
    #[serde(default)]
    pub name: String,
    /// Who they are; blank gets a stock role.
    #[serde(default)]
    pub role: String,
    /// The model; unknown gets the default.
    #[serde(default)]
    pub model: String,
}

/// `POST /api/parties`: a new party, which starts talking at once.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPartyRequest {
    /// The topic or opening line (at most 4,000 characters).
    #[serde(default)]
    pub topic: String,
    /// 2 to 5 personas, each with a name of its own.
    #[serde(default)]
    pub personas: Vec<NewPersona>,
    /// A title; none takes the topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The turn cap, 1 to 100 (12 when left out).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<f64>,
    /// The token cap, 5,000 to 2,000,000 (150,000 when left out).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<f64>,
}

/// `POST /api/parties/{id}/say`: the person's words join the talk (`say`), or become a direction
/// everyone follows (`steer`). A party that waits starts again to answer them.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SayRequest {
    /// What they say (at most 4,000 characters).
    #[serde(default)]
    pub text: String,
    /// `say` (the default) or `steer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// Why a running party was paused at start-up.
const RESTARTED: &str = "The server restarted";

/// How often a turn being written is sent to the live stream, at most.
const SPEAKING_EVERY: Duration = Duration::from_millis(120);

/// How many parties the list shows.
const LIST: u64 = 200;

/// Why a party change was not made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PartyError {
    /// The party, or what was said, is not one the rules take; the text is for the app.
    #[error(transparent)]
    Refused(#[from] PartyRefusal),
    /// No such party.
    #[error("no such party")]
    NotFound,
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// One change in a party, for its live stream.
#[derive(Clone, Debug)]
pub struct PartyNews {
    /// The party.
    pub party: PartyId,
    /// What changed.
    pub event: api::PartyEvent,
}

/// Parties: making them, the person's controls, and the talk.
#[derive(Clone, Debug)]
pub struct PartyService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    store: PartyStore,
    clock: Arc<dyn Clock>,
    model: Arc<dyn ChatModel>,
    stopping: CancellationToken,
    tracker: TaskTracker,
    feed: broadcast::Sender<PartyNews>,
    live: Mutex<Live>,
}

/// What runs now: who is speaking in each party, the stop of each turn in flight, and the
/// parties being driven.
#[derive(Debug, Default)]
struct Live {
    speaking: HashMap<PartyId, api::Speaking>,
    turns: HashMap<PartyId, CancellationToken>,
    driving: HashSet<PartyId>,
}

impl PartyService {
    /// A service over `store`, talking through `model`; `stopping` is cancelled when the
    /// server shuts down.
    #[must_use]
    pub fn new(
        store: PartyStore,
        clock: Arc<dyn Clock>,
        model: Arc<dyn ChatModel>,
        stopping: CancellationToken,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                clock,
                model,
                stopping,
                tracker: TaskTracker::new(),
                feed: broadcast::channel(256).0,
                live: Mutex::default(),
            }),
        }
    }

    fn live(&self) -> MutexGuard<'_, Live> {
        self.inner
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Pauses what was left running: at start-up.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn pause_left_running(&self) -> Result<u64, DbError> {
        self.inner
            .store
            .pause_running(RESTARTED, self.inner.clock.now())
            .await
    }

    /// Every change in every party, as it happens.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<PartyNews> {
        self.inner.feed.subscribe()
    }

    fn push(&self, party: PartyId, event: api::PartyEvent) {
        // No one listening is fine.
        let _unheard = self.inner.feed.send(PartyNews { party, event });
    }

    async fn push_party(&self, id: PartyId) -> Result<(), DbError> {
        if let Some(party) = self.inner.store.get(id).await? {
            let party = self.view(&party, None);
            self.push(id, api::PartyEvent::Party { party: Some(party) });
        }
        Ok(())
    }

    /// `party` as the app sees it, with who is speaking now.
    fn view(&self, party: &Party, last: Option<String>) -> api::Party {
        api::Party {
            id: emberhut_api::ids::PartyId::new(party.id.get()),
            title: party.title.clone(),
            topic: party.topic.clone(),
            status: party.status.as_str().into(),
            why: party.why.clone(),
            personas: party.personas.iter().map(persona).collect(),
            turns: party.turns,
            max_turns: party.max_turns,
            tokens: party.tokens,
            max_tokens: party.max_tokens,
            cost: (party.cost * 10_000.0).round() / 10_000.0,
            created: timestamp_to_api(party.created),
            updated: timestamp_to_api(party.updated),
            speaking: self.live().speaking.get(&party.id).cloned(),
            last,
        }
    }

    /// The list: the parties, the presets, the person's own past personas and the models.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn page(&self) -> Result<api::PartiesPage, DbError> {
        let parties = self
            .inner
            .store
            .list(LIST)
            .await?
            .into_iter()
            .map(|(party, last)| self.view(&party, last))
            .collect();
        Ok(api::PartiesPage {
            parties,
            presets: party::presets().iter().map(persona).collect(),
            recent: self.recent().await?,
            models: party::MODELS
                .iter()
                .map(|m| api::PartyModel {
                    id: m.id.to_owned(),
                    name: m.name.to_owned(),
                    price_in: m.input,
                    price_out: m.output,
                    note: m.note.to_owned(),
                })
                .collect(),
            defaults: Some(api::PartyDefaults {
                max_turns: party::DEFAULT_MAX_TURNS,
                max_tokens: party::DEFAULT_MAX_TOKENS,
                model: party::DEFAULT_MODEL.to_owned(),
            }),
        })
    }

    /// The person's own personas from the last 100 parties (not the presets), newest first,
    /// one per name, at most 12.
    async fn recent(&self) -> Result<Vec<api::Persona>, DbError> {
        let presets = party::presets();
        let mut seen: HashSet<(String, String)> = presets
            .iter()
            .map(|p| (p.name.clone(), p.role.clone()))
            .collect();
        let mut names = HashSet::new();
        let mut mine = Vec::new();
        for cast in self.inner.store.recent_casts(100).await? {
            for p in cast {
                if mine.len() < 12
                    && seen.insert((p.name.clone(), p.role.clone()))
                    && names.insert(p.name.to_lowercase())
                {
                    mine.push(api::Persona {
                        id: emberhut_api::ids::PersonaId::new(format!("recent{}", mine.len() + 1)),
                        name: p.name,
                        role: p.role,
                        model: p.model,
                        color: String::new(),
                    });
                }
            }
        }
        Ok(mine)
    }

    /// Party `id` and its talk.
    ///
    /// # Errors
    ///
    /// [`PartyError::NotFound`], [`PartyError::Database`].
    pub async fn get(&self, id: PartyId) -> Result<PartyPage, PartyError> {
        let party = self.party(id).await?;
        let messages = self.inner.store.messages(id).await?;
        Ok(PartyPage {
            party: self.view(&party, None),
            messages: messages.iter().map(message).collect(),
        })
    }

    async fn party(&self, id: PartyId) -> Result<Party, PartyError> {
        self.inner.store.get(id).await?.ok_or(PartyError::NotFound)
    }

    async fn current(&self, id: PartyId) -> Result<api::Party, PartyError> {
        Ok(self.view(&self.party(id).await?, None))
    }

    /// Makes a party and, with `start`, sets it talking.
    ///
    /// # Errors
    ///
    /// [`PartyError::Refused`] for a party the rules don't take, [`PartyError::Database`].
    pub async fn create(&self, new: &NewParty, start: bool) -> Result<api::Party, PartyError> {
        let plan = party::plan(new)?;
        let status = if start {
            PartyStatus::Running
        } else {
            PartyStatus::Paused
        };
        let opening = NewPartyMessage::host(MessageKind::Say, plan.topic.clone());
        let made = self
            .inner
            .store
            .create(&plan, status, &opening, self.inner.clock.now())
            .await?;
        tracing::info!(
            party = made.id.get(),
            cast = %made
                .personas
                .iter()
                .map(|p| format!("{} on {}", p.name, party::model_name(&p.model)))
                .collect::<Vec<_>>()
                .join(", "),
            "a party started"
        );
        if start {
            self.drive(made.id);
        }
        Ok(self.view(&made, None))
    }

    /// The person's words join the talk (`steer`: a direction everyone follows). A party that
    /// waits (paused, done or stopped) starts again to answer them.
    ///
    /// # Errors
    ///
    /// [`PartyError::NotFound`], [`PartyError::Refused`] for blank or long words,
    /// [`PartyError::Database`].
    pub async fn say(&self, id: PartyId, text: &str, kind: &str) -> Result<api::Party, PartyError> {
        let party = self.party(id).await?;
        let text = party::said(text)?;
        let kind = if kind == "steer" {
            MessageKind::Steer
        } else {
            MessageKind::Say
        };
        let added = self
            .inner
            .store
            .add_message(
                id,
                &NewPartyMessage::host(kind, text),
                self.inner.clock.now(),
            )
            .await?;
        self.push(
            id,
            api::PartyEvent::Message {
                message: message(&added),
            },
        );
        if party.status == PartyStatus::Running {
            self.current(id).await
        } else {
            self.resume(id).await
        }
    }

    /// Pauses a running party: the turn being said finishes, then it waits.
    ///
    /// # Errors
    ///
    /// [`PartyError::NotFound`], [`PartyError::Database`].
    pub async fn pause(&self, id: PartyId) -> Result<api::Party, PartyError> {
        let party = self.party(id).await?;
        if party.status == PartyStatus::Running {
            let why = self
                .live()
                .speaking
                .get(&id)
                .map(|s| format!("{} finishes the turn, then it waits", s.name));
            self.set_status(id, PartyStatus::Paused, why.as_deref())
                .await?;
        }
        self.current(id).await
    }

    /// Stops a running or paused party, and cuts the turn in flight (what was said is kept).
    ///
    /// # Errors
    ///
    /// [`PartyError::NotFound`], [`PartyError::Database`].
    pub async fn stop(&self, id: PartyId) -> Result<api::Party, PartyError> {
        let party = self.party(id).await?;
        if matches!(party.status, PartyStatus::Running | PartyStatus::Paused) {
            self.set_status(id, PartyStatus::Stopped, Some("Stopped by you"))
                .await?;
        }
        if let Some(turn) = self.live().turns.get(&id) {
            turn.cancel();
        }
        self.current(id).await
    }

    /// Sets a party talking again. One that reached a cap gets room for more: 10 more turns,
    /// half its token cap again.
    ///
    /// # Errors
    ///
    /// [`PartyError::NotFound`], [`PartyError::Database`].
    pub async fn resume(&self, id: PartyId) -> Result<api::Party, PartyError> {
        let party = self.party(id).await?;
        if party.status != PartyStatus::Running {
            let (turns, tokens) = party::resumed_caps(&party);
            self.inner
                .store
                .set_caps(id, Some(turns), Some(tokens))
                .await?;
            self.set_status(id, PartyStatus::Running, None).await?;
        }
        self.drive(id);
        self.current(id).await
    }

    /// Sets a party's caps (turns 1 to 200, tokens 5,000 to 5,000,000); one left out stays.
    ///
    /// # Errors
    ///
    /// [`PartyError::NotFound`], [`PartyError::Database`].
    pub async fn caps(
        &self,
        id: PartyId,
        turns: Option<f64>,
        tokens: Option<f64>,
    ) -> Result<api::Party, PartyError> {
        let turns = turns.filter(|t| !t.is_nan()).map(party::caps_turns);
        let tokens = tokens.filter(|t| !t.is_nan()).map(party::caps_tokens);
        if !self.inner.store.set_caps(id, turns, tokens).await? {
            return Err(PartyError::NotFound);
        }
        self.push_party(id).await?;
        self.current(id).await
    }

    /// Deletes a party and its talk, cutting the turn in flight.
    ///
    /// # Errors
    ///
    /// [`PartyError::NotFound`], [`PartyError::Database`].
    pub async fn delete(&self, id: PartyId) -> Result<(), PartyError> {
        if let Some(turn) = self.live().turns.get(&id) {
            turn.cancel();
        }
        if self.inner.store.delete(id).await? {
            Ok(())
        } else {
            Err(PartyError::NotFound)
        }
    }

    /// The list in a few words a line, for the assistant.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn list_text(&self) -> Result<String, DbError> {
        let parties = self.inner.store.list(LIST).await?;
        if parties.is_empty() {
            return Ok("No parties yet.".to_owned());
        }
        Ok(parties
            .iter()
            .map(|(p, _)| {
                format!(
                    "Party {}: {} · {} · {} turns · ${:.2}",
                    p.id,
                    p.title,
                    p.status.as_str(),
                    p.turns,
                    p.cost
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// One party's talk in full, for the assistant.
    ///
    /// # Errors
    ///
    /// [`PartyError::NotFound`], [`PartyError::Database`].
    pub async fn party_text(&self, id: PartyId) -> Result<String, PartyError> {
        let p = self.party(id).await?;
        let said = self.inner.store.messages(id).await?;
        let why = p
            .why
            .as_ref()
            .map_or_else(String::new, |why| format!(": {why}"));
        Ok(format!(
            "Party {}: {} ({}{why}), {}/{} turns, ${:.2}\n\n{}",
            p.id,
            p.title,
            p.status.as_str(),
            p.turns,
            p.max_turns,
            p.cost,
            party::transcript(&said, 200)
        ))
    }

    async fn set_status(
        &self,
        id: PartyId,
        status: PartyStatus,
        why: Option<&str>,
    ) -> Result<(), DbError> {
        self.inner
            .store
            .set_status(id, status, why, self.inner.clock.now())
            .await?;
        self.push_party(id).await
    }

    /// Starts the talk on its own task, unless it is being driven already (the loop sees the
    /// new state at its next turn).
    fn drive(&self, id: PartyId) {
        if !self.live().driving.insert(id) {
            return;
        }
        let service = self.clone();
        self.inner.tracker.spawn(async move {
            if let Err(error) = service.talk(id).await {
                tracing::warn!(party = id.get(), %error, "a party's talk failed");
                let running = matches!(
                    service.inner.store.get(id).await,
                    Ok(Some(p)) if p.status == PartyStatus::Running
                );
                if running {
                    let why = format!(
                        "Something went wrong: {}",
                        party::cut(&error.to_string(), 200)
                    );
                    if let Err(error) = service
                        .set_status(id, PartyStatus::Paused, Some(&why))
                        .await
                    {
                        tracing::error!(party = id.get(), %error, "could not pause a party");
                    }
                }
            }
            let mut live = service.live();
            live.driving.remove(&id);
            live.speaking.remove(&id);
            live.turns.remove(&id);
        });
    }

    /// Turn after turn while the party runs.
    async fn talk(&self, id: PartyId) -> Result<(), DbError> {
        while let Some(p) = self.inner.store.get(id).await? {
            if p.status != PartyStatus::Running || self.inner.stopping.is_cancelled() {
                return Ok(());
            }
            if let Some(why) = party::cap_reached(&p) {
                return self.set_status(id, PartyStatus::Done, Some(&why)).await;
            }
            let said = self.inner.store.messages(id).await?;
            let Some((who, end)) = self.next(&p, &said).await? else {
                return Ok(());
            };
            let still = self.inner.store.get(id).await?;
            if still.is_none_or(|p| p.status != PartyStatus::Running) {
                return Ok(());
            }
            if end {
                return self
                    .set_status(id, PartyStatus::Done, Some("They reached the end of it"))
                    .await;
            }
            self.speak(&p, &who, &said).await?;
        }
        Ok(())
    }

    /// Who speaks next and whether the talk has ended: by the rules, or the supervisor's pick
    /// (the next in turn when it fails or gives none).
    async fn next(
        &self,
        p: &Party,
        said: &[PartyMessage],
    ) -> Result<Option<(Persona, bool)>, DbError> {
        Ok(match party::next(p, said) {
            None => None,
            Some(Next::Speaks(who)) => Some((who, false)),
            Some(Next::Ask {
                choices,
                may_end,
                fallback,
            }) => match self.supervise(p, said, &choices, may_end).await {
                Ok((pick, end)) => Some((pick.unwrap_or(fallback), end && may_end)),
                Err(error) => {
                    tracing::warn!(party = p.id.get(), %error, "the supervisor failed; the next in turn speaks");
                    Some((fallback, false))
                }
            },
        })
    }

    async fn supervise(
        &self,
        p: &Party,
        said: &[PartyMessage],
        choices: &[Persona],
        may_end: bool,
    ) -> Result<(Option<Persona>, bool), SuperviseError> {
        let (system, user) = party::supervisor_prompt(p, said, choices, may_end);
        let request = ChatRequest {
            model: party::SUPERVISOR_MODEL.to_owned(),
            system,
            lines: vec![ChatLine::user(user)],
            max_tokens: Some(party::SUPERVISOR_MAX_TOKENS),
            sampling: Map::new(),
        };
        let reply = self
            .inner
            .model
            .chat(request, &|_| {}, &self.inner.stopping)
            .await?;
        let cost = party::price(party::SUPERVISOR_MODEL, reply.tokens_in, reply.tokens_out);
        self.inner
            .store
            .add_use(
                p.id,
                false,
                u64::from(reply.tokens_in) + u64::from(reply.tokens_out),
                cost,
            )
            .await?;
        Ok(read_pick(&reply.text, choices))
    }

    /// One persona's turn: streamed to `speaking` as it is written, then stored and counted.
    /// A turn cut by a stop keeps what was said; a failed one pauses the party.
    async fn speak(&self, p: &Party, who: &Persona, said: &[PartyMessage]) -> Result<(), DbError> {
        let stop = self.inner.stopping.child_token();
        let speaking = api::Speaking {
            who: emberhut_api::ids::PersonaId::new(who.id.clone()),
            name: who.name.clone(),
            text: String::new(),
        };
        {
            let mut live = self.live();
            live.turns.insert(p.id, stop.clone());
            live.speaking.insert(p.id, speaking.clone());
        }
        self.push(
            p.id,
            api::PartyEvent::Speaking {
                speaking: Some(speaking),
            },
        );
        let (system, user) = party::speaker_prompt(p, who, said);
        let request = ChatRequest {
            model: who.model.clone(),
            system,
            lines: vec![ChatLine::user(user)],
            max_tokens: Some(party::TURN_MAX_TOKENS),
            sampling: Map::new(),
        };
        let written = Mutex::new((String::new(), None::<tokio::time::Instant>));
        let pieces = |piece: ChatPiece| {
            let ChatPiece::Text(text) = piece else {
                return;
            };
            let mut written = written.lock().unwrap_or_else(PoisonError::into_inner);
            written.0.push_str(&text);
            let now = tokio::time::Instant::now();
            let speaking = api::Speaking {
                who: emberhut_api::ids::PersonaId::new(who.id.clone()),
                name: who.name.clone(),
                text: party::clean(&who.name, &written.0),
            };
            self.live().speaking.insert(p.id, speaking.clone());
            if written
                .1
                .is_none_or(|at| now.duration_since(at) >= SPEAKING_EVERY)
            {
                written.1 = Some(now);
                self.push(
                    p.id,
                    api::PartyEvent::Speaking {
                        speaking: Some(speaking),
                    },
                );
            }
        };
        let answer = self.inner.model.chat(request, &pieces, &stop).await;
        let text = written
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .0;
        {
            let mut live = self.live();
            live.speaking.remove(&p.id);
            live.turns.remove(&p.id);
        }
        self.push(p.id, api::PartyEvent::Speaking { speaking: None });
        let (tokens_in, tokens_out, cut) = match answer {
            Ok(reply) => (reply.tokens_in, reply.tokens_out, false),
            Err(_) if stop.is_cancelled() => (0, 0, true),
            Err(error) => {
                if !text.trim().is_empty() {
                    let line = turn_line(who, text.trim().to_owned(), 0, 0.0, true);
                    self.add(p.id, &line).await?;
                }
                let why = format!(
                    "{} couldn't speak ({})",
                    who.name,
                    party::cut(&party::one_line(&error.to_string()), 200)
                );
                return self.set_status(p.id, PartyStatus::Paused, Some(&why)).await;
            }
        };
        let words = party::clean(&who.name, &text);
        let cost = party::price(&who.model, tokens_in, tokens_out);
        let tokens = u64::from(tokens_in) + u64::from(tokens_out);
        self.inner.store.add_use(p.id, true, tokens, cost).await?;
        if !words.is_empty() || cut {
            let words = if words.is_empty() {
                "…".to_owned()
            } else {
                words
            };
            self.add(p.id, &turn_line(who, words, tokens, cost, cut))
                .await?;
        }
        self.push_party(p.id).await
    }

    async fn add(&self, id: PartyId, line: &NewPartyMessage) -> Result<(), DbError> {
        let added = self
            .inner
            .store
            .add_message(id, line, self.inner.clock.now())
            .await?;
        self.push(
            id,
            api::PartyEvent::Message {
                message: message(&added),
            },
        );
        Ok(())
    }

    /// Resolves once no party is being driven (for tests).
    pub async fn idle(&self) {
        while !self.live().driving.is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

/// Why the supervisor gave no pick.
#[derive(Debug, thiserror::Error)]
enum SuperviseError {
    #[error(transparent)]
    Model(#[from] ChatModelError),
    #[error(transparent)]
    Database(#[from] DbError),
}

/// The supervisor's answer: the JSON in it (from its first `{` to its last `}`), its `next`
/// among `choices` and its `end`.
fn read_pick(text: &str, choices: &[Persona]) -> (Option<Persona>, bool) {
    let json = match (text.find('{'), text.rfind('}')) {
        (Some(from), Some(to)) if from < to => serde_json::from_str::<Value>(&text[from..=to]).ok(),
        _ => None,
    };
    let Some(json) = json else {
        return (None, false);
    };
    let next = json.get("next").and_then(Value::as_str);
    let pick = choices
        .iter()
        .find(|c| Some(c.id.as_str()) == next)
        .cloned();
    (pick, json.get("end").and_then(Value::as_bool) == Some(true))
}

fn turn_line(who: &Persona, text: String, tokens: u64, cost: f64, cut: bool) -> NewPartyMessage {
    NewPartyMessage {
        who: who.id.clone(),
        name: who.name.clone(),
        kind: MessageKind::Turn,
        text,
        model: Some(who.model.clone()),
        tokens,
        cost,
        cut,
    }
}

fn persona(p: &Persona) -> api::Persona {
    api::Persona {
        id: emberhut_api::ids::PersonaId::new(p.id.clone()),
        name: p.name.clone(),
        role: p.role.clone(),
        model: p.model.clone(),
        color: p.color.clone(),
    }
}

fn message(m: &PartyMessage) -> api::PartyLine {
    api::PartyLine {
        id: emberhut_api::ids::PartyLineId::new(m.id.get()),
        party: emberhut_api::ids::PartyId::new(m.party.get()),
        who: emberhut_api::ids::PersonaId::new(m.who.clone()),
        name: m.name.clone(),
        kind: m.kind.as_str().into(),
        text: m.text.clone(),
        model: m.model.clone(),
        tokens: m.tokens,
        cost: m.cost,
        cut: m.cut,
        at: timestamp_to_api(m.at),
    }
}

/// A cast from the assistant's tool: a preset's name (or id) alone takes its role.
#[must_use]
pub fn cast_from_tool(name: &str, role: Option<&str>, model: Option<&str>) -> party::PersonaDraft {
    let preset = party::presets()
        .into_iter()
        .find(|p| p.name.eq_ignore_ascii_case(name) || p.id == name);
    party::PersonaDraft {
        name: preset.as_ref().map_or(name, |p| p.name.as_str()).to_owned(),
        role: role
            .map(str::to_owned)
            .or_else(|| preset.map(|p| p.role))
            .unwrap_or_default(),
        model: model.unwrap_or(party::DEFAULT_MODEL).to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_supervisor_s_pick_is_read_from_its_json() {
        let cast: Vec<Persona> = party::presets().into_iter().take(2).collect();
        let (pick, end) = read_pick("Sure: {\"next\": \"skeptic\", \"end\": true} done", &cast);
        assert_eq!(pick.map(|p| p.name), Some("Victor".to_owned()));
        assert!(end);
        assert_eq!(read_pick("no json", &cast), (None, false));
        assert_eq!(read_pick("{\"next\": \"nobody\"}", &cast).0, None);
    }

    #[test]
    fn a_preset_named_alone_takes_its_role() {
        let draft = cast_from_tool("victor", None, None);
        assert_eq!(draft.name, "Victor");
        assert!(draft.role.starts_with("A skeptic"));
        let own = cast_from_tool("Ann", Some("A poet."), Some("claude-haiku-5-5"));
        assert_eq!(
            (own.role.as_str(), own.model.as_str()),
            ("A poet.", "claude-haiku-5-5")
        );
    }
}
