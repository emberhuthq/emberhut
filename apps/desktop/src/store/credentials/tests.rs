//! The picker of what the ember thinks with: its rows' words and rules on their own, then the
//! store over a scripted backend: what is read, a check and its refusal, a save, a removal, and
//! where the key typed goes.

use std::sync::Arc;

use emberhut_api::agent_provider::{
    AgentProvider, ProviderKind, ProviderModel, ProviderRequest, ProviderTest,
};
use emberhut_api::ids::RoomId;
use gpui_kit::{Entity, TestAppContext};

use super::*;
use crate::backend::fake::{Call, FakeBackend};
use crate::store::{Page, ThemeState};

/// An example key, long enough to look like one.
const KEY: &str = "sk-ant-example-key-0000";

fn store(cx: &mut TestAppContext, backend: &Arc<FakeBackend>) -> Entity<Store> {
    let backend = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
    cx.new(|_| Store::idle(backend, None, ThemeState::load(None), RoomId::MAIN))
}

/// What a server answers with `kind` saved.
fn saved(kind: ProviderKind, model: Option<&str>, usable: bool) -> AgentProvider {
    AgentProvider {
        provider: Some(kind),
        key_set: true,
        key_hint: Some("0000".to_owned()),
        model: model.map(str::to_owned),
        usable,
        in_use: usable,
        ..AgentProvider::NONE
    }
}

fn models(ids: &[&str]) -> ProviderTest {
    ProviderTest {
        checked: true,
        models: ids
            .iter()
            .map(|id| ProviderModel {
                id: (*id).to_owned(),
                name: None,
            })
            .collect(),
    }
}

fn refused(status: u16, sentence: &str) -> Error {
    Error::Refused {
        status: Some(status),
        message: Some(sentence.to_owned()),
    }
}

#[test]
fn six_rows_and_the_ones_needing_the_bridge_cant_be_picked() {
    let kinds: Vec<_> = ROWS.iter().map(|row| row.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::DeepSeek,
            ProviderKind::OpenRouter,
            ProviderKind::Ollama,
            ProviderKind::OpenAiCompatible,
        ]
    );
    let mut picker = Credentials::default();
    assert!(!picker.pick(&ProviderKind::OpenAi), "coming next");
    assert!(!picker.pick(&ProviderKind::OpenAiCompatible));
    assert!(!picker.pick(&ProviderKind::from("unheard-of")));
    assert_eq!(picker.picked(), None);
    assert!(picker.pick(&ProviderKind::DeepSeek));
    assert_eq!(picker.picked(), Some(&ProviderKind::DeepSeek));
}

#[test]
fn the_servers_word_on_the_saved_provider_wins() {
    let bridged = saved(ProviderKind::OpenAi, None, true);
    assert!(runs_today(&ProviderKind::OpenAi, Some(&bridged)), "lit");
    let broken = saved(ProviderKind::DeepSeek, None, false);
    assert!(!runs_today(&ProviderKind::DeepSeek, Some(&broken)));
    assert!(
        runs_today(&ProviderKind::Ollama, Some(&broken)),
        "the others by the rule"
    );
    assert!(!runs_today(&ProviderKind::OpenAiCompatible, None));
}

#[test]
fn what_is_saved_reads_in_a_line() {
    let line = saved_line(&saved(ProviderKind::Anthropic, Some("example-model"), true));
    assert_eq!(line, "Claude · example-model · key ending 0000");
    assert_eq!(saved_line(&AgentProvider::NONE), "Not chosen yet");
    let own = AgentProvider {
        provider: Some(ProviderKind::Claude),
        usable: true,
        ..AgentProvider::NONE
    };
    assert_eq!(saved_line(&own), "Claude, on its own sign-in");
    let mut accounts = saved(ProviderKind::DeepSeek, None, true);
    accounts.in_use = false;
    assert_eq!(
        not_in_use(&accounts),
        Some("Saved, not in use: this server's own accounts come first.")
    );
    assert_eq!(
        not_in_use(&saved(ProviderKind::OpenAi, None, false)),
        Some("Saved. Your ember can't think with it yet.")
    );
    assert_eq!(not_in_use(&saved(ProviderKind::DeepSeek, None, true)), None);
    assert_eq!(not_in_use(&AgentProvider::NONE), None);
    assert_eq!(passed_line(0), "It works");
    assert_eq!(passed_line(1), "It works · 1 model");
    assert_eq!(passed_line(12), "It works · 12 models");
}

#[test]
fn a_request_takes_only_what_its_provider_takes() {
    let mut picker = Credentials::default();
    picker.pick(&ProviderKind::Ollama);
    assert!(picker.can_test(), "no key, and its address has a default");
    assert_eq!(
        picker.request(false),
        Some(ProviderRequest {
            provider: ProviderKind::Ollama,
            api_key: None,
            base_url: None,
            model: None,
            fast_model: None,
        })
    );
    picker.set_key(KEY);
    picker.set_base_url("  http://192.0.2.7:11434 ");
    let request = picker.request(false).unwrap();
    assert_eq!(request.api_key, None, "Ollama takes no key");
    assert_eq!(request.base_url.as_deref(), Some("http://192.0.2.7:11434"));

    picker.pick(&ProviderKind::Anthropic);
    assert!(!picker.can_test(), "a key is needed");
    picker.set_key("   ");
    assert!(!picker.can_test());
    picker.set_key(&format!(" {KEY} "));
    assert_eq!(picker.request(false).unwrap().api_key.as_deref(), Some(KEY));
    assert!(!picker.can_save(), "not checked yet");
}

#[test]
fn the_key_typed_never_shows_in_debug() {
    let mut picker = Credentials::default();
    picker.pick(&ProviderKind::Anthropic);
    picker.set_key(KEY);
    let shown = format!("{picker:?}");
    assert!(!shown.contains(KEY), "{shown}");
    assert!(!shown.contains("example-key"), "{shown}");
    let request = format!("{:?}", picker.request(true).unwrap());
    assert!(!request.contains(KEY), "{request}");
}

#[test]
fn an_answer_for_values_since_changed_is_dropped() {
    let mut picker = Credentials::default();
    picker.pick(&ProviderKind::DeepSeek);
    picker.set_key(KEY);
    let (round, _) = picker.test_started().unwrap();
    assert_eq!(picker.check(), &Check::Testing);
    picker.set_key("sk-another-example-key");
    picker.test_done(round, Ok(models(&["example-model"])));
    assert_eq!(picker.check(), &Check::Idle);
    assert_eq!(picker.model(), None);
}

#[gpui_kit::test]
fn the_saved_provider_is_read_and_its_row_picked(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.provider.push_status(Ok(saved(
        ProviderKind::DeepSeek,
        Some("example-model"),
        true,
    )));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.show_in_test_column(Some(Page::Settings), cx);
    });
    cx.run_until_parked();
    assert!(backend.calls().contains(&Call::AgentProvider));
    store.read_with(cx, |store, _| {
        let picker = store.credentials();
        assert_eq!(picker.picked(), Some(&ProviderKind::DeepSeek));
        assert_eq!(picker.saved_key_hint(&ProviderKind::DeepSeek), Some("0000"));
        assert_eq!(picker.saved_key_hint(&ProviderKind::Anthropic), None);
        assert!(!picker.has_key(), "the key isn't read back");
    });
}

#[gpui_kit::test]
fn a_check_lists_the_models_and_picks_the_saved_one(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend
        .provider
        .push_status(Ok(saved(ProviderKind::OpenRouter, Some("example/b"), true)));
    backend
        .provider
        .push_test(Ok(models(&["example/a", "example/b"])));
    backend
        .provider
        .push_test(Ok(models(&["example/a", "example/c"])));
    let store = store(cx, &backend);
    store.update(cx, Store::load_credentials);
    cx.run_until_parked();
    store.update(cx, |store, cx| {
        store.type_provider_key(KEY, cx);
        store.test_provider(cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let picker = store.credentials();
        let Check::Passed { models, answer } = picker.check() else {
            panic!("passed: {:?}", picker.check());
        };
        assert_eq!(models.len(), 2);
        assert_eq!(answer, &None, "no turn ran");
        assert_eq!(picker.model(), Some("example/b"), "the saved model");
        assert!(picker.can_save());
    });
    assert_eq!(
        backend.calls().last(),
        Some(&Call::TestAgentProvider(ProviderRequest {
            provider: ProviderKind::OpenRouter,
            api_key: Some(KEY.to_owned()),
            base_url: None,
            model: None,
            fast_model: None,
        }))
    );
    store.update(cx, |store, cx| {
        store.type_provider_key("sk-another-example-key", cx);
        store.test_provider(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        store.read_with(cx, |store, _| store
            .credentials()
            .model()
            .map(str::to_owned)),
        Some("example/a".to_owned()),
        "the saved one isn't listed: the first"
    );
}

#[gpui_kit::test]
fn a_failed_check_shows_the_servers_sentence(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let sentence = "DeepSeek didn't accept that key. Check that it was copied whole.";
    backend.provider.push_test(Err(refused(422, sentence)));
    backend
        .provider
        .push_test(Err(Error::Unreachable("timed out".into())));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.pick_provider(&ProviderKind::DeepSeek, cx);
        store.type_provider_key(KEY, cx);
        store.test_provider(cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let picker = store.credentials();
        assert_eq!(picker.check(), &Check::Failed(sentence.to_owned()));
        assert!(!picker.can_save());
    });
    store.update(cx, Store::test_provider);
    cx.run_until_parked();
    let Check::Failed(why) = store.read_with(cx, |store, _| store.credentials().check().clone())
    else {
        panic!("failed");
    };
    assert!(why.starts_with("Can't reach"), "{why}");
}

#[gpui_kit::test]
fn use_this_saves_what_was_checked_and_drops_the_key(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend
        .provider
        .push_test(Ok(models(&["example-model-a", "example-model-b"])));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.pick_provider(&ProviderKind::Anthropic, cx);
        store.type_provider_key(KEY, cx);
        store.save_provider(cx);
    });
    cx.run_until_parked();
    assert!(
        !backend
            .calls()
            .iter()
            .any(|call| matches!(call, Call::SaveAgentProvider(_))),
        "not before a check passed"
    );
    store.update(cx, Store::test_provider);
    cx.run_until_parked();
    store.update(cx, |store, cx| {
        store.choose_provider_model("example-model-b", cx);
        store.choose_provider_model("not-listed", cx);
        store.save_provider(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        backend.calls().last(),
        Some(&Call::SaveAgentProvider(ProviderRequest {
            provider: ProviderKind::Anthropic,
            api_key: Some(KEY.to_owned()),
            base_url: None,
            model: Some("example-model-b".to_owned()),
            fast_model: None,
        }))
    );
    store.read_with(cx, |store, _| {
        let picker = store.credentials();
        assert_eq!(picker.saves(), 1);
        assert!(!picker.has_key(), "the key isn't kept");
        assert!(!format!("{picker:?}").contains("example-key"));
        assert_eq!(
            picker.saved().map(saved_line).as_deref(),
            Some("Claude · example-model-b · key ending 0000")
        );
        assert_eq!(picker.check(), &Check::Idle);
    });
}

#[gpui_kit::test]
fn a_refused_save_keeps_the_fields_and_says_why(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend.provider.push_test(Ok(models(&[])));
    let sentence = "DeepSeek couldn't be reached to check it, so nothing was saved.";
    backend.provider.push_save(Err(refused(502, sentence)));
    let store = store(cx, &backend);
    store.update(cx, |store, cx| {
        store.pick_provider(&ProviderKind::DeepSeek, cx);
        store.type_provider_key(KEY, cx);
        store.test_provider(cx);
    });
    cx.run_until_parked();
    store.update(cx, Store::save_provider);
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let picker = store.credentials();
        assert_eq!(picker.save_problem(), Some(sentence));
        assert_eq!(picker.saves(), 0);
        assert!(picker.has_key(), "to try again");
        assert!(picker.can_save(), "no model listed: the provider's default");
    });
}

#[gpui_kit::test]
fn forget_this_key_asks_once_then_removes_it(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    backend
        .provider
        .push_status(Ok(saved(ProviderKind::Anthropic, None, true)));
    let store = store(cx, &backend);
    store.update(cx, Store::load_credentials);
    cx.run_until_parked();
    store.update(cx, Store::forget_provider);
    cx.run_until_parked();
    assert!(
        !backend.calls().contains(&Call::ForgetAgentProvider),
        "not unconfirmed"
    );
    store.update(cx, |store, cx| {
        store.ask_forget_provider(true, cx);
        assert!(store.credentials().forget_asked());
        store.forget_provider(cx);
    });
    cx.run_until_parked();
    assert_eq!(backend.calls().last(), Some(&Call::ForgetAgentProvider));
    store.read_with(cx, |store, _| {
        let picker = store.credentials();
        assert_eq!(picker.saved(), Some(&AgentProvider::NONE));
        assert!(!picker.forget_asked());
    });
}

#[gpui_kit::test]
fn a_refused_removal_keeps_what_is_saved_and_says_why(cx: &mut TestAppContext) {
    let backend = Arc::new(FakeBackend::new([]));
    let kept = saved(ProviderKind::DeepSeek, None, true);
    backend.provider.push_status(Ok(kept.clone()));
    backend
        .provider
        .push_forget(Err(Error::Unreachable("connection refused".into())));
    let store = store(cx, &backend);
    store.update(cx, Store::load_credentials);
    cx.run_until_parked();
    store.update(cx, |store, cx| {
        store.ask_forget_provider(true, cx);
        store.forget_provider(cx);
    });
    cx.run_until_parked();
    store.read_with(cx, |store, _| {
        let picker = store.credentials();
        assert_eq!(picker.saved(), Some(&kept));
        assert!(!picker.is_forgetting());
        assert!(
            picker
                .save_problem()
                .is_some_and(|why| why.starts_with("Can't reach")),
            "{:?}",
            picker.save_problem()
        );
    });
}
