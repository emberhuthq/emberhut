//! Points in time and days, as the server writes them.
//!
//! The wire carries times as text: an instant is JavaScript's `toISOString()`
//! (`2026-11-24T21:57:53.410Z`: UTC, always milliseconds) and a day is `2026-11-24`. This crate
//! keeps that text as it came instead of parsing it with `chrono` or `time`: nothing in the
//! contract computes with times, a value written back is byte for byte the server's, and the
//! crate stays free of a date library. A client that has to act on a time (when an access token
//! runs out) parses that one value itself.
//!
//! Both types order like the server orders them, by comparing the text: the fixed-width UTC
//! format sorts chronologically.

use std::fmt;

use serde::{Deserialize, Serialize};

/// An instant, as the server writes it: `2026-11-24T21:57:53.410Z` (UTC, milliseconds).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema), schema(value_type = String, format = DateTime))]
pub struct Timestamp(String);

/// A calendar day, as the server writes it: `2026-11-24`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema), schema(value_type = String, format = Date))]
pub struct Day(String);

macro_rules! text_time {
    ($name:ident) => {
        impl $name {
            /// Wraps the server's text. It isn't checked: the contract passes it on as it is.
            #[must_use]
            pub fn new(text: impl Into<String>) -> Self {
                Self(text.into())
            }

            /// The text as the server wrote it.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(text: String) -> Self {
                Self(text)
            }
        }

        impl From<&str> for $name {
            fn from(text: &str) -> Self {
                Self(text.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

text_time!(Timestamp);
text_time!(Day);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_text_as_it_came() {
        let at: Timestamp = serde_json::from_str(r#""2026-11-24T21:57:53.000Z""#).unwrap();
        assert_eq!(at.as_str(), "2026-11-24T21:57:53.000Z");
        assert_eq!(
            serde_json::to_string(&at).unwrap(),
            r#""2026-11-24T21:57:53.000Z""#
        );
        assert_eq!(Day::from("2026-11-24").to_string(), "2026-11-24");
    }

    #[test]
    fn orders_chronologically() {
        assert!(
            Timestamp::new("2026-11-24T09:00:00.000Z")
                < Timestamp::from("2026-11-24T10:00:00.000Z")
        );
        assert!(Day::new("2026-09-30") < Day::from("2026-10-01".to_owned()));
        assert_eq!(Day::new("2026-10-01").as_ref(), "2026-10-01");
    }
}
