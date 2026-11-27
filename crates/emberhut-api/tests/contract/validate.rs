//! The request types' validation rules (the `validate` feature).

use emberhut_api::answers::AnswerRequest;
use emberhut_api::auth::{PairRequest, RefreshRequest};
use emberhut_api::chat::HistoryQuery;
use emberhut_api::chrome::ChromeAction;
use emberhut_api::ids::{MessageId, RoomId, TaskId};
use emberhut_api::live::RoomQuery;
use emberhut_api::seen::SeenRequest;
use emberhut_api::settings::SettingsRequest;
use emberhut_api::switches::SwitchAction;
use emberhut_api::talk::TalkRequest;
use emberhut_api::theme::{AccentChange, ThemeChange};
use emberhut_api::uploads::UploadQuery;
use emberhut_api::wake::EventRequest;
use garde::Validate;
use serde_json::json;

use crate::support::{from_value, read};

/// The fixture with one field replaced fails validation.
fn fails<T: Validate<Context = ()> + serde::de::DeserializeOwned>(
    name: &str,
    field: &str,
    value: &serde_json::Value,
) {
    let mut body = crate::support::fixture(name);
    body[field] = value.clone();
    let request: T = from_value(body);
    assert!(
        request.validate().is_err(),
        "{name} with {field} = {value} should be refused"
    );
}

#[test]
fn the_fixtures_are_valid() {
    read::<PairRequest>("pair_request.json").validate().unwrap();
    read::<RefreshRequest>("refresh_request.json")
        .validate()
        .unwrap();
    read::<TalkRequest>("talk_request.json").validate().unwrap();
    read::<SeenRequest>("seen_request.json").validate().unwrap();
    read::<AnswerRequest>("answer_request.json")
        .validate()
        .unwrap();
    read::<ThemeChange>("theme_change.json").validate().unwrap();
    for name in ["settings_fact_set.json", "settings_task_set.json"] {
        read::<SettingsRequest>(name).validate().unwrap();
    }
    read::<SwitchAction>("switch_request.json")
        .validate()
        .unwrap();
    read::<ChromeAction>("chrome_request.json")
        .validate()
        .unwrap();
}

#[test]
fn a_change_names_what_it_changes() {
    fails::<SwitchAction>("switch_request.json", "name", &json!("  "));
    fails::<ChromeAction>("chrome_request.json", "label", &json!(""));
    fails::<ChromeAction>("chrome_request.json", "label", &json!("x".repeat(25)));
    fails::<ChromeAction>("chrome_request.json", "label", &json!("!!!"));
    fails::<ChromeAction>("chrome_request.json", "id", &json!(0));
    let mut fact = crate::support::fixture("settings_fact_set.json");
    fact["factSet"]["text"] = json!(" ");
    assert!(from_value::<SettingsRequest>(fact).validate().is_err());
}

#[test]
fn talk_needs_text_or_a_file() {
    TalkRequest::text(RoomId::MAIN, "Hello").validate().unwrap();
    assert!(TalkRequest::text(RoomId::MAIN, "  \n").validate().is_err());
    let mut with_file: TalkRequest = read("talk_request.json");
    with_file.text = String::new();
    with_file.validate().unwrap();
    fails::<TalkRequest>("talk_request.json", "room", &json!(0));
    fails::<TalkRequest>("talk_request.json", "replyTo", &json!(-1));
    fails::<TalkRequest>("talk_request.json", "text", &json!("x".repeat(100_001)));
}

#[test]
fn talk_files_are_the_servers_own() {
    let mut body = crate::support::fixture("talk_request.json");
    body["files"][0]["path"] = json!("/data/uploads/../secrets");
    assert!(from_value::<TalkRequest>(body.clone()).validate().is_err());
    body["files"][0].as_object_mut().unwrap().remove("path");
    assert!(
        from_value::<TalkRequest>(body).validate().is_err(),
        "a file sent back needs its path"
    );
}

#[test]
fn pairing_needs_a_code_and_short_names() {
    fails::<PairRequest>("pair_request.json", "code", &json!("   "));
    fails::<PairRequest>("pair_request.json", "code", &json!("A".repeat(33)));
    fails::<PairRequest>("pair_request.json", "install", &json!("i".repeat(65)));
    fails::<PairRequest>("pair_request.json", "name", &json!("n".repeat(61)));
    fails::<PairRequest>("pair_request.json", "model", &json!("m".repeat(41)));
    let mut body = crate::support::fixture("pair_request.json");
    body["name"] = json!("😀".repeat(30));
    from_value::<PairRequest>(body).validate().unwrap();
}

#[test]
fn a_refresh_needs_a_token() {
    fails::<RefreshRequest>("refresh_request.json", "refresh", &json!(""));
}

#[test]
fn rooms_and_tasks_are_positive_ids() {
    fails::<SeenRequest>("seen_request.json", "room", &json!(-3));
    fails::<AnswerRequest>("answer_request.json", "taskDecide", &json!(0));
    fails::<AnswerRequest>("answer_request.json", "choice", &json!(" "));
    assert!(
        HistoryQuery::before(RoomId::MAIN, MessageId::new(0))
            .validate()
            .is_err()
    );
    HistoryQuery::after(RoomId::new(2), MessageId::new(7))
        .validate()
        .unwrap();
    assert!(RoomQuery::new(RoomId::new(0)).validate().is_err());
    RoomQuery::default().validate().unwrap();
    AnswerRequest::new(TaskId::new(1), "Done")
        .validate()
        .unwrap();
}

#[test]
fn a_theme_change_is_checked() {
    fails::<ThemeChange>("theme_change.json", "appearance", &json!("sepia"));
    fails::<ThemeChange>("theme_change.json", "accent", &json!("blue"));
    fails::<ThemeChange>("theme_change.json", "preset", &json!(""));
    let change = ThemeChange {
        accent: AccentChange::Set("#36f".to_owned()),
        ..ThemeChange::default()
    };
    change.validate().unwrap();
}

#[test]
fn any_upload_name_is_taken() {
    UploadQuery::new("photo.png").validate().unwrap();
    let unnamed: UploadQuery = serde_json::from_value(json!({})).unwrap();
    assert_eq!(unnamed.name, "file");
    unnamed.validate().unwrap();
}

#[test]
fn a_project_change_and_a_device_sign_out_need_real_ids() {
    use emberhut_api::auth::RevokeRequest;
    use emberhut_api::projects::ProjectSetRequest;

    read::<ProjectSetRequest>("project_set_request.json")
        .validate()
        .unwrap();
    read::<RevokeRequest>("auth_revoke_request.json")
        .validate()
        .unwrap();
    fails::<RevokeRequest>("auth_revoke_request.json", "authRevoke", &json!(0));
    let mut body = crate::support::fixture("project_set_request.json");
    body["projectSet"]["id"] = json!(0);
    assert!(from_value::<ProjectSetRequest>(body).validate().is_err());
}

#[test]
fn a_chrome_name_and_a_pause_of_up_to_ten_minutes_pass() {
    let mut body = crate::support::fixture("chrome_request.json");
    body["label"] = json!("  My   work laptop 2026  ");
    from_value::<ChromeAction>(body).validate().unwrap();
    for minutes in [0, 10] {
        let pause: ChromeAction =
            from_value(json!({ "kind": "pause", "id": 2, "minutes": minutes }));
        pause.validate().unwrap();
    }
    let longer: ChromeAction = from_value(json!({ "kind": "pause", "id": 2, "minutes": 11 }));
    assert!(longer.validate().is_err());
    let stop: ChromeAction = from_value(json!({ "kind": "stop", "id": 2 }));
    assert!(matches!(stop, ChromeAction::Pause { minutes: None, .. }));
}

#[test]
fn a_new_party_needs_a_topic_and_two_to_five_named_personas() {
    use emberhut_api::dark::DarkRequest;
    use emberhut_api::party::{NewParty, PartySay};

    read::<NewParty>("party_new.json").validate().unwrap();
    fails::<NewParty>("party_new.json", "topic", &json!("  "));
    fails::<NewParty>("party_new.json", "maxTurns", &json!(0));
    fails::<NewParty>("party_new.json", "maxTokens", &json!(100));
    let one = json!([{ "name": "Nora", "role": "", "model": "claude-opus-5-5" }]);
    fails::<NewParty>("party_new.json", "personas", &one);
    let unnamed = json!([
        { "name": "Nora", "role": "", "model": "claude-opus-5-5" },
        { "name": " ", "role": "", "model": "claude-opus-5-5" }
    ]);
    fails::<NewParty>("party_new.json", "personas", &unnamed);
    read::<PartySay>("party_say.json").validate().unwrap();
    fails::<PartySay>("party_say.json", "text", &json!(""));
    read::<DarkRequest>("dark_request.json").validate().unwrap();
    fails::<DarkRequest>("dark_request.json", "text", &json!("\n"));
}

#[test]
fn an_event_has_bounded_text_and_a_plain_kind() {
    let event = |body: serde_json::Value| -> EventRequest { from_value(body) };
    event(json!({ "text": "The backup finished", "kind": "backup_2" }))
        .validate()
        .unwrap();
    event(json!({ "text": "é".repeat(100_000) }))
        .validate()
        .unwrap();
    assert!(
        event(json!({ "text": "é".repeat(100_001) }))
            .validate()
            .is_err()
    );
    event(json!({ "text": "x", "kind": "k".repeat(32) }))
        .validate()
        .unwrap();
    for kind in [
        "k".repeat(33),
        "x] done\n- [checkin".to_owned(),
        "Backup".to_owned(),
        "a b".to_owned(),
    ] {
        assert!(
            event(json!({ "text": "x", "kind": kind }))
                .validate()
                .is_err(),
            "{kind}"
        );
    }
}
