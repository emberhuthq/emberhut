//! A decision's options as cards: read from the wire, written back in the same shape.

use emberhut_api::ids::ArtifactId;
use emberhut_api::tasks::Decision;

use crate::support::assert_writes_shape;

#[test]
fn reads_and_writes_option_cards() {
    // `chat` (where it was asked) isn't modelled.
    let decision: Decision = assert_writes_shape("decision_choices.json", &["chat"]);
    let choices = decision.choices.as_deref().unwrap();
    assert_eq!(choices.len(), 3);
    assert_eq!(choices[0].title.as_deref(), Some("The small plan"));
    assert_eq!(choices[0].fields.as_ref().unwrap()[1].value, "a week");
    assert_eq!(choices[0].media.as_deref(), Some(&[ArtifactId::new(7)][..]));
    assert!(!choices[2].is_card(), "a plain answer beside the cards");
    assert_eq!(decision.card_count(), 2);
}

#[test]
fn a_decision_without_cards_counts_none() {
    let decision: Decision = serde_json::from_value(serde_json::json!({
        "question": "Yes or no?", "options": ["Yes", "No"]
    }))
    .unwrap();
    assert_eq!(decision.card_count(), 0);
    assert!(decision.choices.is_none());
}
