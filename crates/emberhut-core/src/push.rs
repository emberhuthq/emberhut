//! Push notifications: the addresses phones register, and what an alert says.
//!
//! An iPhone registers its APNs device token (hex) and whether it is a development build
//! (`sandbox`) or a release (`production`); an Android phone registers its FCM registration
//! token. Each of the assistant's replies then reaches every registered phone as an alert,
//! unless the room is on screen already.

use std::fmt;

use crate::RoomId;

/// The longest alert body, in UTF-16 units.
pub const BODY_LIMIT: usize = 240;

/// The scheme of the links an alert opens: the app's own.
const LINK_SCHEME: &str = "emberhut://";

/// An address a phone gave that is not one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PushTokenError {
    /// Not an APNs device token: 64 to 200 hex digits.
    #[error("not a device token")]
    NotApns,
    /// Not an FCM registration token: 32 to 4096 letters, digits, `_`, `:` or `-`.
    #[error("not an FCM registration token")]
    NotFcm,
}

/// An iPhone's APNs device token: 64 to 200 hex digits, in either case.
#[derive(Clone, PartialEq, Eq)]
pub struct ApnsToken(String);

impl ApnsToken {
    /// The token the phone sent, if it is one.
    ///
    /// ```
    /// use emberhut_core::push::ApnsToken;
    ///
    /// assert!(ApnsToken::parse(&"ab".repeat(32)).is_ok());
    /// assert!(ApnsToken::parse("not hex").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`PushTokenError::NotApns`].
    pub fn parse(token: &str) -> Result<Self, PushTokenError> {
        let fits =
            (64..=200).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit());
        fits.then(|| Self(token.to_owned()))
            .ok_or(PushTokenError::NotApns)
    }

    /// The token as stored and sent to Apple.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApnsToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A push address is not a secret, but it is no use in a log line either.
        f.write_str("ApnsToken(..)")
    }
}

/// An Android phone's FCM registration token: 32 to 4096 ASCII letters, digits, `_`, `:` or
/// `-`.
#[derive(Clone, PartialEq, Eq)]
pub struct FcmToken(String);

impl FcmToken {
    /// The token the phone sent, if it is one.
    ///
    /// ```
    /// use emberhut_core::push::FcmToken;
    ///
    /// assert!(FcmToken::parse(&format!("app:{}", "x".repeat(40))).is_ok());
    /// assert!(FcmToken::parse("short").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`PushTokenError::NotFcm`].
    pub fn parse(token: &str) -> Result<Self, PushTokenError> {
        let fits = (32..=4096).contains(&token.len())
            && token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'-'));
        fits.then(|| Self(token.to_owned()))
            .ok_or(PushTokenError::NotFcm)
    }

    /// The token as stored and sent to Google.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for FcmToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FcmToken(..)")
    }
}

/// Which of Apple's push services an iPhone's token belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ApnsEnv {
    /// A development build: Apple's sandbox service. Anything a phone sends that is not
    /// `production` means this one.
    #[default]
    Sandbox,
    /// A release build: Apple's production service.
    Production,
}

impl ApnsEnv {
    /// The environment a phone named: `production`, or the sandbox for anything else (or
    /// nothing).
    #[must_use]
    pub fn from_wire(env: Option<&str>) -> Self {
        if env == Some("production") {
            Self::Production
        } else {
            Self::Sandbox
        }
    }

    /// Its name, as stored.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Production => "production",
        }
    }
}

/// A registered iPhone: its token and its service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApnsDevice {
    /// The device token.
    pub token: String,
    /// Sandbox or production.
    pub env: ApnsEnv,
}

/// One alert, as every phone shows it: a title, a line of text, and the room a tap opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    title: String,
    body: String,
    room: Option<RoomId>,
}

impl Alert {
    /// An alert with this title and body (cut to [`BODY_LIMIT`]), opening `room` when tapped
    /// (or the chat as it was, without one).
    #[must_use]
    pub fn new(title: impl Into<String>, body: &str, room: Option<RoomId>) -> Self {
        Self {
            title: title.into(),
            body: cut_utf16(body, BODY_LIMIT).to_owned(),
            room,
        }
    }

    /// The alert for the assistant's reply in `room` (named `room_name`, unless it is Main):
    /// the reply as plain text, titled with the room.
    ///
    /// ```
    /// use emberhut_core::RoomId;
    /// use emberhut_core::push::Alert;
    ///
    /// let alert = Alert::reply(RoomId::new(2), Some("Garden"), "**Done**: see [the plan](x)");
    /// assert_eq!(alert.title(), "Emberhut · Garden");
    /// assert_eq!(alert.body(), "Done: see the plan");
    /// assert_eq!(alert.link(), "emberhut://chat?room=2");
    /// ```
    #[must_use]
    pub fn reply(room: RoomId, room_name: Option<&str>, text: &str) -> Self {
        let title = match room_name {
            Some(name) if room != RoomId::MAIN => format!("Emberhut · {name}"),
            _ => "Emberhut".to_owned(),
        };
        Self::new(title, &plain(text), Some(room))
    }

    /// The title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The text, at most [`BODY_LIMIT`] UTF-16 units.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The room a tap opens.
    #[must_use]
    pub fn room(&self) -> Option<RoomId> {
        self.room
    }

    /// What a tap opens: `emberhut://chat?room=<id>`, or `emberhut://chat`.
    #[must_use]
    pub fn link(&self) -> String {
        match self.room {
            Some(room) => format!("{LINK_SCHEME}chat?room={room}"),
            None => format!("{LINK_SCHEME}chat"),
        }
    }

    /// How a phone groups its alerts: one group per room.
    #[must_use]
    pub fn thread(&self) -> String {
        match self.room {
            Some(room) => format!("emberhut-{room}"),
            None => "emberhut".to_owned(),
        }
    }
}

/// `text` as one plain line, for an alert: a Markdown link becomes its text, the marks
/// (`*`, `_`, a backtick, `#` and `>`) go, and every run of whitespace is one space.
///
/// ```
/// use emberhut_core::push::plain;
///
/// assert_eq!(plain("# Hi\n\n*there*, [see](https://example.com)"), "Hi there, see");
/// ```
#[must_use]
pub fn plain(text: &str) -> String {
    let unlinked = unlink(text);
    let unmarked: String = unlinked
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '`' | '#' | '>'))
        .collect();
    unmarked.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every `[text](target)` in `text` as its text alone.
fn unlink(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let link = after
            .find(']')
            .filter(|close| *close > 0)
            .and_then(|close| {
                let target = after[close + 1..].strip_prefix('(')?;
                let end = target.find(')')?;
                let used = close + 1 + 1 + end + 1;
                Some((&after[..close], used))
            });
        if let Some((label, used)) = link {
            out.push_str(label);
            rest = &after[used..];
        } else {
            out.push('[');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// The longest start of `text` that is at most `limit` UTF-16 units, never splitting a
/// character.
fn cut_utf16(text: &str, limit: usize) -> &str {
    let mut units = 0;
    for (at, c) in text.char_indices() {
        units += c.len_utf16();
        if units > limit {
            return &text[..at];
        }
    }
    text
}

/// Whether Apple's answer says the token will never work again (the app is gone, or the token
/// isn't one of this topic's): 410, or `BadDeviceToken` or `Unregistered` in its body.
#[must_use]
pub fn apns_token_gone(status: u16, reply: &str) -> bool {
    status == 410 || reply.contains("BadDeviceToken") || reply.contains("Unregistered")
}

/// Whether Google's answer says the token will never work again (the app was uninstalled, or
/// the token isn't one of this project's).
#[must_use]
pub fn fcm_token_gone(reply: &str) -> bool {
    [
        "UNREGISTERED",
        "SENDER_ID_MISMATCH",
        "not a valid FCM registration token",
    ]
    .iter()
    .any(|sign| reply.contains(sign))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apns_tokens_are_64_to_200_hex_digits() {
        assert!(ApnsToken::parse(&"A".repeat(64)).is_ok());
        assert!(ApnsToken::parse(&"0".repeat(200)).is_ok());
        assert_eq!(
            ApnsToken::parse(&"0".repeat(63)),
            Err(PushTokenError::NotApns)
        );
        assert!(ApnsToken::parse(&"0".repeat(201)).is_err());
        assert!(ApnsToken::parse(&format!("{}g", "0".repeat(63))).is_err());
        assert!(ApnsToken::parse("").is_err());
    }

    #[test]
    fn fcm_tokens_are_32_to_4096_word_characters() {
        assert!(FcmToken::parse(&"a".repeat(32)).is_ok());
        assert!(FcmToken::parse(&format!("dX-1:APA91b_{}", "Q".repeat(140))).is_ok());
        assert_eq!(
            FcmToken::parse(&"a".repeat(31)),
            Err(PushTokenError::NotFcm)
        );
        assert!(FcmToken::parse(&"a".repeat(4097)).is_err());
        assert!(FcmToken::parse(&format!("{} ", "a".repeat(40))).is_err());
        assert!(FcmToken::parse(&format!("{}é", "a".repeat(40))).is_err());
    }

    #[test]
    fn the_errors_name_the_token_kind() {
        assert_eq!(PushTokenError::NotApns.to_string(), "not a device token");
        assert_eq!(
            PushTokenError::NotFcm.to_string(),
            "not an FCM registration token"
        );
    }

    #[test]
    fn only_production_is_production() {
        assert_eq!(ApnsEnv::from_wire(Some("production")), ApnsEnv::Production);
        for env in [Some("sandbox"), Some("Production"), Some(""), None] {
            assert_eq!(ApnsEnv::from_wire(env), ApnsEnv::Sandbox, "{env:?}");
        }
    }

    #[test]
    fn a_reply_alert_is_titled_with_its_room() {
        let main = Alert::reply(RoomId::MAIN, Some("Main"), "hi");
        assert_eq!(main.title(), "Emberhut");
        assert_eq!(main.link(), "emberhut://chat?room=1");
        assert_eq!(main.thread(), "emberhut-1");
        let other = Alert::reply(RoomId::new(3), None, "hi");
        assert_eq!(other.title(), "Emberhut");
        let plain = Alert::new("Title", "x", None);
        assert_eq!(plain.link(), "emberhut://chat");
        assert_eq!(plain.thread(), "emberhut");
    }

    #[test]
    fn the_body_is_cut_to_240_utf16_units_on_a_character() {
        let long = "a".repeat(300);
        assert_eq!(Alert::new("t", &long, None).body().len(), 240);
        // 120 characters of two units each fill it exactly; one more does not fit.
        let wide = "😀".repeat(121);
        let body = Alert::new("t", &wide, None).body().to_owned();
        assert_eq!(body.chars().count(), 120);
        let odd = format!("a{}", "😀".repeat(120));
        assert_eq!(Alert::new("t", &odd, None).body().chars().count(), 120);
    }

    #[test]
    fn plain_drops_markdown() {
        assert_eq!(plain("[a](b) and [c](d)"), "a and c");
        assert_eq!(
            plain("[] (x) [no link] [x](unclosed"),
            "[] (x) [no link] [x](unclosed"
        );
        assert_eq!(plain("[[nested](x)"), "[nested");
        assert_eq!(plain("> quote\n- `code` _it_"), "quote - code it");
        assert_eq!(plain("  \n "), "");
    }

    #[test]
    fn gone_tokens_are_recognised() {
        assert!(apns_token_gone(410, ""));
        assert!(apns_token_gone(400, r#"{"reason":"BadDeviceToken"}"#));
        assert!(!apns_token_gone(500, r#"{"reason":"InternalServerError"}"#));
        assert!(fcm_token_gone(r#"{"error":{"status":"UNREGISTERED"}}"#));
        assert!(fcm_token_gone("SENDER_ID_MISMATCH"));
        assert!(!fcm_token_gone(r#"{"error":{"status":"UNAVAILABLE"}}"#));
    }

    #[test]
    fn debug_hides_the_tokens() {
        let apns = ApnsToken::parse(&"ab".repeat(32)).unwrap();
        assert_eq!(format!("{apns:?}"), "ApnsToken(..)");
        let fcm = FcmToken::parse(&"x".repeat(40)).unwrap();
        assert_eq!(format!("{fcm:?}"), "FcmToken(..)");
    }
}
