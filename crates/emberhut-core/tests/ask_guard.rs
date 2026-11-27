//! The ask guard's rules: what always reaches the owner, a reply that asks, and the judge's
//! answer checked against the owner's real words.

use emberhut_core::guard::ask_guard::{self, Ask, JudgeAnswer, Line};
use emberhut_core::task::wire::KEY_PREFIX;

/// Task `n`'s key, as the tasks spell it.
fn key(n: u32) -> String {
    format!("{KEY_PREFIX}{n}")
}

fn ask(text: &str) -> Ask {
    Ask {
        text: text.to_owned(),
        ..Ask::default()
    }
}

#[test]
fn some_asks_always_reach_the_owner() {
    assert!(ask_guard::patterns_compile());
    let draft = Ask {
        draft: Some("hey Sam, it's live".to_owned()),
        ..ask("Send Sam the update?")
    };
    assert_eq!(
        ask_guard::reserved(&draft),
        Some("a message or post as the owner")
    );
    for hard in [
        "Pay the $99 developer fee?",
        "Turn on two-factor for the Acme Google account?",
    ] {
        assert_eq!(
            ask_guard::reserved(&ask(hard)),
            Some("payments, passwords, codes or account security"),
            "{hard}"
        );
    }
    for delete in [
        "Delete the old renders from Drive?",
        "Remove the 3 old backup folders?",
    ] {
        assert_eq!(
            ask_guard::reserved(&ask(delete)),
            Some("deleting the owner's data"),
            "{delete}"
        );
    }
    for ordinary in [
        "Set the Manage Bids step on Search Ads?",
        "The voices code is pushed; the subscription screen in order to show $4?",
        "Daily budget: $5 or $10?",
    ] {
        assert_eq!(ask_guard::reserved(&ask(ordinary)), None, "{ordinary}");
    }
    assert!(ask_guard::is_merge("Say \"merge it\" for PR #675"));
    assert!(ask_guard::is_merge("Want me to push it to main?"));
    assert!(!ask_guard::is_merge("Want me to put it on production?"));
}

#[test]
fn a_reply_ending_on_a_request_for_the_ok_is_asking() {
    assert!(ask_guard::asking(
        "The production hotfix is ready and only needs your yes. Want me to put it on production?"
    ));
    assert!(ask_guard::asking(
        "The voices code is pushed and its checks pass. Say \"merge it\" for PR #675 and I'll merge it."
    ));
    assert!(ask_guard::asking(
        "جهزت الإعلان والميزانية ٥ دولار باليوم. تبيني أشغله؟"
    ));
    assert!(!ask_guard::asking(
        "Done: the hotfix is live on production."
    ));
    assert!(!ask_guard::asking(
        "Which date is the exam, the 24th or the 25th?"
    ));
}

fn lines() -> Vec<Line> {
    vec![
        Line {
            tag: "m4".to_owned(),
            text: format!("The owner's request for {} (Main, 09:30): run the ads on the store, do it end to end", key(7)),
            said: "run the ads on the store, do it end to end".to_owned(),
        },
        Line {
            tag: "m9".to_owned(),
            text: "The owner (Acme, 10:02): 5 a day is fine\n   (The assistant before it: What daily budget, $5 or $10?)".to_owned(),
            said: "5 a day is fine".to_owned(),
        },
        Line {
            tag: "m11".to_owned(),
            text: "The owner (Acme, 10:05): merge the voices PR and deploy it".to_owned(),
            said: "merge the voices PR and deploy it".to_owned(),
        },
    ]
}

fn judge(quote: &str, tag: &str, answer: Option<&str>) -> JudgeAnswer {
    JudgeAnswer {
        covered: true,
        quote: Some(quote.to_owned()),
        tag: Some(tag.to_owned()),
        answer: answer.map(str::to_owned),
        why: "because".to_owned(),
        ..JudgeAnswer::default()
    }
}

#[test]
fn a_covered_ask_is_sent_back_with_the_owner_s_own_words() {
    let v = ask_guard::check(
        judge("do it end to end", "m4", None),
        &ask("Set the bid?"),
        &lines(),
    );
    assert!(v.covered);
    assert!(
        ask_guard::sent_back(&v).starts_with(&format!(
            "Not asked: covered, do it. The owner's own words already OK this: «do it end to end» (their request for {} at 09:30 in Main)",
            key(7)
        )),
        "{}",
        ask_guard::sent_back(&v)
    );
    let budget = Ask {
        options: vec!["$5 a day".to_owned(), "$10 a day".to_owned()],
        ..ask("Pick the daily budget")
    };
    let v = ask_guard::check(
        judge("5 a day is fine", "m0", Some("$5 a day")),
        &budget,
        &lines(),
    );
    assert_eq!(v.place.as_deref(), Some("their message at 10:02 in Acme"));
    assert!(ask_guard::sent_back(&v).contains("Their pick: \"$5 a day\""));
}

#[test]
fn a_quote_that_is_not_theirs_or_a_merge_they_did_not_name_reaches_the_owner() {
    let v = ask_guard::check(
        judge("they said yes to everything", "m4", None),
        &ask("Run the ads?"),
        &lines(),
    );
    assert!(!v.covered, "a made-up quote");
    let v = ask_guard::check(
        judge("What daily budget, $5 or $10?", "m9", None),
        &ask("Budget?"),
        &lines(),
    );
    assert!(
        !v.covered,
        "the assistant's line is context, not the owner's OK"
    );
    let merge = ask("Say \"merge it\" for PR #675");
    assert!(!ask_guard::check(judge("do it end to end", "m4", None), &merge, &lines()).covered);
    assert!(
        ask_guard::check(
            judge("merge the voices PR and deploy it", "m0", None),
            &merge,
            &lines()
        )
        .covered
    );
    let reserved = JudgeAnswer {
        reserved: Some("message or post as the owner".to_owned()),
        ..judge("do it end to end", "m4", None)
    };
    let v = ask_guard::check(reserved, &ask("Post the ad?"), &lines());
    assert!(!v.covered && v.reserved.is_some());
    assert_eq!(ask_guard::outcome(Some(&v), false), "reserved");
    assert_eq!(ask_guard::outcome(None, false), "no_verdict");
    assert_eq!(ask_guard::outcome(None, true), "overridden");
}

#[test]
fn the_judge_reads_the_ask_and_every_line_with_its_tag() {
    let p = ask_guard::prompt(
        &ask("Set the bid?"),
        &format!("{}: Store ads", key(7)),
        "Main",
        &lines(),
        "today 10:30",
    );
    assert!(p.contains(&format!("# The ask\n{}: Store ads\nSet the bid?", key(7))));
    assert!(p.contains("[m9] The owner (Acme, 10:02): 5 a day is fine"));
    assert!(p.contains("# Evidence (now: today 10:30)"));
}
