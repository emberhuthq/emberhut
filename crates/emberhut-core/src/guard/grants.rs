//! The owner's OK, in one model: one job, one OK at most.
//!
//! - **The job's grant.** The owner's request for a task, or one approval of theirs on it (a Go
//!   ahead, Allow, Send it or another yes on any of its cards; their yes in the chat or on the
//!   task) covers every ordinary step of that task and its subtasks while it is open. Their Stop
//!   on one of its cards takes it back until their next OK.
//! - **A message as the owner, or a public post**, needs its one OK per text, and that OK is
//!   enough: their Send it, or their clear go in the chat on the exact text, approves that text
//!   on that task for any channel, once.
//! - **Hard steps** ask every time, and only the owner's tap on that card clears them: payments
//!   and purchases, passwords and codes, account security.
//!
//! These are the rules on words; where the grants are kept is the server's.

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization as _;

use super::go;

static HARD: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(buy|purchase|pay|payment|order|checkout|check out|subscribe|subscription|upgrade|donate|transfer|billing|banking|password|passcode|passkey|code|codes|otp|one-time|two-factor|2fa|mfa|security|recovery|authenticator|revoke|(?:delete|close|deactivate|remove) (?:my |your |this |the )?account)\b").ok()
});
static HARD_PAGE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(checkout|payment|billing|purchase|password|security|two-factor|2fa|recovery|delete account|close account)\b").ok()
});
/// Slack's Events API "subscribes" an app to events: setup, not a purchase.
static EVENT_SUBSCRIBE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:subscribe to (?:bot |app |workspace |user )?events(?: on behalf of users)?|(?:bot |app |user )?event subscriptions?)\b").ok()
});
static SLACK_APP_PAGE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)\bhttps?://api\.slack\.com/apps/A[A-Z0-9]+(?:[/?#]|$)").ok());
static SUBSCRIBE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:subscribe|subscription)s?\b").ok());
static SAID: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(send|reply|respond|post|tweet|retweet|repost|publish|message|dm|comment|email|invite|share|go live)\b").ok()
});
static PERSONAL_PAGE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(mail\.google\.com|outlook\.(?:live|office)\.com|web\.whatsapp\.com|messenger\.com|linkedin\.com/messaging|(?:x|twitter)\.com/(?:compose|messages))").ok()
});
static APP_CONSOLE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(play\.google\.com/console|appstoreconnect\.apple\.com|developer\.apple\.com/account|chromewebstore\.google\.com|chrome\.google\.com/webstore/devconsole|console\.firebase\.google\.com|expo\.dev|partner\.microsoft\.com)").ok()
});
static OPENER: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:reply|reply all|reply to all|forward|compose|new message|new email|new mail|write|write a reply|start a post|write a post|create a post|new post)$").ok()
});
static TO_PEOPLE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(send|reply|respond|message|dm|email|invite|share)\b").ok()
});
static APPROVE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:go ahead|go|allow|allow it|yes|ok|okay|approve|approved|send it|send|post it|post|publish|(?:the )?assistant sends it|do it|push|deploy|merge|install|create|switch it on|save it)\b").ok()
});
static SENDY: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|\s)(?:send|post|publish|ابعت\S*|ارسل\S*)(?:\s|$|,)").ok()
});
static BRAKES: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:stop|leave it|don't allow|no)$").ok());
static POSTY: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:post|publish|schedule)\b").ok());
static SEND_CARD: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^Send this (?:Slack|WhatsApp) message").ok());

fn is(re: &LazyLock<Option<Regex>>, text: &str) -> bool {
    re.as_ref().is_some_and(|re| re.is_match(text))
}

/// The answer a draft's card offers when the draft can't be sent from the card itself (no
/// chat it can post to): the owner's tap hands the draft to the assistant to send.
pub const ASSISTANT_SENDS: &str = "The assistant sends it";

/// What a step is, by its label and the page it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    /// Money or account security: its own card every time.
    Hard,
    /// Said as the owner, or public: its text needs the owner's OK.
    Said,
    /// The rest: the job's grant covers it.
    Ordinary,
}

/// Whether a step is about money or account security, by its label (`what`) or its page.
#[must_use]
pub fn hard(what: &str, page: &str) -> bool {
    let mut w = EVENT_SUBSCRIBE.as_ref().map_or_else(
        || what.to_owned(),
        |re| re.replace_all(what, " ").into_owned(),
    );
    if is(&SLACK_APP_PAGE, page)
        && let Some(re) = SUBSCRIBE.as_ref()
    {
        w = re.replace_all(&w, " ").into_owned();
    }
    is(&HARD, &w) || is(&HARD_PAGE, page)
}

/// What a step is: hard, said as the owner (unless it's their app's release on its console,
/// or any step on a mail or chat page), or ordinary.
#[must_use]
pub fn kind(what: &str, page: &str) -> StepKind {
    if hard(what, page) {
        return StepKind::Hard;
    }
    if is(&SAID, what) && !is(&APP_CONSOLE, page) {
        return StepKind::Said;
    }
    if is(&PERSONAL_PAGE, page) {
        StepKind::Said
    } else {
        StepKind::Ordinary
    }
}

/// A button that only opens a box to write in: it sends nothing while nothing is written.
#[must_use]
pub fn opener(what: &str) -> bool {
    is(&OPENER, crate::messaging::text::js_trim(what))
}

/// A step the job's grant covers by itself.
#[must_use]
pub fn on_request(what: &str, page: &str) -> bool {
    kind(what, page) == StepKind::Ordinary
}

/// Whether `page` is a mail or chat page (every step there speaks as the owner).
#[must_use]
pub fn personal_page(page: &str) -> bool {
    is(&PERSONAL_PAGE, page)
}

/// Whether a said step is a message to someone (not a post or a comment).
#[must_use]
pub fn to_people(what: &str) -> bool {
    is(&TO_PEOPLE, what)
}

/// The key a text's OK is kept under: its spacing and curly quotes don't count, every word
/// and mark does.
#[must_use]
pub fn text_key(text: &str) -> String {
    text.nfc()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            other => other,
        })
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// What the owner's tap on a card of a task means for the grants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tap {
    /// Nothing new from the owner (no card open, or an answer that is neither a yes nor a no).
    Nothing,
    /// Their brakes on the job: the grant is off until their next OK.
    StopJob,
    /// A no to that one step or text only: the job keeps its grant.
    NoToStep,
    /// A yes: `text` approved for any channel (with where it was for), the job's post
    /// approved, and the job granted unless it was a hard step's card.
    Approve {
        /// The draft the yes approves, when it does.
        text: Option<String>,
        /// A post with no text of its own (a Publish) is approved for the job.
        post: bool,
        /// The job is granted (not for a hard step's card).
        job: bool,
    },
}

/// The owner tapped `choice` on the card asking `question`, which carries `draft` (a
/// message's text) when it has one; `card_kind` is what that card was about when it was a
/// step's card (a hard one's Go ahead is that step's only; a message's Stop is a no to that
/// text, not to the job).
#[must_use]
pub fn heard_tap(
    choice: &str,
    question: Option<&str>,
    draft: Option<&str>,
    card_kind: Option<StepKind>,
) -> Tap {
    let Some(question) = question.filter(|q| !q.is_empty()) else {
        return Tap::Nothing;
    };
    let choice = crate::messaging::text::js_trim(choice);
    let draft = draft.filter(|d| !d.is_empty());
    if is(&BRAKES, choice) {
        return if card_kind.is_none() && draft.is_none() && !is(&SEND_CARD, question) {
            Tap::StopJob
        } else {
            Tap::NoToStep
        };
    }
    if !(is(&APPROVE, choice) || go::is_yes(choice)) {
        return Tap::Nothing;
    }
    let lower = choice.to_lowercase();
    let text = draft
        .filter(|_| {
            is(&SENDY, choice)
                || lower.contains(&ASSISTANT_SENDS.to_lowercase())
                || go::is_yes(choice)
                || lower.starts_with("approve")
        })
        .map(str::to_owned);
    let post = draft.is_none() && is(&POSTY, choice);
    Tap::Approve {
        text,
        post,
        job: card_kind != Some(StepKind::Hard),
    }
}

/// The owner's Mac's tools that never ask: plain dev and system tools.
#[must_use]
pub fn pre_allowed(app: &str) -> bool {
    static PRE_ALLOWED: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r"(?i)^(?:Finder|Xcode(?:[- ]?[\d.]+)?(?:[- ]beta)?|Simulator|Device Hub|Devices|Instruments|Accessibility Inspector|Console|Preview|FileMerge|Terminal)$").ok()
    });
    is(&PRE_ALLOWED, app)
}

/// Minutes the owner's Mac has been idle after which the whole screen starts without asking.
pub const IDLE_MINUTES: u64 = 5;

/// Every pattern here compiles.
#[must_use]
pub fn patterns_compile() -> bool {
    [
        &HARD,
        &HARD_PAGE,
        &EVENT_SUBSCRIBE,
        &SLACK_APP_PAGE,
        &SUBSCRIBE,
        &SAID,
        &PERSONAL_PAGE,
        &APP_CONSOLE,
        &OPENER,
        &TO_PEOPLE,
        &APPROVE,
        &SENDY,
        &BRAKES,
        &POSTY,
        &SEND_CARD,
    ]
    .iter()
    .all(|re| re.is_some())
}
