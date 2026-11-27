//! An agent's accounts: which credentials its MCP servers use, named, never shown.

use std::collections::HashSet;

use super::error::AgentError;
use crate::secret_link::SecretName;

/// The longest label, in bytes.
const LABEL_MAX: usize = 32;

/// The environment variable prefix an account's secret file path is handed to an agent's MCP
/// servers under.
pub const ACCOUNT_ENV_PREFIX: &str = "EMBERHUT_AGENT_ACCOUNT_";

/// A credential an agent's MCP servers may use: a label and the secret file holding it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentAccount {
    /// Its label: 1 to 32 of `a-z`, `0-9` and `_`, starting with a letter.
    pub label: String,
    /// The secret file's name.
    pub secret: SecretName,
}

impl AgentAccount {
    /// The environment variable that hands its secret file's path (never its contents) to the
    /// agent's MCP servers: `EMBERHUT_AGENT_ACCOUNT_<LABEL>`.
    #[must_use]
    pub fn env_var(&self) -> String {
        format!("{ACCOUNT_ENV_PREFIX}{}", self.label.to_ascii_uppercase())
    }

    fn check(&self) -> Result<(), &'static str> {
        let fits = self
            .label
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase())
            && self.label.len() <= LABEL_MAX
            && self
                .label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if fits {
            Ok(())
        } else {
            Err("a label is 1 to 32 of a-z, 0-9 and _, starting with a letter")
        }
    }
}

/// Checks an agent's whole list of accounts: each label's shape, and no label twice.
///
/// # Errors
///
/// [`AgentError::Account`] naming the first one refused.
pub fn validate_accounts(accounts: &[AgentAccount]) -> Result<(), AgentError> {
    let mut seen = HashSet::new();
    for (index, account) in accounts.iter().enumerate() {
        account
            .check()
            .map_err(|why| AgentError::Account { index, why })?;
        if !seen.insert(account.label.as_str()) {
            return Err(AgentError::Account {
                index,
                why: "label listed twice",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(label: &str) -> AgentAccount {
        AgentAccount {
            label: label.to_owned(),
            secret: SecretName::parse("example-admin-token").unwrap(),
        }
    }

    #[test]
    fn labels_name_an_environment_variable() {
        assert_eq!(
            account("admin_2").env_var(),
            "EMBERHUT_AGENT_ACCOUNT_ADMIN_2"
        );
        assert_eq!(
            validate_accounts(&[account("admin"), account("ads")]),
            Ok(())
        );
    }

    #[test]
    fn bad_or_repeated_labels_are_refused() {
        for label in ["", "Admin", "2fa", "a-b", &"a".repeat(33)] {
            assert!(
                matches!(
                    validate_accounts(&[account(label)]),
                    Err(AgentError::Account { index: 0, .. })
                ),
                "{label}"
            );
        }
        assert_eq!(
            validate_accounts(&[account("admin"), account("admin")]),
            Err(AgentError::Account {
                index: 1,
                why: "label listed twice"
            })
        );
    }
}
