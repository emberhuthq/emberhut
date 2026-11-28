//! The ember's name: what the person calls their assistant ("Pip", "Juniper"), one name for
//! every device and for the assistant itself. `GET /api/ember/name` gives it,
//! `POST /api/ember/name` changes it, and the state carries it as `emberName`, so an app shows
//! a new name at its next poll. Without one the ember is "Your ember".

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::time::Timestamp;

/// What the ember is called when the person gave it no name.
pub const DEFAULT_NAME: &str = "Your ember";

/// The most characters a name may have.
pub const MAX_CHARS: usize = 24;

/// The ember's name as every device shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EmberName {
    /// The name to show: the person's, or "Your ember".
    pub name: String,
    /// The person gave it; `false`: the default.
    pub custom: bool,
    /// When it last changed; none: never.
    pub at: Option<Timestamp>,
}

impl Default for EmberName {
    /// "Your ember", never changed.
    fn default() -> Self {
        Self {
            name: DEFAULT_NAME.to_owned(),
            custom: false,
            at: None,
        }
    }
}

impl EmberName {
    /// The name `custom` gives (none: the default), changed at `at`.
    #[must_use]
    pub fn new(custom: Option<String>, at: Option<Timestamp>) -> Self {
        match custom {
            Some(name) => Self {
                name,
                custom: true,
                at,
            },
            None => Self {
                at,
                ..Self::default()
            },
        }
    }

    /// The person's own name for it; none for the default.
    #[must_use]
    pub fn custom(&self) -> Option<&str> {
        self.custom.then_some(self.name.as_str())
    }
}

/// `POST /api/ember/name`: the new name, or `null` (or `""`) for the default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EmberNameChange {
    /// One line of 1 to 24 characters once trimmed; `null` or blank: back to "Your ember".
    pub name: Option<String>,
}

/// `POST /api/ember/name`'s answer: `{ ok: true }` and the name as it is now. A name the server
/// doesn't take is refused with 400 and an `ErrorBody` (`{ ok: false, error }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EmberNameReply {
    /// Always `true`.
    pub ok: bool,
    /// The name.
    #[serde(flatten)]
    pub ember: EmberName,
}

/// Why a name was not taken. Its text is what the apps show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NameRefusal {
    /// Over [`MAX_CHARS`] characters once trimmed.
    TooLong,
    /// A line break, a tab or another control or invisible character inside it.
    NotOneLine,
}

impl fmt::Display for NameRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong => write!(f, "name: {MAX_CHARS} characters at most"),
            Self::NotOneLine => f.write_str("name: one line of plain text"),
        }
    }
}

impl std::error::Error for NameRefusal {}

/// The name `text` gives, as it is kept: trimmed; none when it is blank or the default's own
/// words (in any case), which mean "Your ember".
///
/// ```
/// use emberhut_api::ember_name::{NameRefusal, clean_name};
///
/// assert_eq!(clean_name("  Pip "), Ok(Some("Pip".to_owned())));
/// assert_eq!(clean_name(""), Ok(None));
/// assert_eq!(clean_name("your ember"), Ok(None));
/// assert_eq!(clean_name("Ot\nto"), Err(NameRefusal::NotOneLine));
/// ```
///
/// # Errors
///
/// [`NameRefusal`] for a name over [`MAX_CHARS`] characters, or one with a line break, a
/// control character or a character that hides or reorders text.
pub fn clean_name(text: &str) -> Result<Option<String>, NameRefusal> {
    let name = text.trim();
    if name.is_empty() || name.eq_ignore_ascii_case(DEFAULT_NAME) {
        return Ok(None);
    }
    if name.chars().any(|c| c.is_control() || is_invisible(c)) {
        return Err(NameRefusal::NotOneLine);
    }
    if name.chars().count() > MAX_CHARS {
        return Err(NameRefusal::TooLong);
    }
    Ok(Some(name.to_owned()))
}

/// A character that draws nothing or turns text around: zero-width spaces and joiners, the
/// direction marks and overrides, the byte-order mark.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_and_kept() {
        assert_eq!(clean_name("Juniper"), Ok(Some("Juniper".to_owned())));
        assert_eq!(clean_name("\t Pip \n"), Ok(Some("Pip".to_owned())));
        assert_eq!(clean_name("Émile 🔥"), Ok(Some("Émile 🔥".to_owned())));
    }

    #[test]
    fn blank_or_the_default_s_words_mean_the_default() {
        for blank in ["", "   ", "\n", "Your ember", " YOUR EMBER "] {
            assert_eq!(clean_name(blank), Ok(None), "{blank:?}");
        }
    }

    #[test]
    fn twenty_four_characters_at_most() {
        let longest = "é".repeat(MAX_CHARS);
        assert_eq!(clean_name(&longest), Ok(Some(longest.clone())));
        assert_eq!(
            clean_name(&format!("{longest}x")),
            Err(NameRefusal::TooLong)
        );
    }

    #[test]
    fn one_line_of_plain_text() {
        for bad in [
            "Ot\nto",
            "Ot\rto",
            "Ot\tto",
            "Ot\u{7}to",
            "Ot\u{202E}to",
            "Ot\u{200B}to",
        ] {
            assert_eq!(clean_name(bad), Err(NameRefusal::NotOneLine), "{bad:?}");
        }
        assert_eq!(
            NameRefusal::TooLong.to_string(),
            "name: 24 characters at most"
        );
    }

    #[test]
    fn the_default_and_a_custom_name() {
        let default = EmberName::default();
        assert_eq!(default.name, DEFAULT_NAME);
        assert_eq!(default.custom(), None);
        let named = EmberName::new(Some("Pip".to_owned()), None);
        assert_eq!(named.custom(), Some("Pip"));
        let reply = EmberNameReply {
            ok: true,
            ember: named,
        };
        assert_eq!(
            serde_json::to_value(&reply).unwrap(),
            serde_json::json!({ "ok": true, "name": "Pip", "custom": true, "at": null })
        );
    }
}
