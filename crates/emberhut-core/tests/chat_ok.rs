//! The owner's go in the chat on an exact text, and their yes in the chat about a task.

use emberhut_core::guard::chat_ok::{self, ChatLine};
use emberhut_core::task::wire::KEY_PREFIX;
use jiff::{SignedDuration, Timestamp};

/// Task `n`'s key, as the tasks spell it.
fn key(n: u32) -> String {
    format!("{KEY_PREFIX}{n}")
}

fn line(id: i64, minutes: i64, mine: bool, text: &str) -> ChatLine {
    ChatLine {
        id,
        at: Timestamp::UNIX_EPOCH + SignedDuration::from_mins(1000 + minutes),
        text: text.to_owned(),
        mine,
        reply_to: None,
    }
}

fn at(minutes: i64) -> Timestamp {
    Timestamp::UNIX_EPOCH + SignedDuration::from_mins(1000 + minutes)
}

#[test]
fn the_text_shown_then_a_go_is_the_ok_for_that_text_and_that_person() {
    let lines = [
        line(
            1,
            0,
            false,
            "Here is the reply to Sam:\n> hey Sam, it’s live\nSend it?",
        ),
        line(2, 2, true, "send it"),
    ];
    let ok = chat_ok::find(
        &lines,
        "hey Sam, it's live",
        "slack:acme:Sam",
        false,
        None,
        &[],
        at(5),
    );
    assert_eq!(ok.map(|ok| ok.id), Some(2));
    assert!(
        chat_ok::find(
            &lines,
            "hey Sam, it's live",
            "slack:acme:Robin",
            false,
            None,
            &[],
            at(5)
        )
        .is_none(),
        "never an OK for another person"
    );
    assert!(
        chat_ok::find(
            &lines,
            "hey Sam, it's live",
            "slack:acme:Sam",
            false,
            None,
            &[2],
            at(5)
        )
        .is_none(),
        "spent already"
    );
    assert!(
        chat_ok::find(
            &lines,
            "hey Sam, it's live",
            "slack:acme:Sam",
            false,
            None,
            &[],
            at(40)
        )
        .is_none(),
        "used too late"
    );
    assert!(
        chat_ok::find(
            &lines,
            "something else",
            "slack:acme:Sam",
            false,
            None,
            &[],
            at(5)
        )
        .is_none(),
        "not the text shown"
    );
    let waited = [
        line(1, 0, false, "the text: ok"),
        line(2, 3, true, "wait, change it"),
    ];
    assert!(chat_ok::find(&waited, "ok", "x", true, None, &[], at(5)).is_none());
    let card = [line(5, 4, true, "go ahead")];
    assert_eq!(
        chat_ok::find(&card, "any", "x", false, Some(at(3)), &[], at(6)).map(|ok| ok.id),
        Some(5),
        "the next message after the card asked"
    );
}

#[test]
fn a_yes_about_a_task_names_it_or_answers_the_question_about_it() {
    let question = format!("Shall I set up {} now?", key(12));
    let asked = [line(1, 0, false, &question), line(2, 1, true, "yes")];
    assert_eq!(
        chat_ok::task_ok(&asked, &key(12), at(5)).map(|ok| ok.id),
        Some(2)
    );
    assert!(chat_ok::task_ok(&asked, &key(13), at(5)).is_none());
    let named = [line(3, 0, true, &format!("{} go ahead", key(12)))];
    assert_eq!(
        chat_ok::task_ok(&named, &key(12), at(5)).map(|ok| ok.id),
        Some(3)
    );
    let taken_back = [
        line(1, 0, false, &question),
        line(2, 1, true, "yes"),
        line(3, 2, true, "wait"),
    ];
    assert!(chat_ok::task_ok(&taken_back, &key(12), at(5)).is_none());
    assert!(chat_ok::names_key(&format!("see {}.", key(12)), &key(12)));
    assert!(!chat_ok::names_key(&format!("see {}", key(123)), &key(12)));
}

#[test]
fn quotes_and_line_breaks_do_not_matter_in_the_text() {
    assert_eq!(chat_ok::flat("> hey\n>  there “you”"), "hey there \"you\"");
    assert!(chat_ok::named("slack:acme:Sam Lee", "for sam"));
    assert!(!chat_ok::named("wa:12", "for sam"));
}
