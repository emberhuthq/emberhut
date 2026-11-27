//! A decision laid out to read in a glance and answer, on its task's page: why it is asked now
//! and by when, what the user needs to know (the assistant's few bullets, else the question
//! short), when it was asked and last checked, the message drafted to send, each option as a
//! card to compare (or as a row with what it does), the assistant's pick marked. Worded from a
//! `now` passed in, so it is tested without a clock.

use emberhut_api::choices::{Choice, ChoiceField};
use emberhut_api::ids::ArtifactId;
use emberhut_api::tasks::{Artifact, Decision, Issue};
use jiff::{Timestamp, ZonedDateTime};

use super::ask_text::AskText;
use super::asks::answer_label;
use super::lookup;
use super::tasks::clock;
use crate::brand::APP_NAME;

/// How long an ask can go unchecked before its page warns it may be outdated.
const STALE_AFTER_SECONDS: i64 = 12 * 3600;

/// How many of a card's facts show before More.
const FIELDS_SHOWN: usize = 4;

/// A fact this long, or a summary this long, waits behind More.
const LONG_FIELD: usize = 140;
const LONG_SUMMARY: usize = 220;

/// The decision as its page lays it out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DecisionPage<'a> {
    /// Why it is asked now.
    pub(crate) context: Option<&'a str>,
    /// `Decide by Fri 28 Nov · 3 options to compare`.
    pub(crate) by_line: Option<String>,
    /// The assistant's few bullets.
    pub(crate) brief: &'a [String],
    /// The question without its draft.
    pub(crate) summary: String,
    /// When it was asked and checked.
    pub(crate) asked: AskedWhen,
    /// The message drafted to send, if it comes with one.
    pub(crate) draft: Option<Draft>,
    /// What the question says after the draft.
    pub(crate) after: Option<String>,
    /// The options.
    pub(crate) options: Options<'a>,
}

/// When an ask was asked and last made sure of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AskedWhen {
    /// `Asked 2 hours ago`.
    pub(crate) asked: Option<String>,
    /// `Emberhut re-checked it 1 hour ago`, when it was checked after it was asked.
    pub(crate) checked: Option<String>,
    /// Not checked in over 12 hours: it may be outdated.
    pub(crate) stale: bool,
}

/// The stale warning's words.
#[must_use]
pub(crate) fn stale_warning() -> String {
    format!(
        "Not re-checked in over 12 hours, so it may be outdated. {APP_NAME} re-checks it on its own."
    )
}

/// The message drafted to send, as its own block with Copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Draft {
    /// Its title: the line that named it, else `Message to send`.
    pub(crate) label: String,
    /// The message.
    pub(crate) text: String,
    /// It came in its own field: copied exactly, else as a message to paste.
    pub(crate) plain: bool,
}

/// How the options show.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Options<'a> {
    /// No options: only the user's own words answer.
    None,
    /// A row each, with what it does under it.
    Rows(Vec<OptionRow>),
    /// Cards to compare side by side, and the plain answers beside them as rows under them.
    Cards {
        /// The cards.
        cards: Vec<OptionCard<'a>>,
        /// The plain answers.
        rows: Vec<OptionRow>,
    },
}

/// An answer as a row: what it is, what it does, the assistant's pick marked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OptionRow {
    /// The answer sent back.
    pub(crate) choice: String,
    /// As it reads (`Done` reads `I did it`).
    pub(crate) text: String,
    /// What it does.
    pub(crate) note: Option<String>,
    /// The assistant's pick.
    pub(crate) pick: bool,
}

/// An option as a card.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OptionCard<'a> {
    /// Which option it is (its More key).
    pub(crate) index: usize,
    /// The option as the wire has it.
    pub(crate) choice: &'a Choice,
    /// Its name.
    pub(crate) title: &'a str,
    /// Its button: the answer as it reads.
    pub(crate) button: String,
    /// The assistant's pick.
    pub(crate) pick: bool,
    /// Its pictures and videos, as the task has them.
    pub(crate) media: Vec<&'a Artifact>,
    /// It has more than shows folded: it offers More.
    pub(crate) long: bool,
}

impl<'a> OptionCard<'a> {
    /// Its facts as shown: the first four, all of them with More open.
    #[must_use]
    pub(crate) fn fields(&self, more: bool) -> &'a [ChoiceField] {
        let fields = self.choice.fields.as_deref().unwrap_or_default();
        if more {
            fields
        } else {
            &fields[..fields.len().min(FIELDS_SHOWN)]
        }
    }
}

/// Lays out `task`'s decision, with the artifacts its cards show from `artifacts`.
#[must_use]
pub(crate) fn page<'a>(
    task: &'a Issue,
    decision: &'a Decision,
    artifacts: &'a [Artifact],
    now: &ZonedDateTime,
) -> DecisionPage<'a> {
    let text = AskText::of(decision);
    let cards = decision.card_count();
    let by = task
        .decide_by
        .as_ref()
        .and_then(|day| lookup::civil_day(day.as_str()))
        .map(|day| format!("Decide by {}", day.strftime("%a %-d %b")));
    let compare = (cards > 1).then(|| format!("{cards} options to compare"));
    let by_line = [by, compare].into_iter().flatten().collect::<Vec<_>>();
    DecisionPage {
        context: decision
            .context
            .as_deref()
            .filter(|context| !context.trim().is_empty()),
        by_line: (!by_line.is_empty()).then(|| by_line.join(" · ")),
        brief: decision.brief.as_deref().unwrap_or_default(),
        summary: text.summary,
        asked: asked_when(task, decision, now.timestamp()),
        draft: text.draft.map(|draft| Draft {
            label: text.label.unwrap_or_else(|| "Message to send".to_owned()),
            text: draft,
            plain: text.plain,
        }),
        after: (!text.after.is_empty()).then_some(text.after),
        options: options(decision, artifacts),
    }
}

/// When it was asked (its decision's time, else the task's last change), and checked.
fn asked_when(task: &Issue, decision: &Decision, now: Timestamp) -> AskedWhen {
    let asked = decision.asked.as_ref().unwrap_or(&task.updated);
    let checked = decision
        .checked
        .as_ref()
        .or(decision.asked.as_ref())
        .unwrap_or(&task.updated);
    let asked_at = clock::instant(asked);
    let checked_at = clock::instant(checked);
    AskedWhen {
        asked: asked_at.map(|at| format!("Asked {}", clock::relative(at, now))),
        checked: checked_at
            .filter(|_| decision.checked.is_some() && decision.checked != decision.asked)
            .map(|at| format!("{APP_NAME} re-checked it {}", clock::relative(at, now))),
        stale: checked_at.is_some_and(|at| now.as_second() - at.as_second() > STALE_AFTER_SECONDS),
    }
}

/// The options: cards when any is one, else rows.
fn options<'a>(decision: &'a Decision, artifacts: &'a [Artifact]) -> Options<'a> {
    let note = |i: usize| {
        decision
            .notes
            .as_ref()
            .and_then(|notes| notes.get(i))
            .filter(|note| !note.trim().is_empty())
            .cloned()
    };
    let pick = |i: usize| decision.recommend.and_then(|r| usize::try_from(r).ok()) == Some(i);
    let row = |i: usize, choice: &str| OptionRow {
        choice: choice.to_owned(),
        text: answer_label(choice),
        note: note(i),
        pick: pick(i),
    };
    let choices = decision.choices.as_deref().unwrap_or_default();
    if decision.card_count() == 0 {
        if decision.options.is_empty() {
            return Options::None;
        }
        return Options::Rows(
            decision
                .options
                .iter()
                .enumerate()
                .map(|(i, option)| row(i, option))
                .collect(),
        );
    }
    let mut cards = Vec::new();
    let mut rows = Vec::new();
    for (i, choice) in choices.iter().enumerate() {
        let Some(title) = choice.title.as_deref() else {
            rows.push(row(i, &choice.label));
            continue;
        };
        let media = choice
            .media
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|id| artifacts.iter().find(|artifact| artifact.id == *id))
            .collect();
        cards.push(OptionCard {
            index: i,
            choice,
            title,
            button: answer_label(&choice.label),
            pick: pick(i),
            media,
            long: is_long(choice),
        });
    }
    Options::Cards { cards, rows }
}

/// Whether a card has more than shows folded: more than four facts, a long version, a long
/// fact or a long summary.
fn is_long(choice: &Choice) -> bool {
    let fields = choice.fields.as_deref().unwrap_or_default();
    fields.len() > FIELDS_SHOWN
        || choice
            .more
            .as_deref()
            .is_some_and(|more| !more.trim().is_empty())
        || fields
            .iter()
            .any(|field| field.value.chars().count() > LONG_FIELD)
        || choice
            .summary
            .as_deref()
            .is_some_and(|summary| summary.chars().count() > LONG_SUMMARY)
}

/// The artifacts a decision's cards show: the page lists them on their cards, not again under
/// Files and results.
#[must_use]
pub(crate) fn card_media(task: &Issue) -> Vec<ArtifactId> {
    task.decision
        .as_ref()
        .and_then(|decision| decision.choices.as_deref())
        .unwrap_or_default()
        .iter()
        .flat_map(|choice| choice.media.as_deref().unwrap_or_default())
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use jiff::tz::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::fixtures::issue;

    fn now() -> ZonedDateTime {
        "2026-11-25T12:00:00Z"
            .parse::<Timestamp>()
            .unwrap()
            .to_zoned_datetime(TimeZone::UTC)
    }

    fn asking(decision: serde_json::Value, extra: serde_json::Value) -> Issue {
        let mut fields = json!({"decideBy": "2026-11-28", "updated": "2026-11-25T08:00:00Z"});
        fields["decision"] = decision;
        if let (Some(fields), serde_json::Value::Object(extra)) = (fields.as_object_mut(), extra) {
            fields.extend(extra);
        }
        issue(1, "waiting", fields)
    }

    #[test]
    fn the_context_says_why_now_by_when_and_how_many_to_compare() {
        let task = asking(
            json!({"question": "Which?", "options": ["A", "B", "More"], "context": "The offer ends Friday.",
                   "choices": [{"label": "A", "title": "Plan A"}, {"label": "B", "title": "Plan B"}, {"label": "More"}]}),
            json!({}),
        );
        let decision = task.decision.as_ref().unwrap();
        let page = page(&task, decision, &[], &now());
        assert_eq!(page.context, Some("The offer ends Friday."));
        assert_eq!(
            page.by_line.as_deref(),
            Some("Decide by Sat 28 Nov · 2 options to compare")
        );
        let Options::Cards { cards, rows } = page.options else {
            panic!("cards")
        };
        assert_eq!(cards.len(), 2);
        assert_eq!(
            rows[0].choice, "More",
            "a plain answer is a row under the cards"
        );
    }

    #[test]
    fn asked_and_checked_times_and_the_stale_warning() {
        let task = asking(
            json!({"question": "Go?", "options": ["Yes"], "asked": "2026-11-24T10:00:00Z",
                   "checked": "2026-11-24T20:00:00Z"}),
            json!({}),
        );
        let asked = page(&task, task.decision.as_ref().unwrap(), &[], &now()).asked;
        assert_eq!(asked.asked.as_deref(), Some("Asked yesterday"));
        assert_eq!(
            asked.checked.as_deref(),
            Some("Emberhut re-checked it 16 hours ago")
        );
        assert!(asked.stale);
        let fresh = asking(json!({"question": "Go?", "options": ["Yes"]}), json!({}));
        let asked = page(&fresh, fresh.decision.as_ref().unwrap(), &[], &now()).asked;
        assert_eq!(asked.asked.as_deref(), Some("Asked 4 hours ago"));
        assert_eq!(asked.checked, None, "never re-checked");
        assert!(!asked.stale);
    }

    #[test]
    fn a_draft_is_its_own_block_and_what_follows_it_too() {
        let question = "Reply to the bakery?\nDraft reply to the bakery:\n\u{201c}Hi, the ads are ready and go out on Sunday morning, as we said.\u{201d}\nI close it after.";
        let task = asking(
            json!({"question": question, "options": ["Send it", "Leave it"]}),
            json!({}),
        );
        let page = page(&task, task.decision.as_ref().unwrap(), &[], &now());
        assert_eq!(page.summary, "Reply to the bakery?");
        let draft = page.draft.unwrap();
        assert_eq!(draft.label, "Draft reply to the bakery");
        assert!(draft.text.starts_with("Hi, the ads"));
        assert!(!draft.plain);
        assert_eq!(page.after.as_deref(), Some("I close it after."));
        let own = asking(
            json!({"question": "Send?", "draft": "Exactly this", "options": ["Send"]}),
            json!({}),
        );
        let draft = super::page(&own, own.decision.as_ref().unwrap(), &[], &now())
            .draft
            .unwrap();
        assert_eq!(
            (draft.label.as_str(), draft.plain),
            ("Message to send", true)
        );
    }

    #[test]
    fn rows_carry_their_notes_and_the_pick_and_done_reads_i_did_it() {
        let task = asking(
            json!({"question": "Done?", "options": ["Done", "Not yet"], "notes": ["It closes the task", " "],
                   "recommend": 0}),
            json!({}),
        );
        let Options::Rows(rows) = page(&task, task.decision.as_ref().unwrap(), &[], &now()).options
        else {
            panic!("rows")
        };
        assert_eq!(rows[0].text, "I did it");
        assert_eq!(rows[0].choice, "Done");
        assert_eq!(rows[0].note.as_deref(), Some("It closes the task"));
        assert!(rows[0].pick && !rows[1].pick);
        assert_eq!(rows[1].note, None, "a blank note isn't shown");
    }

    #[test]
    fn a_card_folds_what_is_long_and_shows_its_own_pictures() {
        let artifact: Artifact = serde_json::from_value(json!({"id": 5, "kind": "image", "caption": null,
            "at": "2026-11-25T08:00:00Z", "name": "a.png", "url": "/api/artifacts/5", "thumb": "/t/5"}))
        .unwrap();
        let fields: Vec<_> = (0..5)
            .map(|i| json!({"k": format!("K{i}"), "v": "v"}))
            .collect();
        let task = asking(
            json!({"question": "Which?", "options": ["A", "B"], "recommend": 1,
                   "choices": [{"label": "A", "title": "Plan A", "fields": fields, "media": [5, 9]},
                               {"label": "B", "title": "Plan B", "why": "Cheaper"}]}),
            json!({}),
        );
        let artifacts = [artifact];
        let Options::Cards { cards, .. } =
            page(&task, task.decision.as_ref().unwrap(), &artifacts, &now()).options
        else {
            panic!("cards")
        };
        assert!(cards[0].long && !cards[1].long);
        assert_eq!(cards[0].fields(false).len(), 4);
        assert_eq!(cards[0].fields(true).len(), 5);
        assert_eq!(
            cards[0].media.len(),
            1,
            "media the task doesn't have are left out"
        );
        assert!(cards[1].pick && !cards[0].pick);
        assert_eq!(card_media(&task), [ArtifactId::new(5), ArtifactId::new(9)]);
    }
}
