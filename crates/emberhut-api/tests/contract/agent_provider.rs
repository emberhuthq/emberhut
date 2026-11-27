//! The agent's model provider: what the apps send, and what they read back about it.

use emberhut_api::agent_provider::{AgentProvider, ProviderKind, ProviderRequest, ProviderTest};
use emberhut_api::time::Timestamp;
use serde_json::json;

use crate::support::{assert_round_trip, fixture, from_value, to_value, with_unknown_fields};

#[test]
fn reads_and_writes_what_the_server_holds() {
    let saved: AgentProvider = assert_round_trip("agent_provider.json");
    assert_eq!(saved.provider, Some(ProviderKind::Anthropic));
    assert!(saved.key_set && saved.usable && saved.in_use);
    assert_eq!(saved.key_hint.as_deref(), Some("x7Qa"));
    assert_eq!(saved.base_url, None);
    assert_eq!(
        saved.checked_at.as_ref().map(Timestamp::as_str),
        Some("2026-11-27T05:58:00.000Z")
    );
    assert_eq!(
        to_value(&AgentProvider::NONE),
        json!({ "keySet": false, "usable": false, "inUse": false }),
        "what isn't there is left out"
    );
    let test: ProviderTest = assert_round_trip("agent_provider_test.json");
    assert_eq!(test.models.len(), 2);
    assert_eq!(test.models[1].name.as_deref(), Some("Claude Opus 5.5"));
}

#[test]
fn the_provider_tolerates_unknown_fields_and_keeps_words_it_does_not_know() {
    let mut body = fixture("agent_provider.json");
    with_unknown_fields(&mut body);
    assert_eq!(
        from_value::<AgentProvider>(body),
        from_value::<AgentProvider>(fixture("agent_provider.json"))
    );
    let newer: AgentProvider = from_value(
        json!({ "provider": "gemini", "keySet": true, "usable": false, "inUse": false }),
    );
    assert_eq!(newer.provider, Some(ProviderKind::from("gemini")));
    assert_eq!(to_value(&newer)["provider"], json!("gemini"));
}

#[test]
fn a_request_is_camel_case_and_its_debug_leaves_the_key_out() {
    let request: ProviderRequest = from_value(json!({
        "provider": "openai-compatible", "apiKey": "sk-secret",
        "baseUrl": "https://models.example/v1", "model": "qwen3"
    }));
    assert_eq!(request.provider, ProviderKind::OpenAiCompatible);
    assert_eq!(request.api_key.as_deref(), Some("sk-secret"));
    assert_eq!(
        to_value(&request),
        json!({ "provider": "openai-compatible", "apiKey": "sk-secret",
            "baseUrl": "https://models.example/v1", "model": "qwen3" })
    );
    let shown = format!("{request:?}");
    assert!(!shown.contains("secret"), "{shown}");
    assert!(shown.contains("models.example"), "{shown}");
    let bare: ProviderRequest = from_value(json!({ "provider": "ollama" }));
    assert_eq!(to_value(&bare), json!({ "provider": "ollama" }));
}
