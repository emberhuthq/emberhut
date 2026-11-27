//! The setup checklist.

use emberhut_api::setup::{SetupItem, SetupItemId, SetupPage, SetupState};
use serde_json::json;

use crate::support::{assert_round_trip, fixture, from_value, to_value, with_unknown_fields};

#[test]
fn reads_and_writes_the_checklist() {
    let page: SetupPage = assert_round_trip("setup.json");
    assert_eq!(page.items.len(), 10);
    let public = page.item(&SetupItemId::PublicUrl).unwrap();
    assert_eq!(public.state, SetupState::Ready);
    assert_eq!(public.detail.as_deref(), Some("https://box.example"));
    let whatsapp = page.item(&SetupItemId::WhatsApp).unwrap();
    assert_eq!(whatsapp.state, SetupState::Off);
    assert_eq!(whatsapp.detail, None, "a missing detail is left out");
}

#[test]
fn the_checklist_keeps_words_it_does_not_know_and_tolerates_unknown_fields() {
    let mut body = fixture("setup.json");
    with_unknown_fields(&mut body);
    assert_eq!(
        from_value::<SetupPage>(body),
        from_value::<SetupPage>(fixture("setup.json"))
    );
    let newer: SetupPage = from_value(json!({ "items": [
        { "id": "calendar", "state": "checking", "detail": "Asking the calendar." },
    ] }));
    let item = &newer.items[0];
    assert_eq!(item.id, SetupItemId::from("calendar"));
    assert!(!item.id.is_known());
    assert!(!item.state.is_known());
    assert_eq!(
        to_value(&newer),
        json!({ "items": [
            { "id": "calendar", "state": "checking", "detail": "Asking the calendar." },
        ] }),
        "written back as it came"
    );
    assert_eq!(
        to_value(&SetupItem::new(
            SetupItemId::Mac,
            SetupState::Error,
            "Off the network."
        )),
        json!({ "id": "mac", "state": "error", "detail": "Off the network." })
    );
}
