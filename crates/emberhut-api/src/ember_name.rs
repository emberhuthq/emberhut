//! The ember's name: what the person calls their assistant ("Pip", "Juniper"), one name for
//! every device and for the assistant itself. `GET /api/ember/name` gives it,
//! `POST /api/ember/name` changes it, and the state carries it as `emberName`, so an app shows
//! a new name at its next poll. Without one the ember is "Your ember".

use std::fmt;

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation as _;

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
    /// One line of 1 to 24 characters (an emoji is one) once trimmed, runs of spaces made one;
    /// `null` or blank: back to "Your ember".
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
    /// Over [`MAX_CHARS`] characters (as a person counts them) once trimmed.
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

/// The name `text` gives, as it is kept: trimmed, each run of spaces inside it made one; none
/// when it is blank or the default's own words (in any case), which mean "Your ember". Its
/// length is in the characters a person sees: an emoji, even one joined of several, is one.
///
/// ```
/// use emberhut_api::ember_name::{NameRefusal, clean_name};
///
/// assert_eq!(clean_name("  Pip "), Ok(Some("Pip".to_owned())));
/// assert_eq!(clean_name("Juniper   the  Brave"), Ok(Some("Juniper the Brave".to_owned())));
/// assert_eq!(clean_name(""), Ok(None));
/// assert_eq!(clean_name("your  ember"), Ok(None));
/// assert_eq!(clean_name("Pi\np"), Err(NameRefusal::NotOneLine));
/// ```
///
/// # Errors
///
/// [`NameRefusal`] for a name over [`MAX_CHARS`] characters, or one with a line break, a
/// control character or a character that hides or reorders text.
pub fn clean_name(text: &str) -> Result<Option<String>, NameRefusal> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !is_plain(trimmed) {
        return Err(NameRefusal::NotOneLine);
    }
    let name = collapsed(trimmed);
    if name.eq_ignore_ascii_case(DEFAULT_NAME) {
        return Ok(None);
    }
    if name.graphemes(true).count() > MAX_CHARS {
        return Err(NameRefusal::TooLong);
    }
    Ok(Some(name))
}

/// How long `text` is as a name, in the characters a person sees once it is trimmed and its
/// spaces made one: what a field counts against [`MAX_CHARS`].
///
/// ```
/// use emberhut_api::ember_name::name_length;
///
/// assert_eq!(name_length(" Pip  🔥 "), 5);
/// assert_eq!(name_length("👩\u{200D}🚀"), 1);
/// ```
#[must_use]
pub fn name_length(text: &str) -> usize {
    collapsed(text.trim()).graphemes(true).count()
}

/// The longest start of `text` that is not over [`MAX_CHARS`] as a name: what a field keeps of
/// a name typed or pasted past the limit.
///
/// ```
/// use emberhut_api::ember_name::fit;
///
/// assert_eq!(fit(&"x".repeat(30)), "x".repeat(24));
/// assert_eq!(fit("  Pip  "), "  Pip  ");
/// ```
#[must_use]
pub fn fit(text: &str) -> &str {
    let mut end = 0;
    for (at, grapheme) in text.grapheme_indices(true) {
        let next = at + grapheme.len();
        if name_length(&text[..next]) > MAX_CHARS {
            break;
        }
        end = next;
    }
    &text[..end]
}

/// `name` in at most `most` characters a person sees: itself, or its first `most - 1` and an
/// ellipsis. For where a long name has little room (the composer's prompt).
///
/// ```
/// use emberhut_api::ember_name::shorten;
///
/// assert_eq!(shorten("Pip", 8), "Pip");
/// assert_eq!(shorten("Juniper the Brave", 8), "Juniper…");
/// ```
#[must_use]
pub fn shorten(name: &str, most: usize) -> String {
    if name.graphemes(true).count() <= most {
        return name.to_owned();
    }
    let mut short: String = name.graphemes(true).take(most.saturating_sub(1)).collect();
    short.truncate(short.trim_end().len());
    short.push('\u{2026}');
    short
}

/// `text` with each run of spaces (of any width) made one plain space.
fn collapsed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// Whether `name` is one line of text that shows as it reads: no control character or line
/// separator, and nothing invisible but the joiner inside an emoji (`👩‍🚀`), which joins with
/// what follows it in one character.
fn is_plain(name: &str) -> bool {
    name.graphemes(true).all(|grapheme| {
        let last = grapheme.chars().count() - 1;
        grapheme.chars().enumerate().all(|(at, c)| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                return false;
            }
            !is_invisible(c) || (c == ZERO_WIDTH_JOINER && at < last)
        })
    })
}

/// The joiner that makes one emoji of several.
const ZERO_WIDTH_JOINER: char = '\u{200D}';

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
            "Ju\nniper",
            "Ju\rniper",
            "Ju\tniper",
            "Ju\u{7}niper",
            "Ju\u{202E}niper",
            "Ju\u{200B}niper",
            "Ju\u{2028}niper",
            "Ju\u{200D}niper",
            "Pip\u{200D}",
        ] {
            assert_eq!(clean_name(bad), Err(NameRefusal::NotOneLine), "{bad:?}");
        }
        assert_eq!(
            NameRefusal::TooLong.to_string(),
            "name: 24 characters at most"
        );
    }

    #[test]
    fn runs_of_spaces_inside_are_made_one() {
        assert_eq!(
            clean_name("Juniper \u{a0}\u{3000} the   Brave"),
            Ok(Some("Juniper the Brave".to_owned()))
        );
        assert_eq!(
            clean_name("Your \u{2002} Ember"),
            Ok(None),
            "the default's words"
        );
        assert_eq!(name_length("  Pip   Pip  "), 7);
    }

    #[test]
    fn an_emoji_is_one_character() {
        let astronaut = "\u{1F469}\u{200D}\u{1F680}";
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let wave = "\u{1F44B}\u{1F3FD}";
        let flag = "\u{1F1F8}\u{1F1E6}";
        for emoji in [astronaut, family, wave, flag, "e\u{301}"] {
            assert_eq!(name_length(emoji), 1, "{emoji:?}");
            assert_eq!(clean_name(emoji), Ok(Some(emoji.to_owned())), "{emoji:?}");
        }
        let longest = family.repeat(MAX_CHARS);
        assert_eq!(clean_name(&longest), Ok(Some(longest.clone())));
        assert_eq!(
            clean_name(&format!("{longest}{flag}")),
            Err(NameRefusal::TooLong)
        );
    }

    #[test]
    fn what_is_past_the_limit_is_cut() {
        let flag = "\u{1F1F8}\u{1F1E6}";
        assert_eq!(fit(&flag.repeat(30)), flag.repeat(MAX_CHARS));
        let spaced = format!("  {}   {}", "a".repeat(12), "b".repeat(20));
        assert_eq!(name_length(fit(&spaced)), MAX_CHARS);
        assert_eq!(fit("Juniper"), "Juniper");
    }

    #[test]
    fn a_long_name_is_shortened_with_an_ellipsis() {
        assert_eq!(shorten("Juniper", 7), "Juniper");
        assert_eq!(
            shorten("Juniper the Brave", 9),
            "Juniper\u{2026}",
            "no space before it"
        );
        let flags = "\u{1F1F8}\u{1F1E6}".repeat(5);
        assert_eq!(
            shorten(&flags, 3),
            format!("{}\u{2026}", "\u{1F1F8}\u{1F1E6}".repeat(2))
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
