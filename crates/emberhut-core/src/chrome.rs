//! The owner's own Chrome profiles, each with the browser extension paired to this server: the
//! names they go by, their pairing codes, the extension's versions and its fixed id. No I/O.

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::random::{self, RandomError, RandomSource};

/// The letters a pairing code is made of: no 0/O, 1/I/L.
const CODE_LETTERS: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// A profile's short name for the tools (`chrome:<name>`), from the label the owner gave it:
/// lowercase letters and digits, dashes between, at most 20 characters.
#[must_use]
pub fn slug(label: &str) -> String {
    let lower = label.trim().to_lowercase();
    let mut out = String::new();
    let mut dash = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
    }
    out.chars()
        .take(20)
        .collect::<String>()
        .trim_end_matches('-')
        .to_owned()
}

/// A label that is not one a profile can take.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("A short name for that Chrome profile, like Work or Personal")]
pub struct BadLabel;

/// The label as kept: trimmed, each run of spaces one space; at most 24 characters, with a
/// [`slug`] that isn't empty.
///
/// # Errors
///
/// [`BadLabel`] otherwise.
pub fn clean_label(label: &str) -> Result<String, BadLabel> {
    let clean = label.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() || clean.chars().count() > 24 || slug(&clean).is_empty() {
        return Err(BadLabel);
    }
    Ok(clean)
}

/// A pairing code as kept: its letters and digits only, uppercase.
#[must_use]
pub fn code_of(code: &str) -> String {
    code.to_uppercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

/// A code as shown: `ABCD-EFGH`.
#[must_use]
pub fn show_code(raw: &str) -> String {
    let at = raw.char_indices().nth(4).map_or(raw.len(), |(at, _)| at);
    format!("{}-{}", &raw[..at], &raw[at..])
}

/// A new pairing code: 8 of `CODE_LETTERS`.
///
/// # Errors
///
/// [`RandomError`] when `random` fails.
pub fn new_code(random: &dyn RandomSource) -> Result<String, RandomError> {
    let bytes = random::bytes::<8>(random)?;
    Ok(bytes
        .iter()
        .map(|b| char::from(CODE_LETTERS[usize::from(*b) % CODE_LETTERS.len()]))
        .collect())
}

/// A profile's own token, given once when it pairs: 32 random bytes, URL-safe base64.
///
/// # Errors
///
/// [`RandomError`] when `random` fails.
pub fn new_token(random: &dyn RandomSource) -> Result<String, RandomError> {
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random::bytes::<32>(random)?))
}

/// A token as stored: its SHA-256, URL-safe base64.
#[must_use]
pub fn token_hash(token: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

/// One part of a version, as a number (a part that isn't one counts as 0).
fn part(parts: &[&str], i: usize) -> f64 {
    parts
        .get(i)
        .and_then(|p| p.trim().parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// Whether version `a` (none: `0`) is older than `b`, part by part.
#[must_use]
pub fn older(version: Option<&str>, than: &str) -> bool {
    let mine: Vec<&str> = version.unwrap_or("0").split('.').collect();
    let theirs: Vec<&str> = than.split('.').collect();
    for at in 0..mine.len().max(theirs.len()) {
        let (own, other) = (part(&mine, at), part(&theirs, at));
        if (own - other).abs() > f64::EPSILON {
            return own < other;
        }
    }
    false
}

/// The extension's id, fixed by its manifest's `key` (base64 of its public key): the key's
/// SHA-256, its first 32 hex digits written `a` to `p`, as Chrome makes it. None for a key that
/// isn't base64.
#[must_use]
pub fn extension_id(key: &str) -> Option<String> {
    let clean: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '-' | '_'))
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    if clean.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(clean.trim_end_matches('='))
        .ok()?;
    let sum = Sha256::digest(&bytes);
    Some(
        sum.iter()
            .take(16)
            .flat_map(|b| [b >> 4, b & 0x0f])
            .map(|n| char::from(b'a' + n))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_their_names() {
        assert_eq!(slug("  My Work!  "), "my-work");
        assert_eq!(slug("Personal"), "personal");
        assert_eq!(slug("***"), "");
        assert_eq!(
            clean_label("  Side   project "),
            Ok("Side project".to_owned())
        );
        assert_eq!(clean_label("!!"), Err(BadLabel));
        assert_eq!(clean_label(&"x".repeat(25)), Err(BadLabel));
    }

    #[test]
    fn codes_read_either_way() {
        assert_eq!(code_of("abcd-efgh"), "ABCDEFGH");
        assert_eq!(show_code("ABCDEFGH"), "ABCD-EFGH");
        let code = new_code(&crate::OsRandom).unwrap_or_default();
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|b| CODE_LETTERS.contains(&b)));
    }

    #[test]
    fn versions_compare_part_by_part() {
        assert!(older(Some("0.6.4"), "0.6.10"));
        assert!(!older(Some("1.0"), "1.0.0"));
        assert!(older(None, "0.1"));
        assert!(!older(Some("2"), "1.9"));
    }

    #[test]
    fn the_id_is_the_key_s_sum_in_a_to_p() {
        let id = extension_id("AAAA").unwrap_or_default();
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| (b'a'..=b'p').contains(&b)));
        assert_eq!(extension_id(""), None);
        assert_eq!(token_hash("x").len(), 43);
    }
}
