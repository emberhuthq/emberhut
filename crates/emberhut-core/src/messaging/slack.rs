//! The owner's Slack, mirrored: one row per message, from the Mac helper (Slack's
//! notifications and the owner's own messages from the Slack app's saved state) and, in a
//! workspace where the owner installed the Slack app with a user token, from Slack's Web API.
//!
//! A conversation is a workspace and its name as the owner sees it (a person for a DM,
//! `#name` for a channel, the others for a group DM). Its chat id is
//! `slack:<workspace>:<name>`.

use jiff::Timestamp;

/// A Slack message as it is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlackMessage {
    /// `<workspace>:<channel>:<ts>` for one read from Slack; the Mac helper's own id for one
    /// it sent.
    pub id: String,
    /// The workspace's name.
    pub workspace: String,
    /// The conversation's name.
    pub who: String,
    /// What it says, as the assistant reads it.
    pub body: String,
    /// When it was sent.
    pub at: Timestamp,
    /// The owner wrote it.
    pub from_me: bool,
    /// Who wrote it, when known.
    pub sender: Option<String>,
    /// Slack's channel id, for one read from Slack (none for a notification).
    pub channel: Option<String>,
}

/// A conversation with messages in a stretch of time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlackConversation {
    /// The workspace's name.
    pub workspace: String,
    /// The conversation's name.
    pub who: String,
    /// Its messages in the stretch.
    pub messages: u64,
    /// The owner's among them.
    pub mine: u64,
    /// When the newest one was sent.
    pub last_at: Timestamp,
}

/// The prefix every Slack chat id starts with.
pub const CHAT_PREFIX: &str = "slack:";

/// The chat id of `who` in workspace `workspace`.
///
/// ```
/// use emberhut_core::messaging::slack::{chat_id, parse_chat};
///
/// let id = chat_id("Acme", "#releases");
/// assert_eq!(id, "slack:Acme:#releases");
/// assert_eq!(parse_chat(&id), Some(("Acme".to_owned(), "#releases".to_owned())));
/// ```
#[must_use]
pub fn chat_id(workspace: &str, who: &str) -> String {
    format!("{CHAT_PREFIX}{workspace}:{who}")
}

/// The workspace and conversation of a chat id: whatever follows `slack:<workspace>:` is the
/// name (it may hold `:`). None when the id has no workspace part, or the name is empty.
#[must_use]
pub fn parse_chat(id: &str) -> Option<(String, String)> {
    let rest = id.strip_prefix(CHAT_PREFIX)?;
    let (workspace, who) = rest.split_once(':')?;
    (!workspace.is_empty() && !who.is_empty()).then(|| (workspace.to_owned(), who.to_owned()))
}

/// Whether a conversation's name is a channel's or a group DM's: more than one person reads
/// it.
#[must_use]
pub fn is_group(who: &str) -> bool {
    who.contains('#') || who.contains(',')
}

/// Whether `id` looks like a Slack file's id (`F` and capitals or digits).
#[must_use]
pub fn is_file_id(id: &str) -> bool {
    id.len() > 1
        && id.starts_with('F')
        && id[1..]
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// The secret file that holds a workspace's user token, by the workspace's short name.
#[must_use]
pub fn user_token_secret(short: &str) -> String {
    format!("slack-{short}-user")
}

/// The workspace's short name in a secret file's name `slack-<short>-user`.
///
/// ```
/// use emberhut_core::messaging::slack::workspace_of_secret;
///
/// assert_eq!(workspace_of_secret("slack-acme-user"), Some("acme"));
/// assert_eq!(workspace_of_secret("slack-acme-app"), None);
/// assert_eq!(workspace_of_secret("slack-Acme-user"), None);
/// ```
#[must_use]
pub fn workspace_of_secret(file: &str) -> Option<&str> {
    let short = file.strip_prefix("slack-")?.strip_suffix("-user")?;
    (!short.is_empty()
        && short
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
    .then_some(short)
}

/// Slack's three control characters in `text`, escaped so they show as typed.
#[must_use]
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The instant of a Slack `ts` (`"1792310400.000100"`, seconds since the epoch).
#[must_use]
pub fn ts_instant(ts: &str) -> Option<Timestamp> {
    let (seconds, fraction) = ts.split_once('.').unwrap_or((ts, ""));
    let seconds: i64 = seconds.parse().ok()?;
    let millis: i64 = format!("{fraction:0<3}")
        .get(..3)
        .and_then(|millis| millis.parse().ok())
        .unwrap_or(0);
    Timestamp::from_millisecond(seconds.checked_mul(1000)?.checked_add(millis)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_ids_need_a_workspace_and_a_name() {
        assert_eq!(parse_chat("slack:Acme"), None);
        assert_eq!(parse_chat("slack::x"), None);
        assert_eq!(parse_chat("wa:12"), None);
        assert_eq!(
            parse_chat("slack:Acme:a:b"),
            Some(("Acme".to_owned(), "a:b".to_owned()))
        );
    }

    #[test]
    fn a_ts_reads_to_the_millisecond() {
        assert_eq!(
            ts_instant("1792310400.000100").map(Timestamp::as_millisecond),
            Some(1_792_310_400_000)
        );
        assert_eq!(
            ts_instant("1792310400.5").map(Timestamp::as_millisecond),
            Some(1_792_310_400_500)
        );
        assert_eq!(ts_instant("soon"), None);
    }

    #[test]
    fn file_ids_and_groups() {
        assert!(is_file_id("F0ABC12"));
        assert!(!is_file_id("F"));
        assert!(!is_file_id("f0abc"));
        assert!(is_group("#releases"));
        assert!(is_group("Ann, Bo"));
        assert!(!is_group("Ann"));
        assert_eq!(escape("a<b>&c"), "a&lt;b&gt;&amp;c");
    }
}
