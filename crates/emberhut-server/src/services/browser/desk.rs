//! What the browser and the Mac tools need from the rest of the server, behind two small seams:
//!
//! - [`TaskDesk`]: the tasks a step is for (a note on one, its "Needs you" line, a Go ahead /
//!   Stop card and the owner's choice on it, a file attached), and the assistant's own events
//!   ("the owner handed the browser back"). The tasks area wires its own; without it,
//!   [`NoDesk`] keeps nothing and asks nothing.
//! - [`StepGate`]: whether a final step may go (the owner's OK on it, on its job, or the job's
//!   plan). [`CardGate`] is this area's: one Go ahead / Stop card open per task (a new ask
//!   replaces the one before: one pending step per task), answered
//!   only for the step (or the job's plan) it was asked for. The owner's Go ahead lets that step
//!   through once and, for an ordinary step's card only, covers the rest of the job; a message,
//!   a post, money and account security ask every time. A richer gate (standing grants, OK'd
//!   texts) replaces it without the tools changing.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use emberhut_core::Clock;
use emberhut_core::browser::{StepKind, is_hard, step_kind};
use futures_util::future::BoxFuture;
use jiff::Timestamp;

/// Who writes a note on a task: the assistant, or one of its workers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteBy {
    /// The assistant.
    Assistant,
    /// A worker on the task.
    Worker,
}

/// A Go ahead / Stop (or a pick) card on a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// The question.
    pub question: String,
    /// Its answers.
    pub options: Vec<String>,
    /// The task's "Needs you" line while it waits (90 characters).
    pub action: String,
    /// The lines under the question.
    pub brief: Vec<String>,
    /// `approve` or `choose`.
    pub kind: &'static str,
}

/// The tasks and the assistant's events, as the browser and the Mac tools reach them.
pub trait TaskDesk: fmt::Debug + Send + Sync {
    /// Whether task `task` exists.
    fn exists(&self, task: i64) -> BoxFuture<'_, bool>;

    /// A line in task `task`'s history.
    fn note(&self, task: i64, by: NoteBy, line: String) -> BoxFuture<'_, ()>;

    /// Task `task`'s "Needs you" line (none: clear it).
    fn set_action(&self, task: i64, action: Option<String>) -> BoxFuture<'_, ()>;

    /// Clears task `task`'s "Needs you" line, if it still starts with `prefix`, and puts it
    /// back to working.
    fn clear_action(&self, task: i64, prefix: String) -> BoxFuture<'_, ()>;

    /// Asks the owner on task `task`.
    fn ask(&self, task: i64, card: Card) -> BoxFuture<'_, ()>;

    /// The owner's first choice on task `task` after `at` ("Go ahead", "Stop", a pick).
    fn chose_since(&self, task: i64, at: Timestamp) -> BoxFuture<'_, Option<String>>;

    /// Attaches a file the browsing made (its log, a picture) to task `task`.
    fn attach(&self, task: i64, path: String, title: String) -> BoxFuture<'_, ()>;

    /// An event for the assistant (`browser`, `ask`, `chrome`): it wakes and reads `text`.
    fn wake(&self, kind: &'static str, text: String) -> BoxFuture<'_, ()>;
}

/// No tasks area wired: nothing is kept, nothing asked, nobody woken.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDesk;

impl TaskDesk for NoDesk {
    fn exists(&self, _task: i64) -> BoxFuture<'_, bool> {
        Box::pin(async { false })
    }

    fn note(&self, _task: i64, _by: NoteBy, _line: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn set_action(&self, _task: i64, _action: Option<String>) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn clear_action(&self, _task: i64, _prefix: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn ask(&self, _task: i64, _card: Card) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn chose_since(&self, _task: i64, _at: Timestamp) -> BoxFuture<'_, Option<String>> {
        Box::pin(async { None })
    }

    fn attach(&self, _task: i64, _path: String, _title: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn wake(&self, _kind: &'static str, _text: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// A final step about to happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// The task it is for.
    pub task: i64,
    /// What it is: the button's label, the form's submit, the menu item.
    pub what: String,
    /// The page's title and address (or the app it is in).
    pub page: String,
    /// The card's question.
    pub question: String,
    /// The card's "Needs you" line.
    pub action: String,
    /// The card's lines.
    pub brief: Vec<String>,
    /// For a message or a post: the text written in the box it sends.
    pub composed: Option<Vec<String>>,
}

/// Whether a step may go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gated {
    /// It may: by what (`their Go ahead`, `in the job they OK'd`).
    Through(String),
    /// Not now: what the agent is told (asked, still waiting, refused).
    Wait(String),
}

/// The owner's OK on final steps.
pub trait StepGate: fmt::Debug + Send + Sync {
    /// Whether `step` may go now; asks the owner when it can't.
    fn step(&self, step: Step) -> BoxFuture<'_, Gated>;

    /// A job's final steps named once, up front (`browser_plan`): what the agent is told.
    fn plan(&self, task: i64, job: String, steps: Vec<String>) -> BoxFuture<'_, String>;

    /// The job's OK on task `task`, as a step says it, when there is one.
    fn job_grant(&self, task: i64) -> BoxFuture<'_, Option<String>>;
}

/// What a task's open card was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AskedFor {
    /// A final step, by what it is.
    Step(String),
    /// The job planned with `browser_plan`.
    Plan,
}

/// The card open on a task, asked and not yet settled: the owner's next choice on the task
/// answers it, and only it.
#[derive(Clone, Debug)]
struct Asked {
    what: AskedFor,
    at: Timestamp,
}

/// A job planned with `browser_plan`.
#[derive(Clone, Debug)]
struct Plan {
    job: String,
    steps: Vec<String>,
    asked: Timestamp,
}

#[derive(Debug, Default)]
struct Book {
    /// Task → its open card (one per task: a new ask replaces it).
    asked: HashMap<i64, Asked>,
    /// Task → its job's OK, as said.
    grants: HashMap<i64, String>,
    /// Task → its planned job.
    plans: HashMap<i64, Plan>,
}

/// The gate of Go ahead / Stop cards on the step's task (see the module's words).
pub struct CardGate {
    desk: Arc<dyn TaskDesk>,
    clock: Arc<dyn Clock>,
    book: Mutex<Book>,
}

impl fmt::Debug for CardGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CardGate").finish_non_exhaustive()
    }
}

/// "HH:MM" of an instant, in UTC.
pub(crate) fn hhmm(at: Timestamp) -> String {
    let text = at.to_string();
    text.get(11..16).unwrap_or_default().to_owned()
}

/// A list of steps as said: `"Create app", "Install"`.
fn listed(steps: &[String]) -> String {
    steps
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The text of a card's first line, 90 characters at most.
fn line90(text: &str) -> String {
    text.chars().take(90).collect()
}

impl CardGate {
    /// The gate over `desk`'s cards.
    #[must_use]
    pub fn new(desk: Arc<dyn TaskDesk>, clock: Arc<dyn Clock>) -> Self {
        Self {
            desk,
            clock,
            book: Mutex::new(Book::default()),
        }
    }

    fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Keeps `plan` as task `task`'s job, its card the one open on the task.
    fn planned(&self, task: i64, plan: Plan) {
        let mut book = self.book();
        book.asked.insert(
            task,
            Asked {
                what: AskedFor::Plan,
                at: plan.asked,
            },
        );
        book.plans.insert(task, plan);
    }

    /// The card open on task `task` when it was asked for `what`.
    fn open_for(&self, task: i64, what: &AskedFor) -> Option<Asked> {
        self.book()
            .asked
            .get(&task)
            .filter(|asked| asked.what == *what)
            .cloned()
    }

    /// The planned job's OK, once the owner said Go ahead on its card (while its card is the
    /// one open on the task: another card asked since replaced it).
    async fn plan_ok(&self, task: i64) -> Option<String> {
        let plan = self.book().plans.get(&task).cloned()?;
        let asked = self.open_for(task, &AskedFor::Plan)?;
        let chose = self.desk.chose_since(task, asked.at).await?;
        self.book().asked.remove(&task);
        if chose == "Go ahead" {
            let by = format!(
                "their Go ahead on the job ({}: {})",
                plan.job,
                listed(&plan.steps)
            );
            self.book().grants.insert(task, by.clone());
            Some(by)
        } else {
            self.book().plans.remove(&task);
            None
        }
    }
}

impl StepGate for CardGate {
    fn step(&self, step: Step) -> BoxFuture<'_, Gated> {
        Box::pin(async move {
            let kind = step_kind(&step.what, &step.page);
            let mine = AskedFor::Step(step.what.clone());
            if let Some(asked) = self.open_for(step.task, &mine) {
                match self.desk.chose_since(step.task, asked.at).await {
                    Some(chose) if chose == "Go ahead" => {
                        self.book().asked.remove(&step.task);
                        // Only an ordinary step's card is the job's OK: a message, a post,
                        // money and account security are OK'd one at a time.
                        if kind == StepKind::Ordinary {
                            self.book()
                                .grants
                                .insert(step.task, "their Go ahead".to_owned());
                        }
                        return Gated::Through("their Go ahead".to_owned());
                    }
                    Some(chose) => {
                        self.book().asked.remove(&step.task);
                        return Gated::Wait(format!(
                            "The owner said {chose}: \"{}\" was not done. Don't ask again unless \
                             they bring it up.",
                            step.what
                        ));
                    }
                    None => {
                        return Gated::Wait(format!(
                            "Still waiting for the owner's Go ahead on \"{}\" (asked {}). Stop \
                             for now; you'll hear when they answer.",
                            step.what,
                            hhmm(asked.at)
                        ));
                    }
                }
            }
            if kind == StepKind::Ordinary {
                let granted = self.book().grants.get(&step.task).cloned();
                if let Some(by) = granted {
                    return Gated::Through(by);
                }
                let plan = self.book().plans.get(&step.task).cloned();
                if let Some(plan) = plan {
                    if let Some(by) = self.plan_ok(step.task).await {
                        return Gated::Through(by);
                    }
                    if self.book().plans.contains_key(&step.task)
                        && self.open_for(step.task, &AskedFor::Plan).is_some()
                    {
                        return Gated::Wait(format!(
                            "Paused: \"{}\" waits for the owner's OK on the job you planned \
                             (asked {}). Stop for now; you'll hear when they answer.",
                            step.what,
                            hhmm(plan.asked)
                        ));
                    }
                }
            }
            let mut brief = step.brief.clone();
            if kind == StepKind::Said
                && let Some(text) = step
                    .composed
                    .as_ref()
                    .and_then(|c| c.iter().map(|x| x.trim()).find(|x| !x.is_empty()))
            {
                brief.push(format!(
                    "Text: {}",
                    text.chars().take(300).collect::<String>()
                ));
                brief.truncate(4);
            }
            let at = self.clock.now();
            self.book()
                .asked
                .insert(step.task, Asked { what: mine, at });
            self.desk
                .ask(
                    step.task,
                    Card {
                        question: step.question.clone(),
                        options: vec!["Go ahead".to_owned(), "Stop".to_owned()],
                        action: line90(&step.action),
                        brief,
                        kind: "approve",
                    },
                )
                .await;
            Gated::Wait(
                "Paused: the owner was asked first (it's under Needs you). When they say Go \
                 ahead, do the same step again. Stop for now."
                    .to_owned(),
            )
        })
    }

    fn plan(&self, task: i64, job: String, steps: Vec<String>) -> BoxFuture<'_, String> {
        Box::pin(async move {
            let mut seen = Vec::new();
            for step in steps {
                let step: String = step.trim().chars().take(60).collect();
                if !step.is_empty() && !seen.contains(&step) {
                    seen.push(step);
                }
            }
            seen.truncate(12);
            let held: Vec<String> = seen.iter().filter(|s| is_hard(s, "")).cloned().collect();
            let still = if held.is_empty() {
                String::new()
            } else {
                format!(
                    " {} still ask{} the owner on the step each time (payments, purchases, \
                     passwords, codes and account security never ride on an OK).",
                    listed(&held),
                    if held.len() == 1 { "s" } else { "" }
                )
            };
            let later: Vec<String> = seen
                .iter()
                .filter(|s| step_kind(s, "") == StepKind::Said)
                .cloned()
                .collect();
            let ok: Vec<String> = seen
                .iter()
                .filter(|s| step_kind(s, "") == StepKind::Ordinary)
                .cloned()
                .collect();
            if ok.is_empty() && later.is_empty() {
                return format!("Nothing planned:{still}");
            }
            let pre = if later.is_empty() {
                String::new()
            } else {
                format!(
                    "{}: a message as the owner or a public post asks them for its text at the \
                     click. ",
                    listed(&later)
                )
            };
            if ok.is_empty() {
                return format!("{pre}{still}").trim().to_owned();
            }
            if let Some(by) = self.job_grant(task).await {
                return format!(
                    "OK'd already ({by}): click {} when you reach them; they go through with no \
                     card while the task is open. {pre}{}",
                    listed(&ok),
                    still.trim()
                )
                .trim()
                .to_owned();
            }
            let question = format!(
                "The assistant's browser job on task {task}: {job}. The final steps it will \
                 click: {}. OK the whole job?",
                listed(&ok)
            );
            self.planned(
                task,
                Plan {
                    job: job.clone(),
                    steps: ok.clone(),
                    asked: self.clock.now(),
                },
            );
            self.desk
                .ask(
                    task,
                    Card {
                        question,
                        options: vec!["Go ahead".to_owned(), "Stop".to_owned()],
                        action: line90(&format!("OK the job: {job}")),
                        brief: vec![
                            format!("Job: {job}").chars().take(160).collect(),
                            format!("Final steps: {}", listed(&ok))
                                .chars()
                                .take(160)
                                .collect(),
                            "Payments, purchases, passwords and account security still ask you \
                             each time"
                                .to_owned(),
                        ],
                        kind: "approve",
                    },
                )
                .await;
            format!(
                "{pre}Asked the owner once for the whole job (Go ahead / Stop, under Needs you). \
                 Stop for now; you'll hear when they answer. After their Go ahead, every step of \
                 the job goes through without asking again.{still}"
            )
        })
    }

    fn job_grant(&self, task: i64) -> BoxFuture<'_, Option<String>> {
        Box::pin(async move {
            let granted = self.book().grants.get(&task).cloned();
            match granted {
                Some(by) => Some(by),
                None if self.book().plans.contains_key(&task) => self.plan_ok(task).await,
                None => None,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use emberhut_core::ManualClock;
    use jiff::SignedDuration;

    use super::*;

    /// A desk whose owner's taps are the test's: each one at its time, on the task.
    #[derive(Debug, Default)]
    struct Taps {
        asked: Mutex<Vec<(i64, String)>>,
        taps: Mutex<Vec<(i64, Timestamp, String)>>,
    }

    impl Taps {
        fn tap(&self, task: i64, at: Timestamp, choice: &str) {
            self.taps
                .lock()
                .unwrap()
                .push((task, at, choice.to_owned()));
        }

        fn asks(&self) -> usize {
            self.asked.lock().unwrap().len()
        }
    }

    impl TaskDesk for Taps {
        fn exists(&self, _task: i64) -> BoxFuture<'_, bool> {
            Box::pin(async { true })
        }
        fn note(&self, _task: i64, _by: NoteBy, _line: String) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
        fn set_action(&self, _task: i64, _action: Option<String>) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
        fn clear_action(&self, _task: i64, _prefix: String) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
        fn ask(&self, task: i64, card: Card) -> BoxFuture<'_, ()> {
            self.asked.lock().unwrap().push((task, card.question));
            Box::pin(async {})
        }
        fn chose_since(&self, task: i64, at: Timestamp) -> BoxFuture<'_, Option<String>> {
            let chose = self
                .taps
                .lock()
                .unwrap()
                .iter()
                .find(|(t, when, _)| *t == task && *when > at)
                .map(|(_, _, choice)| choice.clone());
            Box::pin(async move { chose })
        }
        fn attach(&self, _task: i64, _path: String, _title: String) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
        fn wake(&self, _kind: &'static str, _text: String) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
    }

    fn step(task: i64, what: &str, page: &str) -> Step {
        Step {
            task,
            what: what.to_owned(),
            page: page.to_owned(),
            question: format!("Click \"{what}\"?"),
            action: format!("OK \"{what}\""),
            brief: Vec::new(),
            composed: None,
        }
    }

    struct Bench {
        desk: Arc<Taps>,
        clock: Arc<ManualClock>,
        gate: CardGate,
    }

    impl Bench {
        fn new() -> Self {
            let desk = Arc::new(Taps::default());
            let clock = Arc::new(ManualClock::new(Timestamp::UNIX_EPOCH));
            let gate = CardGate::new(
                Arc::clone(&desk) as Arc<dyn TaskDesk>,
                Arc::clone(&clock) as Arc<dyn Clock>,
            );
            Self { desk, clock, gate }
        }

        /// A minute on, the owner taps `choice` on task `task`'s card.
        fn tap(&self, task: i64, choice: &str) {
            self.clock.advance(SignedDuration::from_mins(1));
            self.desk.tap(task, self.clock.now(), choice);
            self.clock.advance(SignedDuration::from_mins(1));
        }

        async fn go(&self, task: i64, what: &str, page: &str) -> Gated {
            self.clock.advance(SignedDuration::from_secs(1));
            self.gate.step(step(task, what, page)).await
        }
    }

    fn through(gated: &Gated) -> bool {
        matches!(gated, Gated::Through(_))
    }

    #[tokio::test]
    async fn a_hard_step_asks_again_after_its_own_go_ahead() {
        let bench = Bench::new();
        assert!(!through(&bench.go(1, "Pay now", "Checkout").await));
        bench.tap(1, "Go ahead");
        assert!(through(&bench.go(1, "Pay now", "Checkout").await));
        // Its Go ahead was for that one payment: the next one asks again.
        let again = bench.go(1, "Pay now", "Checkout").await;
        assert!(!through(&again), "{again:?}");
        assert_eq!(bench.desk.asks(), 2);
        assert_eq!(bench.gate.job_grant(1).await, None);
    }

    #[tokio::test]
    async fn a_hard_step_is_not_covered_by_another_card_s_go_ahead() {
        let bench = Bench::new();
        assert!(!through(&bench.go(2, "Pay now", "Checkout").await));
        // Another step asks on the same task: its card replaces the payment's.
        assert!(!through(&bench.go(2, "Create app", "Console").await));
        bench.tap(2, "Go ahead");
        let paid = bench.go(2, "Pay now", "Checkout").await;
        assert!(!through(&paid), "{paid:?}");
        assert_eq!(bench.desk.asks(), 3);
        // The tap after the payment's new card is that card's own.
        bench.tap(2, "Go ahead");
        assert!(through(&bench.go(2, "Pay now", "Checkout").await));
    }

    #[tokio::test]
    async fn a_planned_job_s_card_replaced_by_a_step_s_is_not_its_go_ahead() {
        let bench = Bench::new();
        bench
            .gate
            .plan(3, "Set up".to_owned(), vec!["Create app".to_owned()])
            .await;
        assert!(!through(&bench.go(3, "Pay now", "Checkout").await));
        bench.tap(3, "Go ahead");
        assert!(through(&bench.go(3, "Pay now", "Checkout").await));
        // The tap was on the payment's card, which had replaced the job's.
        assert_eq!(bench.gate.job_grant(3).await, None);
    }

    #[tokio::test]
    async fn only_an_ordinary_step_s_go_ahead_becomes_the_job_s_ok() {
        let bench = Bench::new();
        assert!(!through(&bench.go(4, "Send", "Mail").await));
        bench.tap(4, "Go ahead");
        assert!(through(&bench.go(4, "Send", "Mail").await));
        assert_eq!(bench.gate.job_grant(4).await, None);
        // The next ordinary step still asks: a message's OK is not the job's.
        assert!(!through(&bench.go(4, "Create app", "Console").await));
        bench.tap(4, "Go ahead");
        assert!(through(&bench.go(4, "Create app", "Console").await));
        assert_eq!(
            bench.gate.job_grant(4).await.as_deref(),
            Some("their Go ahead")
        );
        assert!(through(&bench.go(4, "Install", "Console").await));
        // Neither covers a message or a payment.
        assert!(!through(&bench.go(4, "Send", "Mail").await));
        assert!(!through(&bench.go(4, "Pay now", "Checkout").await));
    }

    #[test]
    fn a_time_is_said_as_hours_and_minutes() {
        let at: Timestamp = "2026-11-26T16:34:05Z".parse().unwrap_or_default();
        assert_eq!(hhmm(at), "16:34");
        assert_eq!(listed(&["A".to_owned(), "B".to_owned()]), "\"A\", \"B\"");
    }
}
