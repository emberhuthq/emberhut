//! `POST /api/uploads?name=`: store a file for the chat. The body is the file itself, raw (not
//! JSON, not a form); the answer is the stored file as a [`ChatFile`], to send back whole with a
//! [`TalkRequest`](crate::talk::TalkRequest).
//!
//! The server reads the kind (picture, video or other) from the name's extension, and the
//! size from `Content-Length` only: a body over [`MAX_SIZE`] is refused before it is read.

use serde::{Deserialize, Serialize};

use crate::chat::ChatFile;

/// The largest file the server stores: 500 MiB (`500 * 1024 * 1024` bytes). A larger `Content-Length` is refused with 400 `file is larger than 500 MB`.
pub const MAX_SIZE: u64 = 500 * 1024 * 1024;

/// `POST /api/uploads`'s query: the file's name.
///
/// The server keeps the name as given for the chat, and stores the file under 8 random characters,
/// `-`, and the name's last 120 characters once each run of anything but ASCII letters and digits,
/// `_`, `.`, `-` and spaces has become one `_` (`file` when nothing is left: a name in Arabic is
/// stored as `_`). A query without a name reads as `file`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams), into_params(parameter_in = Query))]
#[cfg_attr(feature = "validate", derive(garde::Validate))]
pub struct UploadQuery {
    /// The file's name, extension included (`photo.png`): it decides the file's kind. Any text:
    /// none is refused.
    #[serde(default = "UploadQuery::unnamed")]
    #[cfg_attr(feature = "validate", garde(skip))]
    pub name: String,
}

impl UploadQuery {
    /// The name of a file that has none.
    pub const UNNAMED: &'static str = "file";

    /// The query for a file called `name`; a blank name is sent as [`Self::UNNAMED`], which the
    /// server would store it as anyway.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        if name.trim().is_empty() {
            return Self {
                name: Self::UNNAMED.to_owned(),
            };
        }
        Self { name }
    }

    fn unnamed() -> String {
        Self::UNNAMED.to_owned()
    }

    /// The query string, without the `?`: `name=` and the name percent-encoded as UTF-8, every
    /// byte but RFC 3986's unreserved characters (`A-Z a-z 0-9 - . _ ~`) escaped, so any
    /// server's decoder reads it back the same (a `+` or `&` in a name stays itself).
    ///
    /// ```
    /// use emberhut_api::uploads::UploadQuery;
    ///
    /// assert_eq!(UploadQuery::new("Week 3 & 4.pdf").to_query(), "name=Week%203%20%26%204.pdf");
    /// ```
    #[must_use]
    pub fn to_query(&self) -> String {
        let mut query = String::with_capacity(5 + self.name.len());
        query.push_str("name=");
        crate::query::push_encoded(&mut query, &self.name);
        query
    }
}

/// `POST /api/uploads`'s answer: the stored file. A refusal (over [`MAX_SIZE`], or the server
/// couldn't store it) is 400 with an [`ErrorBody`](crate::error::ErrorBody) `{ ok: false, error }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UploadReply {
    /// Always `true`: a refusal is an error body.
    pub ok: bool,
    /// The file as stored: its name as given, its `url` (`/uploads/<stored name>`), its kind
    /// from the extension, and its `path` on the server, all four always written.
    pub file: ChatFile,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_is_sent_as_it_is() {
        assert_eq!(
            UploadQuery::new("photo-1764000000.png").to_query(),
            "name=photo-1764000000.png"
        );
        assert_eq!(UploadQuery::new("a~b_c").to_query(), "name=a~b_c");
    }

    #[test]
    fn a_blank_name_is_sent_as_file() {
        assert_eq!(UploadQuery::new("").to_query(), "name=file");
        assert_eq!(UploadQuery::new("  ").name, "file");
    }

    #[test]
    fn anything_else_is_escaped_byte_by_byte() {
        assert_eq!(
            UploadQuery::new("a+b=c?d#e/f").to_query(),
            "name=a%2Bb%3Dc%3Fd%23e%2Ff"
        );
        assert_eq!(UploadQuery::new("é.txt").to_query(), "name=%C3%A9.txt");
    }
}
