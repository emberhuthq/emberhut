//! `GET /health` and `GET /api/loop`: whether the server answers, and how late its runtime is.
//!
//! A probe off the request path asks the server's runtime to run a step once a second; how long
//! the step waits for the runtime is the lag. Work that held the runtime for 50 ms or more is
//! noted by name (a request's method and path, ids as `:x`). Times are whole milliseconds.

use serde::{Deserialize, Serialize};

/// The lag now and over the last minute and five, in whole milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LoopLag {
    /// The latest sample.
    pub ms: u64,
    /// The 95th percentile of the last minute's samples (0 without any).
    pub p95_1m: u64,
    /// The longest of the last minute.
    pub max_1m: u64,
    /// The 95th percentile of the last five minutes' samples.
    pub p95_5m: u64,
    /// The longest of the last five minutes.
    pub max_5m: u64,
}

/// `GET /health`'s answer: `{ ok: true, lag }`. The route needs no sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Health {
    /// Always `true`: the server answered.
    pub ok: bool,
    /// How late the runtime is.
    pub lag: LoopLag,
}

/// Work that held the runtime for 50 ms or more in the last five minutes, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SlowWork {
    /// What it was: a request's method and path (`GET /api/state`), ids and keys as `:x`.
    pub what: String,
    /// How many times.
    pub n: u32,
    /// How long, all told.
    pub ms: u64,
    /// The longest once.
    pub max: u64,
}

/// `GET /api/loop`'s answer: the lag, and the slow work of the last five minutes, the longest
/// in all first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LoopReport {
    /// How late the runtime is.
    pub lag: LoopLag,
    /// What held it.
    pub slow: Vec<SlowWork>,
}
