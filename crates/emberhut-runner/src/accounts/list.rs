//! The accounts a runner may run sessions on: one agent configuration folder per subscription
//! login, or a provider paid per token (an API key, AWS Bedrock). The list is a JSON file the
//! owner edits (`emberhut-runner accounts add` writes it too); the runner never holds a login
//! itself: the agent keeps it in that account's folder.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// How an account reaches the model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// A subscription login in its folder.
    #[default]
    Claude,
    /// An API key, paid per token.
    Api,
    /// AWS Bedrock.
    Bedrock,
}

impl Provider {
    /// Its word.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Api => "api",
            Self::Bedrock => "bedrock",
        }
    }
}

/// One account, as the accounts file lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// Its id (`[A-Za-z0-9_-]`), how the server and the sessions name it.
    pub id: String,
    /// Its name in words.
    pub name: String,
    /// How it reaches the model (`claude` when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Provider>,
    /// The agent's configuration folder (absent: the agent's default folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    /// The login it must have; another login in its folder is never picked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Tried first when lower (none: after every numbered one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i64>,
    /// False: listed, never picked or looked at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// A line for the owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The work groups it takes (the settings' `groups`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<String>>,
    /// It takes only its groups' work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups_only: Option<bool>,
    /// Picked on its own only for work that asks for it, or when every other is out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opt_in: Option<bool>,
    /// `api` and `bedrock`: the file in the runner's secrets folder holding the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_file: Option<String>,
    /// `api`: an Anthropic-compatible endpoint (none: Anthropic's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// `bedrock`: the AWS region.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// `bedrock`: a model id to the provider's id or inference profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<BTreeMap<String, String>>,
}

impl Account {
    /// Its provider.
    #[must_use]
    pub fn provider(&self) -> Provider {
        self.provider.unwrap_or_default()
    }

    /// Whether it may be picked or looked at.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled != Some(false)
    }

    /// Whether it is opt-in.
    #[must_use]
    pub fn is_opt_in(&self) -> bool {
        self.opt_in == Some(true)
    }

    /// Whether it may take work in these groups: group work only on an account of one of them,
    /// other work only on one not kept to its groups.
    #[must_use]
    pub fn takes(&self, in_groups: &[String]) -> bool {
        self.is_enabled()
            && if in_groups.is_empty() {
                self.groups_only != Some(true)
            } else {
                in_groups
                    .iter()
                    .any(|g| self.groups.as_ref().is_some_and(|gs| gs.contains(g)))
            }
    }

    /// Whether this account's agent can run the model: Bedrock only what it has an id for.
    #[must_use]
    pub fn can_run(&self, model: Option<&str>) -> bool {
        let bedrock = self.provider() == Provider::Bedrock;
        let Some(m) = model.filter(|m| !m.is_empty()) else {
            return true;
        };
        if !bedrock {
            return !is_bedrock_model(m);
        }
        let base = strip_wide(m).0;
        is_bedrock_model(base)
            || ["opus", "sonnet", "haiku", "fable", "default", "opusplan"]
                .iter()
                .any(|a| a.eq_ignore_ascii_case(base))
            || self.models.as_ref().is_some_and(|ms| ms.contains_key(base))
    }

    /// The model as this account's agent takes it: on Bedrock a model id becomes its Bedrock id.
    #[must_use]
    pub fn model_for(&self, m: &str) -> String {
        let (base, wide) = strip_wide(m);
        match self.models.as_ref().and_then(|ms| ms.get(base)) {
            Some(mapped) if self.provider() == Provider::Bedrock && !is_bedrock_model(base) => {
                format!("{mapped}{wide}")
            }
            _ => m.to_owned(),
        }
    }

    /// The key file it reads: its own, or the provider's usual name.
    #[must_use]
    pub fn key_file(&self) -> Option<&str> {
        self.key_file.as_deref().or(match self.provider() {
            Provider::Bedrock => Some("aws-bedrock-api-key"),
            Provider::Api => Some("anthropic-api-key"),
            Provider::Claude => None,
        })
    }
}

/// `m` without a `[1m]` ending, and the ending.
fn strip_wide(m: &str) -> (&str, &str) {
    let n = m.len();
    if n >= 4 && m.is_char_boundary(n - 4) && m[n - 4..].eq_ignore_ascii_case("[1m]") {
        (&m[..n - 4], &m[n - 4..])
    } else {
        (m, "")
    }
}

static BEDROCK: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^(?:arn:aws:bedrock:|(?:[a-z]{2,7}\.)?anthropic\.)").ok());
static ID: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]{1,40}$").ok());

/// A model id or inference profile only AWS Bedrock runs.
#[must_use]
pub fn is_bedrock_model(m: &str) -> bool {
    BEDROCK.as_ref().is_some_and(|re| re.is_match(m))
}

/// Whether `id` is an account id the server takes.
#[must_use]
pub fn is_account_id(id: &str) -> bool {
    ID.as_ref().is_some_and(|re| re.is_match(id))
}

/// A list that is wrong, with why.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct AccountsError(pub String);

/// The list a new runner starts with: the agent's default folder, whatever login it has.
#[must_use]
pub fn starter() -> Vec<Account> {
    vec![Account {
        id: "default".to_owned(),
        name: "This machine's login".to_owned(),
        provider: Some(Provider::Claude),
        ..Account::default()
    }]
}

/// Reads the accounts file, writing the starter list first when there is none.
///
/// # Errors
/// The file can't be read, isn't a list of accounts, or lists one twice.
pub fn read_accounts(file: &Path) -> Result<Vec<Account>, AccountsError> {
    if !file.exists() {
        write_accounts(file, &starter())?;
    }
    let text = std::fs::read_to_string(file)
        .map_err(|e| AccountsError(format!("cannot read {}: {e}", file.display())))?;
    parse_accounts(&text, &file.display().to_string())
}

/// Parses and checks a list of accounts.
///
/// # Errors
/// It isn't a list of accounts, or lists one twice.
pub fn parse_accounts(text: &str, at: &str) -> Result<Vec<Account>, AccountsError> {
    let raw: Vec<serde_json::Value> = serde_json::from_str(text)
        .map_err(|e| AccountsError(format!("{at} must hold a list of accounts: {e}")))?;
    let mut list = Vec::with_capacity(raw.len());
    for (i, v) in raw.into_iter().enumerate() {
        let a: Account =
            serde_json::from_value(v).map_err(|e| AccountsError(format!("{at} [{i}]: {e}")))?;
        if !is_account_id(&a.id) {
            return Err(AccountsError(format!(
                "{at} [{i}]: id must be letters, digits, - and _"
            )));
        }
        if a.name.is_empty() {
            return Err(AccountsError(format!(
                "{at} [{i}] ({}): name is missing",
                a.id
            )));
        }
        if list.iter().any(|x: &Account| x.id == a.id) {
            return Err(AccountsError(format!(
                "{at}: account {} is listed twice",
                a.id
            )));
        }
        list.push(a);
    }
    Ok(list)
}

/// Writes the accounts file (only the owner may read it).
///
/// # Errors
/// It can't be written.
pub fn write_accounts(file: &Path, list: &[Account]) -> Result<(), AccountsError> {
    let fail = |e: std::io::Error| AccountsError(format!("cannot write {}: {e}", file.display()));
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let mut text = serde_json::to_string_pretty(list).map_err(|e| AccountsError(e.to_string()))?;
    text.push('\n');
    crate::config::write_private(file, text.as_bytes()).map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bedrock() -> Account {
        Account {
            id: "aws".to_owned(),
            name: "AWS".to_owned(),
            provider: Some(Provider::Bedrock),
            models: Some(
                [(
                    "claude-opus-5-5".to_owned(),
                    "global.anthropic.claude-opus-5-5".to_owned(),
                )]
                .into(),
            ),
            ..Account::default()
        }
    }

    #[test]
    fn models_run_where_their_provider_can() {
        let sub = Account {
            id: "a".to_owned(),
            name: "A".to_owned(),
            ..Account::default()
        };
        assert!(
            sub.can_run(Some("opus")) && !sub.can_run(Some("global.anthropic.claude-opus-5-5"))
        );
        let aws = bedrock();
        assert!(
            aws.can_run(Some("claude-opus-5-5[1m]"))
                && aws.can_run(Some("sonnet"))
                && !aws.can_run(Some("claude-other"))
        );
        assert_eq!(
            aws.model_for("claude-opus-5-5[1m]"),
            "global.anthropic.claude-opus-5-5[1m]"
        );
        assert_eq!(sub.model_for("claude-opus-5-5"), "claude-opus-5-5");
    }

    #[test]
    fn groups_keep_work_to_their_accounts() {
        let kept = Account {
            id: "k".to_owned(),
            name: "K".to_owned(),
            groups: Some(vec!["client".to_owned()]),
            groups_only: Some(true),
            ..Account::default()
        };
        let any = Account {
            id: "a".to_owned(),
            name: "A".to_owned(),
            ..Account::default()
        };
        assert!(kept.takes(&["client".to_owned()]) && !kept.takes(&[]));
        assert!(any.takes(&[]) && !any.takes(&["client".to_owned()]));
    }

    #[test]
    fn a_wrong_list_says_where() {
        assert!(
            parse_accounts(r#"[{"id": "a b", "name": "x"}]"#, "f")
                .unwrap_err()
                .0
                .contains("[0]")
        );
        assert!(
            parse_accounts(
                r#"[{"id": "a", "name": "x"}, {"id": "a", "name": "y"}]"#,
                "f"
            )
            .unwrap_err()
            .0
            .contains("twice")
        );
        let l = parse_accounts(
            r#"[{"id": "a", "name": "x", "configDir": "~/.agent-a", "somethingElse": 1}]"#,
            "f",
        )
        .unwrap();
        assert_eq!(l[0].config_dir.as_deref(), Some("~/.agent-a"));
    }
}
