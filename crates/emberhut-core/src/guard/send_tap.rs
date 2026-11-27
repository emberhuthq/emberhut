//! The owner's Send it on a message's card sends it there and then: which answers are a
//! send-type answer ("Send it", "Send", "Send to Sam", "Yes, send it", "Post it", "ابعت"),
//! never "The assistant sends it", "Don't send", "Send it later", "Edit and send" or "Send it
//! to someone instead".

use std::sync::LazyLock;

use regex::Regex;

use super::go;
use super::grants::ASSISTANT_SENDS;

static SEND_WORD: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)(?:send|post|ابعت\S*|ابعث\S*|ارسل\S*|[ie]b3at\S*)(?:\s|$)").ok()
});
static POST: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^(?:yes )?post(?: it)?(?: now)?$").ok());
static SEND_TO: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^(?:yes )?(?:send|post)(?: it| this| that| the (?:message|reply|note|msg))?(?: now)? to ([\p{L}\p{N}']+(?: [\p{L}\p{N}']+)?)$").ok()
});
static NOT_NOW: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)(?:instead|later|tomorrow|after|first|too|also|then|me)(?:\s|$)").ok()
});

fn is(re: &LazyLock<Option<Regex>>, text: &str) -> bool {
    re.as_ref().is_some_and(|re| re.is_match(text))
}

/// Whether the answer `option` sends the card's message.
#[must_use]
pub fn sends_it(option: &str) -> bool {
    let s = go::words(option);
    if s.is_empty() || s == go::loose(ASSISTANT_SENDS) || s == go::words(ASSISTANT_SENDS) {
        return false;
    }
    if !is(&SEND_WORD, &s) {
        return false;
    }
    if go::is_go(option) || is(&POST, &s) {
        return true;
    }
    SEND_TO
        .as_ref()
        .and_then(|re| re.captures(&s))
        .is_some_and(|caps| !is(&NOT_NOW, &caps[1]))
}

/// Every pattern here compiles.
#[must_use]
pub fn patterns_compile() -> bool {
    SEND_WORD.is_some() && POST.is_some() && SEND_TO.is_some() && NOT_NOW.is_some()
}
