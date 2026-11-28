//! The theme page, its changes and their answers.

use emberhut_api::theme::{AccentChange, Appearance, ThemeChange, ThemePage, ThemeSetReply};

use crate::support::{assert_round_trip, fixture, from_value, with_unknown_fields};

#[test]
fn the_theme_page_round_trips() {
    let page: ThemePage = assert_round_trip("theme_page.json");
    assert_eq!(page.version, 1);
    assert_eq!(page.choice.appearance, Appearance::Dark);
    assert_eq!(page.tokens.light.accent_soft, "#E5ECFD");
    assert!(page.contrast.aa);
    assert_eq!(page.names.len(), 14);
    assert_eq!(page.choice.ember_tint, "amber");
    assert_eq!(page.tints[0].id, "ember", "the default first");
}

#[test]
fn the_theme_page_tolerates_unknown_fields() {
    let mut body = fixture("theme_page.json");
    with_unknown_fields(&mut body);
    assert_eq!(
        from_value::<ThemePage>(body),
        from_value::<ThemePage>(fixture("theme_page.json"))
    );
}

#[test]
fn a_theme_change_round_trips() {
    let change: ThemeChange = assert_round_trip("theme_change.json");
    assert_eq!(change.accent, AccentChange::Reset);
    assert_eq!(change.ember_tint.as_deref(), Some("sage"));
    assert!(change.preview);
    let reply: ThemeSetReply = assert_round_trip("theme_set_reply.json");
    assert!(reply.ok);
    assert_eq!(
        reply.page,
        from_value::<ThemePage>(fixture("theme_page.json"))
    );
}

#[test]
fn a_presets_colours_come_from_the_page() {
    let page: ThemePage = from_value(fixture("theme_page.json"));
    let graphite = page.preset_tokens("graphite").unwrap();
    assert_eq!(graphite.light.accent, "#374151");
    assert_eq!(graphite.dark.accent, "#D1D5DB");
    let paper = page.preset_tokens("paper").unwrap();
    assert_eq!(
        (paper.light.accent.as_str(), paper.light.background.as_str()),
        ("#B83D0C", "#F6F0E6"),
        "the default: ember on kraft paper"
    );
    assert_eq!(paper.dark.background, "#15110E");
    assert_eq!(
        page.preset_tokens("blue").unwrap(),
        page.tokens,
        "the page's own choice is blue, uncustomised"
    );
    assert!(page.preset_tokens("nowhere").is_none());
}
