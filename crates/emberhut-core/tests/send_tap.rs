//! A card's send-type answers: the ones whose tap sends the message there and then.

use emberhut_core::guard::send_tap;

#[test]
fn a_send_answer_sends_and_the_others_do_not() {
    assert!(send_tap::patterns_compile());
    for yes in [
        "Send it",
        "Send",
        "Send to Sam",
        "Yes, send it",
        "Post it",
        "ابعت",
        "ارسلها",
        "send it now",
    ] {
        assert!(send_tap::sends_it(yes), "{yes}");
    }
    for no in [
        "The assistant sends it",
        "Don't send",
        "Send it later",
        "Edit and send",
        "Send it to Alex instead",
        "Cancel",
        "Leave it",
        "",
    ] {
        assert!(!send_tap::sends_it(no), "{no}");
    }
}
