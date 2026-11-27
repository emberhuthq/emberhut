//! What a card on Needs you says: the task, the thing to do in bold (or the ask it stands for),
//! what it costs (a quick yes, a read and a pick, something only the user can do), its project,
//! when it was asked or that it may be outdated, its decide-by chip, and the answers it offers
//! right on the card. Worded from a `now` passed in, so it is tested without a clock.

use emberhut_api::ids::TaskId;
use emberhut_api::state::State;
use emberhut_api::tasks::{Artifact, AskKind};
use emberhut_api::work::WorkRow;
use jiff::Timestamp;
use jiff::civil::Date;

use super::ask_text::AskText;
use super::lookup;
use super::needs::Needs;
use super::tasks::clock;
use crate::brand::APP_NAME;

/// How long an ask can go unchecked before its card says it may be outdated.
const STALE_AFTER_SECONDS: i64 = 12 * 3600;

/// The answer that says a thing to do is done.
pub(crate) const DONE: &str = "Done";

/// A card's content.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NeedCard<'a> {
    /// The task it opens; none for a thread with no task.
    pub(crate) task: Option<TaskId>,
    /// The task's title.
    pub(crate) title: &'a str,
    /// What to do, in bold: the action, or the ask the task stands for.
    pub(crate) action: String,
    /// A message to send comes with it: read it (on the task's page) first.
    pub(crate) draft: bool,
    /// What it costs: `Quick yes`, `Read & decide`, `Only you`.
    pub(crate) kind: &'static str,
    /// Its project's name, where the cards aren't grouped by it.
    pub(crate) project: Option<&'a str>,
    /// When it was asked, or that it may be outdated.
    pub(crate) asked: Asked,
    /// When it has to be decided by.
    pub(crate) chip: Option<DecideChip>,
    /// The answers on the card, when it offers them.
    pub(crate) answers: Option<CardAnswers>,
    /// Its newest picture's thumbnail.
    pub(crate) thumb: Option<&'a str>,
    /// An X post to reply to: its replies show on the card instead of answers.
    pub(crate) reply: Option<&'a Artifact>,
}

/// When an ask was asked, as its card says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Asked {
    /// `asked 3 hours ago`.
    Ago(String),
    /// Not checked in over 12 hours: `may be outdated`, in orange.
    Outdated,
    /// No time to say.
    Unknown,
}

/// How a decide-by chip is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChipTone {
    /// Late: red.
    Late,
    /// Someone else's deadline: orange, with a flag.
    Hard,
    /// Today: the accent.
    Today,
    /// Later on: muted.
    Plain,
}

/// The day an ask has to be decided by, on its card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DecideChip {
    /// The words: `2d late`, `Today`, `by Fri`; with tiers `Overdue · 2d`, `This week · Fri`,
    /// `This month · 3 Dec`.
    pub(crate) text: String,
    /// How it is drawn.
    pub(crate) tone: ChipTone,
    /// Someone else's deadline: a flag before the words.
    pub(crate) hard: bool,
}

/// The answers on a card: the first prominent; one more beside it, or the rest in a menu; or
/// only "I did it" for a thing to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardAnswers {
    /// The task they answer (the listed one, or the one its action names).
    pub(crate) to: TaskId,
    /// The answers as sent back, the first prominent.
    pub(crate) options: Vec<String>,
}

impl CardAnswers {
    /// The first answer, the prominent one.
    #[must_use]
    pub(crate) fn first(&self) -> Option<&str> {
        self.options.first().map(String::as_str)
    }

    /// A second answer shown beside the first: only when there are exactly two.
    #[must_use]
    pub(crate) fn second(&self) -> Option<&str> {
        (self.options.len() == 2).then(|| self.options[1].as_str())
    }

    /// The answers in the `…` menu: the rest, when there are three or more.
    #[must_use]
    pub(crate) fn menu(&self) -> &[String] {
        if self.options.len() > 2 {
            &self.options[1..]
        } else {
            &[]
        }
    }
}

/// What a card is drawn from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CardContext<'a> {
    /// The state.
    pub(crate) state: &'a State,
    /// Its lists.
    pub(crate) needs: &'a Needs<'a>,
    /// Now.
    pub(crate) now: Timestamp,
    /// Whether the card names its project (its group doesn't).
    pub(crate) project: bool,
    /// Whether the chip names its group (Needs you by project, not Later).
    pub(crate) tiers: bool,
}

impl<'a> CardContext<'a> {
    /// The card for `row`.
    #[must_use]
    pub(crate) fn card(&self, row: &'a WorkRow) -> NeedCard<'a> {
        let today = self.needs.today();
        let stale = is_stale(row, self.now);
        let tiers = self.tiers && !self.needs.is_later(row);
        let reply = row.reply.as_ref().filter(|artifact| artifact.x.is_some());
        NeedCard {
            task: row.task,
            title: &row.title,
            action: self.action(row),
            draft: row.draft,
            kind: kind_name(row),
            project: self
                .project
                .then(|| lookup::project_name(self.state, row.pid))
                .flatten(),
            asked: if stale {
                Asked::Outdated
            } else {
                lookup::asked(row)
                    .or(row.at.as_ref())
                    .and_then(clock::instant)
                    .map_or(Asked::Unknown, |at| {
                        Asked::Ago(format!("asked {}", clock::relative(at, self.now)))
                    })
            },
            chip: decide_chip(row, today, tiers),
            answers: if reply.is_some() || stale {
                None
            } else {
                self.answers(row)
            },
            thumb: row.thumb.as_deref(),
            reply,
        }
    }

    /// What to do: a row standing for another task's ask shows that ask; else its action, its
    /// line, or a word to open it.
    fn action(&self, row: &WorkRow) -> String {
        let own = row.task.and_then(|task| lookup::issue(self.state, task));
        let stands_for = own
            .and_then(|own| self.needs.ask_of(own))
            .filter(|&ask| Some(ask) != row.task)
            .and_then(|ask| lookup::issue(self.state, ask));
        stands_for
            .and_then(|other| {
                other
                    .action
                    .clone()
                    .or_else(|| other.decision.as_ref().map(|d| AskText::of(d).summary))
            })
            .or_else(|| row.action.clone())
            .or_else(|| row.line.clone())
            .unwrap_or_else(|| format!("Open it to see what {APP_NAME} needs"))
    }

    /// The answers right on the card: only for a quick yes or a Done (not a pick, which is read
    /// on its page first), with a thing to do, on a task.
    fn answers(&self, row: &WorkRow) -> Option<CardAnswers> {
        let task = row.task?;
        let need = &row.waiting.as_ref()?.need;
        if *need == AskKind::Choose || row.action.is_none() {
            return None;
        }
        let own = lookup::issue(self.state, task);
        let target = match own {
            Some(own) => Some(lookup::issue(self.state, self.needs.ask_of(own)?)?),
            None => None,
        };
        let to = target.map_or(task, |target| target.id);
        let decision = target.and_then(|target| target.decision.as_ref());
        let options = match decision {
            Some(decision) => decision.options.clone(),
            None if to == task => row.options.clone().unwrap_or_default(),
            None => Vec::new(),
        };
        if !options.is_empty() {
            Some(CardAnswers { to, options })
        } else if decision.is_none() {
            Some(CardAnswers {
                to,
                options: vec![DONE.to_owned()],
            })
        } else {
            None
        }
    }
}

/// What an ask costs, as the small label on its card says it.
#[must_use]
pub(crate) fn kind_name(row: &WorkRow) -> &'static str {
    match row.waiting.as_ref().map(|waiting| &waiting.need) {
        Some(AskKind::Approve) => "Quick yes",
        Some(AskKind::Do) => "Only you",
        _ => "Read & decide",
    }
}

/// Whether an ask hasn't been checked in over 12 hours (its last check, else its row's time).
#[must_use]
pub(crate) fn is_stale(row: &WorkRow, now: Timestamp) -> bool {
    row.needs_you()
        && row
            .waiting
            .as_ref()
            .and_then(|waiting| waiting.checked.as_ref())
            .or(row.at.as_ref())
            .and_then(clock::instant)
            .is_some_and(|checked| now.as_second() - checked.as_second() > STALE_AFTER_SECONDS)
}

/// The decide-by chip of `row` seen from `today`; with `tiers` it names the group it would be
/// in, and an ask with no day yet reads `This week`.
#[must_use]
pub(crate) fn decide_chip(row: &WorkRow, today: Date, tiers: bool) -> Option<DecideChip> {
    let hard = row
        .waiting
        .as_ref()
        .and_then(|waiting| waiting.deadline.as_ref())
        .is_some_and(|deadline| deadline.hard_by);
    let Some(days) = lookup::days_to_decide(row, today) else {
        return tiers.then(|| DecideChip {
            text: "This week".to_owned(),
            tone: ChipTone::Plain,
            hard: false,
        });
    };
    let day = lookup::decide_by(row).and_then(lookup::civil_day)?;
    let date = match days {
        1 => "Tomorrow".to_owned(),
        ..7 => day.strftime("%a").to_string(),
        _ => day.strftime("%-d %b").to_string(),
    };
    let text = match days {
        ..0 if tiers => format!("Overdue · {}d", -days),
        ..0 => format!("{}d late", -days),
        0 => "Today".to_owned(),
        1..=6 if tiers => format!("This week · {date}"),
        7..=30 if tiers => format!("This month · {date}"),
        1 => "Tomorrow".to_owned(),
        _ => format!("by {date}"),
    };
    let tone = if days < 0 {
        ChipTone::Late
    } else if hard {
        ChipTone::Hard
    } else if days == 0 {
        ChipTone::Today
    } else {
        ChipTone::Plain
    };
    Some(DecideChip { text, tone, hard })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::*;
    use crate::fixtures::{issue, state, waiting};
    use crate::wire::TASK_KEY_PREFIX;

    const TODAY: Date = jiff::civil::date(2026, 11, 25);

    fn now() -> Timestamp {
        "2026-11-25T12:00:00Z".parse().unwrap()
    }

    fn on(day: Option<&str>, hard: bool) -> WorkRow {
        waiting(1, json!({"decideBy": day, "hardBy": hard}))
    }

    #[test]
    fn the_chip_says_the_day_or_its_tier() {
        let chip =
            |day: &str, tiers: bool| decide_chip(&on(Some(day), false), TODAY, tiers).unwrap();
        assert_eq!(chip("2026-11-23", false).text, "2d late");
        assert_eq!(chip("2026-11-23", true).text, "Overdue · 2d");
        assert_eq!(chip("2026-11-23", true).tone, ChipTone::Late);
        assert_eq!(chip("2026-11-25", true).text, "Today");
        assert_eq!(chip("2026-11-25", true).tone, ChipTone::Today);
        assert_eq!(chip("2026-11-26", false).text, "Tomorrow");
        assert_eq!(chip("2026-11-26", true).text, "This week · Tomorrow");
        assert_eq!(chip("2026-11-28", false).text, "by Sat");
        assert_eq!(chip("2026-11-28", true).text, "This week · Sat");
        assert_eq!(chip("2026-12-03", true).text, "This month · 3 Dec");
        assert_eq!(chip("2026-12-03", false).text, "by 3 Dec");
        assert_eq!(chip("2027-01-30", true).text, "by 30 Jan");
        assert_eq!(chip("2026-12-03", true).tone, ChipTone::Plain);
        let none = on(None, false);
        assert_eq!(decide_chip(&none, TODAY, false), None);
        assert_eq!(decide_chip(&none, TODAY, true).unwrap().text, "This week");
        let hard = decide_chip(&on(Some("2026-11-27"), true), TODAY, false).unwrap();
        assert!(hard.hard);
        assert_eq!(hard.tone, ChipTone::Hard);
    }

    #[test]
    fn a_card_says_what_it_costs_and_when_it_was_asked() {
        let rows = [
            waiting(
                1,
                json!({"need": "approve", "action": "Say yes", "options": ["Yes", "No"],
                       "asked": "2026-11-25T09:00:00Z", "checked": "2026-11-25T09:00:00Z", "draft": true}),
            ),
            waiting(
                2,
                json!({"need": "do", "action": "Log in", "checked": "2026-11-24T20:00:00Z"}),
            ),
            waiting(
                3,
                json!({"need": "choose", "action": "Pick one", "options": ["A", "B"]}),
            ),
        ];
        let lists = state(json!({"working": rows}));
        let gone = HashSet::new();
        let needs = Needs::new(&lists, &gone, TODAY);
        let context = CardContext {
            state: &lists,
            needs: &needs,
            now: now(),
            project: true,
            tiers: false,
        };
        let yes = context.card(&lists.working[0]);
        assert_eq!(yes.kind, "Quick yes");
        assert_eq!(yes.asked, Asked::Ago("asked 3 hours ago".to_owned()));
        assert!(yes.draft);
        let answers = yes.answers.unwrap();
        assert_eq!(
            (answers.first(), answers.second()),
            (Some("Yes"), Some("No"))
        );
        assert_eq!(answers.menu().len(), 0);

        let login = context.card(&lists.working[1]);
        assert_eq!(login.kind, "Only you");
        assert_eq!(login.asked, Asked::Outdated, "not checked in 16 hours");
        assert_eq!(login.answers, None, "an outdated ask is opened first");

        let pick = context.card(&lists.working[2]);
        assert_eq!(pick.kind, "Read & decide");
        assert_eq!(pick.answers, None, "a pick is read on its page");
    }

    #[test]
    fn a_thing_to_do_says_i_did_it_and_three_answers_put_the_rest_in_a_menu() {
        let rows = [
            waiting(
                1,
                json!({"need": "do", "action": "Log in", "checked": "2026-11-25T11:00:00Z"}),
            ),
            waiting(
                2,
                json!({"need": "approve", "action": "Go?", "options": ["Yes", "Later", "No"],
                       "checked": "2026-11-25T11:00:00Z"}),
            ),
        ];
        let lists = state(json!({"working": rows}));
        let gone = HashSet::new();
        let needs = Needs::new(&lists, &gone, TODAY);
        let context = CardContext {
            state: &lists,
            needs: &needs,
            now: now(),
            project: true,
            tiers: false,
        };
        let done = context.card(&lists.working[0]).answers.unwrap();
        assert_eq!(done.options, [DONE]);
        let three = context.card(&lists.working[1]).answers.unwrap();
        assert_eq!(three.second(), None);
        assert_eq!(three.menu(), ["Later", "No"]);
    }

    #[test]
    fn a_task_standing_for_another_shows_and_answers_that_ask() {
        let lists = state(json!({
            "issues": [
                issue(1, "waiting", json!({"action": format!("One yes on {TASK_KEY_PREFIX}2 for both")})),
                issue(2, "waiting", json!({"decision": {"question": "Ship it?", "options": ["Ship", "Wait"]}})),
            ],
            "working": [waiting(1, json!({"need": "approve", "action": format!("One yes on {TASK_KEY_PREFIX}2 for both"),
                                          "checked": "2026-11-25T11:00:00Z"}))],
        }));
        let gone = HashSet::new();
        let needs = Needs::new(&lists, &gone, TODAY);
        let context = CardContext {
            state: &lists,
            needs: &needs,
            now: now(),
            project: false,
            tiers: true,
        };
        let card = context.card(&lists.working[0]);
        assert_eq!(card.action, "Ship it?");
        let answers = card.answers.unwrap();
        assert_eq!(answers.to, TaskId::new(2));
        assert_eq!(answers.options, ["Ship", "Wait"]);
        assert_eq!(
            card.chip.unwrap().text,
            "This week",
            "no day yet: this week's"
        );
    }
}
