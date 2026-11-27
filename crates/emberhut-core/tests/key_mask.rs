//! Keys and tokens masked in a line and in a page's accessibility tree, and the sites keys are
//! never saved from.

use emberhut_core::guard::key_mask;

#[test]
fn tokens_are_masked_by_their_shape_and_the_page_s_values_first() {
    assert!(key_mask::patterns_compile());
    let slack = format!("xoxp-{}", "1234567890-abcdef");
    let line = format!("your token is {slack} now");
    assert_eq!(
        key_mask::mask(&line, &[], Some("e12")),
        "your token is [secret: xoxp-…, use browser_save_secret ref e12] now"
    );
    let openai = format!("sk-{}", "proj1234567890abcdefghij");
    assert_eq!(key_mask::mask(&openai, &[], None), "[secret: sk-…]");
    assert_eq!(
        key_mask::mask("pip install sk-learn-something-long-enough", &[], None),
        "pip install sk-learn-something-long-enough",
        "an sk- word with no digit stays"
    );
    let github = format!("ghp_{}", "a".repeat(36));
    assert!(key_mask::has_token(&github));
    let aws = format!("AKIA{}", "ABCDEFGHIJKLMNOP");
    assert_eq!(key_mask::mask(&aws, &[], None), "[secret: AKIA…]");
    let value = "q8Zr2VbN5mXy7LkP".to_owned();
    assert_eq!(
        key_mask::mask(
            &format!("value={value}"),
            &[value.clone(), "short".to_owned()],
            None
        ),
        "value=[secret: …]"
    );
}

#[test]
fn a_tree_s_masked_key_points_at_the_element_that_holds_it() {
    let token = format!("xoxb-{}", "99999999-zzzz");
    let tree = format!(
        "- form [ref=e3]\n  - textbox \"Token\" [ref=e7]: {token}\n  - text: {token}\n\n- text: {token}"
    );
    let masked = key_mask::mask_tree(&tree, &[]);
    let lines: Vec<&str> = masked.lines().collect();
    assert!(
        lines[1].ends_with("[secret: xoxb-…, use browser_save_secret ref e7]"),
        "{masked}"
    );
    assert!(
        lines[2].ends_with("[secret: xoxb-…, use browser_save_secret ref e3]"),
        "{masked}"
    );
    assert!(lines[4].ends_with("[secret: xoxb-…]"), "{masked}");
}

#[test]
fn password_and_account_sites_are_blocked() {
    let got: Vec<bool> = [
        "https://my.1password.com/vaults",
        "https://passwords.google.com/",
        "https://appleid.apple.com/",
        "https://api.slack.com/apps",
        "https://myaccount.google.com/security/2fa",
        "https://myaccount.google.com/profile",
        "file:///etc/passwd",
        "not a url",
    ]
    .iter()
    .map(|url| key_mask::blocked_site(url))
    .collect();
    assert_eq!(got, [true, true, true, false, true, false, true, true]);
}
