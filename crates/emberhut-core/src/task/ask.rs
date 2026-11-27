//! Asks: the one decision a task needs from the person, as its question, its answers (plain
//! ones, or cards to read and compare), what each does and the assistant's pick.

use std::sync::LazyLock;

use regex::{Captures, Regex};

use super::ArtifactId;

/// An answer as the assistant gives it: a plain string, or a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionIn {
    /// A plain answer: its button's words.
    Plain(String),
    /// A card.
    Card(CardIn),
}

/// A card as the assistant gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardIn {
    /// Its button: the answer it sends.
    pub label: String,
    /// Its name.
    pub title: Option<String>,
    /// A sentence or two.
    pub summary: Option<String>,
    /// The facts to compare, in order.
    pub fields: Vec<(String, String)>,
    /// A short number at a glance.
    pub badge: Option<String>,
    /// The assistant advises this one.
    pub pick: bool,
    /// Why, when it is the pick.
    pub why: Option<String>,
    /// What happens when it is picked.
    pub then: Option<String>,
    /// The long version.
    pub more: Option<String>,
    /// Its pictures and videos, already the task's artifacts.
    pub media: Vec<ArtifactId>,
}

/// A card as it is stored and shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Card {
    /// Its button.
    pub label: String,
    /// Its name (a card always has one, maybe empty; a plain answer's card has none).
    pub title: Option<String>,
    /// A sentence or two.
    pub summary: Option<String>,
    /// A short number at a glance.
    pub badge: Option<String>,
    /// What happens when it is picked.
    pub then: Option<String>,
    /// The long version.
    pub more: Option<String>,
    /// The facts to compare.
    pub fields: Option<Vec<(String, String)>>,
    /// Why, on the pick.
    pub why: Option<String>,
    /// Its pictures and videos.
    pub media: Option<Vec<ArtifactId>>,
}

/// An ask's answers as stored: the labels (what a tap sends), the cards when any came as one,
/// the notes (each card's `then`, else its summary: what an older app shows under the button)
/// and the pick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choices {
    /// The labels.
    pub options: Vec<String>,
    /// The cards, one per option, when any came as one.
    pub cards: Option<Vec<Card>>,
    /// What each answer does.
    pub notes: Option<Vec<String>>,
    /// The pick: an index into `options`.
    pub recommend: Option<usize>,
}

/// Why an ask's answers are refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AskRefusal {
    /// An answer with no words.
    #[error("an option needs a label: the words on its button")]
    NoLabel,
    /// Two answers with the same words.
    #[error("two options have the same label: each button needs its own words")]
    SameLabel,
    /// More than four plain answers.
    #[error(
        "at most 4 plain answers; to compare more, give them as option cards (label, title, summary, fields)"
    )]
    TooManyPlain,
    /// More than six cards.
    #[error("at most 6 options")]
    TooMany,
    /// Two picks.
    #[error("one pick at most: mark only the option you advise")]
    TwoPicks,
    /// A pick and a `recommend` that disagree.
    #[error("recommend and pick name different options")]
    PickDisagrees,
}

fn given(text: Option<&String>) -> Option<String> {
    text.map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// The stored answers of an ask: plain ones are 4 at most; with cards, 6 (5 to compare and
/// a "more ideas"). Two picks, or a pick that disagrees with `recommend`, are refused.
///
/// # Errors
///
/// [`AskRefusal`].
#[expect(
    clippy::too_many_lines,
    reason = "the checks and the cards, in the order they apply"
)]
pub fn choices_of(
    options: &[OptionIn],
    notes: Option<Vec<String>>,
    recommend: Option<usize>,
) -> Result<Choices, AskRefusal> {
    let labels: Vec<String> = options
        .iter()
        .map(|option| match option {
            OptionIn::Plain(text) => text.trim().to_owned(),
            OptionIn::Card(card) => card.label.trim().to_owned(),
        })
        .collect();
    if labels.iter().any(String::is_empty) {
        return Err(AskRefusal::NoLabel);
    }
    let mut lower: Vec<String> = labels.iter().map(|label| label.to_lowercase()).collect();
    lower.sort();
    lower.dedup();
    if lower.len() < labels.len() {
        return Err(AskRefusal::SameLabel);
    }
    let cards = options
        .iter()
        .filter(|option| matches!(option, OptionIn::Card(_)))
        .count();
    if cards == 0 {
        if labels.len() > 4 {
            return Err(AskRefusal::TooManyPlain);
        }
        return Ok(Choices {
            options: labels,
            cards: None,
            notes,
            recommend,
        });
    }
    if labels.len() > 6 {
        return Err(AskRefusal::TooMany);
    }
    let picks: Vec<usize> = options
        .iter()
        .enumerate()
        .filter(|(_, option)| matches!(option, OptionIn::Card(card) if card.pick))
        .map(|(index, _)| index)
        .collect();
    if picks.len() > 1 {
        return Err(AskRefusal::TwoPicks);
    }
    if let (Some(pick), Some(recommend)) = (picks.first(), recommend)
        && *pick != recommend
    {
        return Err(AskRefusal::PickDisagrees);
    }
    let pick = picks.first().copied().or(recommend);
    let made: Vec<Card> = options
        .iter()
        .zip(&labels)
        .enumerate()
        .map(|(index, (option, label))| match option {
            OptionIn::Plain(_) => Card {
                label: label.clone(),
                ..Card::default()
            },
            OptionIn::Card(card) => {
                let fields: Vec<(String, String)> = card
                    .fields
                    .iter()
                    .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                    .filter(|(k, v)| !k.is_empty() && !v.is_empty())
                    .collect();
                Card {
                    label: label.clone(),
                    title: Some(card.title.as_deref().unwrap_or_default().trim().to_owned()),
                    summary: given(card.summary.as_ref()),
                    badge: given(card.badge.as_ref()),
                    then: given(card.then.as_ref()),
                    more: given(card.more.as_ref()),
                    fields: (!fields.is_empty()).then_some(fields),
                    why: given(card.why.as_ref()).filter(|_| pick == Some(index)),
                    media: (!card.media.is_empty()).then(|| card.media.clone()),
                }
            }
        })
        .collect();
    let said: Vec<String> = made
        .iter()
        .enumerate()
        .map(|(index, card)| {
            card.then
                .clone()
                .or_else(|| notes.as_ref().and_then(|notes| notes.get(index).cloned()))
                .or_else(|| card.summary.clone())
                .unwrap_or_default()
        })
        .collect();
    let notes = if said.iter().any(|line| !line.is_empty()) {
        Some(said)
    } else {
        notes
    };
    Ok(Choices {
        options: labels,
        cards: Some(made),
        notes,
        recommend: pick,
    })
}

/// The answer as a worker hears it: its label, and its card's title when it has another one
/// ("Make #1 (The Pump)").
#[must_use]
pub fn answer_line(options: &[String], cards: Option<&[Card]>, choice: &str) -> String {
    let title = options
        .iter()
        .position(|option| option == choice)
        .and_then(|index| cards?.get(index))
        .and_then(|card| card.title.as_deref())
        .filter(|title| !title.is_empty() && *title != choice);
    match title {
        Some(title) => format!("{choice} ({title})"),
        None => choice.to_owned(),
    }
}

/// An answer that puts the task away instead of carrying it on (`Leave it`, `Not now`, `No`).
#[must_use]
pub fn cancels(choice: &str) -> bool {
    ["leave it", "not now", "no"]
        .iter()
        .any(|word| choice.trim().eq_ignore_ascii_case(word))
}

/// A pattern of this module's: none if it didn't compile (its tests say so), and then what it
/// would take out stays.
fn rx(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

static BOLD: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\*\*(.+?)\*\*|__(.+?)__"));
static EMPHASIS: LazyLock<Option<Regex>> =
    LazyLock::new(|| rx(r"(?m)(^|[\s(])[*_]([^*_\n]+?)[*_]([\s).,!?:;]|$)"));
static CODE: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"`{1,3}([^`]*)`{1,3}"));
static LINK: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\[([^\]]+)\]\(([^)]+)\)"));
static HEADING: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"(?m)^#{1,6}\s+"));
static BULLET: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"(?m)^([ \t]*)[-*•▪◦]\s+"));
static QUOTES: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r#"^\s*["“]|["”]\s*$"#));

/// `text` with `pattern`'s matches replaced (as is when the pattern is missing).
fn replace(pattern: &LazyLock<Option<Regex>>, text: &str, with: impl regex::Replacer) -> String {
    pattern.as_ref().map_or_else(
        || text.to_owned(),
        |re| re.replace_all(text, with).into_owned(),
    )
}

/// A message the person sends to someone is plain text, never markdown: bold, bullet symbols,
/// backticks, headings and `[text](url)` links come out. Numbered lines and emoji stay.
#[must_use]
pub fn plain_text(text: &str) -> String {
    let mut s = replace(&BOLD, text, "$1$2");
    // Emphasis needs the character after it, which a match takes: run again until it is gone.
    for _ in 0..8 {
        let next = replace(&EMPHASIS, &s, "$1$2$3");
        if next == s {
            break;
        }
        s = next;
    }
    s = replace(&CODE, &s, "$1");
    s = replace(&LINK, &s, |caps: &Captures<'_>| {
        let (label, url) = (&caps[1], &caps[2]);
        if url.contains(label) {
            url.to_owned()
        } else {
            format!("{label} ({url})")
        }
    });
    s = replace(&HEADING, &s, "");
    s = replace(&BULLET, &s, "$1");
    s = replace(&QUOTES, &s, "");
    s.trim().to_owned()
}

/// The text's sentences and lines: split after `.`, `?` or `!` followed by space, and at line
/// breaks.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            out.push(std::mem::take(&mut current));
            continue;
        }
        current.push(c);
        if matches!(c, '.' | '?' | '!') && chars.peek().is_some_and(|next| next.is_whitespace()) {
            out.push(std::mem::take(&mut current));
            while chars
                .peek()
                .is_some_and(|next| next.is_whitespace() && *next != '\n')
            {
                chars.next();
            }
        }
    }
    out.push(current);
    out.into_iter().map(|s| s.trim().to_owned()).collect()
}

fn starts_with_word(text: &str, words: &[&str]) -> bool {
    let lower = text.to_lowercase();
    words.iter().any(|word| {
        lower.strip_prefix(word).is_some_and(|rest| {
            rest.chars()
                .next()
                .is_none_or(|next| !next.is_alphanumeric() && next != '_')
        })
    })
}

/// An action for an ask that came without one: the question's last short question, or its
/// answers. The assistant rewrites these.
#[must_use]
pub fn action_from(question: &str, options: &[String]) -> String {
    let asks: Vec<String> = sentences(&plain_text(question))
        .into_iter()
        .filter(|s| s.ends_with('?') && s.chars().count() <= 90)
        .collect();
    if let Some(last) = asks.last() {
        let mut chars = last.chars();
        let first: String = chars
            .next()
            .into_iter()
            .flat_map(char::to_lowercase)
            .collect();
        return format!("Decide: {first}{}", chars.as_str());
    }
    let real: Vec<&String> = options
        .iter()
        .filter(|option| {
            !["not now", "leave it", "no"]
                .iter()
                .any(|word| option.eq_ignore_ascii_case(word))
        })
        .collect();
    let joined = real
        .iter()
        .map(|option| option.as_str())
        .collect::<Vec<_>>()
        .join(" or ");
    if !real.is_empty() && joined.chars().count() <= 80 {
        format!("Pick one: {joined}")
    } else {
        "Answer the question".to_owned()
    }
}

/// What an ask costs the person when it didn't say: cards are read and compared (choose);
/// something they did ("I added it", "Done") is a do; a short yes or no an approve.
#[must_use]
pub fn kind_of(kind: Option<&str>, options: &[String], cards: Option<&[Card]>) -> &'static str {
    if cards.is_some_and(|cards| {
        cards
            .iter()
            .any(|card| card.title.as_deref().is_some_and(|title| !title.is_empty()))
    }) {
        return "choose";
    }
    match kind {
        Some("approve") => return "approve",
        Some("choose") => return "choose",
        Some("do") => return "do",
        _ => {}
    }
    let first = options.first().map_or_default(String::as_str);
    if starts_with_word(first, &["i", "done"]) || first.to_lowercase().starts_with("i'") {
        return "do";
    }
    let real: Vec<&String> = options
        .iter()
        .filter(|option| {
            !starts_with_word(option, &["not now", "not yet", "later", "no", "leave it"])
        })
        .collect();
    if real.len() <= 2
        && real.first().is_some_and(|first| {
            starts_with_word(
                first,
                &["approve", "turn", "start", "yes", "ok", "go", "send"],
            )
        })
    {
        return "approve";
    }
    "choose"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(label: &str, title: &str) -> OptionIn {
        OptionIn::Card(CardIn {
            label: label.to_owned(),
            title: Some(title.to_owned()),
            ..CardIn::default()
        })
    }

    #[test]
    fn every_pattern_compiles() {
        assert!(BOLD.is_some(), "BOLD");
        assert!(EMPHASIS.is_some(), "EMPHASIS");
        assert!(CODE.is_some(), "CODE");
        assert!(LINK.is_some(), "LINK");
        assert!(HEADING.is_some(), "HEADING");
        assert!(BULLET.is_some(), "BULLET");
        assert!(QUOTES.is_some(), "QUOTES");
    }

    #[test]
    fn plain_answers_are_four_at_most_and_each_has_its_own_words() {
        let plain = |words: &[&str]| -> Vec<OptionIn> {
            words
                .iter()
                .map(|w| OptionIn::Plain((*w).to_owned()))
                .collect()
        };
        let choices = choices_of(&plain(&["Yes", " No "]), None, Some(0)).unwrap();
        assert_eq!(choices.options, ["Yes", "No"]);
        assert!(choices.cards.is_none());
        assert_eq!(
            choices_of(&plain(&["a", "b", "c", "d", "e"]), None, None),
            Err(AskRefusal::TooManyPlain)
        );
        assert_eq!(
            choices_of(&plain(&["Yes", "yes"]), None, None),
            Err(AskRefusal::SameLabel)
        );
        assert_eq!(
            choices_of(&plain(&["Yes", " "]), None, None),
            Err(AskRefusal::NoLabel)
        );
    }

    #[test]
    fn cards_take_one_pick_and_say_what_they_do() {
        let mut picked = CardIn {
            label: "Make #1".to_owned(),
            title: Some(" The Pump ".to_owned()),
            then: Some("It gets made".to_owned()),
            why: Some("Cheapest".to_owned()),
            pick: true,
            fields: vec![
                ("Cost".to_owned(), "3".to_owned()),
                (String::new(), "x".to_owned()),
            ],
            ..CardIn::default()
        };
        let options = [
            OptionIn::Card(picked.clone()),
            card("Make #2", "The Valve"),
            OptionIn::Plain("More ideas".to_owned()),
        ];
        let choices = choices_of(&options, None, None).unwrap();
        let cards = choices.cards.clone().unwrap();
        assert_eq!(choices.recommend, Some(0));
        assert_eq!(cards[0].title.as_deref(), Some("The Pump"));
        assert_eq!(cards[0].why.as_deref(), Some("Cheapest"));
        assert_eq!(cards[0].fields.as_ref().unwrap().len(), 1);
        assert_eq!(cards[2].title, None);
        assert_eq!(
            choices.notes.unwrap(),
            ["It gets made".to_owned(), String::new(), String::new()]
        );
        assert_eq!(
            answer_line(&choices.options, Some(&cards), "Make #1"),
            "Make #1 (The Pump)"
        );
        assert_eq!(
            answer_line(&choices.options, Some(&cards), "More ideas"),
            "More ideas"
        );
        assert_eq!(
            choices_of(&options, None, Some(1)),
            Err(AskRefusal::PickDisagrees)
        );
        picked.label = "Make #2".to_owned();
        let two = [
            OptionIn::Card(picked.clone()),
            OptionIn::Card(CardIn {
                label: "x".to_owned(),
                pick: true,
                ..CardIn::default()
            }),
        ];
        assert_eq!(choices_of(&two, None, None), Err(AskRefusal::TwoPicks));
        assert_eq!(kind_of(None, &choices.options, Some(&cards)), "choose");
    }

    #[test]
    fn markdown_comes_out_of_a_message() {
        assert_eq!(
            plain_text("**Hi** _there_ and *you*, see `this` and [the doc](https://example.com/d)"),
            "Hi there and you, see this and the doc (https://example.com/d)"
        );
        assert_eq!(plain_text("# Title\n- one\n- two"), "Title\none\ntwo");
        assert_eq!(
            plain_text("[https://example.com](https://example.com)"),
            "https://example.com"
        );
        assert_eq!(plain_text("\"Quoted draft\""), "Quoted draft");
    }

    #[test]
    fn an_action_comes_from_the_last_short_question_or_the_answers() {
        assert_eq!(
            action_from("I compared both. Which name should the report have?", &[]),
            "Decide: which name should the report have?"
        );
        assert_eq!(
            action_from(
                "Two ways to go.",
                &["Opus".to_owned(), "Haiku".to_owned(), "Not now".to_owned()]
            ),
            "Pick one: Opus or Haiku"
        );
        assert_eq!(action_from("Thoughts.", &[]), "Answer the question");
    }

    #[test]
    fn an_asks_kind_is_guessed_from_its_answers() {
        let words = |w: &[&str]| -> Vec<String> { w.iter().map(|s| (*s).to_owned()).collect() };
        assert_eq!(
            kind_of(None, &words(&["I added it", "Not yet"]), None),
            "do"
        );
        assert_eq!(
            kind_of(None, &words(&["Approve", "Not now"]), None),
            "approve"
        );
        assert_eq!(kind_of(None, &words(&["Opus", "Haiku"]), None), "choose");
        assert_eq!(kind_of(Some("do"), &words(&["Opus"]), None), "do");
        assert!(cancels(" Leave it") && cancels("NO") && !cancels("Not yet"));
    }
}
