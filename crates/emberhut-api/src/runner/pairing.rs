//! Runners paired at run time: a one-time code made on a signed-in device, traded by a new
//! runner for a token of its own, and the list of every runner the server knows.
//!
//! | route | what |
//! |-------|------|
//! | `POST /api/runners/codes` | [`NewRunnerCode`], answered [`RunnerCode`]: a one-time code and the lines that install a runner with it, made on a signed-in device |
//! | `POST /api/runner/pair` | [`PairRunner`], answered [`PairedRunner`]: the runner's name and token; the code is the proof |
//! | `GET  /api/runners` | [`RunnersPage`]: the runners configured on the server and the paired ones |
//! | `DELETE /api/runners/{name}` | forgets a paired runner; its token stops working at once |
//!
//! A refusal is an [`ErrorBody`](crate::error::ErrorBody) whose `error` is for a person: 400
//! for a name or address the server doesn't take, 401 for a code that doesn't work, 409 when
//! no name near the one asked for is free, 429 while too many wrong codes are recent. A name
//! already taken (by a configured runner or a paired one) is not a refusal: the runner is
//! paired under the first free name of [`free_name`]'s, and the answer says which.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::open_enum::open_enum;
use crate::time::Timestamp;

/// The most characters a paired runner's name has.
pub const NAME_MAX: usize = 40;

/// The most characters a paired runner's address has.
pub const URL_MAX: usize = 2048;

/// The one name a paired runner can't have: `DELETE /api/runners/codes` would reach the codes
/// route, not the runner.
pub const RESERVED_NAME: &str = "codes";

/// Whether `name` can be a paired runner's: 1 to [`NAME_MAX`] of `A-Z`, `a-z`, `0-9`, `-` and
/// `_`, and not [`RESERVED_NAME`]. It is the name the runner reports itself by and a path
/// segment of `DELETE /api/runners/{name}`, so nothing else is taken.
///
/// ```
/// use emberhut_api::runner::pairing::is_runner_name;
///
/// assert!(is_runner_name("build-box_2"));
/// assert!(!is_runner_name("build box"));
/// assert!(!is_runner_name(""));
/// assert!(!is_runner_name("codes"));
/// ```
#[must_use]
pub fn is_runner_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && name != RESERVED_NAME
}

/// Whether `url` can be a paired runner's address: `http://` or `https://` and a host, at most
/// [`URL_MAX`] characters, with no white space, control characters, `?` or `#` (the server
/// joins each request's path to it).
///
/// ```
/// use emberhut_api::runner::pairing::is_runner_url;
///
/// assert!(is_runner_url("https://build.example.com:8800/"));
/// assert!(is_runner_url("http://192.0.2.7:8800"));
/// assert!(!is_runner_url("ftp://build.example.com"));
/// assert!(!is_runner_url("https://"));
/// assert!(!is_runner_url("https://build.example.com/?a=1"));
/// ```
#[must_use]
pub fn is_runner_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|rest| {
        url.chars().count() <= URL_MAX
            && !rest.is_empty()
            && !rest.starts_with('/')
            && !rest
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '?' || c == '#')
    })
}

/// The highest number [`free_name`] tries after a name (`build-99`).
pub const FREE_NAME_MAX: u32 = 99;

/// The names a runner that asked for `wanted` may be paired under, in the order they are
/// tried: `wanted`, then `wanted-2`, `wanted-3`, ... up to `wanted-99` ([`FREE_NAME_MAX`]),
/// `wanted` cut short where the number would make the name longer than [`NAME_MAX`]. None
/// when `wanted` isn't a runner's name ([`is_runner_name`]).
///
/// ```
/// use emberhut_api::runner::pairing::name_candidates;
///
/// let names: Vec<String> = name_candidates("build").take(3).collect();
/// assert_eq!(names, ["build", "build-2", "build-3"]);
/// assert_eq!(name_candidates("build").count(), 99);
/// assert_eq!(name_candidates("build box").count(), 0);
/// ```
pub fn name_candidates(wanted: &str) -> impl Iterator<Item = String> + '_ {
    let valid = is_runner_name(wanted);
    let numbered = (2..=FREE_NAME_MAX).filter_map(move |number| {
        let suffix = format!("-{number}");
        // A runner's name is ASCII, so any length is a character boundary.
        let base = wanted.get(..wanted.len().min(NAME_MAX - suffix.len()))?;
        Some(format!("{base}{suffix}"))
    });
    std::iter::once(wanted.to_owned())
        .chain(numbered)
        .filter(move |_| valid)
}

/// The first of [`name_candidates`] that `taken` says isn't taken: the name a runner is
/// paired under when the one it asked for is another runner's. `None` when `wanted` isn't a
/// runner's name or every candidate is taken.
///
/// ```
/// use emberhut_api::runner::pairing::free_name;
///
/// assert_eq!(free_name("build", |_| false).as_deref(), Some("build"));
/// let taken = ["build", "build-2"];
/// assert_eq!(free_name("build", |name| taken.contains(&name)).as_deref(), Some("build-3"));
/// assert_eq!(free_name("build", |_| true), None);
/// ```
#[must_use]
pub fn free_name(wanted: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    name_candidates(wanted).find(|name| !taken(name))
}

/// What a redacting `Debug` shows in place of a secret.
struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// `POST /api/runners/codes`: what the app knows of where a runner reaches this server. The
/// body may be empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct NewRunnerCode {
    /// The server's address as the app reaches it, when the app knows one another machine can
    /// reach: `http://` or `https://` and a host, at most 2048 characters, without `?` or `#`
    /// ([`is_runner_url`]). Left out, the server picks one ([`RunnerCode::server`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::runner_url_opt)))]
    pub server: Option<String>,
}

/// `POST /api/runners/codes`'s answer: a one-time code a new runner pairs with, the server's
/// address the runner is to use, and the lines that install and pair a runner with both. The
/// code works once, for 10 minutes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct RunnerCode {
    /// The code as shown to the person, `ABCD-EFGH`.
    pub code: String,
    /// When it stops working.
    pub expires_at: Timestamp,
    /// The server's address in the lines, without a final `/`: the one the app sent; else the
    /// server's public URL, or the address it listens on, when another machine can reach
    /// them; else the address the request came in on; else the server's own.
    pub server: String,
    /// Whether another machine can reach [`Self::server`]: false for a loopback address
    /// (`127.0.0.1`, `localhost`), which reaches only the server's own computer.
    pub reachable: bool,
    /// The lines that install a runner and pair it with this code.
    pub install: RunnerInstall,
}

/// The lines that install a runner and pair it with a code, each one line to copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunnerInstall {
    /// With the runner already on the machine: `emberhut-runner install --server URL --pair
    /// CODE`.
    pub command: String,
    /// macOS and Linux: the release's `install-runner.sh` through `curl`, which downloads the
    /// runner, checks it and runs the command.
    pub shell: String,
    /// Windows, in PowerShell as an administrator: the release's `install-runner.ps1` through
    /// `irm`, with the address and code in its environment.
    pub powershell: String,
}

// Its `Debug` leaves the code out: a one-time code is a password until it is used. (A plain
// comment, not a doc comment: the schema's description is the type's doc comment.)
impl fmt::Debug for RunnerCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunnerCode")
            .field("code", &Redacted)
            .field("expires_at", &self.expires_at)
            .field("server", &self.server)
            .field("reachable", &self.reachable)
            .finish_non_exhaustive()
    }
}

/// `POST /api/runner/pair`: a new runner, with the one-time code that proves the person let it
/// in. A name another runner has is not refused: the runner is paired under the first free
/// one ([`free_name`]), which the answer gives.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct PairRunner {
    /// The code as typed: case, spaces and the dash don't matter. Not blank, at most 32
    /// characters.
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::not_blank), length(chars, max = 32))
    )]
    pub code: String,
    /// The runner's name, the one it reports itself by: 1 to 40 of `A-Z`, `a-z`, `0-9`, `-` and
    /// `_`, not `codes` ([`is_runner_name`]).
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::runner_name)))]
    pub name: String,
    /// The runner's API, `http://` or `https://` and a host, at most 2048 characters, without
    /// `?` or `#` ([`is_runner_url`]); a final `/` is dropped.
    #[cfg_attr(feature = "validate", garde(custom(crate::rules::runner_url)))]
    pub url: String,
    /// Its machine in words (`Build box`), on one line, at most 60 characters. Left out or blank,
    /// the machine is called by the runner's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "validate",
        garde(custom(crate::rules::one_line_opt), length(chars, max = 60))
    )]
    pub place: Option<String>,
    /// The runner's system (`linux`, `macos`, `windows`), on one line, 1 to 40 characters.
    #[cfg_attr(
        feature = "validate",
        garde(
            custom(crate::rules::not_blank),
            custom(crate::rules::one_line),
            length(chars, max = 40)
        )
    )]
    pub platform: String,
    /// The runner's own version, on one line, 1 to 40 characters.
    #[cfg_attr(
        feature = "validate",
        garde(
            custom(crate::rules::not_blank),
            custom(crate::rules::one_line),
            length(chars, max = 40)
        )
    )]
    pub version: String,
}

// Its `Debug` leaves the code out, as `RunnerCode`'s does.
impl fmt::Debug for PairRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairRunner")
            .field("code", &Redacted)
            .field("name", &self.name)
            .field("url", &self.url)
            .field("place", &self.place)
            .field("platform", &self.platform)
            .field("version", &self.version)
            .finish()
    }
}

/// `POST /api/runner/pair`'s answer: the name the runner is paired under (the one it asked
/// for, or the first free one near it) and its token, which the server sends
/// it with every call (`Authorization: Bearer <token>`) and it presents when it reports back.
/// Shown once.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PairedRunner {
    /// The runner's name on the server: the one it reports itself by from now on.
    pub name: String,
    /// Its token: 32 random bytes in URL-safe base64, without padding.
    pub token: String,
}

// Its `Debug` leaves the token out.
impl fmt::Debug for PairedRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairedRunner")
            .field("name", &self.name)
            .field("token", &Redacted)
            .finish()
    }
}

open_enum! {
    /// Where the server knows a runner from.
    pub enum RunnerSource {
        /// The server's configuration (`--runner NAME=URL`).
        Flag = crate::wire::RUNNER_FROM_FLAG,
        /// Paired at run time (`POST /api/runner/pair`).
        Paired = crate::wire::RUNNER_PAIRED,
    }
}

/// One runner the server knows. Its token is never listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunnerRow {
    /// The runner's name, the one it reports itself by.
    pub name: String,
    /// Its machine in words (its name when it has none).
    pub place: String,
    /// Its API's address.
    pub url: String,
    /// Where the server knows it from.
    pub source: RunnerSource,
    /// Its system, as it said when it paired; left out for a configured runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Its own version, as it said when it paired; left out for a configured runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// When it first paired; left out for a configured runner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<Timestamp>,
}

/// `GET /api/runners`: the runners the server's configuration names, in its order, then the
/// paired ones, by name. A paired runner whose name the configuration also gives is left out:
/// the configured one is the one the server reaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunnersPage {
    /// The runners.
    pub runners: Vec<RunnerRow>,
}

impl RunnersPage {
    /// The runner called `name`, if the server knows one.
    #[must_use]
    pub fn runner(&self, name: &str) -> Option<&RunnerRow> {
        self.runners.iter().find(|runner| runner.name == name)
    }
}
