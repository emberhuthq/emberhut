//! The owner's clear go in the chat ("send it", "do it", "go ahead"), a clear yes to a question
//! about a task, and a stop: the words alone. In doubt it's no: a missed go costs one
//! follow-up, a false one sends something as the owner.
//!
//! The words are Arabic as well as English. A go may be followed by a name for the assistant:
//! its wire word ([`crate::task::wire::ASSISTANT`]) or the product's own.

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization as _;
use unicode_normalization::char::is_combining_mark;

use crate::task::wire;

/// For names and words: no case, accents or apostrophes (straight, curly, the modifier letter
/// and the prime as phones and keyboards type them), the Arabic alef forms as one.
#[must_use]
pub fn loose(text: &str) -> String {
    let lower: String = text
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase();
    lower
        .chars()
        .filter_map(|c| match c {
            'أ' | 'إ' | 'آ' => Some('ا'),
            '\'' | '\u{2018}' | '\u{2019}' | '\u{02bc}' | '\u{2032}' | '\u{ff07}' | '`' => None,
            other => Some(other),
        })
        .collect()
}

/// [`loose`], with everything but letters, digits and apostrophes as one space, trimmed.
#[must_use]
pub fn words(said: &str) -> String {
    let s = loose(said);
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_alphanumeric() || c == '\'' {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        } else {
            space = true;
        }
    }
    out
}

/// The letters and digits of `text` as words, the rest left out.
#[must_use]
pub fn loose_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

const CORE: &[&str] = &[
    "send(?: it| that| this| them| the message| the msg)?",
    "do it",
    "go ahead",
    "go for it",
    "ship it",
    "ابعت(?:ها|ه|هم|و|لو|له|يها)?",
    "ابعث(?:ها|ه)?",
    "ارسل(?:ها|ه|هم)?",
    "[ie]b3at(?:ha|o|lo)?",
];
const TAIL: &[&str] = &[
    "please",
    "pls",
    "plz",
    "now",
    "thanks",
    "thank you",
    "thx",
    "ty",
    "emberhut",
    "لو سمحت",
    "هلا",
    "هسا",
    "هسه",
    "الحين",
];
const YES: &[&str] = &[
    "yes",
    "yeah",
    "yep",
    "yup",
    "ok",
    "okay",
    "sure",
    "approved",
    "approve",
    "i approve",
    "confirm",
    "confirmed",
    "proceed",
    "go",
    "yalla",
    "yallah",
    "تمام",
    "اه",
    "ايوه",
    "ايوا",
    "اوكي",
    "نعم",
    "يلا",
    "اكيد",
    "ماشي",
    "موافق",
];
const LOOKS: &[&str] = &[
    "looks? (?:good|great|fine|perfect)",
    "sounds? (?:good|great|fine|perfect)",
    "good to go",
    "lgtm",
];
/// These only colour a yes: alone, none of them is one.
const SOFT: &[&str] = &[
    "perfect",
    "great",
    "good",
    "fine",
    "cool",
    "awesome",
    "excellent",
    "alright",
    "please",
    "pls",
    "plz",
    "emberhut",
    "طيب",
    "حلو",
    "ممتاز",
    "خلص",
];

fn join(lists: &[&[&str]], extra: &[String]) -> String {
    let mut all: Vec<String> = lists
        .iter()
        .flat_map(|list| list.iter().map(|s| (*s).to_owned()))
        .collect();
    all.extend(extra.iter().cloned());
    all.join("|")
}

/// The assistant's name as a go's words read it.
fn names() -> Vec<String> {
    vec![words(wire::ASSISTANT)]
}

/// A task's key as a go's words read it: `ot 12`, `on ot 12`.
fn task_ref() -> String {
    let prefix = words(wire::KEY_PREFIX);
    format!(r"(?:(?:on|for) )?{prefix} \d+")
}

fn lead() -> String {
    format!(
        r"(?:(?:{}|go ahead|just|then|and|{})\s+)*(?:lets\s+)?",
        task_ref(),
        join(&[YES, LOOKS, SOFT], &names())
    )
}

static GO: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^{}(?:{})(?:\s+(?:{}|{}))*$",
        lead(),
        CORE.join("|"),
        task_ref(),
        join(&[TAIL], &names())
    ))
    .ok()
});

static ON_CHANNEL: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:but\s+)?(?:on|via|over|through)\s+(?:slack|whatsapp|email|gmail|telegram|سلاك|واتساب)\s+(?:{})(?:\s+(?:{}))*$",
        CORE.join("|"),
        join(&[TAIL], &names())
    ))
    .ok()
});

static CLAUSE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"[,،;؛.!:\n]+").ok());

static NO: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)(?:no|not|dont|don't|do not|never|wait|hold|stop|cancel|change|edit|fix|instead|rewrite|rephrase|translate|shorter|longer|remove|delete|add|without|except|later|tomorrow|after|before|until|once|if|maybe|unless|لا|مش|ما|بلاش|استنى|غير|عدل|بعدين|لما|اذا|ليش|شو|وين|متى|كيف)(?:\s|$)").ok()
});

static ASKING: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^(?:did|didnt|didn't|have|has|is|are|can|could|should|would|will|does|do you|why|how|what|where|when|who|هل)(?:\s|$)").ok()
});

const OBJECT: &str = "(?:it|this|that|them|these|those)";
const ALL: &str = r"(?:all|everything|both|كلهم|الاثنين|التنين)(?: \d+| two| three| four| five| six)?(?: of (?:them|these|those))?";

static YES_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    let action = [
        format!(
            "(?:do|post|publish|submit|merge|deploy|push|ship|install|reinstall|create|run|start|restart|launch|schedule|approve) (?:{OBJECT}|{ALL})"
        ),
        "set it up".to_owned(),
        "switch it on".to_owned(),
        "post".to_owned(),
        "publish".to_owned(),
        "submit".to_owned(),
        "merge".to_owned(),
        "deploy".to_owned(),
        "continue".to_owned(),
        "send (?:him|her)".to_owned(),
        "انشر(?:ها|ه|هم)?".to_owned(),
    ];
    let head = join(&[CORE], &action);
    Regex::new(&format!(
        r"^{}(?:{}|{})(?:\s+(?:{}|(?:(?:to|for) )?{ALL}|{}))*$",
        lead(),
        head,
        join(&[YES, LOOKS], &[]),
        task_ref(),
        join(&[YES, SOFT, TAIL], &names())
    ))
    .ok()
});

static ACK_THANKS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^(?:ok|okay|sure|alright|تمام|ماشي|اوكي|sounds? (?:good|great|fine|perfect))\s+(?:thanks|thank you|thx|ty)$").ok()
});

static STOP: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)(?:stop|wait|hold|hang on|pause|abort|scrap|cancel|dont|do not|not yet|not now|never mind|nevermind|no|nope|nah|لا|لاء|استنى|استنا|استني|انتظر|بلاش|وقف(?:ها|ه|هم)?|اوقف(?:ها|ه|هم)?|توقف|الغي(?:ها|ه|هم)?|الغاء|ما ?تنشر(?:ها|ه|هم)?|(?:مش|مو) (?:هلق|هلا|هسا|هسه|الحين|الان)|ليس الان)(?:\s|$)").ok()
});

fn is(re: &LazyLock<Option<Regex>>, text: &str) -> bool {
    re.as_ref().is_some_and(|re| re.is_match(text))
}

fn asks(said: &str) -> bool {
    said.contains('?') || said.contains('؟')
}

/// How many words `s` has (`s` as [`words`] gives it).
fn count(s: &str) -> usize {
    s.split(' ').count()
}

/// A clear go: the whole reply is one, with only yes-words, please and thanks around it, and
/// it asks or changes nothing. Ending on a go isn't enough ("he told me to send it").
#[must_use]
pub fn is_go(said: &str) -> bool {
    if asks(said) {
        return false;
    }
    let s = words(said);
    if s.is_empty() || count(&s) > 16 || is(&NO, &s) || is(&ASKING, &s) {
        return false;
    }
    if is(&GO, &s) {
        return true;
    }
    // A go for one channel as its own last clause ("..., but on slack do it").
    let last = CLAUSE.as_ref().map_or_else(String::new, |re| {
        re.split(said)
            .filter(|clause| !words(clause).is_empty())
            .last()
            .unwrap_or("")
            .to_owned()
    });
    is(&ON_CHANNEL, &words(&last))
}

/// A clear yes to a question about a task: a go, or a reply that is nothing but a yes (yes,
/// ok, approved, looks good, "ok post it", "yes to all"). A sentence with a yes-word in it
/// isn't one, and a thank-you for news isn't an answer. Never an OK to send a message: that
/// is [`is_go`].
#[must_use]
pub fn is_yes(said: &str) -> bool {
    if is_go(said) {
        return true;
    }
    if asks(said) {
        return false;
    }
    let s = words(said);
    !s.is_empty()
        && count(&s) <= 16
        && is(&YES_RE, &s)
        && !is(&ACK_THANKS, &s)
        && !is(&NO, &s)
        && !is(&ASKING, &s)
}

/// The owner took it back: "stop", "wait", "don't", "not yet". Wide on purpose: a wrong hit
/// costs one more ask, a miss can send something they stopped.
#[must_use]
pub fn is_stop(said: &str) -> bool {
    is(&STOP, &words(said))
}

/// A stop and little else ("no", "wait, hold it"): a longer message with a stop word in it
/// may be about something else.
#[must_use]
pub fn is_plain_stop(said: &str) -> bool {
    is_stop(said) && count(&words(said)) <= 8
}

/// Every pattern here compiles.
#[must_use]
pub fn patterns_compile() -> bool {
    [
        &GO,
        &ON_CHANNEL,
        &CLAUSE,
        &NO,
        &ASKING,
        &YES_RE,
        &ACK_THANKS,
        &STOP,
    ]
    .iter()
    .all(|re| re.is_some())
}
