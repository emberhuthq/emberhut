//! One check: list the releases on GitHub, take the highest on the channel, read its feed,
//! choose, and take the chosen payload through every check before it is staged. At most one
//! payload is downloaded a check, and nothing of the running app changes.

use std::cell::Cell;
use std::time::SystemTime;

use emberhut_api::releases::{Build, FORMAT, ReleaseFeed};
use emberhut_api::time::Timestamp;

use super::channel::Channel;
use super::choose::{Choice, Offer, Unfit, choose};
use super::download::{self, Payload};
use super::github::{self, FetchError, OnChannel};
use super::installer::Staged;
use super::marker::Marker;
use super::record::{Held, Record};
use super::{Engine, signature, system, trusted};

/// The most releases skipped on the way to the one offered whose notes are fetched.
const EARLIER_NOTES: usize = 5;

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// No release on the channel for this system yet.
    NothingPublished,
    /// Nothing newer for this computer on the channel.
    Latest {
        /// When the feed read was published, if one was.
        published: Option<Timestamp>,
        /// Why a newer release isn't offered, if one isn't.
        unfit: Option<String>,
        /// This copy's own release notes, when its release is listed.
        notes: Option<String>,
    },
    /// The build to offer is the one already staged.
    AlreadyReady,
    /// A newer build is downloaded, verified and staged.
    Ready(Ready),
    /// The feed is in a format newer than this app reads.
    ByHand,
}

/// A build ready to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ready {
    /// The staged bundle.
    pub(crate) staged: Staged,
    /// Its notes, verified with the payload.
    pub(crate) notes: String,
    /// The releases on the channel between this copy and it, newest first (their notes aren't
    /// verified).
    pub(crate) earlier: Vec<Earlier>,
    /// When the feed was published.
    pub(crate) published: Timestamp,
}

/// A release skipped on the way to the one offered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Earlier {
    /// Its version.
    pub(crate) version: String,
    /// Its notes, as its feed has them.
    pub(crate) notes: String,
}

/// Why a check failed, worded for the user.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CheckError {
    /// GitHub couldn't be reached.
    #[error("Couldn't reach GitHub")]
    Unreachable,
    /// GitHub limits this network's requests until then.
    #[error("GitHub is limiting requests from this network; Emberhut checks again at {}", clock(*.0))]
    Limited(SystemTime),
    /// GitHub refused or failed.
    #[error("GitHub couldn't say: {0}")]
    Github(String),
    /// The release's feed is for another app or platform, or another release.
    #[error("The release's update file isn't for this app")]
    NotOurs,
    /// The payload failed a check: nothing was installed.
    #[error("The update didn't verify; Emberhut stays at {running} ({why})")]
    Refused {
        /// This copy's version.
        running: String,
        /// The check that failed.
        why: String,
    },
    /// A payload failed verification lately: the next download waits.
    #[error("An update didn't verify lately; Emberhut tries again later")]
    Held,
    /// The download or a step on this computer failed.
    #[error("Couldn't get the update: {0}")]
    Failed(String),
}

/// `at` as the local time of day, `14:05`.
fn clock(at: SystemTime) -> String {
    jiff::Timestamp::try_from(at).map_or_else(
        |_| "a later time".to_owned(),
        |at| {
            at.to_zoned_datetime(jiff::tz::TimeZone::system())
                .strftime("%H:%M")
                .to_string()
        },
    )
}

impl Engine {
    /// Checks the releases on `channel` at `now`, `staged` being the build already staged (if
    /// any), telling `on_download` the version whose payload it downloads and how far it has
    /// come (in percent, once known); gives the record as it is after.
    pub(crate) fn check(
        &self,
        mut record: Record,
        staged: Option<Build>,
        channel: Channel,
        now: SystemTime,
        on_download: &dyn Fn(&str, Option<u8>),
    ) -> (Record, Result<Found, CheckError>) {
        let found = self.check_with(&mut record, staged, channel, now, on_download);
        (record, found)
    }

    fn check_with(
        &self,
        record: &mut Record,
        staged: Option<Build>,
        channel: Channel,
        now: SystemTime,
        on_download: &dyn Fn(&str, Option<u8>),
    ) -> Result<Found, CheckError> {
        if let Some(until) = record.limited_at(now) {
            return Err(CheckError::Limited(until));
        }
        record.attempted(now);
        let listed = self
            .source
            .releases()
            .map_err(|error| fetch_error(error, record))?;
        let platform = &self.here.platform;
        let releases = github::on_channel(&listed, channel, platform);
        let Some(top) = releases.first() else {
            record.checked(now, None);
            return Ok(Found::NothingPublished);
        };
        if top.tag.build <= self.build {
            // Nothing newer on the channel (a candidate on Stable waits for a final above it).
            // This copy's own notes, from its release's feed: one small file.
            let own = github::on_channel(&listed, Channel::Candidates, platform);
            let notes = own
                .iter()
                .find(|release| release.tag.build == self.build)
                .and_then(|release| self.notes_of(release));
            record.checked(now, None);
            return Ok(Found::Latest {
                published: None,
                unfit: None,
                notes,
            });
        }
        let mut feed = self
            .source
            .feed(top.feed)
            .map_err(|error| fetch_error(error, record))?;
        record.checked(now, Some(feed.published.as_str().to_owned()));
        if feed.format > FORMAT {
            return Ok(Found::ByHand);
        }
        // The feed must describe its own release: its tag's version and build.
        feed.releases
            .retain(|release| release.build == top.tag.build && release.version == top.tag.version);
        if feed.releases.is_empty() {
            log::error!(
                "the feed of {} doesn't describe its release",
                top.tag.version
            );
            return Err(CheckError::NotOurs);
        }
        let refused = self.refused(&feed, now);
        match choose(&feed, self.build, &self.here, refused) {
            Choice::ByHand => Ok(Found::ByHand),
            Choice::NotOurs => Err(CheckError::NotOurs),
            Choice::Latest(unfit) => Ok(Found::Latest {
                published: Some(feed.published.clone()),
                unfit: unfit.map(|unfit| self.word(&unfit)),
                notes: None,
            }),
            Choice::Newer(offer) if staged == Some(offer.release.build) => Ok(Found::AlreadyReady),
            Choice::Newer(offer) => {
                let staged = self.take(record, &offer, now, on_download)?;
                let earlier = releases
                    .iter()
                    .filter(|release| {
                        release.tag.build > self.build && release.tag.build < offer.release.build
                    })
                    .take(EARLIER_NOTES)
                    .filter_map(|release| {
                        self.notes_of(release).map(|notes| Earlier {
                            version: release.tag.version.clone(),
                            notes,
                        })
                    })
                    .collect();
                Ok(Found::Ready(Ready {
                    staged,
                    notes: offer.release.notes.clone(),
                    earlier,
                    published: feed.published.clone(),
                }))
            }
        }
    }

    /// The notes `release`'s feed gives it, if the feed can be read (they aren't verified).
    fn notes_of(&self, release: &OnChannel<'_>) -> Option<String> {
        let feed = self
            .source
            .feed(release.feed)
            .inspect_err(|error| log::debug!("no notes for {}: {error}", release.tag.version))
            .ok()?;
        feed.releases
            .into_iter()
            .find(|listed| listed.build == release.tag.build)
            .map(|listed| listed.notes)
    }

    /// The build the marker refuses, removing a refusal record that has done its work.
    fn refused(&self, feed: &ReleaseFeed, now: SystemTime) -> Option<Build> {
        let path = self.marker_path();
        let found = Marker::read(&path)?;
        let newest = feed.newest().map(|release| release.build);
        let refused = found.refused(now, newest);
        if refused.is_none()
            && found.state == super::marker::State::RolledBack
            && found.refusal_over(now, newest)
        {
            download::remove(&path);
        }
        refused
    }

    /// Downloads, verifies and stages `offer`'s payload.
    fn take(
        &self,
        record: &mut Record,
        offer: &Offer<'_>,
        now: SystemTime,
        on_download: &dyn Fn(&str, Option<u8>),
    ) -> Result<Staged, CheckError> {
        let asset = offer.asset;
        let version = offer.release.version.as_str();
        record.may_download(&asset.sha256, now).map_err(|held| {
            if let Held::Until(until) = held {
                log::info!("no update download until {until:?}: one failed lately");
            }
            CheckError::Held
        })?;
        let refuse = |record: &mut Record, why: String| {
            record.failed_verification(&asset.sha256, now);
            log::error!(
                "the update to build {} didn't verify: {why}",
                offer.release.build
            );
            CheckError::Refused {
                running: self.running.version.to_owned(),
                why,
            }
        };
        let free = download::free_space(self.staging());
        let name = self.payload_name(offer.release.build);
        on_download(version, None);
        let shown = Cell::new(None);
        let progress = |came: u64| {
            let percent = percent(came, asset.size);
            if shown.replace(Some(percent)) != Some(percent) {
                on_download(version, Some(percent));
            }
        };
        let Payload { path, sha256 } = download::fetch(
            &*self.source,
            asset,
            &name,
            (self.staging(), free),
            &progress,
        )
        .map_err(|error| {
            if error.is_verification() {
                refuse(record, error.to_string())
            } else {
                CheckError::Failed(error.to_string())
            }
        })?;
        let comment =
            trusted::expected(offer.release, &self.here.platform, &self.here.arch, &sha256);
        let checked = signature::verify(&path, &asset.signature, &comment, self.keys)
            .map_err(|error| refuse(record, error.to_string()))
            .and_then(|()| {
                self.installer.stage(&path, offer.release).map_err(|error| {
                    if error.is_verification() {
                        refuse(record, error.to_string())
                    } else {
                        CheckError::Failed(error.to_string())
                    }
                })
            });
        // The payload is staged (or refused): the download itself has done its work.
        download::remove(&path);
        let staged = checked?;
        record.verified();
        Ok(staged)
    }

    /// Why a newer release isn't offered, worded for the user.
    fn word(&self, unfit: &Unfit<'_>) -> String {
        match unfit {
            Unfit::NeedsNewerSystem(release) => format!(
                "Emberhut {} needs {} {}",
                release.version,
                system::system_name(&self.here.platform),
                release.minimum_os.as_deref().unwrap_or("later than this")
            ),
            Unfit::NoPayload(release) => {
                format!(
                    "Emberhut {} has no build for this computer",
                    release.version
                )
            }
            Unfit::RolledBack(release) => format!(
                "Emberhut {} didn't start on this computer; Emberhut stays at {}",
                release.version, self.running.version
            ),
        }
    }
}

/// `came` bytes of `size` in whole percent, 100 at most.
fn percent(came: u64, size: u64) -> u8 {
    let percent = came.saturating_mul(100).checked_div(size).unwrap_or(100);
    u8::try_from(percent.min(100)).unwrap_or(100)
}

/// A fetch's error, as a check's; a limit is kept in `record`, so no check asks before it ends.
fn fetch_error(error: FetchError, record: &mut Record) -> CheckError {
    log::warn!("the update check couldn't read GitHub: {error}");
    match error {
        FetchError::Unreachable(_) => CheckError::Unreachable,
        FetchError::RateLimited { until } => {
            record.limited(until);
            CheckError::Limited(until)
        }
        FetchError::Status(status) => CheckError::Github(format!("it answered {status}")),
        FetchError::Malformed(why) => CheckError::Github(why),
        FetchError::TooLarge => CheckError::Github("its answer was too large".to_owned()),
        FetchError::NotOurs(_) => CheckError::NotOurs,
        FetchError::Local(error) => CheckError::Failed(error.to_string()),
    }
}
