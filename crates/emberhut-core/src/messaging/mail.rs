//! The owner's email, read only: each Gmail account signed in, its mail kept one row per
//! message. A chat is one correspondent in one account: `email:<account>:<address>`.

use std::sync::LazyLock;

use jiff::Timestamp;
use regex::Regex;

use super::text::html_decode;

/// A message as it is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailMessage {
    /// `<account>:<Gmail's message id>`.
    pub id: String,
    /// The account's short name.
    pub account: String,
    /// The chat: `email:<account>:<the correspondent's address>`.
    pub chat: String,
    /// The correspondent's name (their address when they gave none).
    pub who: String,
    /// Who wrote it, `Name <address>`; [`super::OWNER`] for the owner.
    pub sender: String,
    /// The owner wrote it.
    pub from_me: bool,
    /// Its subject.
    pub subject: String,
    /// Its text, without the quoted mail under it.
    pub body: String,
    /// When Gmail says it was sent or received.
    pub at: Timestamp,
    /// When the server got it (never before `at`): what is new since a look.
    pub got: Timestamp,
    /// In the inbox or sent, and not in Promotions, Social or Forums: worth waking for.
    pub loud: bool,
    /// Gmail's thread id.
    pub thread: Option<String>,
}

/// A correspondent's mail in a stretch of time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailConversation {
    /// The chat id.
    pub chat: String,
    /// The account.
    pub account: String,
    /// The correspondent's name.
    pub who: String,
    /// Their messages in the stretch.
    pub messages: u64,
    /// The owner's among them.
    pub mine: u64,
    /// The newest one's time.
    pub last_at: Timestamp,
}

/// The prefix every email chat id starts with.
pub const CHAT_PREFIX: &str = "email:";

/// The one scope the server asks Google for: reading mail, nothing else.
pub const SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";

/// The secret file of the server's Google OAuth client id.
pub const CLIENT_ID_SECRET: &str = "gmail-client-id";

/// The secret file of the server's Google OAuth client secret.
pub const CLIENT_SECRET_SECRET: &str = "gmail-client-secret";

/// Whether `account` is a usable short name: 2 to 20 lowercase letters and digits.
#[must_use]
pub fn is_account_name(account: &str) -> bool {
    (2..=20).contains(&account.len())
        && account
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// The secret file that holds `account`'s refresh token.
#[must_use]
pub fn refresh_secret(account: &str) -> String {
    format!("gmail-{account}-refresh")
}

/// The account a secret file's name `gmail-<account>-refresh` signs in.
///
/// ```
/// use emberhut_core::messaging::mail::account_of_secret;
///
/// assert_eq!(account_of_secret("gmail-home-refresh"), Some("home"));
/// assert_eq!(account_of_secret("gmail-client-id"), None);
/// ```
#[must_use]
pub fn account_of_secret(file: &str) -> Option<&str> {
    let account = file.strip_prefix("gmail-")?.strip_suffix("-refresh")?;
    (!account.is_empty()
        && account
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
    .then_some(account)
}

/// How an account's mail is named: `Email` for the one called `personal`, `Email <Account>`
/// for any other.
///
/// ```
/// use emberhut_core::messaging::mail::source_of;
///
/// assert_eq!(source_of("personal"), "Email");
/// assert_eq!(source_of("work"), "Email Work");
/// ```
#[must_use]
pub fn source_of(account: &str) -> String {
    if account == "personal" {
        return "Email".to_owned();
    }
    let mut chars = account.chars();
    chars.next().map_or_else(
        || "Email".to_owned(),
        |first| format!("Email {}{}", first.to_ascii_uppercase(), chars.as_str()),
    )
}

/// The chat id of `address` in `account` (the address in lower case).
#[must_use]
pub fn chat_id(account: &str, address: &str) -> String {
    format!("{CHAT_PREFIX}{account}:{}", address.to_lowercase())
}

/// The account and address of an email chat id; none when it isn't one.
///
/// ```
/// use emberhut_core::messaging::mail::parse_chat;
///
/// assert_eq!(
///     parse_chat("email:home:ann@example.com"),
///     Some(("home".to_owned(), "ann@example.com".to_owned()))
/// );
/// assert_eq!(parse_chat("email:Home:ann@example.com"), None);
/// ```
#[must_use]
pub fn parse_chat(chat: &str) -> Option<(String, String)> {
    let (account, address) = chat.strip_prefix(CHAT_PREFIX)?.split_once(':')?;
    let named = !account.is_empty()
        && account
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    (named && !address.is_empty()).then(|| (account.to_owned(), address.to_owned()))
}

/// Whether `chat` is an email chat id.
#[must_use]
pub fn is_mail(chat: &str) -> bool {
    chat.starts_with(CHAT_PREFIX)
}

/// One address in a header: the name given with it (maybe empty) and the address, in lower
/// case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    /// The name given with it, or empty.
    pub name: String,
    /// The address, in lower case.
    pub address: String,
}

/// Every address in a `From`, `To` or `Cc` header, in order.
///
/// ```
/// use emberhut_core::messaging::mail::addresses;
///
/// let found = addresses(r#""Ann Lee" <ANN@example.com>, bo@example.org"#);
/// assert_eq!(found[0].name, "Ann Lee");
/// assert_eq!(found[0].address, "ann@example.com");
/// assert_eq!(found[1].name, "");
/// assert_eq!(found[1].address, "bo@example.org");
/// ```
#[must_use]
pub fn addresses(header: &str) -> Vec<Address> {
    static ADDRESS: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r#"(?:"?([^"<,]*?)"?\s*)?<([^<>\s]+@[^<>\s]+)>|([^\s,<>"]+@[^\s,<>"]+)"#).ok()
    });
    let Some(found) = ADDRESS.as_ref() else {
        return Vec::new();
    };
    found
        .captures_iter(header)
        .filter_map(|m| {
            if let Some(address) = m.get(2) {
                let name = m.get(1).map_or("", |name| name.as_str());
                Some(Address {
                    name: name.trim().trim_matches(['\'', '"']).to_owned(),
                    address: address.as_str().to_lowercase(),
                })
            } else {
                m.get(3).map(|address| Address {
                    name: String::new(),
                    address: address.as_str().to_lowercase(),
                })
            }
        })
        .collect()
}

/// An HTML mail's text: styles, scripts and the head dropped, line breaks where blocks end,
/// tags gone and character references decoded.
#[must_use]
pub fn html_text(html: &str) -> String {
    static HIDDEN: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r"(?i)<style[^>]*>[\s\S]*?</style>|<script[^>]*>[\s\S]*?</script>|<head[^>]*>[\s\S]*?</head>",
        )
        .ok()
    });
    static BREAKS: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>|</(?:p|div|tr|li|h\d)>").ok());
    static TAGS: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new("<[^>]+>").ok());
    let mut text = html.to_owned();
    for (rule, with) in [(&HIDDEN, " "), (&BREAKS, "\n"), (&TAGS, " ")] {
        if let Some(rule) = rule.as_ref() {
            text = rule.replace_all(&text, with).into_owned();
        }
    }
    html_decode(&text)
}

/// What the owner would read of a mail: its lines up to where the quoted earlier mail starts
/// (`On … wrote:`, Outlook's header, Arabic Gmail's), without `>` lines, spaces and blank
/// runs collapsed.
///
/// ```
/// use emberhut_core::messaging::mail::body_text;
///
/// let text = "Sounds good.\n\n\n\nThanks\nOn Mon, 3 Nov 2026 at 09:12, Ann <a@example.com> wrote:\n> earlier";
/// assert_eq!(body_text(text), "Sounds good.\n\nThanks");
/// ```
#[must_use]
pub fn body_text(text: &str) -> String {
    static QUOTED: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r"^(?:On .{6,200} wrote:|-{2,}\s*Original Message|_{10,}|From: .+@|في .{6,200}كتب)",
        )
        .ok()
    });
    static SPACES: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new("[ \t\u{a0}]+").ok());
    static BLANKS: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"\n\s*\n(?:\s*\n)+").ok());
    let unified = text.replace("\r\n", "\n");
    let mut lines = Vec::new();
    for raw in unified.split('\n') {
        let line = raw.trim();
        if QUOTED.as_ref().is_some_and(|quoted| quoted.is_match(line)) {
            break;
        }
        if line.starts_with('>') {
            continue;
        }
        lines.push(line);
    }
    let mut body = lines.join("\n");
    if let Some(spaces) = SPACES.as_ref() {
        body = spaces.replace_all(&body, " ").into_owned();
    }
    if let Some(blanks) = BLANKS.as_ref() {
        body = blanks.replace_all(&body, "\n\n").into_owned();
    }
    body.trim().to_owned()
}

/// The labels that keep a message from waking the assistant (kept for search only).
pub const QUIET_LABELS: [&str; 3] = ["CATEGORY_PROMOTIONS", "CATEGORY_SOCIAL", "CATEGORY_FORUMS"];

/// Whether a message with these labels is worth waking for: in the inbox or sent, and not in
/// a quiet category.
#[must_use]
pub fn is_loud(labels: &[String]) -> bool {
    labels
        .iter()
        .any(|label| label == "INBOX" || label == "SENT")
        && !labels
            .iter()
            .any(|label| QUIET_LABELS.contains(&label.as_str()))
}

/// Whether a message with these labels is one the server never keeps: a draft, spam or trash.
#[must_use]
pub fn is_skipped(labels: &[String]) -> bool {
    labels
        .iter()
        .any(|label| matches!(label.as_str(), "DRAFT" | "SPAM" | "TRASH"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_turns_into_lines_of_text() {
        let html = "<html><head><title>x</title></head><style>p{}</style><p>Hi&nbsp;Ann</p>\
                    <div>Line<br>two &amp; more</div>";
        let text = html_text(html);
        assert!(!text.contains("p{}"), "{text}");
        assert!(!text.contains('x'), "{text}");
        assert!(text.contains("Hi\u{a0}Ann\n"), "{text:?}");
        assert!(text.contains("Line\ntwo & more"), "{text:?}");
    }

    #[test]
    fn loud_and_skipped_labels() {
        let labels = |all: &[&str]| all.iter().map(|&l| l.to_owned()).collect::<Vec<_>>();
        assert!(is_loud(&labels(&["INBOX"])));
        assert!(!is_loud(&labels(&["INBOX", "CATEGORY_SOCIAL"])));
        assert!(!is_loud(&labels(&["IMPORTANT"])));
        assert!(is_skipped(&labels(&["DRAFT"])));
        assert!(!is_skipped(&labels(&["SENT"])));
    }

    #[test]
    fn account_names() {
        assert!(is_account_name("home"));
        assert!(!is_account_name("h"));
        assert!(!is_account_name("Home"));
        assert_eq!(
            chat_id("home", "ANN@example.com"),
            "email:home:ann@example.com"
        );
    }
}
