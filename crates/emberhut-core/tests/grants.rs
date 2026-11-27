//! The owner's OK in one model: what a step is, a text's key, what a tap on a card grants, and
//! the words of a go, a yes and a stop.

use emberhut_core::guard::go;
use emberhut_core::guard::grants::{self, StepKind, Tap};
use emberhut_core::task::wire::{ASSISTANT, KEY_PREFIX};

#[test]
fn a_step_is_hard_said_or_ordinary() {
    assert!(grants::patterns_compile());
    assert_eq!(grants::kind("Pay now", ""), StepKind::Hard);
    assert_eq!(
        grants::kind("Continue", "https://example.com/checkout"),
        StepKind::Hard
    );
    assert_eq!(grants::kind("Revoke", ""), StepKind::Hard);
    assert_eq!(grants::kind("Send", ""), StepKind::Said);
    assert_eq!(
        grants::kind("Continue", "https://mail.google.com/mail/u/0"),
        StepKind::Said
    );
    assert_eq!(
        grants::kind("Publish", "https://play.google.com/console/app/1"),
        StepKind::Ordinary,
        "an app's release on its console"
    );
    assert_eq!(grants::kind("Create app", ""), StepKind::Ordinary);
    assert_eq!(
        grants::kind(
            "Subscribe to bot events",
            "https://api.slack.com/apps/A0123ABC/event-subscriptions"
        ),
        StepKind::Ordinary,
        "an app subscribed to events is setup, not a purchase"
    );
    assert!(grants::opener("Reply all"));
    assert!(!grants::opener("Send"));
    assert!(grants::on_request("Save", ""));
}

#[test]
fn a_text_s_key_ignores_spacing_and_curly_quotes() {
    assert_eq!(
        grants::text_key("it’s  live\n“now”"),
        grants::text_key("it's live \"now\"")
    );
    assert_ne!(
        grants::text_key("it's live"),
        grants::text_key("it's live!")
    );
}

#[test]
fn a_tap_grants_the_job_approves_its_text_or_stops_it() {
    assert_eq!(
        grants::heard_tap("Go ahead", None, None, None),
        Tap::Nothing,
        "no card open"
    );
    assert_eq!(
        grants::heard_tap("Go ahead", Some("OK the job?"), None, None),
        Tap::Approve {
            text: None,
            post: false,
            job: true
        }
    );
    assert_eq!(
        grants::heard_tap("Send it", Some("Send the reply?"), Some("hi Sam"), None),
        Tap::Approve {
            text: Some("hi Sam".to_owned()),
            post: false,
            job: true
        }
    );
    assert_eq!(
        grants::heard_tap("Go ahead", Some("Pay $5?"), None, Some(StepKind::Hard)),
        Tap::Approve {
            text: None,
            post: false,
            job: false
        },
        "a hard step's Go ahead is that step's only"
    );
    assert_eq!(
        grants::heard_tap("Post it", Some("Post the video?"), None, None),
        Tap::Approve {
            text: None,
            post: true,
            job: true
        }
    );
    assert_eq!(
        grants::heard_tap("Stop", Some("OK the job?"), None, None),
        Tap::StopJob
    );
    assert_eq!(
        grants::heard_tap("Stop", Some("Send it?"), Some("hi"), None),
        Tap::NoToStep,
        "a message's Stop is a no to that text"
    );
    assert_eq!(
        grants::heard_tap("Make it smaller", Some("Plan?"), None, None),
        Tap::Nothing
    );
}

#[test]
fn a_go_is_the_whole_reply_and_nothing_that_asks_waits_or_changes() {
    assert!(go::patterns_compile());
    for yes in [
        "send it",
        "Send",
        "do it",
        "yes send",
        "Yes, send it!",
        "go ahead",
        "ok go ahead 👍",
        "send it please",
        "ابعت",
        "ابعتها",
        "ارسل",
        "أرسلها",
        "تمام ابعتلو",
        "perfect, send it now",
        "sam acme , he is the same person , but on slack do it",
        "just send it",
        "just do it",
        "yes please do it",
        "okay perfect lets do it",
        "yes perfect go ahead",
        "go ahead and send it",
        "approve and send it",
        "Looks good, send it",
        format!("{KEY_PREFIX}12 go ahead").as_str(),
        format!("send it {ASSISTANT}").as_str(),
        "طيب ابعتها",
        "خلص ابعتها",
        "on slack do it",
    ] {
        assert!(go::is_go(yes), "{yes}");
    }
    for no in [
        "yes",
        "ok",
        "send it?",
        "did you send it",
        "don't send it",
        "do not send it",
        "wait, send it later",
        "change the first line and send it",
        "send it to Alex",
        "send it tomorrow",
        "no",
        "لا تبعت",
        "ليش ما بعتها",
        "what about the other one",
        "if he replies send it",
        "nice",
        "i pressed go ahead",
        "i tapped multiple times send it",
        "he told me to send it",
        "they said do it",
        "we will send",
        "Sam said go ahead",
        "Done: Tap Send it on the message card (Done on the task doesn't send it)",
        "Chrome is open, go ahead",
        "Chrome fixed, you do it",
        "the name is FlowKit, go ahead",
        "send it to Robin instead of me",
        "ok post it",
        "looks good",
        "approved",
        "Simple setup will be called FlowKit for multichannel setup in one go",
        "الاسم الجديد ابعتها",
        "قال ابعتها",
    ] {
        assert!(!go::is_go(no), "{no}");
    }
}

#[test]
fn a_stop_is_never_a_yes_and_a_yes_is_not_a_stop() {
    for stop in [
        "no",
        "stop",
        "wait",
        "hold on",
        "hold",
        "hold it",
        "hold the post",
        "pause it",
        "not now",
        "never mind",
        "don't post it",
        "do not post it",
        "nah",
        "لا",
        "لاء",
        "وقف",
        "وقفها",
        "أوقفها",
        "توقف",
        "استنى",
        "استنا",
        "انتظر",
        "بلاش",
        "لا تنشرها",
        "ما تنشرها",
        "ماتنشرها",
        "الغيها",
        "مش هلق",
        "مش هسا",
        "مو الحين",
    ] {
        assert!(go::is_stop(stop), "{stop}");
        assert!(!go::is_yes(stop), "a yes: {stop}");
        assert!(!go::is_go(stop), "a go: {stop}");
    }
    for said in [
        "ok",
        "yes",
        "approved",
        "go ahead",
        "post it",
        "looks good",
        "موافق",
        "انشرها",
        "تمام",
        "help me and Nora understand how can we make the app better",
    ] {
        assert!(!go::is_stop(said), "{said}");
    }
    for yes in [
        "ok post it",
        "looks good",
        "approved",
        "yes to all",
        "merge it",
        format!("{KEY_PREFIX}12 yes").as_str(),
    ] {
        assert!(go::is_yes(yes), "{yes}");
    }
    for no in [
        "ok thanks",
        "in one go",
        "good to go live next month",
        "ok so the name is X",
    ] {
        assert!(!go::is_yes(no), "{no}");
    }
    assert!(go::is_plain_stop("wait, hold it"));
    assert!(!go::is_plain_stop(
        "no need for a fee anymore since the plan covers all of it now"
    ));
}
