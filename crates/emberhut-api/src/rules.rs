//! The validation rules of the request types (the `validate` feature), beyond garde's own.
//!
//! garde calls a custom rule with references to the field and to the context, whatever their
//! types, hence the signatures.
#![expect(
    clippy::trivially_copy_pass_by_ref,
    clippy::ref_option,
    reason = "garde calls custom rules as fn(&field, &context)"
)]

use crate::chat::ChatFile;
use crate::ids::{ApproverId, ConfiguredAgentId};
use crate::theme::{AccentChange, Appearance, is_hex_colour};

/// Text that isn't empty or only whitespace: the server trims what it reads, so blank text is
/// no text.
pub(crate) fn not_blank(value: &str, _context: &()) -> garde::Result {
    if value.trim().is_empty() {
        return Err(garde::Error::new("must not be blank"));
    }
    Ok(())
}

/// Text on one line, with no control characters (no line breaks, no escapes): a header field
/// shown inside a line the server writes, which it must not be able to end or forge.
pub(crate) fn one_line(value: &str, _context: &()) -> garde::Result {
    if value
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(garde::Error::new(
            "must be one line, with no control characters",
        ));
    }
    Ok(())
}

/// An optional [`one_line`].
pub(crate) fn one_line_opt(value: &Option<String>, context: &()) -> garde::Result {
    value
        .as_deref()
        .map_or(Ok(()), |value| one_line(value, context))
}

/// A paired runner's name ([`crate::runner::pairing::is_runner_name`]).
pub(crate) fn runner_name(value: &str, _context: &()) -> garde::Result {
    if crate::runner::pairing::is_runner_name(value) {
        Ok(())
    } else {
        Err(garde::Error::new(format!(
            "1 to {} of A-Z, a-z, 0-9, - and _, and not {:?}",
            crate::runner::pairing::NAME_MAX,
            crate::runner::pairing::RESERVED_NAME
        )))
    }
}

/// A paired runner's address ([`crate::runner::pairing::is_runner_url`]).
pub(crate) fn runner_url(value: &str, _context: &()) -> garde::Result {
    if crate::runner::pairing::is_runner_url(value) {
        Ok(())
    } else {
        Err(garde::Error::new(
            "an http:// or https:// address with a host, without ? or #",
        ))
    }
}

/// An optional [`runner_url`].
pub(crate) fn runner_url_opt(value: &Option<String>, context: &()) -> garde::Result {
    value
        .as_deref()
        .map_or(Ok(()), |value| runner_url(value, context))
}

/// A Chrome profile's name: with its runs of whitespace made one space, 1 to 24 characters
/// (UTF-16 units), with a letter or digit (`a-z`, `0-9`) its short name (`work` for `Work`) is made of.
pub(crate) fn chrome_label(value: &str, _context: &()) -> garde::Result {
    let cleaned = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = cleaned.encode_utf16().count();
    if length == 0 || length > CHROME_LABEL_MAX {
        return Err(garde::Error::new(format!(
            "1 to {CHROME_LABEL_MAX} characters"
        )));
    }
    if !cleaned.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(garde::Error::new("a letter or digit (a-z, 0-9)"));
    }
    Ok(())
}

/// The longest Chrome profile name the server takes.
const CHROME_LABEL_MAX: usize = 24;

/// A file's server path, when there is one, never climbs out of its folder.
pub(crate) fn no_parent_segments(path: &Option<String>, _context: &()) -> garde::Result {
    if path.as_deref().is_some_and(|path| path.contains("..")) {
        return Err(garde::Error::new("must not contain `..`"));
    }
    Ok(())
}

/// An appearance, when given, is one this contract knows.
pub(crate) fn known_appearance(appearance: &Option<Appearance>, _context: &()) -> garde::Result {
    if appearance
        .as_ref()
        .is_some_and(|appearance| !appearance.is_known())
    {
        return Err(garde::Error::new(format!(
            "one of {}",
            Appearance::KNOWN.join(", ")
        )));
    }
    Ok(())
}

/// A custom accent, when set, is a hex colour.
pub(crate) fn accent_change(accent: &AccentChange, _context: &()) -> garde::Result {
    match accent {
        AccentChange::Set(colour) if !is_hex_colour(colour) => Err(garde::Error::new(
            "a hex colour like #3366FF, or null for the preset's own",
        )),
        _ => Ok(()),
    }
}

/// Text that isn't blank, unless files come with it.
pub(crate) fn text_or_files(files: &[ChatFile]) -> impl FnOnce(&str, &()) -> garde::Result + '_ {
    move |text, &()| {
        if text.trim().is_empty() && files.is_empty() {
            return Err(garde::Error::new("say something or attach a file"));
        }
        Ok(())
    }
}

/// The longest event kind.
const MAX_KIND: usize = 32;

/// An event's kind, when given: at most 32 of `a-z`, `0-9` and `_`. The kind is written into
/// the wake's `- [kind] text` lines, so a bracket or a line break would forge another event.
pub(crate) fn event_kind(kind: &Option<String>, _context: &()) -> garde::Result {
    let Some(kind) = kind else {
        return Ok(());
    };
    if kind.chars().count() > MAX_KIND
        || !kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(garde::Error::new(format!(
            "at most {MAX_KIND} of a-z, 0-9 and _"
        )));
    }
    Ok(())
}

/// The longest agent or approver id.
const SLUG_MAX: usize = 40;

/// Whether `text` is a slug: 1 to 40 of `a-z`, `0-9` and `-`, not starting with `-`.
fn is_slug(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && text.len() <= SLUG_MAX
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// An agent's id: a slug.
pub(crate) fn agent_id(id: &ConfiguredAgentId, _context: &()) -> garde::Result {
    if is_slug(id.as_str()) {
        Ok(())
    } else {
        Err(garde::Error::new(
            "1 to 40 of a-z, 0-9 and '-', not starting with '-'",
        ))
    }
}

/// An approver's id: a slug, and not `owner` or `none`, which name the other approvers.
pub(crate) fn approver_id(id: &ApproverId, _context: &()) -> garde::Result {
    if is_slug(id.as_str()) && !matches!(id.as_str(), "owner" | "none") {
        Ok(())
    } else {
        Err(garde::Error::new(
            "1 to 40 of a-z, 0-9 and '-', not starting with '-', and not 'owner' or 'none'",
        ))
    }
}

/// A currency code: three capital letters.
pub(crate) fn currency(code: &str, _context: &()) -> garde::Result {
    if code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(garde::Error::new("three capital letters, such as EUR"))
    }
}

/// A SHA-256 written as 64 hex digits.
pub(crate) fn sha256_hex(hex: &str, _context: &()) -> garde::Result {
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(garde::Error::new("64 hex digits"))
    }
}
