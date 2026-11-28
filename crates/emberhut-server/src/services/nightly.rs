//! The nightly memory review, built in and on by default (ADR 0034): what a weekly routine the
//! owner had to write by hand used to do, and more, so a new install's memory keeps itself.
//!
//! Every night at [`AT`] in the server's time zone (a missed night runs at the next start), it:
//! - makes the profile again (`_profile.md`);
//! - keeps every rule on trial that ends within the day with no repeat of the mistake since
//!   (judged by the review: `trial_by: review`); one the owner corrected the assistant on again
//!   is put to the owner that night instead;
//! - and, when there is something to decide, asks Main's assistant with an `Event:`: the
//!   candidates the server noticed that may be corrections, to triage (`correction_triage`),
//!   and on [`WEEKLY`] the weekly card, "What I learned this week", the rules on trial and the
//!   facts waiting to be confirmed, with its advice, one answer applying all of it.
//!
//! `memoryNightly` turns it off.

use std::sync::Arc;
use std::time::Duration;

use emberhut_core::memory::cut;
use emberhut_core::memory::forget::{Tombstone, words};
use emberhut_core::memory::learning::{Correction, TRIAL_DAYS, Trial, TrialJudge};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{DbError, SettingKey, SettingStore};
use jiff::SignedDuration;
use jiff::civil::{Time, Weekday};
use jiff::tz::TimeZone;
use tokio_util::sync::CancellationToken;

use super::facts::{FactError, FactService};
use super::learning::LearningService;
use super::profile::ProfileService;
use super::talk::TalkService;

/// Whether the review runs (on unless turned off).
pub const ON: SettingKey<bool> = SettingKey::new("memoryNightly");

/// The last day it ran (`2026-11-28`, in the server's time zone).
const LAST: SettingKey<String> = SettingKey::new("memoryNightlyAt");

/// When it runs, in the server's time zone.
pub const AT: Time = Time::constant(3, 30, 0, 0);

/// The day the weekly card comes with it.
pub const WEEKLY: Weekday = Weekday::Sunday;

/// The most candidates asked about in one night.
const MOST_CANDIDATES: usize = 40;

/// Why a night's review stopped.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NightlyError {
    /// The database failed.
    #[error(transparent)]
    Database(#[from] DbError),
    /// The facts could not be read or the profile written.
    #[error(transparent)]
    Facts(#[from] FactError),
}

/// What one night did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Night {
    /// The trial rules kept, by id.
    pub kept: Vec<i64>,
    /// The trial rules left for the weekly card (the mistake came back), by id.
    pub held: Vec<i64>,
    /// The candidates the assistant was asked to triage.
    pub triage: usize,
    /// The weekly card was asked for.
    pub weekly: bool,
    /// What Main's assistant was told (none: nothing to decide tonight).
    pub asked: Option<String>,
}

/// What the review is built from.
#[derive(Clone, Debug)]
pub struct NightlyParts {
    /// Where the switch and the last night are kept.
    pub settings: SettingStore,
    /// The corrections and rules.
    pub learning: LearningService,
    /// The facts.
    pub facts: FactService,
    /// The profile.
    pub profile: ProfileService,
    /// Main's assistant, told what to decide.
    pub talk: TalkService,
    /// The clock.
    pub clock: Arc<dyn Clock>,
    /// The server's time zone.
    pub zone: TimeZone,
}

/// Runs the nightly memory review.
#[derive(Clone, Debug)]
pub struct NightlyService {
    parts: Arc<NightlyParts>,
}

impl NightlyService {
    /// The review over `parts`.
    #[must_use]
    pub fn new(parts: NightlyParts) -> Self {
        Self {
            parts: Arc::new(parts),
        }
    }

    /// Whether it runs.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn is_on(&self) -> Result<bool, DbError> {
        Ok(self.parts.settings.get(&ON).await?.unwrap_or(true))
    }

    /// Turns it on or off.
    ///
    /// # Errors
    ///
    /// [`DbError`].
    pub async fn set_on(&self, on: bool) -> Result<(), DbError> {
        self.parts.settings.set(&ON, &on).await
    }

    /// Runs one night's review now.
    ///
    /// # Errors
    ///
    /// [`NightlyError`].
    #[expect(
        clippy::too_many_lines,
        reason = "one night's steps in order, and the brief they make"
    )]
    pub async fn run(&self) -> Result<Night, NightlyError> {
        let parts = &self.parts;
        let now = parts.clock.now();
        parts.profile.profile().await?;
        let mut night = Night::default();
        let corrections = parts.learning.corrections(TRIAL_DAYS * 2).await?;
        let soon = now + SignedDuration::from_hours(26);
        let mut trials = Vec::new();
        for trial in parts.learning.trials().await? {
            if trial.ends > soon {
                trials.push(trial);
            } else if repeated(&trial, &corrections) {
                night.held.push(trial.id);
                trials.push(trial);
            } else if parts
                .learning
                .keep_trial(trial.id, TrialJudge::Review)
                .await?
                .is_some()
            {
                night.kept.push(trial.id);
            }
        }
        let mut candidates = parts.learning.untriaged(TRIAL_DAYS).await?;
        candidates.truncate(MOST_CANDIDATES);
        night.triage = candidates.len();
        let pending: Vec<_> = parts
            .facts
            .facts(None)
            .await?
            .into_iter()
            .filter(|fact| fact.pending)
            .collect();
        let weekday = now.to_zoned_datetime(parts.zone.clone()).weekday();
        night.weekly = weekday == WEEKLY && (!trials.is_empty() || !pending.is_empty());
        // The card: every rule on trial and fact waiting, weekly; and any night, the rules that
        // end today with their mistake back, so none lapses unjudged.
        let card: Vec<&Trial> = trials
            .iter()
            .filter(|trial| night.weekly || night.held.contains(&trial.id))
            .collect();
        if candidates.is_empty() && card.is_empty() {
            return Ok(night);
        }
        let mut brief: Vec<String> = vec!["Nightly memory review.".to_owned()];
        if !candidates.is_empty() {
            brief.push(
                "\nThe server noticed these messages of the owner's that may be corrections of \
                 you. For each, decide: a real correction of something you did gets \
                 correction_triage with its id, the lesson that would have avoided it, and \
                 lessonToOwner (that lesson as one short line to the owner, in the second \
                 person); a bug report, a question or anything else gets correction_triage with \
                 its id alone."
                    .to_owned(),
            );
            brief.extend(candidates.iter().map(candidate_line));
        }
        if !card.is_empty() || night.weekly {
            brief.push(if night.weekly {
                "\nIt is the weekly review. Write the owner one short message, \"What I \
                 learned this week\": each rule on trial below with your advice (keep or drop, \
                 and why in a few words), then each fact waiting for them to confirm it, with \
                 your advice (confirm or drop). End by asking them to reply \"yes\" to apply all \
                 of your advice, or to say what to change. When they answer, apply it: lesson_set \
                 with trial to keep a rule, trial_drop to drop one, fact_confirm or fact_drop \
                 for the facts."
                    .to_owned()
            } else {
                "\nThese rules on trial end today, but the owner corrected you on the same thing \
                 since. Ask the owner in one short message whether to keep each (reworded, if \
                 it should be) or drop it, with your advice. When they answer, apply it: \
                 lesson_set with trial to keep a rule, trial_drop to drop one."
                    .to_owned()
            });
            for trial in &card {
                brief.push(format!(
                    "- T{} ({}{}): {}",
                    trial.id,
                    trial.at.strftime("%-d %b"),
                    if night.held.contains(&trial.id) {
                        ", the mistake came back since"
                    } else {
                        ""
                    },
                    cut(&trial.text, 300)
                ));
            }
            for fact in pending.iter().filter(|_| night.weekly) {
                brief.push(format!(
                    "- To confirm: {}: {}{}",
                    fact.id,
                    cut(&fact.text, 300),
                    fact.from
                        .as_deref()
                        .map_or_default(|from| format!(" (from {from})"))
                ));
            }
        } else {
            brief.push("\nThen reply exactly NO_REPLY.".to_owned());
        }
        let brief = brief.join("\n");
        match parts.talk.post(RoomId::MAIN, brief.clone()).await {
            Ok(_) => night.asked = Some(brief),
            Err(error) => {
                tracing::warn!(%error, "the nightly memory review could not ask the assistant");
            }
        }
        Ok(night)
    }

    /// Runs the review every night at [`AT`] until `stop`, and at the first start after a missed
    /// night; never twice in a day.
    pub async fn keep_running(self, stop: CancellationToken) {
        loop {
            if let Err(error) = self.run_if_due().await {
                tracing::warn!(%error, "the nightly memory review failed; it runs again tomorrow");
            }
            let wait = self.until_next();
            tokio::select! {
                () = stop.cancelled() => return,
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    /// Runs tonight's review when it is on, past [`AT`], and not done today.
    async fn run_if_due(&self) -> Result<(), NightlyError> {
        if !self.is_on().await? {
            return Ok(());
        }
        let now = self
            .parts
            .clock
            .now()
            .to_zoned_datetime(self.parts.zone.clone());
        if now.time() < AT {
            return Ok(());
        }
        let today = now.date().to_string();
        if self.parts.settings.get(&LAST).await?.as_deref() == Some(today.as_str()) {
            return Ok(());
        }
        let night = self.run().await?;
        tracing::info!(
            kept = night.kept.len(),
            held = night.held.len(),
            triage = night.triage,
            weekly = night.weekly,
            "the nightly memory review ran"
        );
        self.parts.settings.set(&LAST, &today).await?;
        Ok(())
    }

    /// How long until the next [`AT`] (a minute at least, a day at most).
    fn until_next(&self) -> Duration {
        let now = self.parts.clock.now();
        let zoned = now.to_zoned_datetime(self.parts.zone.clone());
        let next = zoned
            .date()
            .to_datetime(AT)
            .to_zoned_datetime(self.parts.zone.clone())
            .ok()
            .map(|at| at.timestamp())
            .map_or_else(
                || now + SignedDuration::from_hours(24),
                |at| {
                    if at <= now {
                        at + SignedDuration::from_hours(24)
                    } else {
                        at
                    }
                },
            );
        let wait = next.duration_since(now).unsigned_abs();
        wait.clamp(Duration::from_secs(60), Duration::from_hours(24))
    }
}

/// Whether the owner corrected the assistant on the same thing again after `trial` started: a
/// later correction holding two of its lesson's words (all when it has fewer).
fn repeated(trial: &Trial, corrections: &[Correction]) -> bool {
    let lesson = Tombstone::new("", &words(&trial.text), &[]);
    corrections
        .iter()
        .filter(|correction| correction.id != trial.id && correction.at > trial.at)
        .any(|correction| {
            lesson.matches(&format!(
                "{}\n{}\n{}",
                correction.said,
                correction.wrong,
                correction.lesson.as_deref().unwrap_or_default()
            ))
        })
}

/// A candidate's line in the brief.
fn candidate_line(candidate: &Correction) -> String {
    format!(
        "- #{} ({}) they said: \"{}\". {}",
        candidate.id,
        candidate.at.strftime("%-d %b"),
        cut(&candidate.said, 400),
        cut(&candidate.wrong, 400)
    )
}
