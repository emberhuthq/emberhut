//! One check: read the feed, choose, and take the chosen payload through every check before it
//! is staged. At most one payload is downloaded a check, and nothing of the running app changes.

use std::time::SystemTime;

use emberhut_api::releases::{Build, FORMAT, Platform, ReleaseFeed};
use emberhut_api::time::Timestamp;

use super::choose::{Choice, Offer, Unfit, choose};
use super::download::{self, Payload};
use super::installer::Staged;
use super::marker::Marker;
use super::record::{Held, Record};
use super::{Engine, signature, trusted};
use crate::backend::{self, Backend};

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// The server has published nothing.
    NothingPublished,
    /// Nothing newer for this Mac; why the newest newer one doesn't fit, if one doesn't.
    Latest {
        /// When the feed was published.
        published: Timestamp,
        /// Why a newer release isn't offered.
        unfit: Option<String>,
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
    /// The releases between this copy and it, newest first (their notes aren't verified).
    pub(crate) earlier: Vec<Earlier>,
    /// When the feed was published.
    pub(crate) published: Timestamp,
}

/// A release skipped on the way to the one offered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Earlier {
    /// Its version.
    pub(crate) version: String,
    /// Its notes, as the feed has them.
    pub(crate) notes: String,
}

/// Why a check failed, worded for the user.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CheckError {
    /// The server couldn't be reached.
    #[error("Couldn't reach your server")]
    Unreachable,
    /// This device has to sign in.
    #[error("Sign in to check for updates")]
    SignIn,
    /// The server refused or failed.
    #[error("Your server couldn't say: {0}")]
    Server(String),
    /// The feed is for another app or platform.
    #[error("Your server's update feed is for another app")]
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
    /// The download or a step on this Mac failed.
    #[error("Couldn't get the update: {0}")]
    Failed(String),
}

impl Engine {
    /// Checks the feed with `backend` at `now`, `staged` being the build already staged (if
    /// any), telling `on_download` the version whose payload it starts to download; gives the
    /// record as it is after.
    pub(crate) fn check(
        &self,
        backend: &dyn Backend,
        mut record: Record,
        staged: Option<Build>,
        now: SystemTime,
        on_download: &dyn Fn(&str),
    ) -> (Record, Result<Found, CheckError>) {
        let found = self.check_with(backend, &mut record, staged, now, on_download);
        (record, found)
    }

    fn check_with(
        &self,
        backend: &dyn Backend,
        record: &mut Record,
        staged: Option<Build>,
        now: SystemTime,
        on_download: &dyn Fn(&str),
    ) -> Result<Found, CheckError> {
        let feed = backend
            .release_feed(&Platform::Macos, FORMAT)
            .map_err(server_error)?;
        let Some(feed) = feed else {
            record.checked(now, None);
            return Ok(Found::NothingPublished);
        };
        record.checked(now, Some(feed.published.as_str().to_owned()));
        let refused = self.refused(&feed, now);
        match choose(&feed, self.build, &self.here, refused) {
            Choice::ByHand => Ok(Found::ByHand),
            Choice::NotOurs => Err(CheckError::NotOurs),
            Choice::Latest(unfit) => Ok(Found::Latest {
                published: feed.published.clone(),
                unfit: unfit.map(|unfit| self.word(&unfit)),
            }),
            Choice::Newer(offer) if staged == Some(offer.release.build) => Ok(Found::AlreadyReady),
            Choice::Newer(offer) => {
                on_download(&offer.release.version);
                self.take(backend, record, &offer, now).map(|staged| {
                    Found::Ready(Ready {
                        staged,
                        notes: offer.release.notes.clone(),
                        earlier: offer
                            .earlier
                            .iter()
                            .map(|release| Earlier {
                                version: release.version.clone(),
                                notes: release.notes.clone(),
                            })
                            .collect(),
                        published: feed.published.clone(),
                    })
                })
            }
        }
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
        backend: &dyn Backend,
        record: &mut Record,
        offer: &Offer<'_>,
        now: SystemTime,
    ) -> Result<Staged, CheckError> {
        let asset = offer.asset;
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
        let Payload { path, sha256 } =
            download::fetch(backend, asset, offer.release.build, self.staging(), free).map_err(
                |error| {
                    if error.is_verification() {
                        refuse(record, error.to_string())
                    } else {
                        CheckError::Failed(error.to_string())
                    }
                },
            )?;
        let comment = trusted::expected(offer.release, &self.here.arch, &sha256);
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
        // The image's bundle is staged (or refused): the image itself has done its work.
        download::remove(&path);
        let staged = checked?;
        record.verified();
        Ok(staged)
    }

    /// Why a newer release isn't offered, worded for the user.
    fn word(&self, unfit: &Unfit<'_>) -> String {
        match unfit {
            Unfit::NeedsMacos(release) => format!(
                "Emberhut {} needs macOS {}",
                release.version,
                release.minimum_os.as_deref().unwrap_or("later than this")
            ),
            Unfit::NoPayload(release) => {
                format!("Emberhut {} has no build for this Mac", release.version)
            }
            Unfit::RolledBack(release) => format!(
                "Emberhut {} didn't start on this Mac; Emberhut stays at {}",
                release.version, self.running.version
            ),
        }
    }
}

/// The backend's error, as a check's.
fn server_error(error: backend::Error) -> CheckError {
    match error {
        backend::Error::Unreachable(_) => CheckError::Unreachable,
        backend::Error::SignInRequired => CheckError::SignIn,
        other => CheckError::Server(
            other
                .server_message()
                .map_or_else(|| other.to_string(), str::to_owned),
        ),
    }
}
