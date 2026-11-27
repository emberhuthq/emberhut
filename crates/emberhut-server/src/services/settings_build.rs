//! The newest build of the app per platform: the state's `builds`, and the settings change
//! `{ "build": { "platform", "number" } }` that the release step sends once a store has a build
//! ready (`POST /api/settings`).
//!
//! The settings action `build` calls [`BuildService::apply`] with the body's `build`; the state
//! reads [`BuildService::latest`].

use emberhut_api::builds::LatestBuilds as ApiBuilds;
use emberhut_core::builds::{BuildReport, InvalidBuild, LatestBuilds};
use emberhut_db::{BuildStore, DbError};
use serde_json::Value;

/// The settings key this service takes.
pub const KEY: &str = "build";

/// Why a build was not taken.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BuildError {
    /// The report is not a platform and a whole build number (400, `{ ok: false, error }`).
    #[error(transparent)]
    Invalid(#[from] InvalidBuild),
    /// The database failed (500).
    #[error("could not keep the build")]
    Database(#[from] DbError),
}

/// Reads and takes the newest builds.
#[derive(Clone, Debug)]
pub struct BuildService {
    store: BuildStore,
}

impl BuildService {
    /// The service over `store`.
    #[must_use]
    pub fn new(store: BuildStore) -> Self {
        Self { store }
    }

    /// The newest builds, as the state carries them.
    ///
    /// # Errors
    ///
    /// [`DbError`] when the store fails.
    pub async fn latest(&self) -> Result<ApiBuilds, DbError> {
        Ok(to_api(self.store.latest().await?))
    }

    /// Takes a body's `build`, when it is an object; any other
    /// value changes nothing and gives `None`.
    ///
    /// # Errors
    ///
    /// [`BuildError::Invalid`] for an object that is not a report, [`BuildError::Database`]
    /// when the store fails.
    pub async fn apply(&self, build: &Value) -> Result<Option<ApiBuilds>, BuildError> {
        if !build.is_object() {
            return Ok(None);
        }
        let report = report_from(build)?;
        let builds = self.store.report(report).await?;
        tracing::info!(
            build = report.number,
            platform = %report.platform,
            "a build of the app is ready"
        );
        Ok(Some(to_api(builds)))
    }
}

/// `{ platform, number }` read leniently: `platform` the text `ios` or
/// `mac`; `number` as JavaScript's `Number()` reads it (a number, a numeric string, `true`).
fn report_from(build: &Value) -> Result<BuildReport, InvalidBuild> {
    let platform = build.get("platform").and_then(Value::as_str).unwrap_or("");
    let number = match build.get("number") {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(text)) => js_number(text),
        Some(Value::Bool(yes)) => f64::from(u8::from(*yes)),
        _ => f64::NAN,
    };
    BuildReport::new(platform, number)
}

/// JavaScript's `Number(text)`, for the decimal forms a build number takes: blank is 0, and
/// anything that isn't a number is `NaN`.
fn js_number(text: &str) -> f64 {
    let text = text.trim();
    if text.is_empty() {
        return 0.0;
    }
    let plain = text
        .strip_prefix(['+', '-'])
        .unwrap_or(text)
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
    if plain {
        text.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

fn to_api(builds: LatestBuilds) -> ApiBuilds {
    ApiBuilds {
        ios: builds.ios,
        mac: builds.mac,
    }
}

#[cfg(test)]
mod tests {
    use emberhut_core::builds::Platform;
    use serde_json::json;

    use super::*;

    #[test]
    fn a_report_reads_numbers_as_javascript_does() {
        let read = |build: Value| report_from(&build).map(|r| (r.platform, r.number));
        assert_eq!(
            read(json!({ "platform": "ios", "number": 12 })),
            Ok((Platform::Ios, 12))
        );
        assert_eq!(
            read(json!({ "platform": "mac", "number": " 7 " })),
            Ok((Platform::Mac, 7))
        );
        assert_eq!(
            read(json!({ "platform": "mac", "number": true })),
            Ok((Platform::Mac, 1))
        );
        assert_eq!(
            read(json!({ "platform": "ios", "number": 12.0 })),
            Ok((Platform::Ios, 12))
        );
        for refused in [
            json!({ "platform": "ios" }),
            json!({ "platform": "ios", "number": null }),
            json!({ "platform": "ios", "number": "" }),
            json!({ "platform": "ios", "number": "12 builds" }),
            json!({ "platform": "ios", "number": "Infinity" }),
            json!({ "platform": "ios", "number": [12] }),
            json!({ "platform": "ios", "number": 1.5 }),
            json!({ "platform": ["ios"], "number": 3 }),
            json!({ "number": 3 }),
        ] {
            assert_eq!(report_from(&refused), Err(InvalidBuild), "{refused}");
        }
    }
}
