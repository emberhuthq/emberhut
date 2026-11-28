//! Where releases come from: the repository's GitHub Releases. The app lists them through
//! GitHub's REST API, unauthenticated (60 requests an hour per address is plenty for a daily
//! check), picks the newest on its channel by the build its tag gives ([`super::tag`]), and
//! reads that release's feed, the `emberhut-desktop-<platform>.json` file attached to it. The
//! feed's payload URLs are the release's own download links, which GitHub redirects to its file
//! host.
//!
//! No token of any kind is sent: not the hut's device session, not a GitHub login. Only the
//! repository's release downloads are fetched, every answer is read under a cap, and a payload
//! is hashed as it streams to disk. When GitHub limits requests (`403` or `429` with its
//! rate-limit headers) the answer says until when, and the app waits that long.

use std::fmt;
use std::io::{self, Read as _, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use emberhut_api::releases::{Platform, ReleaseFeed};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use ureq::Agent;
use ureq::config::AutoHeaderValue;

use super::channel::Channel;
use super::choose::PRODUCT;
use super::tag::{self, Tag};
use super::version::VERSION;

/// The repository releases are published in.
pub(crate) const REPO: &str = "emberhuthq/emberhut";

/// The most releases listed: the newest page.
const PER_PAGE: u32 = 30;

/// The largest list of releases read (their notes included).
const MAX_LIST: u64 = 4 << 20;

/// The largest feed read: one release, its notes capped at 4,000 characters, and signatures.
const MAX_FEED: u64 = 256 << 10;

/// How long a list or a feed may take, from connecting to the last byte.
const SHORT: Duration = Duration::from_secs(30);

/// How long a payload's download may take.
const LONG: Duration = Duration::from_mins(30);

/// The wait after a limit GitHub gave no time for.
const LIMIT_WAIT: Duration = Duration::from_mins(1);

/// A release as GitHub lists it: only what choosing needs.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct Listed {
    /// Its tag (`v0.1.0-rc.1`).
    pub(crate) tag_name: String,
    /// A draft, not published yet.
    #[serde(default)]
    pub(crate) draft: bool,
    /// Marked a pre-release: a release candidate.
    #[serde(default)]
    pub(crate) prerelease: bool,
    /// The files attached to it.
    #[serde(default)]
    pub(crate) assets: Vec<ListedAsset>,
}

/// A file attached to a release.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub(crate) struct ListedAsset {
    /// Its name.
    pub(crate) name: String,
    /// Where it downloads from.
    pub(crate) browser_download_url: String,
}

/// A release on a channel: its tag read, and where its feed is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OnChannel<'a> {
    /// Its tag, read.
    pub(crate) tag: Tag,
    /// Its feed's download URL.
    pub(crate) feed: &'a str,
}

/// The name of the feed file of `platform`.
#[must_use]
pub(crate) fn feed_name(platform: &Platform) -> String {
    format!("{PRODUCT}-{platform}.json")
}

/// The releases of `listed` on `channel` with a feed for `platform`, the highest build first.
/// Drafts, tags that aren't release tags, and releases without that feed are left out. A
/// release is a candidate when GitHub marks it a pre-release or its tag says so, so a candidate
/// published without the mark still stays off Stable.
#[must_use]
pub(crate) fn on_channel<'a>(
    listed: &'a [Listed],
    channel: Channel,
    platform: &Platform,
) -> Vec<OnChannel<'a>> {
    let name = feed_name(platform);
    let mut found: Vec<OnChannel<'a>> = listed
        .iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let tag = tag::parse(&release.tag_name)?;
            if !channel.takes(release.prerelease || tag.candidate) {
                return None;
            }
            let feed = release.assets.iter().find(|asset| asset.name == name)?;
            Some(OnChannel {
                tag,
                feed: &feed.browser_download_url,
            })
        })
        .collect();
    found.sort_by_key(|release| std::cmp::Reverse(release.tag.build));
    found.dedup_by_key(|release| release.tag.build);
    found
}

/// A payload as it reached the disk: its size and SHA-256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Got {
    /// The bytes written.
    pub(crate) size: u64,
    /// Their SHA-256.
    pub(crate) sha256: [u8; 32],
}

/// The cause behind a [`FetchError`].
pub(crate) type Cause = Box<dyn std::error::Error + Send + Sync>;

/// Why a fetch from GitHub failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FetchError {
    /// No whole answer came: no connection, a timeout, a broken stream.
    #[error("couldn't reach GitHub")]
    Unreachable(#[source] Cause),
    /// GitHub limits this address's requests until then.
    #[error("GitHub is limiting requests from this network")]
    RateLimited {
        /// When it may be asked again.
        until: SystemTime,
    },
    /// Another status than 200.
    #[error("GitHub answered {0}")]
    Status(u16),
    /// The answer didn't read as what was asked for.
    #[error("GitHub's answer didn't read ({0})")]
    Malformed(String),
    /// The file came larger than the cap.
    #[error("the file is larger than it should be")]
    TooLarge,
    /// The address isn't one of the repository's release downloads.
    #[error("{0} isn't one of Emberhut's release files")]
    NotOurs(String),
    /// The file couldn't be written.
    #[error("couldn't save the file")]
    Local(#[source] io::Error),
}

/// Fetches what a check needs. Blocks: call it on a background thread.
pub(crate) trait Source: Send + Sync + fmt::Debug {
    /// The newest releases, as GitHub lists them.
    ///
    /// # Errors
    ///
    /// What [`FetchError`] lists.
    fn releases(&self) -> Result<Vec<Listed>, FetchError>;

    /// The feed at `url`, a release's download.
    ///
    /// # Errors
    ///
    /// What [`FetchError`] lists.
    fn feed(&self, url: &str) -> Result<ReleaseFeed, FetchError>;

    /// Streams the payload at `url`, a release's download, into `to`, never more than `max_len`
    /// bytes, telling `progress` how many bytes have come as they do; its size and SHA-256.
    ///
    /// # Errors
    ///
    /// What [`FetchError`] lists; [`FetchError::TooLarge`] past `max_len`.
    fn download(
        &self,
        url: &str,
        to: &mut dyn Write,
        max_len: u64,
        progress: &dyn Fn(u64),
    ) -> Result<Got, FetchError>;
}

/// When GitHub's answer `status` with these headers says to ask again, at `now`: none unless it
/// is a limit (`403` or `429` with `Retry-After`, or with no requests left before
/// `X-RateLimit-Reset`; a bare `429` waits a minute).
#[must_use]
pub(crate) fn limited_until(
    status: u16,
    retry_after: Option<&str>,
    remaining: Option<&str>,
    reset: Option<&str>,
    now: SystemTime,
) -> Option<SystemTime> {
    if status != 403 && status != 429 {
        return None;
    }
    let number = |text: Option<&str>| text.and_then(|text| text.trim().parse::<u64>().ok());
    if let Some(seconds) = number(retry_after) {
        return Some(now + Duration::from_secs(seconds).max(Duration::from_secs(1)));
    }
    if number(remaining) == Some(0) {
        let reset = number(reset).map(|at| SystemTime::UNIX_EPOCH + Duration::from_secs(at));
        return Some(reset.filter(|at| *at > now).unwrap_or(now + LIMIT_WAIT));
    }
    (status == 429).then_some(now + LIMIT_WAIT)
}

/// [`Source`] over GitHub, with ureq and rustls.
#[derive(Debug)]
pub(crate) struct Github {
    agent: Agent,
    /// The API's list of the repository's releases.
    list: String,
    /// What every release download's address starts with.
    downloads: String,
}

impl Github {
    /// GitHub itself.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::at("https://api.github.com", "https://github.com")
    }

    /// A GitHub whose API is at `api` and whose web pages are at `web` (a stand-in server in
    /// the tests). Redirects are followed (a download goes on to GitHub's file host), only over
    /// HTTPS when `api` is.
    #[must_use]
    pub(crate) fn at(api: &str, web: &str) -> Self {
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .https_only(api.starts_with("https://"))
            .max_redirects(5)
            .timeout_connect(Some(SHORT))
            .user_agent(AutoHeaderValue::Provided(Arc::new(format!(
                "Emberhut/{VERSION}"
            ))))
            .build()
            .new_agent();
        Self {
            agent,
            list: format!("{api}/repos/{REPO}/releases?per_page={PER_PAGE}"),
            downloads: format!("{web}/{REPO}/releases/download/"),
        }
    }

    /// `url`, when it is one of the repository's release downloads.
    fn ours<'u>(&self, url: &'u str) -> Result<&'u str, FetchError> {
        let path = url.strip_prefix(&self.downloads).unwrap_or_default();
        let plain = !path.is_empty()
            && !path.contains("..")
            && !path.contains(['?', '#', '\\'])
            && path.split('/').count() == 2;
        if plain {
            Ok(url)
        } else {
            Err(FetchError::NotOurs(url.to_owned()))
        }
    }

    /// `GET url`, accepting `accept`, within `timeout`; the answer when it is a 200.
    fn get(
        &self,
        url: &str,
        accept: &str,
        timeout: Duration,
    ) -> Result<ureq::http::Response<ureq::Body>, FetchError> {
        let answer = self
            .agent
            .get(url)
            .header("Accept", accept)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .call()
            .map_err(|error| FetchError::Unreachable(Box::new(error)))?;
        let status = answer.status().as_u16();
        if status == 200 {
            return Ok(answer);
        }
        let header = |name: &str| {
            answer
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        let until = limited_until(
            status,
            header("retry-after"),
            header("x-ratelimit-remaining"),
            header("x-ratelimit-reset"),
            SystemTime::now(),
        );
        Err(until.map_or(FetchError::Status(status), |until| {
            FetchError::RateLimited { until }
        }))
    }

    /// The body of a 200 from `url`, read as JSON under `cap` bytes.
    fn json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        accept: &str,
        cap: u64,
    ) -> Result<T, FetchError> {
        let mut answer = self.get(url, accept, SHORT)?;
        let bytes = answer
            .body_mut()
            .with_config()
            .limit(cap)
            .read_to_vec()
            .map_err(|error| match error {
                ureq::Error::BodyExceedsLimit(_) => FetchError::TooLarge,
                other => FetchError::Unreachable(Box::new(other)),
            })?;
        serde_json::from_slice(&bytes).map_err(|error| FetchError::Malformed(error.to_string()))
    }
}

impl Source for Github {
    fn releases(&self) -> Result<Vec<Listed>, FetchError> {
        self.json(&self.list, "application/vnd.github+json", MAX_LIST)
    }

    fn feed(&self, url: &str) -> Result<ReleaseFeed, FetchError> {
        let url = self.ours(url)?;
        self.json(url, "application/octet-stream", MAX_FEED)
    }

    fn download(
        &self,
        url: &str,
        to: &mut dyn Write,
        max_len: u64,
        progress: &dyn Fn(u64),
    ) -> Result<Got, FetchError> {
        let url = self.ours(url)?;
        let mut answer = self.get(url, "application/octet-stream", LONG)?;
        let mut body = answer
            .body_mut()
            .with_config()
            .limit(u64::MAX)
            .reader()
            .take(max_len.saturating_add(1));
        let mut hash = Sha256::new();
        let mut size: u64 = 0;
        let mut buffer = vec![0; 64 << 10];
        loop {
            let read = match body.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(FetchError::Unreachable(Box::new(error))),
            };
            size = size.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
            if size > max_len {
                return Err(FetchError::TooLarge);
            }
            hash.update(&buffer[..read]);
            to.write_all(&buffer[..read]).map_err(FetchError::Local)?;
            progress(size);
        }
        Ok(Got {
            size,
            sha256: hash.finalize().into(),
        })
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A scripted [`Source`]: answers queued in turn, recording every call.

    use std::collections::{HashMap, VecDeque};
    use std::io::Write;
    use std::sync::{Mutex, PoisonError};

    use emberhut_api::releases::ReleaseFeed;
    use sha2::{Digest as _, Sha256};

    use super::{FetchError, Got, Listed, Source};

    /// A call the fake was asked.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum Asked {
        /// The list of releases.
        Releases,
        /// The feed at this URL.
        Feed(String),
        /// The payload at this URL, under this cap.
        Download(String, u64),
    }

    /// The fake.
    #[derive(Debug, Default)]
    pub(crate) struct FakeSource {
        lists: Mutex<VecDeque<Result<Vec<Listed>, FetchError>>>,
        feeds: Mutex<HashMap<String, VecDeque<ReleaseFeed>>>,
        payloads: Mutex<VecDeque<Vec<u8>>>,
        asked: Mutex<Vec<Asked>>,
    }

    impl FakeSource {
        /// Queues the next list of releases (by default: none published).
        pub(crate) fn push_list(&self, list: Result<Vec<Listed>, FetchError>) {
            lock(&self.lists).push_back(list);
        }

        /// Queues the next answer to the feed at `url` (by default: a 404).
        pub(crate) fn push_feed(&self, url: &str, feed: ReleaseFeed) {
            lock(&self.feeds)
                .entry(url.to_owned())
                .or_default()
                .push_back(feed);
        }

        /// Queues the next payload (by default: a 404).
        pub(crate) fn push_payload(&self, bytes: &[u8]) {
            lock(&self.payloads).push_back(bytes.to_vec());
        }

        /// What it was asked, in order.
        pub(crate) fn asked(&self) -> Vec<Asked> {
            lock(&self.asked).clone()
        }

        fn note(&self, asked: Asked) {
            lock(&self.asked).push(asked);
        }
    }

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    impl Source for FakeSource {
        fn releases(&self) -> Result<Vec<Listed>, FetchError> {
            self.note(Asked::Releases);
            lock(&self.lists).pop_front().unwrap_or(Ok(Vec::new()))
        }

        fn feed(&self, url: &str) -> Result<ReleaseFeed, FetchError> {
            self.note(Asked::Feed(url.to_owned()));
            lock(&self.feeds)
                .get_mut(url)
                .and_then(VecDeque::pop_front)
                .ok_or(FetchError::Status(404))
        }

        fn download(
            &self,
            url: &str,
            to: &mut dyn Write,
            max_len: u64,
            progress: &dyn Fn(u64),
        ) -> Result<Got, FetchError> {
            self.note(Asked::Download(url.to_owned(), max_len));
            let bytes = lock(&self.payloads)
                .pop_front()
                .ok_or(FetchError::Status(404))?;
            let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            let kept = usize::try_from(max_len).map_or(bytes.len(), |cap| cap.min(bytes.len()));
            to.write_all(&bytes[..kept]).map_err(FetchError::Local)?;
            if size > max_len {
                return Err(FetchError::TooLarge);
            }
            progress(size);
            Ok(Got {
                size,
                sha256: Sha256::digest(&bytes).into(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::backend::test_server::{Reply, TestServer};

    /// GitHub's list, as its API answers: a stable release, two candidates (one newer than the
    /// final), a draft, a release without a Windows feed, and a tag that isn't a release's.
    pub(crate) fn listed() -> Vec<Listed> {
        let release = |tag: &str, prerelease: bool, draft: bool, platforms: &[&str]| {
            let assets: Vec<_> = platforms
                .iter()
                .map(|platform| {
                    json!({"name": format!("emberhut-desktop-{platform}.json"),
                        "browser_download_url": format!(
                            "https://github.com/{REPO}/releases/download/{tag}/emberhut-desktop-{platform}.json"),
                        "size": 1234})
                })
                .chain([json!({"name": "SHA256SUMS", "browser_download_url": "https://x/SHA256SUMS"})])
                .collect();
            json!({"tag_name": tag, "name": tag, "draft": draft, "prerelease": prerelease,
                "body": "- Rooms open faster (#12)", "assets": assets})
        };
        serde_json::from_value(json!([
            release("v0.2.0-rc.1", true, false, &["macos", "windows"]),
            release("v0.1.0", false, false, &["macos", "windows"]),
            release("v0.3.0", false, true, &["macos", "windows"]),
            release("v0.1.0-rc.2", true, false, &["macos"]),
            release("v0.1.0-rc.1", true, false, &["macos", "windows"]),
            release("v0.1.1-rc.1", false, false, &["macos", "windows"]),
            release("nightly", true, false, &["macos", "windows"]),
        ]))
        .unwrap()
    }

    fn builds(listed: &[Listed], channel: Channel, platform: &Platform) -> Vec<u64> {
        on_channel(listed, channel, platform)
            .iter()
            .map(|release| release.tag.build.get())
            .collect()
    }

    #[test]
    fn stable_takes_finals_and_candidates_take_every_release_the_highest_build_first() {
        let listed = listed();
        // v0.1.1-rc.1 isn't marked a pre-release, but its tag is a candidate's.
        assert_eq!(builds(&listed, Channel::Stable, &Platform::Macos), [10099]);
        assert_eq!(
            builds(&listed, Channel::Candidates, &Platform::Macos),
            [20001, 10101, 10099, 10002, 10001]
        );
        assert_eq!(
            builds(&listed, Channel::Candidates, &Platform::Windows),
            [20001, 10101, 10099, 10001],
            "a release without this platform's feed is passed over"
        );
        let first = &on_channel(&listed, Channel::Stable, &Platform::Windows)[0];
        assert_eq!(
            first.feed,
            "https://github.com/emberhuthq/emberhut/releases/download/v0.1.0/emberhut-desktop-windows.json"
        );
        assert_eq!(on_channel(&[], Channel::Stable, &Platform::Macos), []);
    }

    #[test]
    fn a_limit_says_when_to_ask_again() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
        let at = |secs: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
        assert_eq!(
            limited_until(403, None, Some("0"), Some("1800000600"), now),
            Some(at(1_800_000_600))
        );
        assert_eq!(
            limited_until(429, Some("30"), None, None, now),
            Some(now + Duration::from_secs(30))
        );
        assert_eq!(
            limited_until(429, None, None, None, now),
            Some(now + LIMIT_WAIT)
        );
        assert_eq!(
            limited_until(403, None, Some("0"), Some("1"), now),
            Some(now + LIMIT_WAIT),
            "a reset in the past waits a minute"
        );
        assert_eq!(
            limited_until(403, None, Some("12"), None, now),
            None,
            "a 403 with requests left is a refusal"
        );
        assert_eq!(limited_until(404, Some("30"), None, None, now), None);
    }

    fn feed_json() -> serde_json::Value {
        json!({"format": 1, "product": "emberhut-desktop", "platform": "windows",
            "published": "2026-12-08T10:00:00.000Z", "releases": [{"version": "0.1.0",
            "build": 10099, "date": "2026-12-08", "notes": "- Faster.", "assets": []}]})
    }

    #[test]
    fn the_list_and_a_feed_are_read_from_github_with_no_token() {
        let server = TestServer::start(3, |request| {
            if request.target.starts_with("/repos/") {
                Reply::json(200, &json!([{"tag_name": "v0.1.0", "assets": []}]))
            } else if request
                .target
                .ends_with("/v0.1.0/emberhut-desktop-windows.json")
            {
                // A release download moves on to the file host.
                Reply::text(302, "").header("Location", "/objects/feed.json")
            } else {
                Reply::json(200, &feed_json())
            }
        });
        let base = server.url().to_string();
        let base = base.trim_end_matches('/');
        let github = Github::at(base, base);
        let listed = github.releases().unwrap();
        assert_eq!(listed[0].tag_name, "v0.1.0");
        let feed = github
            .feed(&format!(
                "{base}/{REPO}/releases/download/v0.1.0/emberhut-desktop-windows.json"
            ))
            .unwrap();
        assert_eq!(feed.platform, Platform::Windows);
        let seen = server.finish();
        assert_eq!(
            seen[0].target,
            "/repos/emberhuthq/emberhut/releases?per_page=30"
        );
        assert_eq!(seen[0].headers["accept"], "application/vnd.github+json");
        assert_eq!(
            seen[2].target, "/objects/feed.json",
            "the redirect followed"
        );
        for request in &seen {
            assert!(request.headers["user-agent"].starts_with("Emberhut/"));
            assert!(!request.headers.contains_key("authorization"), "no token");
        }
    }

    #[test]
    fn a_payload_streams_under_its_cap_with_its_hash() {
        let server = TestServer::start(2, |_| Reply::text(200, "a disk image"));
        let base = server.url().to_string();
        let base = base.trim_end_matches('/');
        let github = Github::at(base, base);
        let url = format!("{base}/{REPO}/releases/download/v0.1.0/Emberhut-0.1.0-windows-x64.msi");
        let mut kept = Vec::new();
        let came = std::cell::Cell::new(0);
        let got = github
            .download(&url, &mut kept, 100, &|size| came.set(size))
            .unwrap();
        assert_eq!((got.size, kept.as_slice()), (12, &b"a disk image"[..]));
        assert_eq!(came.get(), 12, "the progress told");
        assert_eq!(
            got.sha256,
            <[u8; 32]>::from(Sha256::digest(b"a disk image"))
        );
        let error = github
            .download(&url, &mut Vec::new(), 5, &|_| {})
            .unwrap_err();
        assert!(matches!(error, FetchError::TooLarge), "{error:?}");
        server.finish();
    }

    #[test]
    fn a_limit_is_told_apart_from_a_refusal() {
        let server = TestServer::start(2, |request| {
            if request.target.contains("per_page") {
                Reply::json(403, &json!({"message": "API rate limit exceeded"}))
                    .header("X-RateLimit-Remaining", "0")
                    .header("X-RateLimit-Reset", "4102444800")
            } else {
                Reply::text(404, "Not Found")
            }
        });
        let base = server.url().to_string();
        let base = base.trim_end_matches('/');
        let github = Github::at(base, base);
        let Err(FetchError::RateLimited { until }) = github.releases() else {
            panic!("a limit")
        };
        assert_eq!(
            until,
            SystemTime::UNIX_EPOCH + Duration::from_hours(1_139_568)
        );
        let missing = github
            .feed(&format!("{base}/{REPO}/releases/download/v9.9.9/x.json"))
            .unwrap_err();
        assert!(matches!(missing, FetchError::Status(404)), "{missing:?}");
        server.finish();
    }

    #[test]
    fn only_the_repositorys_release_downloads_are_fetched() {
        let github = Github::new();
        for url in [
            "https://example.com/emberhuthq/emberhut/releases/download/v0.1.0/x.json",
            "https://github.com/someone/else/releases/download/v0.1.0/x.json",
            "https://github.com/emberhuthq/emberhut/releases/download/v0.1.0/../../x",
            "https://github.com/emberhuthq/emberhut/releases/download/v0.1.0/a/b",
            "https://github.com/emberhuthq/emberhut/releases/download/v0.1.0/x.json?token=1",
            "http://github.com/emberhuthq/emberhut/releases/download/v0.1.0/x.json",
        ] {
            assert!(
                matches!(github.feed(url), Err(FetchError::NotOurs(_))),
                "{url}"
            );
        }
        assert!(
            github
                .ours("https://github.com/emberhuthq/emberhut/releases/download/v0.1.0/x.json")
                .is_ok()
        );
    }
}
