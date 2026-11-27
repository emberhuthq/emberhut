//! The owner's daily routines (at their time the assistant is woken in Main with what to do,
//! once that day) and quiet hours (no routines then, and the assistant's own news reaches the
//! phones only when the owner is chatting).
//!
//! Both are settings: `routines` is `[{ at: "HH:MM", what,
//! day? }]` (day 0 Sunday to 6 Saturday; none: every day), `quiet` is `{ from, to }`, times of
//! day on the server's clock in its time zone. Until the owner sets routines, the morning brief
//! and the evening wrap are the routines ([`emberhut_core::briefs`]).

use std::sync::Arc;

use emberhut_core::briefs::{self, DEFAULT_ROUTINES};
use emberhut_core::event::NewEvent;
use emberhut_core::wake::{QuietHours, hour_minute, routine_due, weekday_of};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{DbError, EventStore, SettingKey, SettingStore};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::services::rooms::RoomsService;

/// The routines.
const ROUTINES: SettingKey<Vec<Routine>> = SettingKey::new("routines");
/// The quiet hours.
const QUIET: SettingKey<Quiet> = SettingKey::new("quiet");

/// A routine: at its time (`HH:MM`), on its weekday or every day, the assistant does `what`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Routine {
    /// Its time of day, `HH:MM`.
    pub at: String,
    /// What to do.
    pub what: String,
    /// Its weekday, 0 Sunday to 6 Saturday; none: every day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<u8>,
}

/// Quiet hours: from `from` up to `to` (`HH:MM`, possibly past midnight).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Quiet {
    /// When they start.
    pub from: String,
    /// When they end.
    pub to: String,
}

/// Keeps the routines and quiet hours, and says which routines are due.
#[derive(Clone, Debug)]
pub struct RoutineService {
    settings: SettingStore,
    events: EventStore,
    clock: Arc<dyn Clock>,
    zone: TimeZone,
}

impl RoutineService {
    /// A service over `settings` and `events`, reading local times in `zone`.
    #[must_use]
    pub fn new(
        settings: SettingStore,
        events: EventStore,
        clock: Arc<dyn Clock>,
        zone: TimeZone,
    ) -> Self {
        Self {
            settings,
            events,
            clock,
            zone,
        }
    }

    /// The routines: the owner's, or the morning brief and the evening wrap until they set
    /// their own.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn routines(&self) -> Result<Vec<Routine>, DbError> {
        Ok(self.settings.get(&ROUTINES).await?.unwrap_or_else(|| {
            DEFAULT_ROUTINES
                .iter()
                .map(|routine| Routine {
                    at: routine.at.to_owned(),
                    what: routine.what.to_owned(),
                    day: None,
                })
                .collect()
        }))
    }

    /// The quiet hours, if the owner set them.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn quiet(&self) -> Result<Option<Quiet>, DbError> {
        self.settings.get(&QUIET).await
    }

    /// Replaces the routines and/or the quiet hours, those given.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set(
        &self,
        routines: Option<&[Routine]>,
        quiet: Option<&Quiet>,
    ) -> Result<(), DbError> {
        if let Some(routines) = routines {
            self.settings.set(&ROUTINES, &routines.to_vec()).await?;
        }
        if let Some(quiet) = quiet {
            self.settings.set(&QUIET, quiet).await?;
        }
        Ok(())
    }

    /// Turns one of the assistant's standing wakes on or off: new messages
    /// ([`crate::services::messaging::news::NEWS_WAKES`]) or meeting prep
    /// ([`crate::services::meeting_prep::MEETING_PREP`]).
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_switch(&self, switch: &SettingKey<bool>, on: bool) -> Result<(), DbError> {
        self.settings.set(switch, &on).await
    }

    /// Whether it is quiet hours now. None set (or unreadable times): never.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn is_quiet(&self) -> Result<bool, DbError> {
        let Some(quiet) = self.quiet().await? else {
            return Ok(false);
        };
        let (Some(from), Some(to)) = (hour_minute(&quiet.from), hour_minute(&quiet.to)) else {
            return Ok(false);
        };
        let now = self.clock.now().to_zoned_datetime(self.zone.clone()).time();
        Ok(QuietHours { from, to }.contains(now))
    }

    /// Wakes Main for each routine due now that hasn't woken it today (an event of kind
    /// `routine`, once per day and routine); a brief carries what the rooms said since the
    /// day before. How many woke it.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn tick(&self, rooms: &RoomsService) -> Result<usize, DbError> {
        let now = self.clock.now();
        let local = now.to_zoned_datetime(self.zone.clone());
        let today = local.date().to_string();
        let mut woke = 0;
        for routine in self.routines().await? {
            let Some(at) = hour_minute(&routine.at) else {
                continue;
            };
            let day = routine.day.and_then(weekday_of);
            if routine.day.is_some() && day.is_none() {
                continue;
            }
            if !routine_due(at, day, local.time(), local.weekday()) {
                continue;
            }
            let key = briefs::routine_key(&today, &routine.at);
            if self.events.exists(&key).await? {
                continue;
            }
            let text = if briefs::is_morning_brief(&routine.what) {
                let since = now
                    .checked_sub(SignedDuration::from_hours(24))
                    .unwrap_or(now);
                let said = rooms.said_since(since, None).await?;
                briefs::brief_text(&routine.what, &said)
            } else {
                routine.what.clone()
            };
            let event = NewEvent::new(RoomId::MAIN, "routine", text).once(key);
            if self.events.add(event, now).await?.is_some() {
                woke += 1;
            }
        }
        Ok(woke)
    }

    /// The time zone local times are read in.
    #[must_use]
    pub fn zone(&self) -> &TimeZone {
        &self.zone
    }

    /// What the clock says.
    #[must_use]
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }
}
