//! The assistant's accounts: every account the runners use and its use (`GET /api/accounts`,
//! the state's `accounts`), the Usage card (the state's `usage`), the account the assistant
//! itself runs on (`assistantAccount`) and the person's pin
//! (`POST /api/accounts/use`).
//!
//! Use is in percent (0 to 100) here; the state's `account` has it as a share (0 to 1).

use serde::{Deserialize, Serialize};

use crate::runner::accounts::ClaudeAccount;
use crate::usage::AssistantAccount;

/// `GET /api/accounts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccountsPage {
    /// Every account the runners use, by machine.
    pub accounts: Vec<ClaudeAccount>,
    /// The account the assistant runs on (`assistantAccount`, listed in [`crate::wire`]).
    #[serde(rename = "assistantAccount")]
    pub assistant: AssistantAccount,
}
