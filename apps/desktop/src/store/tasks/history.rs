//! A task's history on its page: the user's last message on it and what came back (shown over
//! the reply box), what happened since the assistant asked (so an outdated ask shows itself),
//! and where a message on the task went.

use emberhut_api::task_message::{MessageTo, TaskMessageReply};
use emberhut_api::task_page::Moment;
use emberhut_api::tasks::Decision;

use super::clock;
use crate::brand::APP_NAME;
use crate::store::lookup;

/// Whether a step of the history is the user's (a step with no one named is not).
#[must_use]
pub(crate) fn is_mine(moment: &Moment) -> bool {
    moment.who == lookup::PERSON_OWNER
}

/// Whether a step says anything: one the server has no text for is left out wherever the
/// history shows.
#[must_use]
fn says_something(moment: &Moment) -> bool {
    !moment.text.trim().is_empty()
}

/// The steps of the history that show, oldest first.
#[must_use]
pub(crate) fn shown(history: &[Moment]) -> Vec<&Moment> {
    history
        .iter()
        .filter(|moment| says_something(moment))
        .collect()
}

/// The user's last message on the task and what followed it, at most four steps, without the
/// steps that only change its state.
#[must_use]
pub(crate) fn exchange(history: &[Moment]) -> Vec<&Moment> {
    let quiet = |moment: &Moment| !says_something(moment) || lookup::is_quiet_step(&moment.text);
    let Some(from) = history.iter().rposition(|moment| {
        is_mine(moment) && !quiet(moment) && !moment.text.starts_with("Chose: ")
    }) else {
        return Vec::new();
    };
    history[from..]
        .iter()
        .filter(|moment| !quiet(moment))
        .take(4)
        .collect()
}

/// What happened since the assistant asked `decision`, the last four steps, not the user's,
/// nor the ask itself or its re-checks.
#[must_use]
pub(crate) fn since_asked<'a>(history: &'a [Moment], decision: &Decision) -> Vec<&'a Moment> {
    // The ask's own note lands a moment after it.
    let Some(asked) = decision.asked.as_ref().and_then(clock::instant) else {
        return Vec::new();
    };
    let from = asked.as_second() + 5;
    let since: Vec<&Moment> = history
        .iter()
        .filter(|moment| {
            clock::instant(&moment.at).is_some_and(|at| at.as_second() > from)
                && says_something(moment)
                && moment.text != decision.question
                && !moment.text.starts_with("Re-checked")
                && !is_mine(moment)
        })
        .collect();
    let skip = since.len().saturating_sub(4);
    since.into_iter().skip(skip).collect()
}

/// How many steps happened since the ask, all of them counted.
#[must_use]
pub(crate) fn since_asked_count(history: &[Moment], decision: &Decision) -> usize {
    let Some(asked) = decision.asked.as_ref().and_then(clock::instant) else {
        return 0;
    };
    history
        .iter()
        .filter(|moment| {
            clock::instant(&moment.at).is_some_and(|at| at.as_second() > asked.as_second() + 5)
                && says_something(moment)
                && moment.text != decision.question
                && !moment.text.starts_with("Re-checked")
                && !is_mine(moment)
        })
        .count()
}

/// Where a message on the task went, as the note under the box says it.
#[must_use]
pub(crate) fn sent_note(reply: &TaskMessageReply) -> String {
    let queued = reply.queued == Some(true);
    match (&reply.to, queued) {
        (MessageTo::Worker, true) => {
            format!("Sent. {APP_NAME}'s worker reads it when this run ends.")
        }
        (MessageTo::Worker, false) => format!("Sent. {APP_NAME}'s worker picks it up now."),
        (MessageTo::Session, true) => {
            "Sent. The session takes it after the turn it's on.".to_owned()
        }
        (MessageTo::Session, false) => "Sent to the task's session as its next run.".to_owned(),
        _ => format!("Sent. {APP_NAME} answers here on the task."),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::wire::ASSISTANT_OWNER;

    fn moments(steps: &[(&str, &str, &str)]) -> Vec<Moment> {
        steps
            .iter()
            .map(|&(at, who, text)| Moment {
                at: emberhut_api::time::Timestamp::new(at),
                who: who.to_owned(),
                text: text.to_owned(),
            })
            .collect()
    }

    #[test]
    fn the_exchange_starts_at_the_users_last_message() {
        let me = lookup::PERSON_OWNER;
        let history = moments(&[
            ("2026-11-25T08:00:00Z", me, "First"),
            ("2026-11-25T08:01:00Z", ASSISTANT_OWNER, "An answer"),
            ("2026-11-25T08:02:00Z", me, "Can it be shorter?"),
            ("2026-11-25T08:03:00Z", ASSISTANT_OWNER, "Started"),
            ("2026-11-25T08:04:00Z", "worker", "Shortened it"),
            ("2026-11-25T08:05:00Z", me, "Chose: Report A"),
        ]);
        let texts: Vec<&str> = exchange(&history).iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            ["Can it be shorter?", "Shortened it", "Chose: Report A"]
        );
        assert_eq!(exchange(&history[1..2]), Vec::<&Moment>::new());
    }

    #[test]
    fn since_the_ask_leaves_out_the_ask_and_the_users_own_steps() {
        let decision: Decision = serde_json::from_value(json!({
            "question": "Which one?", "asked": "2026-11-25T08:00:00Z"}))
        .unwrap();
        let history = moments(&[
            ("2026-11-25T07:00:00Z", ASSISTANT_OWNER, "Before"),
            ("2026-11-25T08:00:02Z", ASSISTANT_OWNER, "Which one?"),
            (
                "2026-11-25T09:00:00Z",
                ASSISTANT_OWNER,
                "Re-checked: still holds",
            ),
            ("2026-11-25T10:00:00Z", ASSISTANT_OWNER, "The price changed"),
            ("2026-11-25T10:10:00Z", "", " "),
            ("2026-11-25T10:30:00Z", lookup::PERSON_OWNER, "Noted"),
        ]);
        let since: Vec<&str> = since_asked(&history, &decision)
            .iter()
            .map(|m| m.text.as_str())
            .collect();
        assert_eq!(since, ["The price changed"]);
        assert_eq!(since_asked_count(&history, &decision), 1);
    }

    #[test]
    fn a_step_with_no_one_and_nothing_said_doesnt_show() {
        let me = lookup::PERSON_OWNER;
        let history: Vec<Moment> = serde_json::from_value(json!([
            {"at": "2026-11-25T08:00:00Z", "who": me, "text": "Can it be shorter?"},
            {"at": "2026-11-25T08:01:00Z", "who": null, "text": null},
            {"at": "2026-11-25T08:02:00Z"},
            {"at": "2026-11-25T08:03:00Z", "who": "", "text": "Shortened it"},
        ]))
        .unwrap();
        assert!(!is_mine(&history[1]) && !is_mine(&history[3]));
        let shown: Vec<&str> = shown(&history).iter().map(|m| m.text.as_str()).collect();
        assert_eq!(shown, ["Can it be shorter?", "Shortened it"]);
        let texts: Vec<&str> = exchange(&history).iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["Can it be shorter?", "Shortened it"]);
    }

    #[test]
    fn the_note_says_where_a_message_went() {
        let reply = |to: &str, queued: bool| -> TaskMessageReply {
            serde_json::from_value(json!({"ok": true, "to": to, "queued": queued})).unwrap()
        };
        assert_eq!(
            sent_note(&reply("worker", false)),
            format!("Sent. {APP_NAME}'s worker picks it up now.")
        );
        assert_eq!(
            sent_note(&reply("session", true)),
            "Sent. The session takes it after the turn it's on."
        );
        assert_eq!(
            sent_note(&reply(ASSISTANT_OWNER, false)),
            format!("Sent. {APP_NAME} answers here on the task.")
        );
    }
}
