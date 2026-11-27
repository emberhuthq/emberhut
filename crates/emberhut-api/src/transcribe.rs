//! `POST /api/transcribe?name=&words=`: speech to text for dictation. The body is the recording
//! itself, raw; the server hands it to its speech-to-text service, told the words the app uses
//! so names come out right, and answers `{ ok: true, text }`.
//!
//! A recording over [`MAX_SIZE`] is refused with 413 and `{ ok: false, error }`; a failing
//! service answers 502 with the same shape (an [`crate::error::ErrorBody`]).

use serde::{Deserialize, Serialize};

use crate::query::push_encoded;

/// The largest recording the server takes: 25 MiB (`25 << 20` bytes, the speech-to-text
/// service's limit).
pub const MAX_SIZE: u64 = 25 << 20;

/// `POST /api/transcribe`'s query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams), into_params(parameter_in = Query))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct TranscribeQuery {
    /// The recording's file name; its extension tells the service the format
    /// (`dictation.m4a`, `dictation.wav`).
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub name: String,
    /// The words to expect, joined with `, ` (names, products, terms).
    #[serde(default)]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub words: String,
}

impl TranscribeQuery {
    /// The query for a recording called `name`, expecting `words`.
    #[must_use]
    pub fn new<W: AsRef<str>>(name: impl Into<String>, words: &[W]) -> Self {
        let mut joined = String::new();
        for word in words {
            if !joined.is_empty() {
                joined.push_str(", ");
            }
            joined.push_str(word.as_ref());
        }
        Self {
            name: name.into(),
            words: joined,
        }
    }

    /// The query string, without the `?`, each value percent-encoded as
    /// [`UploadQuery::to_query`](crate::uploads::UploadQuery::to_query) does.
    ///
    /// ```
    /// use emberhut_api::transcribe::TranscribeQuery;
    ///
    /// let query = TranscribeQuery::new("dictation.wav", &["Emberhut", "Room 2"]);
    /// assert_eq!(query.to_query(), "name=dictation.wav&words=Emberhut%2C%20Room%202");
    /// ```
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut query = String::with_capacity(12 + self.name.len() + self.words.len());
        query.push_str("name=");
        push_encoded(&mut query, &self.name);
        query.push_str("&words=");
        push_encoded(&mut query, &self.words);
        query
    }
}

/// `POST /api/transcribe`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TranscribeReply {
    /// `true`: a failure is an [`crate::error::ErrorBody`] with its status.
    pub ok: bool,
    /// What was said (empty when nothing was).
    #[serde(default)]
    pub text: String,
}
