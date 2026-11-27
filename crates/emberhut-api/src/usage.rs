//! The Claude accounts' use, for Settings' usage card: the state's `usage` (one row per account
//! the assistant may run on, with its 5-hour and weekly use and when each resets) and the
//! account the assistant runs on now, Auto or pinned by the person.
//!
//! `POST /api/usage/check` asks the server to check every account now (at most once in two
//! minutes there; the state's poll brings the numbers as they land). `POST /api/accounts/use`
//! pins the assistant to an account, or sets it back to Auto; its rooms move between turns.
//! Names and percentages only: never an email or a login.

use serde::{Deserialize, Serialize};

use crate::ids::RoomId;
use crate::time::Timestamp;

/// `scope` of an account that may do any work. Any other scope is the server's word for the
/// work an account is kept for ([`UsageRow::is_kept_for_other_work`]).
pub const SCOPE_ANY: &str = "any";

/// `account` of [`UseAccountRequest`] that lets the assistant pick.
pub const AUTO: &str = "auto";

/// One account's use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UsageRow {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// What it is for: [`SCOPE_ANY`], or the server's word for the work it is kept for (empty
    /// when the server doesn't say).
    #[serde(default)]
    pub scope: String,
    /// It is logged in.
    #[serde(default = "logged_in")]
    pub logged_in: bool,
    /// Its use of the 5-hour window, 0 to 100; none: no data.
    #[serde(default, rename = "used5h")]
    pub used_5h: Option<f64>,
    /// When that window resets.
    #[serde(default, rename = "resets5h")]
    pub resets_5h: Option<Timestamp>,
    /// Its use of the week, 0 to 100; none: no data.
    #[serde(default, rename = "used7d")]
    pub used_7d: Option<f64>,
    /// When the week resets.
    #[serde(default, rename = "resets7d")]
    pub resets_7d: Option<Timestamp>,
    /// It is out until then.
    #[serde(default)]
    pub limited_until: Option<Timestamp>,
    /// When its numbers were checked.
    #[serde(default)]
    pub checked: Option<Timestamp>,
    /// The machines it is on, in words (`Linux PC`, `Mac`).
    #[serde(default)]
    pub places: Vec<String>,
    /// It has the most room of them.
    #[serde(default)]
    pub best: bool,
}

/// An older server leaves `loggedIn` out of a row that is.
fn logged_in() -> bool {
    true
}

impl UsageRow {
    /// Whether the assistant may be pinned to it: one for any work.
    #[must_use]
    pub fn can_be_pinned(&self) -> bool {
        !self.is_kept_for_other_work()
    }

    /// Whether the server keeps it for some work only (a scope other than [`SCOPE_ANY`]).
    #[must_use]
    pub fn is_kept_for_other_work(&self) -> bool {
        !self.scope.is_empty() && self.scope != SCOPE_ANY
    }

    /// Whether it has numbers to show.
    #[must_use]
    pub fn has_numbers(&self) -> bool {
        self.used_5h.is_some() || self.used_7d.is_some()
    }
}

/// The account the assistant runs on, and the person's choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AssistantAccount {
    /// The account Main runs on now.
    #[serde(default)]
    pub on: Option<String>,
    /// The account the person pinned it to; none: Auto, the account with room.
    #[serde(default)]
    pub pin: Option<String>,
    /// `auto` or `pinned`.
    #[serde(default)]
    pub mode: String,
    /// The pinned account can't take work now: the assistant runs elsewhere until it can.
    #[serde(default)]
    pub pin_out: bool,
    /// The open rooms and the account each runs on.
    #[serde(default)]
    pub rooms: Vec<AccountRoom>,
    /// After a change: the rooms that move later, and when (`Room 2 moves when its turn
    /// ends`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits: Option<Vec<String>>,
}

impl AssistantAccount {
    /// Whether the person pinned it to an account.
    #[must_use]
    pub fn is_pinned(&self) -> bool {
        self.pin.is_some()
    }
}

/// An open room and the account it runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AccountRoom {
    /// The room.
    pub room: RoomId,
    /// Its name.
    pub name: String,
    /// Its account.
    #[serde(default)]
    pub account: Option<String>,
}

/// `POST /api/usage/check`'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UsageCheckReply {
    /// `true`.
    pub ok: bool,
    /// Whether a check started; `false` when one did less than two minutes ago.
    pub asked: bool,
}

/// `POST /api/accounts/use`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UseAccountRequest {
    /// An account's id, or [`AUTO`].
    pub account: String,
}

impl UseAccountRequest {
    /// Lets the assistant pick the account with room.
    #[must_use]
    pub fn auto() -> Self {
        Self {
            account: AUTO.to_owned(),
        }
    }

    /// Pins the assistant to account `id`.
    #[must_use]
    pub fn pin(id: impl Into<String>) -> Self {
        Self { account: id.into() }
    }

    /// The account it pins, none for Auto.
    #[must_use]
    pub fn pinned(&self) -> Option<&str> {
        (self.account != AUTO && !self.account.is_empty()).then_some(self.account.as_str())
    }
}

/// `POST /api/accounts/use`'s answer: the account as it is now and every account's use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UseAccountReply {
    /// `true`.
    pub ok: bool,
    /// The account the assistant runs on now, and the choice.
    #[serde(flatten)]
    pub account: AssistantAccount,
    /// Every account's use.
    #[serde(default)]
    pub usage: Vec<UsageRow>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_a_word_and_a_pin_is_an_id() {
        assert_eq!(UseAccountRequest::auto().pinned(), None);
        assert_eq!(UseAccountRequest::pin("a-1").pinned(), Some("a-1"));
        assert_eq!(
            serde_json::to_string(&UseAccountRequest::auto()).unwrap(),
            r#"{"account":"auto"}"#
        );
    }

    #[test]
    fn only_an_account_for_any_work_takes_a_pin() {
        let row: UsageRow =
            serde_json::from_str(r#"{"id": "a", "name": "A", "scope": "example"}"#).unwrap();
        assert!(!row.can_be_pinned() && row.is_kept_for_other_work());
        assert!(row.logged_in, "an older server's row");
        assert!(!row.has_numbers());
    }
}
