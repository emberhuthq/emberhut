//! The user's answers to asks and actions on tasks (done, not now, archive, unarchive), held for
//! a few seconds with Undo before they go to the server, since an answer can't be taken back
//! once the assistant has it. The task leaves every list at once ("gone"), and comes back if the
//! user undoes it or the server refuses it; an action's task comes back with the first state
//! asked for after the server took it, which shows it as it now is (one asked before, still on
//! its way then, would show it as it was). A refusal of an earlier answer never covers the
//! toast of the answer held: it waits, and shows once that answer is sent or undone.
//!
//! [`Answers`] is the state machine; the store sends what it hands out and reports back.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use emberhut_api::answers::AnswerRequest;
use emberhut_api::ids::TaskId;
use emberhut_api::task_actions::TaskAction;

/// How long an answer is held before it is sent.
pub(crate) const UNDO_WINDOW: Duration = Duration::from_secs(4);

/// How long the toast says why an answer wasn't sent before it goes by itself.
pub(crate) const REFUSAL_SHOWN: Duration = Duration::from_secs(6);

/// An answer or an action to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Answer {
    /// The task it is about.
    pub(crate) task: TaskId,
    /// What the user did.
    pub(crate) deed: Deed,
}

/// What the user did to a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Deed {
    /// Answered its ask with one of its options.
    Choice(String),
    /// An action on it (done, put off, archived, back).
    Act(TaskAction),
}

impl Deed {
    /// What the toast says once it is done here: `Answered: Yes`, `Archived`.
    #[must_use]
    fn toast(&self) -> String {
        match self {
            Self::Choice(choice) => toast_text(choice),
            Self::Act(TaskAction::MarkDone(_)) => "Marked done".to_owned(),
            Self::Act(TaskAction::Park(_)) => "Moved to next week".to_owned(),
            Self::Act(TaskAction::Archive(_)) => "Archived".to_owned(),
            Self::Act(TaskAction::Unarchive(_)) => "Back in to do".to_owned(),
            Self::Act(_) => "Saved".to_owned(),
        }
    }

    /// What the toast says when the server refused it, before its reason.
    #[must_use]
    fn failure(&self) -> &'static str {
        match self {
            Self::Choice(_) => "Answer not sent",
            Self::Act(TaskAction::MarkDone(_)) => "Not marked done",
            Self::Act(TaskAction::Park(_)) => "Not moved",
            Self::Act(TaskAction::Archive(_)) => "Not archived",
            Self::Act(TaskAction::Unarchive(_)) => "Not unarchived",
            Self::Act(_) => "Not saved",
        }
    }

    /// Whether it closes the task (an answer such as "Not now"), shown with a quieter mark.
    #[must_use]
    fn closes(&self, task: TaskId) -> bool {
        match self {
            Self::Choice(choice) => AnswerRequest::new(task, choice.as_str()).cancels(),
            Self::Act(_) => false,
        }
    }
}

/// What answering did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Answered {
    /// The task was already answered: nothing changed.
    Already,
    /// The answer is held; the one held before it (if any) is to be sent now.
    Held {
        /// The earlier answer, to send at once.
        send_now: Option<Answer>,
    },
}

/// The line at the foot of the window about the last answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Toast {
    /// Which toast it is: each answer's and each refusal's has its own, so a timer for an
    /// older one leaves a newer one alone.
    pub(crate) serial: u64,
    /// The task it is about.
    pub(crate) task: TaskId,
    /// What happened, e.g. "Answered: Yes", or why the server refused it.
    pub(crate) text: String,
    /// The answer closes the task ("Not now"), shown with a quieter mark.
    pub(crate) closes: bool,
    /// Where the answer is.
    pub(crate) phase: Phase,
}

/// Where the answer a toast is about is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Held for its Undo window: Undo takes it back.
    Held,
    /// On its way to the server.
    Sending,
    /// The server refused it (or couldn't be reached): the text says why, and Hide puts the
    /// toast away.
    Refused,
}

impl Toast {
    /// Whether Undo still takes the answer back (it hasn't been sent).
    #[must_use]
    pub(crate) fn undoable(&self) -> bool {
        self.phase == Phase::Held
    }

    /// Whether the answer is on its way.
    #[must_use]
    pub(crate) fn sending(&self) -> bool {
        self.phase == Phase::Sending
    }

    /// Whether the server refused the answer.
    #[must_use]
    pub(crate) fn failed(&self) -> bool {
        self.phase == Phase::Refused
    }
}

/// The answers given on this device.
#[derive(Debug, Default)]
pub(crate) struct Answers {
    /// The answer waiting out its Undo window.
    held: Option<Answer>,
    toast: Option<Toast>,
    /// A refusal that came while an answer was held, to show once that answer's toast goes.
    refusal_waiting: Option<Toast>,
    /// The refusal toast shown last, whose time to go hasn't been set yet
    /// ([`Answers::refusal_to_time`]).
    refusal_shown: Option<u64>,
    /// Tasks answered or acted on here, off the lists until the server agrees or it is undone.
    gone: HashSet<TaskId>,
    /// The last toast's serial.
    serial: u64,
    /// Tasks whose answer or action is on its way.
    sending: HashSet<TaskId>,
    /// Tasks acted on whose action is on its way or was held: they stay gone until it lands.
    acting: HashSet<TaskId>,
    /// Tasks whose action the server took, and when: they come back with the first state asked
    /// for after that.
    taken: HashMap<TaskId, Instant>,
}

impl Answers {
    /// The tasks answered here.
    #[must_use]
    pub(crate) fn gone(&self) -> &HashSet<TaskId> {
        &self.gone
    }

    /// The toast, if one shows.
    #[must_use]
    pub(crate) fn toast(&self) -> Option<&Toast> {
        self.toast.as_ref()
    }

    /// Whether what was done to `task` is on its way to the server.
    #[must_use]
    pub(crate) fn is_sending(&self, task: TaskId) -> bool {
        self.sending.contains(&task)
    }

    /// The user answers `task`: the answer is held, and the one held before it is handed back
    /// to send now.
    pub(crate) fn answer(&mut self, task: TaskId, choice: String) -> Answered {
        self.hold(task, Deed::Choice(choice))
    }

    /// The user acts on `task` (done, not now, archive, unarchive): held as an answer is.
    pub(crate) fn act(&mut self, task: TaskId, action: TaskAction) -> Answered {
        self.hold(task, Deed::Act(action))
    }

    fn hold(&mut self, task: TaskId, deed: Deed) -> Answered {
        if self.gone.contains(&task) {
            return Answered::Already;
        }
        // A refusal waiting goes on waiting, now for this answer.
        let send_now = self.flush_held();
        self.gone.insert(task);
        if matches!(deed, Deed::Act(_)) {
            self.acting.insert(task);
        }
        self.serial += 1;
        self.toast = Some(Toast {
            serial: self.serial,
            task,
            text: deed.toast(),
            closes: deed.closes(task),
            phase: Phase::Held,
        });
        self.held = Some(Answer { task, deed });
        Answered::Held { send_now }
    }

    /// The held answer, now to be sent (its window ran out, or the app is leaving). A refusal
    /// that waited for it shows now.
    pub(crate) fn flush(&mut self) -> Option<Answer> {
        let answer = self.flush_held()?;
        self.show_waiting_refusal();
        Some(answer)
    }

    /// The held answer, to be sent, its toast saying so.
    fn flush_held(&mut self) -> Option<Answer> {
        let answer = self.held.take()?;
        self.sending.insert(answer.task);
        if let Some(toast) = self
            .toast
            .as_mut()
            .filter(|toast| toast.task == answer.task)
        {
            toast.phase = Phase::Sending;
        }
        Some(answer)
    }

    /// The Undo window of the answer for `task` ran out: it is handed back to send, unless it
    /// was undone or sent already.
    pub(crate) fn due(&mut self, task: TaskId) -> Option<Answer> {
        if self.held.as_ref().is_some_and(|held| held.task == task) {
            self.flush()
        } else {
            None
        }
    }

    /// Undo on the toast `serial`: when it is the held answer's, that answer is dropped and its
    /// task comes back; the toast of an answer already sent just goes. An older toast's Undo
    /// does nothing.
    pub(crate) fn undo(&mut self, serial: u64) {
        let Some(toast) = self.toast.as_ref().filter(|toast| toast.serial == serial) else {
            return;
        };
        if toast.undoable()
            && let Some(held) = self.held.take_if(|held| held.task == toast.task)
        {
            self.gone.remove(&held.task);
            self.acting.remove(&held.task);
        }
        self.toast = None;
        self.show_waiting_refusal();
    }

    /// The serial of the refusal toast shown since this was last asked, to put it away after
    /// [`REFUSAL_SHOWN`] ([`Answers::expire`]).
    pub(crate) fn refusal_to_time(&mut self) -> Option<u64> {
        self.refusal_shown.take()
    }

    /// A refusal shows, unless an answer is held: then it waits for that answer's toast to go.
    fn show_refusal(&mut self, refusal: Toast) {
        if self.toast.as_ref().is_some_and(Toast::undoable) {
            self.refusal_waiting = Some(refusal);
        } else {
            self.refusal_shown = Some(refusal.serial);
            self.toast = Some(refusal);
        }
    }

    /// The refusal that waited for the held answer's toast, now that it has gone.
    fn show_waiting_refusal(&mut self) {
        if let Some(refusal) = self.refusal_waiting.take() {
            self.show_refusal(refusal);
        }
    }

    /// The server took `answer` (`Ok`) or refused it, saying why; its answer came at `landed`.
    /// A refusal brings the task back and shows the reason (once no answer is held,
    /// [`Answers::refusal_to_time`]).
    pub(crate) fn sent(&mut self, answer: &Answer, outcome: Result<(), String>, landed: Instant) {
        let task = answer.task;
        self.sending.remove(&task);
        let acted = self.acting.remove(&task);
        match outcome {
            Ok(()) => {
                if acted {
                    self.taken.insert(task, landed);
                }
                if self
                    .toast
                    .as_ref()
                    .is_some_and(|toast| toast.task == task && !toast.failed())
                {
                    self.toast = None;
                }
            }
            Err(reason) => {
                log::warn!("what was done to task {task} wasn't sent: {reason}");
                self.gone.remove(&task);
                self.serial += 1;
                self.show_refusal(Toast {
                    serial: self.serial,
                    task,
                    text: format!("{}: {reason}", answer.deed.failure()),
                    closes: false,
                    phase: Phase::Refused,
                });
            }
        }
    }

    /// The refusal toast `serial` has been shown long enough: it goes, unless another took
    /// its place.
    pub(crate) fn expire(&mut self, serial: u64) {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| toast.serial == serial && toast.failed())
        {
            self.toast = None;
        }
    }

    /// Hide: a refusal's toast goes now (an answer's stays, its Undo still counts).
    pub(crate) fn hide(&mut self) {
        if self.toast.as_ref().is_some_and(Toast::failed) {
            self.toast = None;
        }
    }

    /// A new state, asked for at `asked`: answered tasks the server no longer has waiting are
    /// done with; those it still has waiting stay off the lists until it catches up. Acted-on
    /// tasks stay gone while their action is held or on its way, and come back with the first
    /// state asked for once the server took it: that one shows them as they now are, while one
    /// asked before would show them as they were, for a poll.
    pub(crate) fn settle(&mut self, asked: Instant, still_waiting: impl Fn(TaskId) -> bool) {
        let held = self.held.as_ref().map(|answer| answer.task);
        let (acting, taken) = (&self.acting, &self.taken);
        self.gone.retain(|&task| {
            Some(task) == held
                || acting.contains(&task)
                || match taken.get(&task) {
                    Some(&landed) => asked < landed,
                    None => still_waiting(task),
                }
        });
        self.taken.retain(|_, &mut landed| asked < landed);
    }
}

/// What the toast says for an answer.
fn toast_text(choice: &str) -> String {
    let lower = choice.to_lowercase();
    if ["leave it", "not now", "no"].contains(&lower.as_str()) {
        format!("{choice}: task closed")
    } else if choice == "Done" {
        "Marked done".to_owned()
    } else if lower == "approve" {
        "Approved".to_owned()
    } else {
        format!("Answered: {choice}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: TaskId = TaskId::new(1);
    const TWO: TaskId = TaskId::new(2);

    fn answer(task: TaskId, choice: &str) -> Answer {
        Answer {
            task,
            deed: Deed::Choice(choice.to_owned()),
        }
    }

    #[test]
    fn an_answer_is_held_with_undo_and_its_task_leaves_the_lists() {
        let mut answers = Answers::default();
        assert_eq!(
            answers.answer(ONE, "Yes".into()),
            Answered::Held { send_now: None }
        );
        assert!(answers.gone().contains(&ONE));
        assert_eq!(
            answers.toast(),
            Some(&Toast {
                serial: 1,
                task: ONE,
                text: "Answered: Yes".into(),
                closes: false,
                phase: Phase::Held,
            })
        );
    }

    #[test]
    fn a_second_answer_to_the_same_task_is_ignored() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        assert_eq!(answers.answer(ONE, "No".into()), Answered::Already);
        assert_eq!(answers.flush(), Some(answer(ONE, "Yes")));
    }

    #[test]
    fn a_new_answer_sends_the_held_one_at_once() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        assert_eq!(
            answers.answer(TWO, "No".into()),
            Answered::Held {
                send_now: Some(answer(ONE, "Yes"))
            }
        );
        assert_eq!(answers.toast().map(|t| t.task), Some(TWO));
        assert!(answers.gone().contains(&ONE) && answers.gone().contains(&TWO));
    }

    #[test]
    fn the_window_running_out_hands_the_answer_over_once() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        assert_eq!(answers.due(ONE), Some(answer(ONE, "Yes")));
        assert_eq!(answers.toast().map(Toast::undoable), Some(false));
        assert_eq!(answers.due(ONE), None);
        assert_eq!(answers.flush(), None);
    }

    #[test]
    fn an_old_timer_doesnt_send_a_newer_answer() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        answers.answer(TWO, "No".into());
        assert_eq!(answers.due(ONE), None);
        assert_eq!(answers.due(TWO), Some(answer(TWO, "No")));
    }

    #[test]
    fn leaving_flushes_the_held_answer_once() {
        let mut answers = Answers::default();
        assert_eq!(answers.flush(), None, "nothing held");
        answers.answer(ONE, "Yes".into());
        assert_eq!(answers.flush(), Some(answer(ONE, "Yes")));
        assert_eq!(answers.toast().map(Toast::undoable), Some(false));
        assert_eq!(answers.flush(), None, "a second flush sends nothing");
        assert_eq!(answers.due(ONE), None, "nor does its window running out");
        assert!(answers.gone().contains(&ONE));
    }

    #[test]
    fn an_undone_answer_is_never_flushed() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        answers.undo(1);
        assert_eq!(answers.flush(), None);
        assert!(answers.gone().is_empty());
    }

    #[test]
    fn undo_takes_a_held_answer_back() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        answers.undo(1);
        assert!(answers.gone().is_empty());
        assert_eq!(answers.toast(), None);
        assert_eq!(answers.due(ONE), None, "nothing left to send");
    }

    #[test]
    fn undo_after_sending_only_hides_the_toast() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        answers.flush();
        answers.undo(1);
        assert!(
            answers.gone().contains(&ONE),
            "it is on its way: it stays answered"
        );
        assert_eq!(answers.toast(), None);
    }

    #[test]
    fn a_refused_answer_brings_the_task_back() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        assert_eq!(answers.toast().map(|t| t.phase), Some(Phase::Held));
        answers.flush();
        assert_eq!(
            answers.toast().map(|t| t.phase),
            Some(Phase::Sending),
            "on its way: a spinner, no Undo"
        );
        answers.sent(
            &answer(ONE, "Yes"),
            Err("Task 1 is done already".into()),
            Instant::now(),
        );
        assert!(!answers.gone().contains(&ONE));
        assert_eq!(
            answers.toast(),
            Some(&Toast {
                serial: 2,
                task: ONE,
                text: "Answer not sent: Task 1 is done already".into(),
                closes: false,
                phase: Phase::Refused,
            })
        );
        assert_eq!(answers.refusal_to_time(), Some(2));
        assert_eq!(answers.refusal_to_time(), None, "timed once");
    }

    #[test]
    fn a_refusal_goes_by_itself_or_on_hide() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        answers.flush();
        answers.sent(&answer(ONE, "Yes"), Err("no".into()), Instant::now());
        let first = answers.refusal_to_time().unwrap();
        answers.hide();
        assert_eq!(answers.toast(), None, "Hide puts it away");

        answers.answer(TWO, "Yes".into());
        answers.hide();
        assert!(answers.toast().is_some(), "Hide leaves an answer's toast");
        answers.flush();
        answers.sent(&answer(TWO, "Yes"), Err("no".into()), Instant::now());
        let second = answers.refusal_to_time().unwrap();
        answers.expire(first);
        assert!(answers.toast().is_some(), "an older timer leaves it");
        answers.expire(second);
        assert_eq!(answers.toast(), None, "its own timer puts it away");
    }

    #[test]
    fn a_refusal_waits_while_an_answer_is_held_and_undo_keeps_to_it() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        let Answered::Held {
            send_now: Some(first),
        } = answers.answer(TWO, "No".into())
        else {
            panic!("the first answer goes now")
        };
        let held = answers.toast().unwrap().serial;
        answers.sent(&first, Err("Task 1 is done already".into()), Instant::now());
        let toast = answers.toast().cloned().unwrap();
        assert_eq!(
            (toast.task, toast.undoable()),
            (TWO, true),
            "the held answer's toast stays"
        );
        assert_eq!(answers.refusal_to_time(), None, "not shown yet");
        assert!(!answers.gone().contains(&ONE), "the refused task is back");
        answers.undo(held - 1);
        assert!(
            answers.gone().contains(&TWO),
            "an older toast's Undo does nothing"
        );
        answers.undo(held);
        assert!(
            !answers.gone().contains(&TWO),
            "Undo takes back the held answer"
        );
        assert_eq!(answers.flush(), None, "nothing left to send");
        let toast = answers.toast().cloned().unwrap();
        assert_eq!(
            (toast.task, toast.failed()),
            (ONE, true),
            "the refusal shows now"
        );
        assert_eq!(answers.refusal_to_time(), Some(toast.serial));
    }

    #[test]
    fn a_waiting_refusal_shows_once_the_held_answer_goes() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        answers.answer(TWO, "No".into());
        answers.sent(&answer(ONE, "Yes"), Err("no".into()), Instant::now());
        answers.answer(TaskId::new(3), "Ok".into());
        assert!(
            answers.toast().unwrap().undoable(),
            "a newer answer, still held"
        );
        assert_eq!(answers.refusal_to_time(), None);
        answers.flush();
        assert!(answers.toast().unwrap().failed(), "shown once it is sent");
        assert!(answers.refusal_to_time().is_some());
    }

    #[test]
    fn a_closing_answer_says_so() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Not now".into());
        assert_eq!(answers.toast().map(|t| t.closes), Some(true));
    }

    #[test]
    fn a_taken_answer_clears_its_toast_only() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        let Answered::Held {
            send_now: Some(first),
        } = answers.answer(TWO, "No".into())
        else {
            panic!("the first answer goes now")
        };
        answers.sent(&first, Ok(()), Instant::now());
        assert_eq!(
            answers.toast().map(|t| t.task),
            Some(TWO),
            "the newer toast stays"
        );
        answers.flush();
        answers.sent(&answer(TWO, "No"), Ok(()), Instant::now());
        assert_eq!(answers.toast(), None);
    }

    #[test]
    fn answered_tasks_return_only_once_the_server_agrees() {
        let mut answers = Answers::default();
        answers.answer(ONE, "Yes".into());
        answers.flush();
        answers.answer(TWO, "Yes".into());
        answers.settle(Instant::now(), |task| task == ONE);
        assert!(
            answers.gone().contains(&ONE),
            "still waiting on the server: stays gone"
        );
        assert!(answers.gone().contains(&TWO), "held: stays gone");
        answers.flush();
        answers.settle(Instant::now(), |_| false);
        assert!(answers.gone().is_empty());
    }

    #[test]
    fn an_action_is_held_like_an_answer_and_words_its_toast() {
        let mut answers = Answers::default();
        answers.act(ONE, TaskAction::archive(ONE));
        assert!(answers.gone().contains(&ONE));
        assert_eq!(answers.toast().map(|t| t.text.as_str()), Some("Archived"));
        assert_eq!(
            answers.act(ONE, TaskAction::mark_done(ONE)),
            Answered::Already
        );
        let sent = answers.flush().unwrap();
        assert!(answers.is_sending(ONE));
        let asked_before = Instant::now();
        answers.settle(asked_before, |_| false);
        assert!(answers.gone().contains(&ONE), "on its way: stays gone");
        let landed = asked_before + Duration::from_secs(1);
        answers.sent(&sent, Ok(()), landed);
        assert!(!answers.is_sending(ONE));
        assert!(answers.gone().contains(&ONE), "until the next state");
        answers.settle(asked_before, |_| true);
        assert!(
            answers.gone().contains(&ONE),
            "a state asked for before the server took it shows it as it was: it stays gone"
        );
        answers.settle(landed + Duration::from_millis(1), |_| true);
        assert!(
            answers.gone().is_empty(),
            "the next state asked for after shows it as it is now"
        );
        answers.settle(asked_before, |_| true);
        assert!(answers.gone().is_empty(), "and that's done with");
    }

    #[test]
    fn a_refused_action_says_why() {
        let mut answers = Answers::default();
        answers.act(TWO, TaskAction::unarchive(TWO));
        let sent = answers.flush().unwrap();
        answers.sent(&sent, Err("no such task".into()), Instant::now());
        assert!(answers.gone().is_empty());
        assert_eq!(
            answers.toast().map(|t| t.text.as_str()),
            Some("Not unarchived: no such task")
        );
    }

    #[test]
    fn an_undone_action_brings_its_task_back() {
        let mut answers = Answers::default();
        answers.act(ONE, TaskAction::mark_done(ONE));
        answers.undo(answers.toast().unwrap().serial);
        assert!(answers.gone().is_empty());
        answers.settle(Instant::now(), |_| true);
        assert!(answers.gone().is_empty());
    }

    #[test]
    fn the_toast_words_the_answer() {
        assert_eq!(toast_text("Not now"), "Not now: task closed");
        assert_eq!(toast_text("no"), "no: task closed");
        assert_eq!(toast_text("Done"), "Marked done");
        assert_eq!(toast_text("Approve"), "Approved");
        assert_eq!(toast_text("Ship it"), "Answered: Ship it");
    }
}
