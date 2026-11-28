//! Pairing runners at run time: a one-time code made on a signed-in device, a new runner's
//! trade of it for a token of its own, the list of every runner the server knows, and
//! forgetting a paired runner.
//!
//! The codes follow device pairing's rules ([`emberhut_core::pairing`]): eight characters,
//! ten minutes, once, only their hashes kept, and a brake on wrong codes for every caller at
//! once. They are kept apart from devices' codes (their own table and their own brake), so a
//! code made for one is never taken for the other, and guessing one doesn't stop the other.
//!
//! A runner's token is 32 random bytes in URL-safe base64. It is kept as text, as a configured
//! runner's is, because the server presents it to the runner whenever it calls it (ADR 0017).
//!
//! A name another runner has (a configured one or a paired one) is never taken over: the new
//! runner is paired under the first free name near it (`build-2`, ...;
//! [`name_candidates`]), and told so. Taking over a name would let anyone holding a code
//! redirect the server's calls for an existing runner to an address of their own, with that
//! runner's sessions behind it; a runner installed again pairs under a new name, and the old
//! one is forgotten from the apps.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use emberhut_api::runner::pairing::name_candidates;
use emberhut_core::pairing::{
    Attempt, CODE_LIFETIME, PairingCode, PairingRefusal, TypedCode, WrongCodeBrake,
};
use emberhut_core::{Clock, RandomError, RandomSource, Token};
use emberhut_db::{DbError, PairedRunner, RunnerPairing, RunnerRedemption};
use jiff::Timestamp;

use crate::services::paired_runners::PairedRunners;
use crate::services::pairing::{CodeMaker, IssuedCode};
use crate::services::runner_api::RunnerApi;
use crate::services::runner_install::{AddressSources, ServerAddress, pick_server};

/// Why a runner was not paired or forgotten, or a code could not be made.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RunnerPairingError {
    /// The code does not work (see [`PairingRefusal`]).
    #[error(transparent)]
    Refused(#[from] PairingRefusal),
    /// The server's configuration gives a runner this name: it can't be forgotten here.
    #[error("the server's configuration names the runner {0}")]
    Configured(String),
    /// Every name near this one is another runner's.
    #[error("no name near {0} is free")]
    NoFreeName(String),
    /// No runner of this name is paired.
    #[error("no paired runner called {0}")]
    NotPaired(String),
    /// A code or a token could not be made.
    #[error("could not make a code or a token")]
    Random(#[from] RandomError),
    /// The database failed.
    #[error("database error")]
    Database(#[from] DbError),
}

/// A runner that asks to pair, its fields already checked against the contract's rules.
#[derive(Clone)]
pub struct PairingAsk {
    /// The code as typed.
    pub code: String,
    /// The name the runner asked for.
    pub name: String,
    /// Its API's address, without a final `/`.
    pub url: String,
    /// Its machine in words.
    pub words: Option<String>,
    /// The system it runs on.
    pub platform: Option<String>,
    /// Its own version.
    pub version: Option<String>,
}

// Its `Debug` leaves the code out: a one-time code is a password until it is used.
impl std::fmt::Debug for PairingAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingAsk")
            .field("name", &self.name)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

/// One runner the server knows, for the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KnownRunner {
    /// One the server's configuration names: its name, its machine in words and its address.
    Configured {
        /// Its name.
        name: String,
        /// Its machine in words.
        place: String,
        /// Its API's address.
        url: String,
    },
    /// One paired at run time.
    Paired(PairedRunner),
}

/// The parts [`RunnerPairingService`] is built from.
#[derive(Clone, Debug)]
pub struct RunnerPairingParts {
    /// The paired runners.
    pub paired: PairedRunners,
    /// The runners the server's configuration names.
    pub configured: Arc<dyn RunnerApi>,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
    /// Where codes and tokens get their randomness.
    pub random: Arc<dyn RandomSource>,
    /// The server's configured addresses another machine can reach, best first for a runner
    /// ([`crate::services::advertise::runner_routes`]).
    pub advertised: Vec<String>,
    /// The server's own address, its links' base.
    pub own: String,
}

/// Makes runner pairing codes, pairs runners with them, lists and forgets them.
#[derive(Debug)]
pub struct RunnerPairingService {
    parts: RunnerPairingParts,
    brake: Mutex<WrongCodeBrake>,
    /// Held while a runner is paired or forgotten, so the rows kept in memory follow the
    /// database's changes in the order they were made.
    writing: tokio::sync::Mutex<()>,
}

impl RunnerPairingService {
    /// The service over `parts`.
    #[must_use]
    pub fn new(parts: RunnerPairingParts) -> Self {
        Self {
            parts,
            brake: Mutex::default(),
            writing: tokio::sync::Mutex::new(()),
        }
    }

    /// Reads the paired runners, as the server starts.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn load(&self) -> Result<(), DbError> {
        self.parts.paired.load().await
    }

    /// Makes a one-time code for a new runner, clearing out expired ones on the way.
    ///
    /// # Errors
    ///
    /// [`RunnerPairingError::Random`] or [`RunnerPairingError::Database`].
    pub async fn new_code(&self, made_by: CodeMaker) -> Result<IssuedCode, RunnerPairingError> {
        let now = self.parts.clock.now();
        let store = self.parts.paired.store();
        store.delete_expired_codes(now).await?;
        let code = PairingCode::generate(self.parts.random.as_ref())?;
        let expires_at = now + CODE_LIFETIME;
        store.insert_code(&code.hash(), now, expires_at).await?;
        tracing::info!(%made_by, "made a pairing code for a new runner");
        Ok(IssuedCode { code, expires_at })
    }

    /// The server's address a new runner is to use: `asked`, the one the app sent, if any;
    /// else a configured one another machine can reach; else `came_in_on`, the address the
    /// request came in on over the server's own listener, when another machine can reach it;
    /// else that one or the server's own, marked unreachable.
    #[must_use]
    pub fn address(&self, asked: Option<&str>, came_in_on: Option<&str>) -> ServerAddress {
        pick_server(AddressSources {
            asked,
            advertised: &self.parts.advertised,
            came_in_on,
            own: &self.parts.own,
        })
    }

    /// Pairs the runner `ask` describes with the code it typed, under the name it asked for
    /// or, when another runner has that one, the first free name near it. The code is used up
    /// in the same transaction that keeps the runner, and the runner's token works from then
    /// on.
    ///
    /// # Errors
    ///
    /// [`RunnerPairingError::Refused`] for a code that does not work, and while too many wrong
    /// codes are recent even for the right one; [`RunnerPairingError::NoFreeName`] when every
    /// name near the one asked for is taken (the code is left unused);
    /// [`RunnerPairingError::Random`] or [`RunnerPairingError::Database`] when the server
    /// fails.
    pub async fn pair(&self, ask: PairingAsk) -> Result<PairedRunner, RunnerPairingError> {
        let now = self.parts.clock.now();
        // The try counts as wrong from here, before the lookup, so tries sent all at once are
        // braked like tries sent one after another.
        let attempt = self.begin(now)?;
        let Some(code) = TypedCode::parse(&ask.code) else {
            self.brake().forgive(attempt);
            return Err(PairingRefusal::Missing.into());
        };
        // A code that does not exist (or a lookup that fails) leaves the try counted.
        let Some(record) = self.parts.paired.store().find_code(&code.hash()).await? else {
            return Err(PairingRefusal::Unknown.into());
        };
        self.brake().forgive(attempt);
        record.check(now)?;
        let configured = self.parts.configured.machines();
        let names: Vec<String> = name_candidates(&ask.name)
            .filter(|name| !configured.contains(name))
            .collect();
        let token = Token::generate(self.parts.random.as_ref())?;
        let pairing = RunnerPairing {
            name: ask.name.clone(),
            url: ask.url,
            words: ask.words,
            token: token.as_str().to_owned(),
            platform: ask.platform,
            version: ask.version,
        };
        let _writing = self.writing.lock().await;
        let paired = match self
            .parts
            .paired
            .store()
            .pair(record.id, &names, &pairing, now)
            .await?
        {
            RunnerRedemption::Paired(paired) => paired,
            RunnerRedemption::NoFreeName => return Err(RunnerPairingError::NoFreeName(ask.name)),
            RunnerRedemption::AlreadyUsed => return Err(PairingRefusal::Used.into()),
            RunnerRedemption::Expired => return Err(PairingRefusal::Expired.into()),
        };
        self.parts.paired.keep(paired.clone());
        tracing::info!(
            runner = %paired.pairing.name,
            asked = %ask.name,
            "paired a runner with a one-time code"
        );
        Ok(paired)
    }

    /// The refusal for a request with no code (missing, not text, or blank): while the brake
    /// is on, that it is on, as for any code; otherwise "type the code", not counted.
    #[must_use]
    pub fn refuse_missing(&self) -> RunnerPairingError {
        match self.begin(self.parts.clock.now()) {
            Ok(attempt) => {
                self.brake().forgive(attempt);
                PairingRefusal::Missing.into()
            }
            Err(braked) => braked,
        }
    }

    /// The refusal for a code no code can be (longer than any code typed): a wrong code,
    /// counted on the brake like any other.
    #[must_use]
    pub fn refuse_wrong(&self) -> RunnerPairingError {
        match self.begin(self.parts.clock.now()) {
            // The try stays counted.
            Ok(_counted) => PairingRefusal::Unknown.into(),
            Err(braked) => braked,
        }
    }

    /// Every runner the server knows: the configured ones in their order, then the paired
    /// ones the configuration doesn't name, by name.
    #[must_use]
    pub fn list(&self) -> Vec<KnownRunner> {
        let configured = &self.parts.configured;
        let names = configured.machines();
        let mut known: Vec<KnownRunner> = names
            .iter()
            .map(|name| KnownRunner::Configured {
                name: name.clone(),
                place: configured.place(name),
                url: configured.address(name).unwrap_or_default(),
            })
            .collect();
        known.extend(
            self.parts
                .paired
                .list()
                .into_iter()
                .filter(|paired| !names.contains(&paired.pairing.name))
                .map(KnownRunner::Paired),
        );
        known
    }

    /// Forgets the paired runner `name`: its token stops working at once, and the server no
    /// longer calls it. A configured runner of the same name, if there is one, stays.
    ///
    /// # Errors
    ///
    /// [`RunnerPairingError::Configured`] for a runner only the configuration names,
    /// [`RunnerPairingError::NotPaired`] for a name no runner has, and
    /// [`RunnerPairingError::Database`].
    pub async fn forget(&self, name: &str) -> Result<(), RunnerPairingError> {
        let _writing = self.writing.lock().await;
        if self.parts.paired.store().forget(name).await? {
            self.parts.paired.drop_runner(name);
            tracing::info!(runner = %name, "forgot a paired runner");
            Ok(())
        } else if self.is_configured(name) {
            Err(RunnerPairingError::Configured(name.to_owned()))
        } else {
            Err(RunnerPairingError::NotPaired(name.to_owned()))
        }
    }

    /// Whether the server's configuration names the runner `name`.
    fn is_configured(&self, name: &str) -> bool {
        self.parts
            .configured
            .machines()
            .iter()
            .any(|known| known == name)
    }

    /// Starts a try on the brake, or the lockout while it is on.
    fn begin(&self, now: Timestamp) -> Result<Attempt, RunnerPairingError> {
        self.brake().begin(now).map_err(|lockout| {
            if lockout.first {
                tracing::warn!(
                    "too many wrong runner pairing codes: no code is taken for up to ten \
                     minutes (restarting the server lifts the brake)"
                );
            }
            PairingRefusal::TooManyWrongCodes {
                retry_after: lockout.retry_after,
            }
            .into()
        })
    }

    fn brake(&self) -> MutexGuard<'_, WrongCodeBrake> {
        // The brake is a list of instants: a panic elsewhere cannot leave it half-written.
        self.brake.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
