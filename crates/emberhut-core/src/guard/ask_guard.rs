//! The ask guard: the owner's words are the OK. Before a task's ask (the assistant's or a
//! worker's) or a "want me to…?" at the end of the assistant's reply reaches the owner, the
//! server reads the owner's request for the task, their messages since, their taps and lines on
//! the task, and their standing OKs, and asks a judge model whether they already cover the
//! step. Covered: the ask goes back as "covered, do it", with the owner's own words quoted
//! (checked against what they really said: a quote that isn't theirs doesn't count).
//!
//! Only these always reach the owner: payments, passwords, codes, account security, messages
//! or posts as the owner, merging to main (unless their words name it: the outward rule) and
//! deleting their data. When the judge can't be reached, the ask reaches the owner.
//!
//! These are the rules without I/O: what always asks, what reads as asking, the judge's
//! instructions, and how its answer is checked.

use std::sync::LazyLock;

use regex::Regex;

use super::{hard_words, outward};

static MERGE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(merg(e|es|ed|ing)|push(es|ed|ing)?\s+(it\s+|this\s+|them\s+)?(to|into|on)\s+(main|master))\b").ok()
});
static DELETE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(delet(e|es|ed|ing)|wip(e|es|ed|ing)|eras(e|es|ed|ing)|purg(e|es|ed|ing)|rm\s+-rf|drop\s+(the\s+)?(table|database|db)|(remov(e|ing)|clear(ing)?|drop(ping)?)\s+(\w+\s+){0,3}(data|files?|folders?|backups?|database|db|emails?|messages?|photos?|videos?|recordings?|history|records?|repo|branch(es)?|account))\b").ok()
});
static ASKING: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"(?i)\b(want me to|do you want me to|would you like me to|shall i|should i (go ahead|proceed|start|run|put|ship|deploy|merge|push|do|make|set|create|book|switch)|ok(ay)? (for me )?to (go ahead|proceed|ship|deploy|start|run|put)|(only|just) needs? your (yes|ok|okay|go|go-ahead|nod)|(just )?say ["“'‘][^"”'’\n]{1,40}["”'’]|(give me|i need) (the|a|your) (go|go-ahead|ok|green light)|go ahead\?|proceed\?)"#,
        r"|(تبيني|تبغاني|تبغى ا|تبي ا|بدك\s*(أ|ا|ن)|تريدني|أكمل\s*؟|اكمل\s*؟|أبدأ\s*؟|ابدأ\s*؟|أنفذ\S*\s*؟|انفذ\S*\s*؟|أمشي\s*؟|نمشي\s*؟|أرفع\S*\s*؟|موافق\s*؟)"
    ))
    .ok()
});

fn is(re: &LazyLock<Option<Regex>>, text: &str) -> bool {
    re.as_ref().is_some_and(|re| re.is_match(text))
}

/// The step to check: what the card or the reply asks the owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ask {
    /// The action, question, context and brief; or the reply's text.
    pub text: String,
    /// The answers offered.
    pub options: Vec<String>,
    /// A message to send as the owner, when the ask carries one.
    pub draft: Option<String>,
    /// Where that message goes.
    pub chat: Option<String>,
}

/// The kind of step that always asks the owner, told from its words alone, or none.
#[must_use]
pub fn reserved(ask: &Ask) -> Option<&'static str> {
    let given = |s: &Option<String>| s.as_deref().is_some_and(|s| !s.trim().is_empty());
    if given(&ask.draft) || given(&ask.chat) {
        return Some("a message or post as the owner");
    }
    if hard_words::is_hard(&ask.text) {
        return Some("payments, passwords, codes or account security");
    }
    if is(&DELETE, &ask.text) {
        return Some("deleting the owner's data");
    }
    None
}

/// Whether an ask is about merging to main.
#[must_use]
pub fn is_merge(text: &str) -> bool {
    is(&MERGE, text)
}

/// Whether a reply ends by asking the owner's OK ("Want me to put it live?", "Shall I…").
#[must_use]
pub fn asking(reply: &str) -> bool {
    let reply = reply.trim();
    let count = reply.chars().count();
    let tail: String = reply.chars().skip(count.saturating_sub(400)).collect();
    is(&ASKING, &tail)
}

/// One line of evidence the judge reads: its tag, its text, and the words a quote from it
/// must come from (the owner's own, or the standing OK's; never the assistant's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// Its tag, `[m12]`, `[rule3]`...
    pub tag: String,
    /// What the judge reads.
    pub text: String,
    /// The words a quote must be from.
    pub said: String,
}

/// The judge's instructions.
pub const RULES: &str = r#"You check one ask before it reaches the owner, from their personal assistant (or one of its workers). The owner's rule: their words are the OK. When their own words already approve this step or already settle this choice, the ask must not reach them: the assistant does it. Their words are: their request for the task, their messages since in any of their rooms, their taps and lines on the task's cards (a "Chose: X" line is their tap on X), and their standing OKs (rules they OK'd, their rooms' standing instructions, people whose asks count as theirs, the facts and values they set, such as a price or a budget).

These always reach them, whatever they said; name the kind in "reserved": paying (a payment, a purchase, a checkout, adding a card), a password, a code (2FA, OTP), account security, a message or post sent as them (Slack, WhatsApp, email, PR comment, social post), deleting their data. A spend they already set (a budget, a price) is not paying: their value answers a question about it. Merging to main is covered only when their words name merging, deploying or putting this work live (quote those words).

Covered when their words clearly approve this step or settle this choice:
- their request for the task ("do it end to end", "build it and deploy", "run the ads") covers its ordinary steps (building, configuring, creating records or apps, settings, no-spend setup steps, running, testing, deploying where the request says so);
- a value they set answers a question about that value;
- their yes to the assistant's proposal covers that proposal;
- their go to do the thing after the assistant advised an option takes the advised option, unless they picked another since. The assistant: "I advise $20 a day" … the owner: "OK, go make the ad": a later "Pick the daily budget: $10, $20 or $30" is covered, answer "$20 a day". A go doesn't need to repeat the number;
- a standing rule covers the cases it names.
Not covered: their words are about something else, an earlier word they took back later, a question of theirs rather than an OK, a real choice they haven't made (two designs, two names) and nothing they said picks one, or a step outside what they asked for. When unsure, it's not covered. The ask itself is the assistant's, never evidence.

Answer with one JSON object and nothing else:
{"reserved": null or "payment" | "password" | "code" | "account security" | "message or post as the owner" | "deleting their data",
 "covered": true or false,
 "answer": the option label their words pick (exactly as listed), or null,
 "quote": their words that cover it, copied character for character from ONE evidence line (a part of it, up to 200 characters), or null,
 "tag": that line's tag (the text in [ ] before it), or null,
 "why": one short sentence}"#;

/// The judge's prompt for `ask`, with its `head` (the task and its parents), the room it was
/// asked in when there is no task, the evidence, and the time now in words.
#[must_use]
pub fn prompt(ask: &Ask, head: &str, room: &str, lines: &[Line], now: &str) -> String {
    let head = if head.is_empty() {
        format!("In the {room} room.")
    } else {
        head.to_owned()
    };
    let options = if ask.options.is_empty() {
        String::new()
    } else {
        format!("Options: {}", ask.options.join(" | "))
    };
    let evidence: Vec<String> = lines
        .iter()
        .map(|l| format!("[{}] {}", l.tag, l.text))
        .collect();
    format!(
        "# The ask\n{head}\n{}\n{options}\n\n# Evidence (now: {now})\n{}",
        ask.text,
        evidence.join("\n")
    )
}

/// The judge's answer, as it came (read from its JSON by the caller).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JudgeAnswer {
    /// It says the owner's words cover the step.
    pub covered: bool,
    /// A kind that always reaches the owner, when it names one.
    pub reserved: Option<String>,
    /// The option the owner's words pick.
    pub answer: Option<String>,
    /// The owner's words it quotes.
    pub quote: Option<String>,
    /// The evidence line it quotes.
    pub tag: Option<String>,
    /// Why, a sentence.
    pub why: String,
}

/// The judge's answer, checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    /// The owner's words cover the step: it goes back as "do it".
    pub covered: bool,
    /// One of the kinds that always reach the owner.
    pub reserved: Option<String>,
    /// The option the owner's words pick, when it's one of the ask's.
    pub answer: Option<String>,
    /// The owner's words that cover it (checked).
    pub quote: Option<String>,
    /// Where they said it.
    pub place: Option<String>,
    /// Why.
    pub why: String,
}

/// The verdict for an ask that always asks.
#[must_use]
pub fn always(kind: &str) -> Verdict {
    Verdict {
        reserved: Some(kind.to_owned()),
        why: "always the owner's".to_owned(),
        ..Verdict::default()
    }
}

/// A text as quotes are compared: no quotes or emphasis marks, single spaces, lower case.
#[must_use]
pub fn norm(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '"' | '“' | '”' | '\'' | '‘' | '’' | '«' | '»' | '`' | '*' | '_'
            )
        })
        .collect();
    kept.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Where a quoted line was said, in words, from its tag and its text.
#[must_use]
pub fn where_of(tag: &str, text: &str) -> String {
    static SAID: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r"^The owner(?:'s request for ([A-Z]+-\d+))? \(([^,]+), ([^)]+)\)").ok()
    });
    static ON: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"^The owner on ([A-Z]+-\d+) \(([^)]+)\)").ok());
    if let Some(caps) = SAID.as_ref().and_then(|re| re.captures(text)) {
        return match caps.get(1) {
            Some(task) => format!(
                "their request for {} at {} in {}",
                task.as_str(),
                &caps[3],
                &caps[2]
            ),
            None => format!("their message at {} in {}", &caps[3], &caps[2]),
        };
    }
    if let Some(caps) = ON.as_ref().and_then(|re| re.captures(text)) {
        return format!("their answer on {} at {}", &caps[1], &caps[2]);
    }
    if let Some(rule) = tag.strip_prefix("rule") {
        return format!("their standing rule {rule}");
    }
    if tag.starts_with("room") {
        return "their room's standing instructions".to_owned();
    }
    if let Some(fact) = tag.strip_prefix("fact:") {
        return format!("what they set ({fact})");
    }
    if tag.starts_with("by") || tag == "requesters" {
        return "their standing OK for that person's asks".to_owned();
    }
    tag.to_owned()
}

/// The judge's `answer` on `ask`, checked against the `lines` it read: covered only when its
/// quote is really the owner's (word for word, in the line it names or any of theirs), and a
/// merge only when the quote names it.
#[must_use]
pub fn check(answer: JudgeAnswer, ask: &Ask, lines: &[Line]) -> Verdict {
    let mut verdict = Verdict {
        covered: answer.covered,
        reserved: answer.reserved.filter(|r| !r.is_empty() && r != "null"),
        answer: answer.answer.filter(|a| ask.options.contains(a)),
        why: answer.why,
        ..Verdict::default()
    };
    if !verdict.covered || verdict.reserved.is_some() {
        verdict.covered = false;
        return verdict;
    }
    let quote = answer.quote.unwrap_or_default().trim().to_owned();
    let wanted = norm(&quote);
    let tag = answer
        .tag
        .unwrap_or_default()
        .trim_matches(|c| matches!(c, '[' | ']' | ' '))
        .to_owned();
    let line = if wanted.chars().count() >= 2 {
        lines
            .iter()
            .find(|l| l.tag == tag && norm(&l.said).contains(&wanted))
            .or_else(|| lines.iter().find(|l| norm(&l.said).contains(&wanted)))
    } else {
        None
    };
    let Some(line) = line else {
        verdict.covered = false;
        verdict.why = format!(
            "its quote isn't the owner's words ({})",
            quote.chars().take(60).collect::<String>()
        );
        return verdict;
    };
    if is_merge(&ask.text) && outward::named(&quote).is_none() {
        verdict.covered = false;
        "merging to main needs the owner's words naming it".clone_into(&mut verdict.why);
        return verdict;
    }
    verdict.quote = Some(quote.chars().take(200).collect());
    verdict.place = Some(where_of(&line.tag, &line.text));
    verdict
}

/// The covering words and where they were said.
#[must_use]
pub fn covered_line(verdict: &Verdict) -> String {
    format!(
        "«{}» ({})",
        verdict.quote.as_deref().unwrap_or(""),
        verdict.place.as_deref().unwrap_or("")
    )
}

/// What a task's ask answers when the owner's words cover it.
#[must_use]
pub fn sent_back(verdict: &Verdict) -> String {
    let pick = verdict
        .answer
        .as_ref()
        .map_or(String::new(), |a| format!(" Their pick: \"{a}\"."));
    format!(
        "Not asked: covered, do it. The owner's own words already OK this: {}.{pick} Don't ask them; do the step now and carry on (task_note what you did). If their words truly don't cover it (the quote is about something else, or it's a payment, password, code, account security, a message or post as them, merging to main or deleting their data), call task_ask again with not_covered: why; then it reaches them.",
        covered_line(verdict)
    )
}

/// What the assistant's room hears when its reply was held: the reply went nowhere, do the
/// step.
#[must_use]
pub fn reply_back(verdict: &Verdict, reply: &str) -> String {
    let pick = verdict
        .answer
        .as_ref()
        .map_or(String::new(), |a| format!(" Their pick: \"{a}\"."));
    format!(
        "[askguard] Your last reply did not reach the owner: it asks their OK for a step their own words already cover: {}.{pick} Don't ask; do it now, then tell them it's done (what you meant to tell them, without the question). If their words truly don't cover it (the quote is about something else, or it's a payment, password, code, account security, a message or post as them, merging to main or deleting their data), say why in one line and ask again: that one reaches them.\nYour held reply: «{}»",
        covered_line(verdict),
        reply.trim().chars().take(1500).collect::<String>()
    )
}

/// The outcome a check is logged with.
#[must_use]
pub fn outcome(verdict: Option<&Verdict>, overridden: bool) -> &'static str {
    match verdict {
        _ if overridden => "overridden",
        None => "no_verdict",
        Some(v) if v.covered => "covered",
        Some(v) if v.reserved.is_some() => "reserved",
        Some(_) => "not_covered",
    }
}

/// Every pattern here compiles.
#[must_use]
pub fn patterns_compile() -> bool {
    hard_words::patterns_compile() && MERGE.is_some() && DELETE.is_some() && ASKING.is_some()
}
