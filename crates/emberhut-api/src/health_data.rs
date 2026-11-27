//! The owner's health and screen time, sent in by their phone and their Mac.
//!
//! - `POST /api/health` (`{ days: [{ date, steps, sleepMin, … }], why?, tz? }`): the phone's
//!   daily Health summaries, never raw samples, one per day (sending a day again replaces it).
//!   `why`: the phone could not send its days and says why. `tz`: the phone's time zone, whose
//!   days are the owner's days.
//! - `POST /api/screentime` (`{ days: [{ date, device, minutes, apps: [{ id, min }] }] }`): the
//!   Mac helper's screen-time totals per day and device.
//!
//! Each day is kept as it came; a day without a `YYYY-MM-DD` `date` (or, for screen time, a
//! `device`) is skipped.

use serde::{Deserialize, Serialize};

/// `POST /api/health`'s body.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HealthUpload {
    /// One summary per day: `date` (`2026-10-12`) and the day's figures (`steps`,
    /// `exerciseMin`, `sleepMin`, `workouts: [{ type, min, at }]`, ...).
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<Object>))]
    pub days: Vec<serde_json::Value>,
    /// Why the phone sent no days (sharing off, the reads refused).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The phone's time zone (`Europe/Paris`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
}

/// `POST /api/screentime`'s body.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ScreenTimeUpload {
    /// One entry per day and device: `date`, `device` (`iPhone`, `Mac`), `minutes`, `apps`.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<Object>))]
    pub days: Vec<serde_json::Value>,
}

/// The answer to either upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Saved {
    /// `true`.
    pub ok: bool,
    /// How many days were kept.
    pub saved: u32,
}
