//! What the user's ember thinks with: the model provider the server's agent runs on
//! (`/api/agent/provider`), and the picker that chooses one. A row is picked, its fields are
//! filled, "Test it" checks them with the provider and lists its models, and "Use this" saves
//! them with the model chosen; the saved provider's key can be forgotten.
//!
//! A key is typed here and sent, never read back: the server answers with its last four
//! characters at most. The key typed is dropped once it is saved, or when another row is picked,
//! and no `Debug` of this module's types shows it.

use std::fmt;
use std::sync::Arc;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use crate::backend::Error;
use crate::store::gate::reason;
use crate::store::{Store, update_or_log};
use emberhut_api::agent_provider::{
    AgentProvider, ProviderKind, ProviderModel, ProviderRequest, ProviderTest,
};

/// Whether a provider takes a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Takes {
    /// It doesn't: the field isn't shown.
    No,
    /// It may: a blank field is left out.
    Optional,
    /// It must have one.
    Required,
}

/// One row of the picker: a provider, how it is named and described, and what it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    /// The provider.
    pub(crate) kind: ProviderKind,
    /// Its name.
    pub(crate) name: &'static str,
    /// What it is, in one line.
    pub(crate) line: &'static str,
    /// A tag beside its name.
    pub(crate) tag: Option<&'static str>,
    /// A hint under its fields, once it is picked.
    pub(crate) hint: Option<&'static str>,
    /// Whether it takes an API key.
    pub(crate) key: Takes,
    /// Whether it takes a base URL, and what the field shows while it is blank.
    pub(crate) base_url: (Takes, &'static str),
}

/// The picker's rows, in their order.
pub(crate) static ROWS: [Row; 6] = [
    Row {
        kind: ProviderKind::Anthropic,
        name: "Claude",
        line: "Best with tools. Uses your Anthropic API key, pay as you go.",
        tag: Some("Recommended"),
        hint: Some(
            "Get a key at console.anthropic.com · signing in with a Claude Pro or Max plan comes \
             later",
        ),
        key: Takes::Required,
        base_url: (Takes::No, ""),
    },
    Row {
        kind: ProviderKind::OpenAi,
        name: "OpenAI",
        line: "GPT models, with your OpenAI key.",
        tag: None,
        hint: None,
        key: Takes::Required,
        base_url: (Takes::No, ""),
    },
    Row {
        kind: ProviderKind::DeepSeek,
        name: "DeepSeek",
        line: "Low cost and strong at code, with your DeepSeek key.",
        tag: None,
        hint: None,
        key: Takes::Required,
        base_url: (Takes::No, ""),
    },
    Row {
        kind: ProviderKind::OpenRouter,
        name: "OpenRouter",
        line: "One key, hundreds of models from many makers.",
        tag: None,
        hint: None,
        key: Takes::Required,
        base_url: (Takes::No, ""),
    },
    Row {
        kind: ProviderKind::Ollama,
        name: "Ollama",
        line: "Runs on the computer your ember lives on. Free and private, but slower.",
        tag: None,
        hint: Some(
            "Nothing leaves the computer your ember lives on. Smaller models handle tools less \
             well.",
        ),
        key: Takes::No,
        // The server's own default when none is sent.
        base_url: (Takes::Optional, "http://localhost:11434"),
    },
    Row {
        kind: ProviderKind::OpenAiCompatible,
        name: "Another service",
        line: "Any OpenAI-style service: its address, a key and a model.",
        tag: None,
        hint: None,
        key: Takes::Optional,
        base_url: (Takes::Required, "https://…/v1"),
    },
];

/// The tag of a row the agent can't run on yet.
pub(crate) const COMING_NEXT: &str = "Coming next";

/// The line under the picker.
pub(crate) const FOOTER: &str = "Your key stays on the computer your ember lives on, in a file \
     only your user account can read. It's sent only to the provider you pick.";

/// A row, by its provider.
#[must_use]
pub(crate) fn row(kind: &ProviderKind) -> Option<&'static Row> {
    ROWS.iter().find(|row| &row.kind == kind)
}

/// Whether the agent can run on `kind` today, so its row can be picked.
///
/// The server says so in `usable`, but only of the provider saved: for that one its word is
/// taken. For the others this follows the server's rule (`docs/adr/0028-agent-model-providers.md`):
/// every provider runs but `openai` and `openai-compatible`, which need a bridge from
/// Anthropic's API the agent doesn't have yet. A provider this app doesn't know doesn't run.
#[must_use]
pub(crate) fn runs_today(kind: &ProviderKind, saved: Option<&AgentProvider>) -> bool {
    match saved {
        Some(saved) if saved.provider.as_ref() == Some(kind) => saved.usable,
        _ => !matches!(
            kind,
            ProviderKind::OpenAi | ProviderKind::OpenAiCompatible | ProviderKind::Other(_)
        ),
    }
}

/// A provider's name, also for the ones the picker has no row for.
#[must_use]
pub(crate) fn name(kind: &ProviderKind) -> &str {
    match kind {
        ProviderKind::Claude => "Claude, on its own sign-in",
        ProviderKind::AnthropicCompatible => "An Anthropic-style service",
        ProviderKind::Other(other) => other,
        kind => row(kind).map_or("A provider", |row| row.name),
    }
}

/// What is saved, in a line: the provider, its model and the key's last characters.
#[must_use]
pub(crate) fn saved_line(saved: &AgentProvider) -> String {
    let Some(kind) = &saved.provider else {
        return "Not chosen yet".to_owned();
    };
    let mut parts = vec![name(kind).to_owned()];
    parts.extend(saved.model.clone());
    parts.extend(
        saved
            .key_hint
            .as_ref()
            .filter(|_| saved.key_set)
            .map(|hint| format!("key ending {hint}")),
    );
    parts.join(" · ")
}

/// Why what is saved isn't what the agent thinks with now, when it isn't.
#[must_use]
pub(crate) fn not_in_use(saved: &AgentProvider) -> Option<&'static str> {
    match (&saved.provider, saved.usable, saved.in_use) {
        (None, ..) | (Some(_), _, true) => None,
        (Some(_), false, false) => Some("Saved. Your ember can't think with it yet."),
        (Some(_), true, false) => Some("Saved, not in use: this server's own accounts come first."),
    }
}

/// What a check that passed says: that it works, and how many models it lists.
#[must_use]
pub(crate) fn passed_line(models: usize) -> String {
    match models {
        0 => "It works".to_owned(),
        1 => "It works · 1 model".to_owned(),
        n => format!("It works · {n} models"),
    }
}

/// The title over a failed check's reason.
pub(crate) const FAILED: &str = "Didn't answer";

/// An error, in the server's sentence when it gave one.
fn sentence(error: &Error) -> String {
    match error {
        Error::Unreachable(_) => {
            "Can't reach your ember's server. Check this computer is online and try again."
                .to_owned()
        }
        error => {
            let words = reason(error);
            let mut chars = words.chars();
            chars
                .next()
                .map_or_default(|first| first.to_uppercase().chain(chars).collect::<String>())
        }
    }
}

/// Where the check of the fields typed is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Check {
    /// Not checked.
    #[default]
    Idle,
    /// Being checked.
    Testing,
    /// The provider let the server in.
    Passed {
        /// The models it lists.
        models: Vec<ProviderModel>,
        /// What the ember answered, once a check runs a turn on the provider (none today:
        /// the check only lists the models).
        answer: Option<String>,
    },
    /// It didn't, in the server's sentence.
    Failed(String),
}

/// Where "Forget this key" is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Forget {
    /// Not asked.
    #[default]
    Idle,
    /// Asked, waiting for its confirmation.
    Asked,
    /// The removal is on its way.
    Running,
}

/// What the user typed in the picked row's fields.
#[derive(Clone, Default, PartialEq, Eq)]
struct Typed {
    key: String,
    base_url: String,
}

impl fmt::Debug for Typed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Typed")
            .field("key", &(!self.key.is_empty()).then_some("…"))
            .field("base_url", &self.base_url)
            .finish()
    }
}

/// The picker's data: what is saved, the row picked, its fields, and the check and save of them.
#[derive(Debug, Default)]
pub(crate) struct Credentials {
    /// What is saved, once read.
    saved: Option<AgentProvider>,
    loading: bool,
    /// Why the last read failed.
    read_problem: Option<String>,
    /// The row picked.
    picked: Option<ProviderKind>,
    typed: Typed,
    check: Check,
    /// The model chosen from the check's list.
    model: Option<String>,
    saving: bool,
    /// Why the last save or removal failed.
    save_problem: Option<String>,
    /// Where "Forget this key" is.
    forget: Forget,
    /// How many saves went through, for a view to tell when one did.
    saves: u64,
    /// Numbers each change of the fields, so a check's late answer for other values is dropped.
    round: u64,
}

impl Credentials {
    /// What is saved, once read.
    #[must_use]
    pub(crate) fn saved(&self) -> Option<&AgentProvider> {
        self.saved.as_ref()
    }

    /// Whether what is saved is being read.
    #[must_use]
    pub(crate) fn is_loading(&self) -> bool {
        self.loading
    }

    /// Why what is saved couldn't be read.
    #[must_use]
    pub(crate) fn read_problem(&self) -> Option<&str> {
        self.read_problem.as_deref()
    }

    /// The row picked.
    #[must_use]
    pub(crate) fn picked(&self) -> Option<&ProviderKind> {
        self.picked.as_ref()
    }

    /// Where the check of the fields is.
    #[must_use]
    pub(crate) fn check(&self) -> &Check {
        &self.check
    }

    /// The model chosen.
    #[must_use]
    pub(crate) fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Whether the provider is being saved.
    #[must_use]
    pub(crate) fn is_saving(&self) -> bool {
        self.saving
    }

    /// Why the last save or removal failed.
    #[must_use]
    pub(crate) fn save_problem(&self) -> Option<&str> {
        self.save_problem.as_deref()
    }

    /// Whether "Forget this key" waits for its confirmation.
    #[must_use]
    pub(crate) fn forget_asked(&self) -> bool {
        self.forget == Forget::Asked
    }

    /// Whether the saved key is being forgotten.
    #[must_use]
    pub(crate) fn is_forgetting(&self) -> bool {
        self.forget == Forget::Running
    }

    /// How many saves went through.
    #[must_use]
    pub(crate) fn saves(&self) -> u64 {
        self.saves
    }

    /// Whether a key is typed in the picked row.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn has_key(&self) -> bool {
        !self.typed.key.trim().is_empty()
    }

    /// Whether `kind`'s row can be picked now.
    #[must_use]
    pub(crate) fn can_pick(&self, kind: &ProviderKind) -> bool {
        runs_today(kind, self.saved.as_ref())
    }

    /// The key saved for `kind`, by its last characters, when `kind` is the provider saved.
    #[must_use]
    pub(crate) fn saved_key_hint(&self, kind: &ProviderKind) -> Option<&str> {
        let saved = self.saved.as_ref()?;
        (saved.provider.as_ref() == Some(kind) && saved.key_set)
            .then_some(saved.key_hint.as_deref())
            .flatten()
    }

    /// Picks `kind`'s row: its fields start blank and unchecked. A row the agent can't run on
    /// isn't picked; whether the row is picked now.
    pub(crate) fn pick(&mut self, kind: &ProviderKind) -> bool {
        if !self.can_pick(kind) {
            return false;
        }
        if self.picked.as_ref() != Some(kind) {
            self.picked = Some(kind.clone());
            self.typed = Typed::default();
            self.edited();
            self.forget = Forget::Idle;
        }
        true
    }

    /// The key typed changed.
    pub(crate) fn set_key(&mut self, key: &str) {
        if self.typed.key != key {
            key.clone_into(&mut self.typed.key);
            self.edited();
        }
    }

    /// The base URL typed changed.
    pub(crate) fn set_base_url(&mut self, url: &str) {
        if self.typed.base_url != url {
            url.clone_into(&mut self.typed.base_url);
            self.edited();
        }
    }

    /// Chooses one of the models the check listed.
    pub(crate) fn choose_model(&mut self, id: &str) {
        if let Check::Passed { models, .. } = &self.check
            && models.iter().any(|model| model.id == id)
        {
            self.model = Some(id.to_owned());
        }
    }

    /// Asks for "Forget this key"'s confirmation, or takes the question back.
    pub(crate) fn ask_forget(&mut self, asked: bool) {
        if self.forget == Forget::Running {
            return;
        }
        let asked = asked && self.saved.as_ref().is_some_and(|saved| saved.key_set);
        self.forget = if asked { Forget::Asked } else { Forget::Idle };
    }

    /// The fields changed: what was checked was other values.
    fn edited(&mut self) {
        self.round += 1;
        self.check = Check::Idle;
        self.model = None;
        self.save_problem = None;
    }

    /// The picked row's request: its fields trimmed, a blank one left out, the model chosen
    /// only for a save. None while a field it needs is blank.
    fn request(&self, with_model: bool) -> Option<ProviderRequest> {
        let row = row(self.picked.as_ref()?)?;
        let given = |text: &str, takes: Takes| {
            let text = text.trim();
            match takes {
                Takes::No => Ok(None),
                _ if !text.is_empty() => Ok(Some(text.to_owned())),
                Takes::Optional => Ok(None),
                Takes::Required => Err(()),
            }
        };
        Some(ProviderRequest {
            provider: row.kind.clone(),
            api_key: given(&self.typed.key, row.key).ok()?,
            base_url: given(&self.typed.base_url, row.base_url.0).ok()?,
            model: self.model.clone().filter(|_| with_model),
            fast_model: None,
        })
    }

    /// Whether "Test it" can run: a row picked, the fields it needs filled, nothing running.
    #[must_use]
    pub(crate) fn can_test(&self) -> bool {
        !self.busy() && self.request(false).is_some()
    }

    /// Whether "Use this" can run: the fields as they are now passed the check, and a model is
    /// chosen when it listed any.
    #[must_use]
    pub(crate) fn can_save(&self) -> bool {
        let chosen = match &self.check {
            Check::Passed { models, .. } => models.is_empty() || self.model.is_some(),
            _ => false,
        };
        chosen && !self.busy() && self.request(true).is_some()
    }

    fn busy(&self) -> bool {
        self.check == Check::Testing || self.saving || self.forget == Forget::Running
    }

    /// Starts a check: its round and request, unless one can't run.
    fn test_started(&mut self) -> Option<(u64, ProviderRequest)> {
        if !self.can_test() {
            return None;
        }
        let request = self.request(false)?;
        self.check = Check::Testing;
        self.save_problem = None;
        Some((self.round, request))
    }

    /// A check of round `round` answered: its models listed (the saved one chosen when it is
    /// among them, else the first), or its refusal. An answer for other values is dropped.
    fn test_done(&mut self, round: u64, answer: Result<ProviderTest, Error>) {
        if round != self.round {
            return;
        }
        match answer {
            Ok(test) => {
                let saved = self
                    .saved
                    .as_ref()
                    .filter(|saved| saved.provider == self.picked)
                    .and_then(|saved| saved.model.as_deref());
                let listed = |id: &str| test.models.iter().any(|model| model.id == id);
                self.model = saved
                    .filter(|id| listed(id))
                    .map(str::to_owned)
                    .or_else(|| test.models.first().map(|model| model.id.clone()));
                self.check = Check::Passed {
                    models: test.models,
                    answer: None,
                };
            }
            Err(error) => self.check = Check::Failed(sentence(&error)),
        }
    }

    /// Starts a save: its request, unless one can't run.
    fn save_started(&mut self) -> Option<ProviderRequest> {
        if !self.can_save() {
            return None;
        }
        self.saving = true;
        self.save_problem = None;
        self.request(true)
    }

    /// A save answered: what is saved now, the key typed dropped; or why it wasn't saved.
    fn save_done(&mut self, answer: Result<AgentProvider, Error>) {
        self.saving = false;
        match answer {
            Ok(saved) => {
                self.saved = Some(saved);
                self.typed = Typed::default();
                self.edited();
                self.saves += 1;
            }
            Err(error) => self.save_problem = Some(sentence(&error)),
        }
    }

    /// Starts forgetting the saved key, once confirmed.
    fn forget_started(&mut self) -> bool {
        if self.forget != Forget::Asked || self.busy() {
            return false;
        }
        self.forget = Forget::Running;
        self.save_problem = None;
        true
    }

    /// The removal answered.
    fn forget_done(&mut self, answer: Result<AgentProvider, Error>) {
        self.forget = Forget::Idle;
        match answer {
            Ok(saved) => {
                self.saved = Some(saved);
                self.typed = Typed::default();
                self.edited();
            }
            Err(error) => self.save_problem = Some(sentence(&error)),
        }
    }

    /// What is saved was read: with no row picked yet, the saved provider's is.
    fn loaded(&mut self, answer: Result<AgentProvider, Error>) {
        self.loading = false;
        match answer {
            Ok(saved) => {
                let kind = saved.provider.clone();
                self.saved = Some(saved);
                self.read_problem = None;
                if let Some(kind) = kind.filter(|kind| row(kind).is_some())
                    && self.picked.is_none()
                {
                    self.pick(&kind);
                }
            }
            Err(error) => {
                self.read_problem = Some(format!(
                    "Couldn't read what your ember thinks with: {}",
                    reason(&error)
                ));
            }
        }
    }
}

impl Store {
    /// The picker of what the ember thinks with.
    #[must_use]
    pub(crate) fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// Reads what is saved (once at a time).
    pub(crate) fn load_credentials(&mut self, cx: &mut Context<'_, Self>) {
        if self.credentials.loading {
            return;
        }
        self.credentials.loading = true;
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { backend.agent_provider() })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.credentials.loaded(answer);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Picks a provider's row (one the agent can't run on stays unpicked).
    pub(crate) fn pick_provider(&mut self, kind: &ProviderKind, cx: &mut Context<'_, Self>) {
        if self.credentials.pick(kind) {
            cx.notify();
        }
    }

    /// The key typed in the picked row changed.
    pub(crate) fn type_provider_key(&mut self, key: &str, cx: &mut Context<'_, Self>) {
        self.credentials.set_key(key);
        cx.notify();
    }

    /// The base URL typed in the picked row changed.
    pub(crate) fn type_provider_url(&mut self, url: &str, cx: &mut Context<'_, Self>) {
        self.credentials.set_base_url(url);
        cx.notify();
    }

    /// Chooses one of the models the check listed.
    pub(crate) fn choose_provider_model(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        self.credentials.choose_model(id);
        cx.notify();
    }

    /// "Test it": checks the picked row's fields with its provider and lists its models.
    pub(crate) fn test_provider(&mut self, cx: &mut Context<'_, Self>) {
        let Some((round, request)) = self.credentials.test_started() else {
            return;
        };
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { backend.test_agent_provider(&request) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.credentials.test_done(round, answer);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// "Use this": saves the checked fields with the model chosen.
    pub(crate) fn save_provider(&mut self, cx: &mut Context<'_, Self>) {
        let Some(request) = self.credentials.save_started() else {
            return;
        };
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { backend.save_agent_provider(&request) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.credentials.save_done(answer);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Asks for "Forget this key"'s confirmation, or takes the question back.
    pub(crate) fn ask_forget_provider(&mut self, asked: bool, cx: &mut Context<'_, Self>) {
        self.credentials.ask_forget(asked);
        cx.notify();
    }

    /// Forgets the saved provider and its key, once confirmed.
    pub(crate) fn forget_provider(&mut self, cx: &mut Context<'_, Self>) {
        if !self.credentials.forget_started() {
            return;
        }
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { backend.forget_agent_provider() })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.credentials.forget_done(answer);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
