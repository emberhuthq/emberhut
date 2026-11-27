//! Parties: two to five AI personas, each a name, a role and a model, talking about the
//! person's topic turn by turn while the person watches. The person can jump in (their words
//! join the talk), steer (a direction the cast follows from then on), pause, resume and stop.
//! Each party has a turn cap and a token cap; its cost is counted per turn at list prices.
//!
//! These are the rules, with no I/O: what a new party is made of, who speaks next, what a
//! persona is told, and what a turn costs. The server's party service drives the talk.

use jiff::Timestamp;

use crate::{PartyId, PartyMessageId};

/// The fewest personas a party has.
pub const MIN_PERSONAS: usize = 2;
/// The most personas a party has.
pub const MAX_PERSONAS: usize = 5;
/// The model a persona talks with when none (or an unknown one) is given.
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// The small model that picks who speaks next.
pub const SUPERVISOR_MODEL: &str = "claude-haiku-5-5";
/// A new party's turn cap when none is given.
pub const DEFAULT_MAX_TURNS: u32 = 12;
/// A new party's token cap when none is given.
pub const DEFAULT_MAX_TOKENS: u64 = 150_000;
/// The longest topic, and the longest thing the person says, in characters.
pub const MAX_TEXT: usize = 4000;
/// Who the person is in a party's talk (the `who` of their lines).
pub const HOST: &str = "owner";
/// The person's name in a party's talk.
pub const HOST_NAME: &str = "You";
/// The most tokens a persona's turn may write.
pub const TURN_MAX_TOKENS: u32 = 2000;
/// The most tokens the supervisor's answer may write.
pub const SUPERVISOR_MAX_TOKENS: u32 = 1000;

/// The colours personas get, in the order they are cast.
pub const COLORS: [&str; 5] = ["blue", "orange", "green", "purple", "pink"];

/// A model a persona can talk with, and its list prices in USD per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelPrice {
    /// The model's id.
    pub id: &'static str,
    /// Its name, for people.
    pub name: &'static str,
    /// USD per million tokens in.
    pub input: f64,
    /// USD per million tokens out.
    pub output: f64,
    /// A line about it, for the app's picker.
    pub note: &'static str,
}

/// The models a persona can talk with.
pub const MODELS: [ModelPrice; 3] = [
    ModelPrice {
        id: "claude-opus-5-5",
        name: "Opus 5.5",
        input: 4.0,
        output: 20.0,
        note: "Best talker · $4/$20 per M tokens",
    },
    ModelPrice {
        id: "claude-sonnet-5-5",
        name: "Sonnet 5.5",
        input: 2.0,
        output: 10.0,
        note: "Half the price · $2/$10",
    },
    ModelPrice {
        id: "claude-haiku-5-5",
        name: "Haiku 5.5",
        input: 0.10,
        output: 0.50,
        note: "Fast and cheap · $0.10/$0.50",
    },
];

/// One persona. In a party its id is `p1`…`p5` and it has a colour; a preset's id is its own
/// (`optimist`) and its colour empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Persona {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Who they are and how they talk: their system prompt's heart.
    pub role: String,
    /// The model they talk with.
    pub model: String,
    /// Their colour, or empty.
    pub color: String,
}

/// The ready-made personas the person picks from (anything else is their own).
#[must_use]
pub fn presets() -> Vec<Persona> {
    [
        (
            "optimist",
            "Nora",
            "An optimist. You see what could go right and build on other people's ideas, but stay concrete about how.",
        ),
        (
            "skeptic",
            "Victor",
            "A skeptic. You poke holes, ask for evidence and say plainly when something won't work.",
        ),
        (
            "engineer",
            "Sam",
            "A practical engineer. You ask how it would actually be built, what it costs and what breaks first.",
        ),
        (
            "historian",
            "Lena",
            "A historian. You bring precedents: how similar things played out before, and what people forget.",
        ),
        (
            "devil",
            "Omar",
            "The devil's advocate. You argue against whatever the room leans toward, sharply but in good faith.",
        ),
        (
            "product",
            "Rita",
            "A product lead. You think about the people it's for, what they'd pay for and what to ship first.",
        ),
        (
            "scientist",
            "Dr. Kim",
            "A scientist. Careful and precise: you separate what's known from guesses and think in experiments.",
        ),
        (
            "comedian",
            "Jules",
            "A comedian. Quick, funny and a little absurd, but every joke lands on a real point.",
        ),
    ]
    .into_iter()
    .map(|(id, name, role)| Persona {
        id: id.to_owned(),
        name: name.to_owned(),
        role: role.to_owned(),
        model: DEFAULT_MODEL.to_owned(),
        color: String::new(),
    })
    .collect()
}

/// Where a party is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartyStatus {
    /// Talking now.
    Running,
    /// Paused by the person, or waiting on something (a cap they can raise, a failure).
    Paused,
    /// It reached its end (a cap, or the talk's natural end).
    Done,
    /// Stopped by the person.
    Stopped,
}

impl PartyStatus {
    /// The status's word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Stopped => "stopped",
        }
    }

    /// The status a word names; an unknown word reads as paused (it waits for the person).
    #[must_use]
    pub fn parse(word: &str) -> Self {
        match word {
            "running" => Self::Running,
            "done" => Self::Done,
            "stopped" => Self::Stopped,
            _ => Self::Paused,
        }
    }
}

/// A party, as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Party {
    /// Its id.
    pub id: PartyId,
    /// Its title.
    pub title: String,
    /// Its topic, or opening line.
    pub topic: String,
    /// Where it is.
    pub status: PartyStatus,
    /// Why it paused or ended, in words.
    pub why: Option<String>,
    /// Its cast, in the order they were cast.
    pub personas: Vec<Persona>,
    /// The personas' turns so far.
    pub turns: u32,
    /// The turn cap.
    pub max_turns: u32,
    /// Tokens used so far, the supervisor's included.
    pub tokens: u64,
    /// The token cap.
    pub max_tokens: u64,
    /// USD at list prices, the supervisor's calls included.
    pub cost: f64,
    /// When it was made.
    pub created: Timestamp,
    /// When it last changed.
    pub updated: Timestamp,
}

/// What a line in a party is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageKind {
    /// A persona's turn.
    Turn,
    /// The person jumped in.
    Say,
    /// The person's direction, which the cast follows from then on.
    Steer,
}

impl MessageKind {
    /// The kind's word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Turn => "turn",
            Self::Say => "say",
            Self::Steer => "steer",
        }
    }

    /// The kind a word names; an unknown word reads as a turn.
    #[must_use]
    pub fn parse(word: &str) -> Self {
        match word {
            "say" => Self::Say,
            "steer" => Self::Steer,
            _ => Self::Turn,
        }
    }
}

/// One line said in a party.
#[derive(Clone, Debug, PartialEq)]
pub struct PartyMessage {
    /// Its id.
    pub id: PartyMessageId,
    /// Its party.
    pub party: PartyId,
    /// [`HOST`], or a persona's id.
    pub who: String,
    /// The speaker's name.
    pub name: String,
    /// What it is.
    pub kind: MessageKind,
    /// What was said.
    pub text: String,
    /// The model a turn was said with.
    pub model: Option<String>,
    /// Tokens the turn took.
    pub tokens: u64,
    /// What the turn cost, USD.
    pub cost: f64,
    /// Stopped mid-turn.
    pub cut: bool,
    /// When it was said.
    pub at: Timestamp,
}

/// A line to add to a party.
#[derive(Clone, Debug, PartialEq)]
pub struct NewPartyMessage {
    /// [`HOST`], or a persona's id.
    pub who: String,
    /// The speaker's name.
    pub name: String,
    /// What it is.
    pub kind: MessageKind,
    /// What was said.
    pub text: String,
    /// The model a turn was said with.
    pub model: Option<String>,
    /// Tokens the turn took.
    pub tokens: u64,
    /// What the turn cost, USD.
    pub cost: f64,
    /// Stopped mid-turn.
    pub cut: bool,
}

impl NewPartyMessage {
    /// The person's words: something said, or a direction.
    #[must_use]
    pub fn host(kind: MessageKind, text: impl Into<String>) -> Self {
        Self {
            who: HOST.to_owned(),
            name: HOST_NAME.to_owned(),
            kind,
            text: text.into(),
            model: None,
            tokens: 0,
            cost: 0.0,
            cut: false,
        }
    }
}

/// A persona as the person casts it: a name, maybe a role and a model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PersonaDraft {
    /// The name.
    pub name: String,
    /// The role; blank gets a stock one.
    pub role: String,
    /// The model; unknown gets [`DEFAULT_MODEL`].
    pub model: String,
}

/// A party as the person asks for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewParty {
    /// The topic or opening line.
    pub topic: String,
    /// The cast.
    pub personas: Vec<PersonaDraft>,
    /// A title; none takes the topic's start.
    pub title: Option<String>,
    /// The turn cap; none is [`DEFAULT_MAX_TURNS`].
    pub max_turns: Option<f64>,
    /// The token cap; none is [`DEFAULT_MAX_TOKENS`].
    pub max_tokens: Option<f64>,
}

/// A new party, checked and filled in: what is stored.
#[derive(Clone, Debug, PartialEq)]
pub struct PartyPlan {
    /// Its title.
    pub title: String,
    /// Its topic.
    pub topic: String,
    /// Its cast, with their ids and colours.
    pub personas: Vec<Persona>,
    /// Its turn cap.
    pub max_turns: u32,
    /// Its token cap.
    pub max_tokens: u64,
}

/// Why a party, or something said in one, was not taken. The text is what the app shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PartyRefusal {
    /// The topic is blank.
    #[error("a party needs a topic or an opening line")]
    NoTopic,
    /// The topic is too long.
    #[error("the topic is at most 4,000 characters")]
    TopicTooLong,
    /// Too few or too many personas.
    #[error("a party has 2 to 5 personas")]
    Cast,
    /// Two personas share a name.
    #[error("each persona needs a name of its own")]
    SameName,
    /// The person's words are blank.
    #[error("say something")]
    Blank,
    /// The person's words are too long.
    #[error("at most 4,000 characters")]
    TooLong,
}

/// The role a persona cast without one gets.
const STOCK_ROLE: &str = "A thoughtful guest with opinions of their own.";

/// Checks `new` and fills it in: a trimmed topic, a cast of 2 to 5 with their own names (blank
/// names dropped; a name cut to 40 characters, a role to 2,000), ids `p1`… and colours, a known
/// model each, caps within bounds (turns 1 to 100, tokens 5,000 to 2,000,000) and a title (the
/// one given, or the topic on one line), cut to 80 characters.
///
/// # Errors
///
/// [`PartyRefusal`] for a blank or long topic, a cast of the wrong size, or two names alike.
pub fn plan(new: &NewParty) -> Result<PartyPlan, PartyRefusal> {
    let topic = new.topic.trim();
    if topic.is_empty() {
        return Err(PartyRefusal::NoTopic);
    }
    if topic.chars().count() > MAX_TEXT {
        return Err(PartyRefusal::TopicTooLong);
    }
    let cast: Vec<&PersonaDraft> = new
        .personas
        .iter()
        .filter(|draft| !draft.name.trim().is_empty())
        .collect();
    if !(MIN_PERSONAS..=MAX_PERSONAS).contains(&cast.len()) {
        return Err(PartyRefusal::Cast);
    }
    let personas: Vec<Persona> = cast
        .iter()
        .zip(COLORS)
        .enumerate()
        .map(|(n, (draft, color))| {
            let role = cut(draft.role.trim(), 2000);
            Persona {
                id: format!("p{}", n + 1),
                name: cut(draft.name.trim(), 40),
                role: if role.is_empty() {
                    STOCK_ROLE.to_owned()
                } else {
                    role
                },
                model: known_model(&draft.model).to_owned(),
                color: color.to_owned(),
            }
        })
        .collect();
    let mut names: Vec<String> = personas.iter().map(|p| p.name.to_lowercase()).collect();
    names.sort();
    names.dedup();
    if names.len() != personas.len() {
        return Err(PartyRefusal::SameName);
    }
    let title = new
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map_or_else(|| one_line(topic), str::to_owned);
    Ok(PartyPlan {
        title: cut(&title, 80),
        topic: topic.to_owned(),
        personas,
        max_turns: clamp_turns(new.max_turns, DEFAULT_MAX_TURNS, 100),
        max_tokens: clamp_tokens(new.max_tokens, DEFAULT_MAX_TOKENS, 2_000_000),
    })
}

/// `model` if it is one of [`MODELS`], else [`DEFAULT_MODEL`].
#[must_use]
pub fn known_model(model: &str) -> &'static str {
    MODELS
        .iter()
        .find(|known| known.id == model)
        .map_or(DEFAULT_MODEL, |known| known.id)
}

/// A turn cap the person sets on a running party: 1 to 200.
#[must_use]
pub fn caps_turns(turns: f64) -> u32 {
    clamp_turns(Some(turns), 1, 200)
}

/// A token cap the person sets on a running party: 5,000 to 5,000,000.
#[must_use]
pub fn caps_tokens(tokens: f64) -> u64 {
    clamp_tokens(Some(tokens), 5_000, 5_000_000)
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 1..=max first, so the value fits"
)]
fn clamp_turns(value: Option<f64>, default: u32, max: u32) -> u32 {
    match value {
        Some(value) if value.is_finite() => value.trunc().clamp(1.0, f64::from(max)) as u32,
        _ => default,
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "clamped to 5,000..=max first (both exact in an f64), so the value fits"
)]
fn clamp_tokens(value: Option<f64>, default: u64, max: u64) -> u64 {
    match value {
        Some(value) if value.is_finite() => value.trunc().clamp(5_000.0, max as f64) as u64,
        _ => default,
    }
}

/// Checks what the person says in a party: trimmed, not blank, at most [`MAX_TEXT`]
/// characters.
///
/// # Errors
///
/// [`PartyRefusal::Blank`] or [`PartyRefusal::TooLong`].
pub fn said(text: &str) -> Result<&str, PartyRefusal> {
    let text = text.trim();
    if text.is_empty() {
        Err(PartyRefusal::Blank)
    } else if text.chars().count() > MAX_TEXT {
        Err(PartyRefusal::TooLong)
    } else {
        Ok(text)
    }
}

/// The caps a party resumes with: one that reached its turn cap gets 10 more turns, one at 95%
/// of its token cap or more gets half its cap again (at least 20,000 more).
#[must_use]
pub fn resumed_caps(party: &Party) -> (u32, u64) {
    let turns = if party.turns >= party.max_turns {
        party.turns.saturating_add(10)
    } else {
        party.max_turns
    };
    let tokens = if party.tokens.saturating_mul(20) >= party.max_tokens.saturating_mul(19) {
        party
            .tokens
            .saturating_add((party.max_tokens / 2).max(20_000))
    } else {
        party.max_tokens
    };
    (turns, tokens)
}

/// Why a running party should end before its next turn, if it should: a cap reached.
#[must_use]
pub fn cap_reached(party: &Party) -> Option<String> {
    if party.turns >= party.max_turns {
        Some(format!("Reached its {} turns", party.max_turns))
    } else if party.tokens >= party.max_tokens {
        Some(format!(
            "Reached its token cap ({})",
            thousands(party.max_tokens)
        ))
    } else {
        None
    }
}

/// Who speaks next, by the rules alone, or the choice a supervisor makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// This persona speaks: the person named them, it is the first round, or there is only one
    /// other and the talk may not end yet.
    Speaks(Persona),
    /// A supervisor picks among `choices` (the personas who didn't just speak); with `may_end`
    /// it may also say the talk has reached its end. `fallback` speaks if it can't pick.
    Ask {
        /// Who may speak.
        choices: Vec<Persona>,
        /// Whether the talk may end now.
        may_end: bool,
        /// Who speaks when the supervisor gives no answer.
        fallback: Persona,
    },
}

/// Who speaks next in `party` after `said` (its talk so far, oldest first).
///
/// The persona the person's latest words name answers (the first name in them); else, in the
/// first round, everyone in the order they were cast; else a supervisor picks among those who
/// didn't just speak, and may end the talk, never before two rounds nor right after the
/// person spoke. With one other persona and no end possible, they simply speak. `None` for a
/// party with no cast.
#[must_use]
pub fn next(party: &Party, said: &[PartyMessage]) -> Option<Next> {
    let cast = &party.personas;
    let last = said.iter().rev().find(|m| m.kind != MessageKind::Steer);
    let last_turn = said.iter().rev().find(|m| m.kind == MessageKind::Turn);
    let others: Vec<Persona> = cast
        .iter()
        .filter(|c| last_turn.is_none_or(|turn| turn.who != c.id))
        .cloned()
        .collect();
    if let Some(last) = last.filter(|m| m.who == HOST) {
        let named = cast
            .iter()
            .filter_map(|c| mention(&last.text, &c.name).map(|at| (at, c)))
            .min_by_key(|(at, _)| *at);
        if let Some((_, named)) = named {
            return Some(Next::Speaks(named.clone()));
        }
    }
    if let Some(fresh) = cast.iter().find(|c| {
        !said
            .iter()
            .any(|m| m.kind == MessageKind::Turn && m.who == c.id)
    }) {
        return Some(Next::Speaks(fresh.clone()));
    }
    let rounds_done = usize::try_from(party.turns).unwrap_or(usize::MAX) >= 2 * cast.len();
    let may_end = rounds_done && said.last().is_none_or(|m| m.who != HOST);
    let fallback = others.first().or(cast.first())?.clone();
    if others.len() <= 1 && !may_end {
        return Some(Next::Speaks(fallback));
    }
    Some(Next::Ask {
        choices: others,
        may_end,
        fallback,
    })
}

/// Where `name` first stands in `text` as a word of its own (case aside: no letter right
/// before or after it), as a character index.
#[must_use]
pub fn mention(text: &str, name: &str) -> Option<usize> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    if name.is_empty() || name.len() > text.len() {
        return None;
    }
    (0..=text.len() - name.len()).find(|&at| {
        text[at..at + name.len()] == name[..]
            && (at == 0 || !text[at - 1].is_alphabetic())
            && text.get(at + name.len()).is_none_or(|c| !c.is_alphabetic())
    })
}

/// The talk as a transcript, the latest `max` lines, for a prompt.
#[must_use]
pub fn transcript(said: &[PartyMessage], max: usize) -> String {
    let from = said.len().saturating_sub(max);
    said[from..]
        .iter()
        .map(|m| match m.kind {
            MessageKind::Steer => format!("[Direction from the host: {}]", m.text),
            MessageKind::Say => format!("The host: {}", m.text),
            MessageKind::Turn => format!(
                "{}: {}{}",
                m.name,
                m.text,
                if m.cut { " [cut off]" } else { "" }
            ),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The supervisor's system prompt and request, for choosing among `choices`.
#[must_use]
pub fn supervisor_prompt(
    party: &Party,
    said: &[PartyMessage],
    choices: &[Persona],
    may_end: bool,
) -> (String, String) {
    let system = format!(
        "You run a lively group conversation between AI personas, for the host who watches. \
         Decide who speaks next: the persona with the most to add right now (someone just \
         addressed or challenged, a view not yet heard, a natural reply). Variety matters: \
         don't let one voice dominate.{} Answer with JSON only: \
         {{\"next\": \"<persona id>\", \"end\": false}}",
        if may_end {
            " Also say whether the talk has reached a natural end (they agree and wrap up, or \
             only repeat themselves)."
        } else {
            ""
        }
    );
    let list = choices
        .iter()
        .map(|c| format!("- {}: {}, {}", c.id, c.name, cut(&c.role, 200)))
        .collect::<Vec<_>>()
        .join("\n");
    let user = format!(
        "Topic: {}\n\nPersonas who may speak next:\n{list}\n\nThe conversation so far (latest \
         last):\n\n{}",
        party.topic,
        transcript(said, 16)
    );
    (system, user)
}

/// A persona's system prompt and request for its turn.
#[must_use]
pub fn speaker_prompt(party: &Party, who: &Persona, said: &[PartyMessage]) -> (String, String) {
    let others: Vec<&str> = party
        .personas
        .iter()
        .filter(|c| c.id != who.id)
        .map(|c| c.name.as_str())
        .collect();
    let steer = said.iter().rev().find(|m| m.kind == MessageKind::Steer);
    let mut system = format!(
        "You are {name}, one voice in a live group conversation (a \"party\") with {others}, \
         watched by the host, who may jump in or give directions.\n\nWho you are: {role}\n\n\
         The topic: {topic}\n\nHow to talk: speak only as yourself, in the first person, as if \
         around a table. One turn is short: 1 to 3 sentences, under 70 words, plain spoken \
         words (no lists, headings, stage directions or your name in front). Build on, answer \
         or push back on what was just said, and name the person you answer when it helps. \
         Stay in character, bring something new each time and never repeat what someone \
         already said. If the host speaks to you, answer them.",
        name = who.name,
        others = names(&others),
        role = who.role,
        topic = party.topic,
    );
    if let Some(steer) = steer {
        system.push_str("\n\nThe host's latest direction, which everyone follows from now on: ");
        system.push_str(&steer.text);
    }
    let user = format!(
        "The conversation so far:\n\n{}\n\nNow it's your turn, {}.",
        transcript(said, 40),
        who.name
    );
    (system, user)
}

/// A reply without the name a model may put in front (`**Nora**: `), trimmed.
#[must_use]
pub fn clean(name: &str, text: &str) -> String {
    let text = text.trim_start();
    let rest = text.trim_start_matches('*');
    let named = rest
        .get(..name.len())
        .is_some_and(|start| !name.is_empty() && start.to_lowercase() == name.to_lowercase());
    if named {
        let after = rest[name.len()..].trim_start_matches('*');
        if let Some(after) = after.strip_prefix(':') {
            return after.trim().to_owned();
        }
    }
    text.trim().to_owned()
}

/// Names joined as people say them: `A`, `A and B`, `A, B and C`.
#[must_use]
pub fn names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A model's name for people: its [`MODELS`] name, or its id.
#[must_use]
pub fn model_name(id: &str) -> &str {
    MODELS.iter().find(|m| m.id == id).map_or(id, |m| m.name)
}

/// What `tokens_in` and `tokens_out` on `model` cost at list prices, USD; an unknown model is
/// free.
#[must_use]
pub fn price(model: &str, tokens_in: u32, tokens_out: u32) -> f64 {
    MODELS.iter().find(|m| m.id == model).map_or(0.0, |m| {
        (f64::from(tokens_in) * m.input + f64::from(tokens_out) * m.output) / 1_000_000.0
    })
}

/// The first `max` characters of `text`.
#[must_use]
pub fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// `text` on one line: every run of whitespace one space.
#[must_use]
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A number with thousands separators: `150,000`.
#[must_use]
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str) -> PersonaDraft {
        PersonaDraft {
            name: name.to_owned(),
            ..PersonaDraft::default()
        }
    }

    fn party(names: &[&str], turns: u32) -> Party {
        let plan = plan(&NewParty {
            topic: "Cities".to_owned(),
            personas: names.iter().map(|n| draft(n)).collect(),
            ..NewParty::default()
        })
        .unwrap();
        Party {
            id: PartyId::new(1),
            title: plan.title,
            topic: plan.topic,
            status: PartyStatus::Running,
            why: None,
            personas: plan.personas,
            turns,
            max_turns: plan.max_turns,
            tokens: 0,
            max_tokens: plan.max_tokens,
            cost: 0.0,
            created: Timestamp::UNIX_EPOCH,
            updated: Timestamp::UNIX_EPOCH,
        }
    }

    fn line(who: &str, kind: MessageKind, text: &str) -> PartyMessage {
        PartyMessage {
            id: PartyMessageId::new(1),
            party: PartyId::new(1),
            who: who.to_owned(),
            name: who.to_owned(),
            kind,
            text: text.to_owned(),
            model: None,
            tokens: 0,
            cost: 0.0,
            cut: false,
            at: Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn a_plan_fills_in_ids_colours_models_caps_and_a_title() {
        let plan = plan(&NewParty {
            topic: "  Should   cities\nban cars?  ".to_owned(),
            personas: vec![
                PersonaDraft {
                    name: " Nora ".to_owned(),
                    role: String::new(),
                    model: "claude-haiku-5-5".to_owned(),
                },
                draft(""),
                PersonaDraft {
                    name: "Victor".to_owned(),
                    role: "A skeptic.".to_owned(),
                    model: "gpt-9".to_owned(),
                },
            ],
            max_turns: Some(500.0),
            max_tokens: Some(10.0),
            ..NewParty::default()
        })
        .unwrap();
        assert_eq!(plan.title, "Should cities ban cars?");
        assert_eq!(plan.personas.len(), 2);
        assert_eq!(plan.personas[0].id, "p1");
        assert_eq!(plan.personas[0].name, "Nora");
        assert_eq!(plan.personas[0].role, STOCK_ROLE);
        assert_eq!(plan.personas[0].model, "claude-haiku-5-5");
        assert_eq!(plan.personas[1].color, "orange");
        assert_eq!(plan.personas[1].model, DEFAULT_MODEL);
        assert_eq!((plan.max_turns, plan.max_tokens), (100, 5_000));
    }

    #[test]
    fn a_plan_refuses_a_blank_topic_a_wrong_cast_and_names_alike() {
        let new = |topic: &str, names: &[&str]| NewParty {
            topic: topic.to_owned(),
            personas: names.iter().map(|n| draft(n)).collect(),
            ..NewParty::default()
        };
        assert_eq!(plan(&new(" ", &["A", "B"])), Err(PartyRefusal::NoTopic));
        assert_eq!(plan(&new("t", &["A"])), Err(PartyRefusal::Cast));
        assert_eq!(
            plan(&new("t", &["A", "B", "C", "D", "E", "F"])),
            Err(PartyRefusal::Cast)
        );
        assert_eq!(
            plan(&new("t", &["Ann", "ann"])),
            Err(PartyRefusal::SameName)
        );
        assert_eq!(
            plan(&new(&"x".repeat(4001), &["A", "B"])),
            Err(PartyRefusal::TopicTooLong)
        );
    }

    #[test]
    fn the_first_round_goes_in_cast_order_then_a_named_persona_answers() {
        let p = party(&["Nora", "Victor", "Sam"], 0);
        assert_eq!(next(&p, &[]), Some(Next::Speaks(p.personas[0].clone())));
        let said = vec![line("p1", MessageKind::Turn, "hi")];
        assert_eq!(next(&p, &said), Some(Next::Speaks(p.personas[1].clone())));
        let said = vec![
            line("p1", MessageKind::Turn, "hi"),
            line(HOST, MessageKind::Say, "Sam, then victor: go"),
        ];
        assert_eq!(next(&p, &said), Some(Next::Speaks(p.personas[2].clone())));
        // A name inside another word is no mention.
        assert_eq!(mention("Samuel says", "Sam"), None);
        assert_eq!(mention("ok, SAM!", "Sam"), Some(4));
    }

    #[test]
    fn after_the_first_round_two_alternate_and_more_are_supervised() {
        let p = party(&["Nora", "Victor"], 2);
        let said = vec![
            line("p1", MessageKind::Turn, "a"),
            line("p2", MessageKind::Turn, "b"),
        ];
        assert_eq!(next(&p, &said), Some(Next::Speaks(p.personas[0].clone())));
        let p = party(&["Nora", "Victor"], 4);
        match next(&p, &said).unwrap() {
            Next::Ask {
                choices, may_end, ..
            } => {
                assert_eq!(choices, vec![p.personas[0].clone()]);
                assert!(may_end);
            }
            other @ Next::Speaks(_) => panic!("{other:?}"),
        }
        let p = party(&["Nora", "Victor", "Sam"], 3);
        let said = vec![
            line("p1", MessageKind::Turn, "a"),
            line("p2", MessageKind::Turn, "b"),
            line("p3", MessageKind::Turn, "c"),
        ];
        match next(&p, &said).unwrap() {
            Next::Ask {
                choices,
                may_end,
                fallback,
            } => {
                assert_eq!(choices.len(), 2);
                assert!(!may_end, "never before two rounds");
                assert_eq!(fallback.id, "p1");
            }
            other @ Next::Speaks(_) => panic!("{other:?}"),
        }
    }

    #[test]
    fn caps_resume_with_room_for_more() {
        let mut p = party(&["A", "B"], 12);
        p.tokens = 149_000;
        assert_eq!(resumed_caps(&p), (22, 149_000 + 75_000));
        assert_eq!(cap_reached(&p).as_deref(), Some("Reached its 12 turns"));
        p.turns = 1;
        p.tokens = 150_000;
        assert_eq!(
            cap_reached(&p).as_deref(),
            Some("Reached its token cap (150,000)")
        );
    }

    #[test]
    fn a_reply_loses_the_name_in_front_and_costs_its_list_price() {
        assert_eq!(clean("Nora", "  **Nora**: I think so. "), "I think so.");
        assert_eq!(clean("Nora", "nora: yes"), "yes");
        assert_eq!(clean("Nora", "Norah is right"), "Norah is right");
        let cost = price("claude-opus-5-5", 1_000_000, 100_000);
        assert!((cost - 6.0).abs() < 1e-9);
        assert!(price("unknown", 5, 5).abs() < f64::EPSILON);
        assert_eq!(names(&["A", "B", "C"]), "A, B and C");
    }

    #[test]
    fn prompts_carry_the_role_the_topic_and_the_latest_direction() {
        let p = party(&["Nora", "Victor"], 0);
        let said = vec![
            line(HOST, MessageKind::Steer, "argue the other side"),
            line("p1", MessageKind::Turn, "fine"),
        ];
        let (system, user) = speaker_prompt(&p, &p.personas[1], &said);
        assert!(system.contains("You are Victor"));
        assert!(system.contains("with Nora"));
        assert!(system.contains("argue the other side"));
        assert!(user.ends_with("Now it's your turn, Victor."));
        assert!(transcript(&said, 40).starts_with("[Direction from the host"));
    }
}
