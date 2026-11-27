//! What the user's ember thinks with: the model provider the server's agent runs on
//! (`/api/agent/provider`), and the picker that chooses one. A row is picked, its fields are
//! filled, "Test it" checks them with the provider and lists its models, and "Use this" saves
//! them with the model chosen; the saved provider's key can be forgotten. "Another service" is
//! one row for two kinds of endpoint, chosen inside it by the API it speaks. As the picker opens,
//! a local Ollama is looked for quickly, so its row can say it was found and list its models.
//!
//! A key is typed here and sent, never read back: the server answers with its last four
//! characters at most. The key typed is dropped once it is saved, or when another row is picked,
//! and no `Debug` of this module's types shows it.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::AppContext as _;
use gpui_kit::Context;

use crate::account::switch::is_loopback_host;
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

/// What a provider takes in the picker's fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fields {
    /// Whether it takes an API key.
    pub(crate) key: Takes,
    /// Whether it takes a base URL.
    pub(crate) base_url: Takes,
    /// What the base URL's field shows while it is blank.
    pub(crate) url_hint: &'static str,
}

/// What `kind` takes, as the server reads its request (`docs/api.md`, "Agent provider"): a
/// provider's usual address is left to the server, so only Ollama's is offered, its default
/// shown.
#[must_use]
pub(crate) fn fields(kind: &ProviderKind) -> Fields {
    let (key, base_url, url_hint) = match kind {
        ProviderKind::Anthropic
        | ProviderKind::OpenAi
        | ProviderKind::DeepSeek
        | ProviderKind::OpenRouter => (Takes::Required, Takes::No, ""),
        ProviderKind::Ollama => (Takes::No, Takes::Optional, "http://localhost:11434"),
        ProviderKind::AnthropicCompatible => (Takes::Required, Takes::Required, "https://…"),
        ProviderKind::OpenAiCompatible => (Takes::Optional, Takes::Required, "https://…/v1"),
        // `claude` takes neither, nor does a provider this app doesn't know.
        _ => (Takes::No, Takes::No, ""),
    };
    Fields {
        key,
        base_url,
        url_hint,
    }
}

/// One row of the picker: a provider (or two, chosen inside it), how it is named and described.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    /// The provider, and the one picked when the row is.
    pub(crate) kind: ProviderKind,
    /// The other provider the row saves as, chosen inside it by the API it speaks.
    pub(crate) also: Option<ProviderKind>,
    /// Its name.
    pub(crate) name: &'static str,
    /// What it is, in one line.
    pub(crate) line: &'static str,
    /// A tag beside its name.
    pub(crate) tag: Option<&'static str>,
    /// A hint under its fields, once it is picked.
    pub(crate) hint: Option<&'static str>,
}

impl Row {
    /// The providers it saves as.
    pub(crate) fn kinds(&self) -> impl Iterator<Item = &ProviderKind> {
        std::iter::once(&self.kind).chain(&self.also)
    }
}

/// The picker's rows, in their order.
pub(crate) static ROWS: [Row; 6] = [
    Row {
        kind: ProviderKind::Anthropic,
        also: None,
        name: "Claude",
        line: "Best with tools. Uses your Anthropic API key, pay as you go.",
        tag: Some("Recommended"),
        hint: Some(
            "Get a key at console.anthropic.com · signing in with a Claude Pro or Max plan comes \
             later",
        ),
    },
    Row {
        kind: ProviderKind::OpenAi,
        also: None,
        name: "OpenAI",
        line: "GPT models, with your OpenAI key.",
        tag: None,
        hint: None,
    },
    Row {
        kind: ProviderKind::DeepSeek,
        also: None,
        name: "DeepSeek",
        line: "Low cost and strong at code, with your DeepSeek key.",
        tag: None,
        hint: None,
    },
    Row {
        kind: ProviderKind::OpenRouter,
        also: None,
        name: "OpenRouter",
        line: "One key, hundreds of models from many makers.",
        tag: None,
        hint: None,
    },
    Row {
        kind: ProviderKind::Ollama,
        also: None,
        name: "Ollama",
        line: "Runs on the computer your ember lives on. Free and private, but slower.",
        tag: None,
        hint: Some(
            "Nothing leaves the computer your ember lives on. Smaller models handle tools less \
             well.",
        ),
    },
    Row {
        kind: ProviderKind::OpenAiCompatible,
        also: Some(ProviderKind::AnthropicCompatible),
        name: "Another service",
        line: "Any OpenAI-style service: its address, a key and a model.",
        tag: None,
        hint: None,
    },
];

/// The tag of a row the agent can't run on yet.
pub(crate) const COMING_NEXT: &str = "Coming next";

/// The line under the picker.
pub(crate) const FOOTER: &str = "Your key stays on the computer your ember lives on, in a file \
     only your user account can read. It's sent only to the provider you pick.";

/// A row, by a provider it saves as.
#[must_use]
pub(crate) fn row(kind: &ProviderKind) -> Option<&'static Row> {
    ROWS.iter().find(|row| row.kinds().any(|its| its == kind))
}

/// The API a provider of a two-way row speaks, as its choice is named.
#[must_use]
pub(crate) fn speaks(kind: &ProviderKind) -> &'static str {
    match kind {
        ProviderKind::AnthropicCompatible => "Anthropic-style",
        _ => "OpenAI-style",
    }
}

/// The words under a two-way row's choice while the agent can't run on the one chosen.
#[must_use]
pub(crate) fn coming_next_for(kind: &ProviderKind) -> String {
    format!(
        "{COMING_NEXT}: your ember can't think with an {} service yet.",
        speaks(kind)
    )
}

/// How long the picker waits for a local Ollama as it opens: it only says whether one answers.
const LOOK_WITHIN: Duration = Duration::from_secs(3);

/// How long a check the user asked for waits: the server gives the provider 15 s.
const CHECK_WITHIN: Duration = Duration::from_secs(30);

/// Where the ember lives, from the address the app reaches its server at: this computer when
/// it is on loopback (named a Mac on a Mac), else the ember's own.
#[must_use]
pub(crate) fn ember_place(server: &str) -> &'static str {
    let authority = server.split_once("://").map_or(server, |(_, rest)| rest);
    match is_loopback_host(authority) {
        true if cfg!(target_os = "macos") => "this Mac",
        true => "this computer",
        false => "your ember's computer",
    }
}

/// A local Ollama, as the picker looked for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Ollama {
    /// Not looked for.
    #[default]
    Unknown,
    /// Being looked for.
    Looking,
    /// It answered at the server's default address, on the computer named, with its models.
    Found {
        on: &'static str,
        models: Vec<ProviderModel>,
    },
    /// It didn't answer.
    Missing,
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
    /// A local Ollama, as looked for when the picker opened.
    ollama: Ollama,
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

    /// The provider picked: its row's, or the other one chosen inside it.
    #[must_use]
    pub(crate) fn picked(&self) -> Option<&ProviderKind> {
        self.picked.as_ref()
    }

    /// The row picked.
    #[must_use]
    pub(crate) fn picked_row(&self) -> Option<&'static Row> {
        row(self.picked.as_ref()?)
    }

    /// Whether `kind`, the row's provider or the other chosen inside it, runs today.
    #[must_use]
    pub(crate) fn runs(&self, kind: &ProviderKind) -> bool {
        runs_today(kind, self.saved.as_ref())
    }

    /// The tag beside a row's name: Coming next while the agent can't run on it, that a local
    /// Ollama was found and where, else its own.
    #[must_use]
    pub(crate) fn row_tag(&self, row: &Row) -> Option<String> {
        if !self.can_pick(row) {
            return Some(COMING_NEXT.to_owned());
        }
        match &self.ollama {
            Ollama::Found { on, .. } if row.kind == ProviderKind::Ollama => {
                Some(format!("Found on {on}"))
            }
            _ => row.tag.map(str::to_owned),
        }
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

    /// Whether a row can be picked now: the agent runs on one of its providers.
    #[must_use]
    pub(crate) fn can_pick(&self, row: &Row) -> bool {
        row.kinds().any(|kind| self.runs(kind))
    }

    /// The key saved for `kind`, by its last characters, when `kind` is the provider saved.
    #[must_use]
    pub(crate) fn saved_key_hint(&self, kind: &ProviderKind) -> Option<&str> {
        let saved = self.saved.as_ref()?;
        (saved.provider.as_ref() == Some(kind) && saved.key_set)
            .then_some(saved.key_hint.as_deref())
            .flatten()
    }

    /// Picks the row `kind` saves as, on `kind`: its fields start blank and unchecked (a local
    /// Ollama found fills its models). A row the agent can't run on isn't picked, and the row
    /// picked already stays as it is; whether the row is picked now.
    pub(crate) fn pick(&mut self, kind: &ProviderKind) -> bool {
        let Some(wanted) = row(kind).filter(|row| self.can_pick(row)) else {
            return false;
        };
        if self.picked_row() != Some(wanted) {
            self.picked = Some(kind.clone());
            self.typed = Typed::default();
            self.edited();
            self.forget = Forget::Idle;
            self.fill_found();
        }
        true
    }

    /// Chooses, inside a two-way row, the provider it saves as (the API the service speaks);
    /// what is typed stays, unchecked.
    pub(crate) fn speak(&mut self, kind: &ProviderKind) {
        let same_row = self
            .picked_row()
            .is_some_and(|row| row.kinds().any(|its| its == kind));
        if same_row && self.picked.as_ref() != Some(kind) {
            self.picked = Some(kind.clone());
            self.edited();
        }
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
            self.fill_found();
        }
    }

    /// With Ollama selected at its default address and nothing checked, the models of the one
    /// found there stand for a check that passed.
    fn fill_found(&mut self) {
        let Ollama::Found { models, .. } = &self.ollama else {
            return;
        };
        let default = self.picked == Some(ProviderKind::Ollama)
            && self.typed.base_url.trim().is_empty()
            && self.check == Check::Idle;
        if default && !models.is_empty() {
            let models = models.clone();
            self.model = self.first_choice(&models);
            self.check = Check::Passed {
                models,
                answer: None,
            };
        }
    }

    /// The model to start on among `models`: the one saved for the provider picked when it is
    /// listed, else the first.
    fn first_choice(&self, models: &[ProviderModel]) -> Option<String> {
        let saved = self
            .saved
            .as_ref()
            .filter(|saved| saved.provider == self.picked)
            .and_then(|saved| saved.model.as_deref());
        let listed = |id: &str| models.iter().any(|model| model.id == id);
        saved
            .filter(|id| listed(id))
            .map(str::to_owned)
            .or_else(|| models.first().map(|model| model.id.clone()))
    }

    /// Starts looking for a local Ollama, unless a look is on its way: its request.
    fn look_started(&mut self) -> Option<ProviderRequest> {
        if self.ollama == Ollama::Looking {
            return None;
        }
        self.ollama = Ollama::Looking;
        Some(ProviderRequest {
            provider: ProviderKind::Ollama,
            api_key: None,
            base_url: None,
            model: None,
            fast_model: None,
        })
    }

    /// The look for a local Ollama answered: found on `on` with its models, or not there (no
    /// error: nothing was asked of the user).
    fn look_done(&mut self, on: &'static str, answer: Result<ProviderTest, Error>) {
        self.ollama = match answer {
            Ok(test) => Ollama::Found {
                on,
                models: test.models,
            },
            Err(error) => {
                log::debug!("no local Ollama answered: {}", reason(&error));
                Ollama::Missing
            }
        };
        self.fill_found();
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

    /// The selected row's request: its fields trimmed, a blank one left out, the model chosen
    /// only for a save. None while a field it needs is blank.
    fn request(&self, with_model: bool) -> Option<ProviderRequest> {
        let kind = self.picked.as_ref()?;
        let fields = fields(kind);
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
            provider: kind.clone(),
            api_key: given(&self.typed.key, fields.key).ok()?,
            base_url: given(&self.typed.base_url, fields.base_url).ok()?,
            model: self.model.clone().filter(|_| with_model),
            fast_model: None,
        })
    }

    /// Whether "Test it" can run: a provider picked that the agent runs on, the fields it
    /// needs filled, nothing running.
    #[must_use]
    pub(crate) fn can_test(&self) -> bool {
        let runs = self.picked.as_ref().is_some_and(|kind| self.runs(kind));
        runs && !self.busy() && self.request(false).is_some()
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
                self.model = self.first_choice(&test.models);
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

    /// The picker opens: what is saved is read, and a local Ollama is looked for.
    pub(crate) fn open_credentials(&mut self, cx: &mut Context<'_, Self>) {
        self.load_credentials(cx);
        self.look_for_ollama(cx);
    }

    /// Asks the server, quickly, whether a local Ollama answers at its default address. Not
    /// finding one is no problem: the row only goes without its tag.
    fn look_for_ollama(&mut self, cx: &mut Context<'_, Self>) {
        let Some(request) = self.credentials.look_started() else {
            return;
        };
        let on = ember_place(self.backend.server().as_str());
        let backend = Arc::clone(&self.backend);
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { backend.test_agent_provider(&request, LOOK_WITHIN) })
                .await;
            update_or_log(&this, cx, |store, cx| {
                store.credentials.look_done(on, answer);
                cx.notify();
            });
        })
        .detach();
    }

    /// Chooses, inside a two-way row, the API the service speaks.
    pub(crate) fn speak_provider(&mut self, kind: &ProviderKind, cx: &mut Context<'_, Self>) {
        self.credentials.speak(kind);
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
                .background_spawn(
                    async move { backend.test_agent_provider(&request, CHECK_WITHIN) },
                )
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
