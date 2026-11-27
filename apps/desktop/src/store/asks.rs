//! Which asks the card over the composer shows. In a project's room only that project's; Main
//! shows them all. A task waiting on the user with a question or a thing to do, the newest
//! asked first; Later ones (a month out, or in a paused project) wait quietly on Needs you;
//! one acted on here is gone at once; one whose action names another task stands for that one
//! and is answered there. The user can put away every card showing ([`AskCards::keys`]); a new
//! question brings its card back.

use std::collections::{BTreeSet, HashSet};
use std::fmt;

use emberhut_api::ids::{RoomId, TaskId};
use emberhut_api::state::State;
use emberhut_api::tasks::{Decision, Issue, TaskStatus};
use serde::{Deserialize, Serialize};

use super::ask_text::AskText;
use super::lookup;
use super::needs::Needs;
use crate::prose::copy;
use crate::wire::TASK_KEY_PREFIX;

/// What the ask card shows over one state, in one room.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AskCards<'a> {
    /// The newest ask not put away: the card.
    pub(crate) first: Option<AskShown<'a>>,
    /// How many more asks wait behind it (the card's "· 3 more").
    pub(crate) more: usize,
    /// In a project's room, the asks waiting in every other room (its "2 more in other rooms").
    pub(crate) elsewhere: u32,
    /// The keys of every ask that waits here, put away or not: Hide puts them all away.
    pub(crate) keys: Vec<AskKey>,
}

impl<'a> AskCards<'a> {
    /// The cards over `state` (its lists as `needs` reads them) in `room`, the keys in
    /// `put_away` left out.
    #[must_use]
    pub(crate) fn new(
        state: &'a State,
        needs: &Needs<'a>,
        room: RoomId,
        put_away: &BTreeSet<AskKey>,
    ) -> Self {
        let here = state.rooms.iter().find(|row| row.id == room);
        let project = here.and_then(|row| row.project);
        let later: HashSet<TaskId> = state
            .working
            .iter()
            .filter(|row| row.needs_you() && needs.is_later(row))
            .filter_map(|row| row.task)
            .collect();
        let asks: Vec<&Issue> = asks_waiting(state)
            .filter(|task| {
                task.parent.is_none()
                    && project.is_none_or(|project| task.pid == Some(project))
                    && !later.contains(&task.id)
                    && needs.waits_on_you(Some(task.id))
            })
            .collect();
        let mut waiting: Vec<&Issue> = asks
            .iter()
            .copied()
            .filter(|task| !put_away.contains(&AskKey::of(task)))
            .collect();
        waiting.sort_by(|a, b| (asked(b), b.id).cmp(&(asked(a), a.id)));
        let elsewhere = if project.is_none() {
            0
        } else {
            state
                .rooms
                .iter()
                .filter(|row| row.id != room)
                .map(|row| row.needs)
                .sum()
        };
        Self {
            first: waiting.first().map(|&listed| AskShown {
                listed,
                ask: needs
                    .ask_of(listed)
                    .and_then(|id| lookup::issue(state, id))
                    .unwrap_or(listed),
            }),
            more: waiting.len().saturating_sub(1),
            elsewhere,
            keys: asks.iter().map(|task| AskKey::of(task)).collect(),
        }
    }
}

/// The tasks that wait on the user with an ask (a question or a thing to do), in any room.
fn asks_waiting(state: &State) -> impl Iterator<Item = &Issue> {
    state.issues.iter().filter(|task| {
        task.status == TaskStatus::Waiting && (task.decision.is_some() || task.action.is_some())
    })
}

/// The keys of every ask waiting in `state`, in any room: what put-away keys are worth keeping.
#[must_use]
pub(crate) fn waiting_keys(state: &State) -> BTreeSet<AskKey> {
    asks_waiting(state).map(AskKey::of).collect()
}

/// Which ask a card stands for, as Hide puts it away: the task, and when it asked (or, with no
/// time, what it asks). The assistant checking an ask again (fresh facts, the same ask) keeps
/// it put away; a new ask brings it back.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct AskKey(String);

impl AskKey {
    /// The key of `task`'s ask: `12@2026-11-25T08:00:00Z` by the decision's time; without one,
    /// `12#` and a hash of its question ([`stable_hash`]); a thing to do by the task's last
    /// change.
    #[must_use]
    pub(crate) fn of(task: &Issue) -> Self {
        let id = task.id;
        Self(match &task.decision {
            Some(decision) => match &decision.asked {
                Some(asked) => format!("{id}@{}", asked.as_str()),
                None => format!("{id}#{:016x}", stable_hash(&decision.question)),
            },
            None => format!("{id}@{}", task.updated.as_str()),
        })
    }
}

impl fmt::Display for AskKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The 64-bit FNV-1a hash of `text`: the same in every build and on every platform, as a key
/// kept on disk needs (the standard library's hasher is free to change between releases).
#[must_use]
fn stable_hash(text: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    text.bytes().fold(OFFSET, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

/// When a task asked: its decision's time, else its last change.
fn asked(task: &Issue) -> &str {
    task.decision
        .as_ref()
        .and_then(|decision| decision.asked.as_ref())
        .unwrap_or(&task.updated)
        .as_str()
}

/// The ask on the card: the task listed, and the ask it stands for (itself, or the task its
/// action names), which the card answers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AskShown<'a> {
    /// The task in the list.
    pub(crate) listed: &'a Issue,
    /// The task answered.
    pub(crate) ask: &'a Issue,
}

/// What the card's Open button says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenButton {
    /// "Open" or "Read the draft".
    pub(crate) label: String,
}

/// One of a decision's options as a card on the ask card, with its answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OptionCard {
    /// Its answer: its button, and what is sent back.
    pub(crate) label: String,
    /// Its name.
    pub(crate) title: String,
    /// A sentence or two about it.
    pub(crate) summary: Option<String>,
    /// A short number to compare by.
    pub(crate) badge: Option<String>,
    /// The facts to compare, the same keys on every card, in order.
    pub(crate) fields: Vec<(String, String)>,
    /// The assistant's reason, on the option it picks.
    pub(crate) why: Option<String>,
    /// Whether it is the option the assistant picks.
    pub(crate) picked: bool,
}

impl AskShown<'_> {
    /// The line over the card: the task's key, then "needs you".
    #[must_use]
    pub(crate) fn eyebrow(&self) -> String {
        let key = if self.ask.key.is_empty() {
            format!("{TASK_KEY_PREFIX}{}", self.ask.id)
        } else {
            self.ask.key.clone()
        };
        format!("{key} needs you")
    }

    /// What it is about: the task's title, or both when the listed task stands for another.
    #[must_use]
    pub(crate) fn title(&self) -> String {
        if self.listed.id == self.ask.id {
            self.ask.title.clone()
        } else {
            format!("{} · {}", self.listed.title, self.ask.title)
        }
    }

    /// The question in a line or two: the thing to do, else the question without its draft,
    /// else the title.
    #[must_use]
    pub(crate) fn question(&self) -> String {
        self.ask
            .action
            .clone()
            .or_else(|| self.text().map(|text| text.summary))
            .filter(|question| !question.trim().is_empty())
            .unwrap_or_else(|| self.ask.title.clone())
    }

    /// The answers for plain buttons, as sent back: every option, or when the options come as
    /// cards ([`AskShown::option_cards`]) those that aren't cards ("More ideas"); none when
    /// there is no question (only a thing to do: [`AskShown::is_a_thing_to_do`]).
    #[must_use]
    pub(crate) fn options(&self) -> Vec<String> {
        let Some(decision) = self.decision() else {
            return Vec::new();
        };
        match decision
            .choices
            .as_deref()
            .filter(|_| decision.card_count() > 0)
        {
            Some(choices) => choices
                .iter()
                .filter(|choice| !choice.is_card())
                .map(|choice| choice.label.clone())
                .collect(),
            None => decision.options.clone(),
        }
    }

    /// The options that come as cards to compare, each with its answer, in order.
    #[must_use]
    pub(crate) fn option_cards(&self) -> Vec<OptionCard> {
        let Some(decision) = self.decision() else {
            return Vec::new();
        };
        let pick = decision
            .recommend
            .and_then(|pick| usize::try_from(pick).ok());
        decision
            .choices
            .iter()
            .flatten()
            .enumerate()
            .filter_map(|(at, choice)| {
                let title = choice.title.clone()?;
                Some(OptionCard {
                    label: choice.label.clone(),
                    title,
                    summary: choice.summary.clone(),
                    badge: choice.badge.clone(),
                    fields: choice
                        .fields
                        .iter()
                        .flatten()
                        .map(|field| (field.key.clone(), field.value.clone()))
                        .collect(),
                    why: choice.why.clone(),
                    picked: pick == Some(at),
                })
            })
            .collect()
    }

    /// Only a thing to do (a login, a call): the card's one answer is "I did it" (`Done`).
    #[must_use]
    pub(crate) fn is_a_thing_to_do(&self) -> bool {
        self.decision().is_none()
    }

    /// The Open button.
    #[must_use]
    pub(crate) fn open_button(&self) -> OpenButton {
        let label = if self.text().is_some_and(|text| text.draft.is_some()) {
            "Read the draft"
        } else {
            "Open"
        };
        OpenButton {
            label: label.to_owned(),
        }
    }

    /// What answering `choice` puts on the clipboard: an answer that sends ("Send it") copies
    /// the ask's draft, ready to paste where it goes (the assistant never sends it itself).
    #[must_use]
    pub(crate) fn copied_for(&self, choice: &str) -> Option<String> {
        copied_draft(choice, self.decision()?)
    }

    fn decision(&self) -> Option<&Decision> {
        self.ask.decision.as_ref()
    }

    fn text(&self) -> Option<AskText> {
        self.decision().map(AskText::of)
    }
}

/// The answers that send the ask's draft, as the assistant words them (any case).
const SENDING_ANSWERS: &[&str] = &["send", "send it", "send as is", "send as-is"];

/// What answering `choice` to `decision` copies: its draft for an answer that sends
/// ([`SENDING_ANSWERS`]), exact when it came in its own field, else ready to paste into a chat
/// app. Copy it only once the answer was taken.
#[must_use]
pub(crate) fn copied_draft(choice: &str, decision: &Decision) -> Option<String> {
    let choice = choice.trim();
    if !SENDING_ANSWERS
        .iter()
        .any(|sending| choice.eq_ignore_ascii_case(sending))
    {
        return None;
    }
    let text = AskText::of(decision);
    let draft = text.draft?;
    Some(if text.plain {
        draft
    } else {
        copy::for_pasting(&draft)
    })
}

/// An answer as its button says it: `Done` reads "I did it".
#[must_use]
pub(crate) fn answer_label(option: &str) -> String {
    if option.eq_ignore_ascii_case("done") {
        "I did it".to_owned()
    } else {
        option.to_owned()
    }
}

/// Main's line for the other rooms: how many asks wait in them.
#[must_use]
pub(crate) fn in_other_rooms(state: &State) -> u32 {
    state
        .rooms
        .iter()
        .filter(|row| !row.is_main())
        .map(|row| row.needs)
        .sum()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::fixtures::{issue, room, state, waiting};

    const TODAY: jiff::civil::Date = jiff::civil::date(2026, 11, 25);

    fn ask(id: i64, pid: &Value, asked: &str) -> Value {
        let issue = issue(
            id,
            "waiting",
            json!({"pid": pid, "decision": {"question": format!("Question {id}?"),
                "options": ["Yes", "No"], "asked": asked}}),
        );
        serde_json::to_value(issue).unwrap()
    }

    fn cards_in(lists: &State, room_id: RoomId, put_away: &BTreeSet<AskKey>) -> Vec<i64> {
        let gone = HashSet::new();
        let needs = Needs::new(lists, &gone, TODAY);
        let cards = AskCards::new(lists, &needs, room_id, put_away);
        let mut ids: Vec<i64> = cards.first.iter().map(|s| s.ask.id.get()).collect();
        ids.push(i64::try_from(cards.more).unwrap());
        ids
    }

    fn lists() -> State {
        state(json!({
            "rooms": [room(1, json!({})), room(2, json!({"needs": 2})), room(3, json!({"needs": 1}))],
            "issues": [
                ask(10, &json!(2), "2026-11-24T08:00:00Z"),
                ask(11, &json!(2), "2026-11-25T08:00:00Z"),
                ask(12, &json!(3), "2026-11-25T09:00:00Z"),
                ask(13, &json!(2), "2026-11-25T10:00:00Z"),
                issue(14, "working", json!({"pid": 2, "action": "Log in"})),
            ],
            "working": [waiting(13, json!({"pid": 2, "decideBy": "2027-03-01"}))],
        }))
    }

    #[test]
    fn a_projects_room_shows_its_newest_ask_main_shows_all() {
        let lists = lists();
        let none = BTreeSet::new();
        assert_eq!(
            cards_in(&lists, RoomId::new(2), &none),
            [11, 1],
            "13 is Later, 12 is another project's, 14 isn't waiting"
        );
        assert_eq!(cards_in(&lists, RoomId::MAIN, &none), [12, 2]);
        let gone = HashSet::new();
        let needs = Needs::new(&lists, &gone, TODAY);
        let cards = AskCards::new(&lists, &needs, RoomId::new(2), &none);
        assert_eq!(cards.elsewhere, 1, "room 3's ask");
        let main = AskCards::new(&lists, &needs, RoomId::MAIN, &none);
        assert_eq!(main.elsewhere, 0, "Main counts no other rooms");
        assert_eq!(in_other_rooms(&lists), 3);
    }

    #[test]
    fn the_waiting_keys_are_every_rooms() {
        let keys = waiting_keys(&lists());
        let words: Vec<String> = keys.iter().map(ToString::to_string).collect();
        assert_eq!(
            words,
            [
                "10@2026-11-24T08:00:00Z",
                "11@2026-11-25T08:00:00Z",
                "12@2026-11-25T09:00:00Z",
                "13@2026-11-25T10:00:00Z",
            ],
            "14 isn't waiting"
        );
    }

    #[test]
    fn an_answered_ask_is_gone_at_once() {
        let lists = lists();
        let gone = HashSet::from([TaskId::new(11)]);
        let needs = Needs::new(&lists, &gone, TODAY);
        let cards = AskCards::new(&lists, &needs, RoomId::new(2), &BTreeSet::new());
        assert_eq!(cards.first.map(|s| s.ask.id), Some(TaskId::new(10)));
        assert_eq!(cards.more, 0);
    }

    #[test]
    fn put_away_asks_stay_away_until_asked_again() {
        let mut lists = lists();
        let gone = HashSet::new();
        let needs = Needs::new(&lists, &gone, TODAY);
        let keys = AskCards::new(&lists, &needs, RoomId::new(2), &BTreeSet::new()).keys;
        let words: Vec<String> = keys.iter().map(ToString::to_string).collect();
        assert_eq!(
            words,
            ["10@2026-11-24T08:00:00Z", "11@2026-11-25T08:00:00Z"]
        );
        let put_away: BTreeSet<AskKey> = keys.into_iter().collect();
        assert_eq!(cards_in(&lists, RoomId::new(2), &put_away), [0]);
        // Asked again: a new time, the card is back.
        if let Some(decision) = lists.issues[0].decision.as_mut() {
            decision.asked = Some(serde_json::from_value(json!("2026-11-25T11:00:00Z")).unwrap());
        }
        assert_eq!(cards_in(&lists, RoomId::new(2), &put_away), [10, 0]);
    }

    #[test]
    fn an_ask_with_no_time_is_known_by_its_question() {
        let asking = |question: &str| {
            issue(
                40,
                "waiting",
                json!({"decision": {"question": question, "options": ["Yes"]}}),
            )
        };
        let key = AskKey::of(&asking("Ship the blue one?"));
        assert_eq!(key, AskKey::of(&asking("Ship the blue one?")));
        assert_ne!(
            key,
            AskKey::of(&asking("Ship the red one?")),
            "the same length, a new ask"
        );
        assert!(key.to_string().starts_with("40#"), "{key}");
        assert_eq!(stable_hash(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(stable_hash("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn only_an_answer_that_sends_copies_the_draft() {
        let decision: Decision = serde_json::from_value(json!({
            "question": "Send this?", "options": ["Send it"], "draft": "Hello there"
        }))
        .unwrap();
        for sending in ["Send", "send it", "SEND AS IS", " Send it "] {
            assert_eq!(
                copied_draft(sending, &decision).as_deref(),
                Some("Hello there"),
                "{sending}"
            );
        }
        for other in ["Sending later", "Send nothing", "Don't send", "Not now"] {
            assert_eq!(copied_draft(other, &decision), None, "{other}");
        }
    }

    #[test]
    fn a_task_standing_for_another_is_answered_there() {
        let lists = state(json!({
            "rooms": [room(1, json!({}))],
            "issues": [
                issue(20, "waiting", json!({"action": format!("One yes on {TASK_KEY_PREFIX}21 for the setup")})),
                issue(21, "waiting", json!({"parent": 20, "decision": {"question": "Go?", "options": ["Approve"]}})),
            ],
        }));
        let gone = HashSet::new();
        let needs = Needs::new(&lists, &gone, TODAY);
        let cards = AskCards::new(&lists, &needs, RoomId::MAIN, &BTreeSet::new());
        let shown = cards.first.unwrap();
        assert_eq!((shown.listed.id.get(), shown.ask.id.get()), (20, 21));
        assert_eq!(shown.eyebrow(), "T-21 needs you");
        assert_eq!(shown.title(), "Task 20 · Task 21");
        assert_eq!(shown.question(), "Go?");
        assert_eq!(shown.options(), ["Approve"]);
        assert!(!shown.is_a_thing_to_do());
    }

    #[test]
    fn the_card_words_its_buttons() {
        let thing = issue(30, "waiting", json!({"action": "Log in to the bank"}));
        let shown = AskShown {
            listed: &thing,
            ask: &thing,
        };
        assert!(shown.is_a_thing_to_do());
        assert_eq!(shown.question(), "Log in to the bank");
        assert_eq!(shown.open_button().label, "Open");

        let draft = issue(
            31,
            "waiting",
            json!({"decision": {"question": "Send this?\n\"Hi, the ads are ready and they go out on Sunday, see you.\"",
                "options": ["Send it", "Not now"]}}),
        );
        let shown = AskShown {
            listed: &draft,
            ask: &draft,
        };
        assert_eq!(shown.question(), "Send this?");
        assert_eq!(shown.open_button().label, "Read the draft");
        assert_eq!(
            shown.copied_for("Send it").as_deref(),
            Some("Hi, the ads are ready and they go out on Sunday, see you.")
        );
        assert_eq!(shown.copied_for("Not now"), None);

        let cards = issue(
            32,
            "waiting",
            json!({"decision": {"question": "Which?", "options": ["A", "B", "More"], "recommend": 1,
                "choices": [{"label": "A", "title": "Plan A", "fields": [{"k": "Cost", "v": "100"}]},
                    {"label": "B", "title": "Plan B", "summary": "Bigger.", "badge": "8/10",
                        "fields": [{"k": "Cost", "v": "400"}], "why": "It lasts"},
                    {"label": "More"}]}}),
        );
        let shown = AskShown {
            listed: &cards,
            ask: &cards,
        };
        assert_eq!(shown.open_button().label, "Open");
        assert_eq!(
            shown.options(),
            ["More"],
            "the plain answer beside the cards"
        );
        assert_eq!(
            shown.option_cards(),
            [
                OptionCard {
                    label: "A".into(),
                    title: "Plan A".into(),
                    summary: None,
                    badge: None,
                    fields: vec![("Cost".into(), "100".into())],
                    why: None,
                    picked: false,
                },
                OptionCard {
                    label: "B".into(),
                    title: "Plan B".into(),
                    summary: Some("Bigger.".into()),
                    badge: Some("8/10".into()),
                    fields: vec![("Cost".into(), "400".into())],
                    why: Some("It lasts".into()),
                    picked: true,
                },
            ]
        );
        assert_eq!(
            AskShown {
                listed: &draft,
                ask: &draft
            }
            .option_cards(),
            [],
            "no cards: plain answers only"
        );
        assert_eq!(answer_label("done"), "I did it");
        assert_eq!(answer_label("Approve"), "Approve");
    }
}
