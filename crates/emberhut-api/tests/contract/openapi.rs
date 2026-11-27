//! The schemas (the `openapi` feature) name the fields as the wire does.

use emberhut_api::chat::HistoryQuery;
use emberhut_api::chrome::{ChromeAction, ChromeReply};
use emberhut_api::error::ErrorBody;
use emberhut_api::facts::FactsPage;
use emberhut_api::health::{Health, LoopReport};
use emberhut_api::live::LiveEvent;
use emberhut_api::machines::MachinesPage;
use emberhut_api::releases::ReleaseFeed;
use emberhut_api::settings::SettingsRequest;
use emberhut_api::state::State;
use emberhut_api::switches::{SwitchAction, SwitchPage};
use emberhut_api::tasks::TaskStatus;
use emberhut_api::theme::{ThemeChange, ThemeSetReply};
use emberhut_api::uploads::{UploadQuery, UploadReply};
use serde_json::Value;
use utoipa::openapi::path::ParameterIn;
use utoipa::openapi::{ComponentsBuilder, OpenApiBuilder};
use utoipa::{IntoParams, PartialSchema, ToSchema};

/// A document with these types' schemas and every schema they use.
fn document() -> Value {
    fn with<T: ToSchema>(components: ComponentsBuilder) -> ComponentsBuilder {
        let mut used = Vec::new();
        T::schemas(&mut used);
        components
            .schema(T::name(), T::schema())
            .schemas_from_iter(used)
    }
    let mut components = ComponentsBuilder::new();
    components = with::<State>(components);
    components = with::<ErrorBody>(components);
    components = with::<ThemeChange>(components);
    components = with::<ThemeSetReply>(components);
    components = with::<MachinesPage>(components);
    components = with::<LiveEvent>(components);
    components = with::<UploadReply>(components);
    components = with::<Health>(components);
    components = with::<LoopReport>(components);
    components = with::<SettingsRequest>(components);
    components = with::<SwitchPage>(components);
    components = with::<SwitchAction>(components);
    components = with::<FactsPage>(components);
    components = with::<ChromeAction>(components);
    components = with::<ChromeReply>(components);
    components = with::<ReleaseFeed>(components);
    let json = OpenApiBuilder::new()
        .components(Some(components.build()))
        .build()
        .to_json()
        .unwrap();
    serde_json::from_str(&json).unwrap()
}

fn properties<'a>(document: &'a Value, schema: &str) -> &'a serde_json::Map<String, Value> {
    document["components"]["schemas"][schema]["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("{schema} has properties"))
}

#[test]
fn schemas_use_the_wire_names() {
    let document = document();
    assert!(properties(&document, "RoomRow").contains_key("lastMine"));
    assert!(properties(&document, "ErrorBody").contains_key("signIn"));
    assert!(properties(&document, "Issue").contains_key("hardDeadline"));
    let reply = &document["components"]["schemas"]["ThemeSetReply"]["allOf"];
    assert_eq!(
        reply[0]["$ref"], "#/components/schemas/ThemePage",
        "the page is flattened in"
    );
    assert!(properties(&document, "ThemeChange").contains_key("accent"));
    assert!(properties(&document, "UploadReply").contains_key("file"));
    assert!(properties(&document, "LoopLag").contains_key("p95_1m"));
    assert!(properties(&document, "SlowWork").contains_key("max"));
    assert!(properties(&document, "SettingsRequest").contains_key("factSet"));
    assert!(properties(&document, "ChromeProfile").contains_key("oldCopy"));
    assert!(properties(&document, "Fact").contains_key("nowAt"));
    assert!(properties(&document, "Release").contains_key("minimumOs"));
}

#[test]
fn defaulted_fields_are_not_required() {
    let document = document();
    let required = document["components"]["schemas"]["State"]["required"]
        .as_array()
        .unwrap();
    let required: Vec<&str> = required.iter().filter_map(Value::as_str).collect();
    for field in ["room", "rooms", "chat", "earlier"] {
        assert!(required.contains(&field), "{field}");
    }
    assert!(!required.contains(&"issues"));
}

#[test]
fn a_live_event_is_live_or_null() {
    let document = document();
    assert!(properties(&document, "Live").contains_key("doing"));
    let schema = serde_json::to_value(LiveEvent::schema()).unwrap();
    let kinds = schema["oneOf"].as_array().unwrap();
    assert!(kinds.iter().any(|kind| kind["type"] == "null"), "{schema}");
}

#[test]
fn open_vocabularies_are_strings() {
    let schema = serde_json::to_value(TaskStatus::schema()).unwrap();
    assert_eq!(schema["type"], "string");
    assert!(
        schema["description"]
            .as_str()
            .unwrap()
            .contains("todo, working, waiting, done, canceled")
    );
}

#[test]
fn queries_are_query_parameters() {
    let names: Vec<String> = HistoryQuery::into_params(|| Some(ParameterIn::Query))
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["room", "before", "after"]);
    let names: Vec<String> = UploadQuery::into_params(|| Some(ParameterIn::Query))
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["name"]);
}
