//! The final steps' Go ahead cards: a planned step and a button are the same step, and the
//! owner's history lines that are taps.

use emberhut_core::guard::go_ahead;

#[test]
fn a_plan_s_step_covers_a_button_with_the_same_words() {
    assert!(go_ahead::patterns_compile());
    assert!(go_ahead::matches("Create app", "Create"));
    assert!(go_ahead::matches("Install", "Install to Acme"));
    assert!(!go_ahead::matches("Create app", "Create API key"));
    assert!(!go_ahead::matches("", "Create"));
}

#[test]
fn a_tap_s_line_is_not_the_owner_s_words() {
    assert!(go_ahead::tapped("Chose: Go ahead"));
    assert!(go_ahead::tapped("Done"));
    assert!(go_ahead::tapped("Turned ultra mode on"));
    assert!(!go_ahead::tapped("yes, do it"));
    assert!(go_ahead::is_choice("Chose: Stop"));
}
