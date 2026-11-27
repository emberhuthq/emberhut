//! Who approves: the owner, an external approver, or nobody; and how an external approver is
//! reached and recognised.

use std::borrow::Cow;
use std::fmt;

use url::Url;

use crate::agents::{is_slack_id, is_slug};
use crate::secret_link::SecretName;

/// The longest approver name, in characters.
pub const APPROVER_NAME_MAX: usize = 80;

/// The words an [`Approver`] is written as that no approver's id may be.
const RESERVED: [&str; 2] = [Approver::OWNER, Approver::NOBODY];

/// An external approver's id: a slug like an agent's (1 to 40 of `a-z`, `0-9` and `-`, not
/// starting with `-`), and never `owner` or `none`, which name the other approvers.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ApproverId(String);

/// Text that is not an approver's id.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{0:?} is not an approver's id: 1 to 40 of a-z, 0-9 and '-', not starting with '-', and \
     not 'owner' or 'none'"
)]
pub struct InvalidApproverId(pub String);

impl ApproverId {
    /// The id, if `text` is one.
    ///
    /// # Errors
    ///
    /// [`InvalidApproverId`].
    pub fn parse(text: &str) -> Result<Self, InvalidApproverId> {
        if is_slug(text) && !RESERVED.contains(&text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(InvalidApproverId(text.to_owned()))
        }
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ApproverId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who approves an action.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Approver {
    /// The owner, from a signed-in device.
    Owner,
    /// An external approver, by id.
    External(ApproverId),
    /// Nobody: the action needs no approval. Never for the fixed core.
    Nobody,
}

impl Approver {
    /// How [`Approver::Owner`] is written.
    pub const OWNER: &'static str = "owner";
    /// How [`Approver::Nobody`] is written.
    pub const NOBODY: &'static str = "none";

    /// How it is written in the database and on the wire: `owner`, `none`, or the approver's
    /// id.
    #[must_use]
    pub fn as_str(&self) -> Cow<'_, str> {
        match self {
            Self::Owner => Cow::Borrowed(Self::OWNER),
            Self::Nobody => Cow::Borrowed(Self::NOBODY),
            Self::External(id) => Cow::Borrowed(id.as_str()),
        }
    }

    /// The approver written as `text`.
    ///
    /// ```
    /// use emberhut_core::approvals::{Approver, ApproverId};
    ///
    /// assert_eq!(Approver::parse("owner"), Ok(Approver::Owner));
    /// assert_eq!(Approver::parse("none"), Ok(Approver::Nobody));
    /// assert_eq!(
    ///     Approver::parse("example-lead"),
    ///     Ok(Approver::External(ApproverId::parse("example-lead").unwrap()))
    /// );
    /// ```
    ///
    /// # Errors
    ///
    /// [`InvalidApproverId`] for text that is neither word nor an id.
    pub fn parse(text: &str) -> Result<Self, InvalidApproverId> {
        match text {
            Self::OWNER => Ok(Self::Owner),
            Self::NOBODY => Ok(Self::Nobody),
            id => ApproverId::parse(id).map(Self::External),
        }
    }

    /// The role an agent is told, never the approver's identity: "the owner", "the approver"
    /// or "nobody".
    #[must_use]
    pub fn role(&self) -> &'static str {
        match self {
            Self::Owner => "the owner",
            Self::External(_) => "the approver",
            Self::Nobody => "nobody",
        }
    }
}

impl fmt::Display for Approver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

/// An approver's Slack identity: the team and the user whose button clicks count.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SlackIdentity {
    /// The team's id (`T0EXAMPLE`).
    pub workspace: String,
    /// The user's id (`U0APPROVER`).
    pub user: String,
}

/// An approver's own app: where requests are posted, and the secret both sides sign with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookTarget {
    /// Where requests are posted: an `https` URL.
    pub url: Url,
    /// The secret file holding the key both sides sign with.
    pub secret: SecretName,
}

/// An external approver as the owner set them up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApproverRecord {
    /// Its id.
    pub id: ApproverId,
    /// Its name, as the apps show it.
    pub name: String,
    /// Where its Slack DMs go and whose clicks count.
    pub slack: Option<SlackIdentity>,
    /// Its own app.
    pub webhook: Option<WebhookTarget>,
    /// A signed link may be sent to it.
    pub link: bool,
}

/// Why an approver can't be what was asked.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ApproverError {
    /// Its name is blank or too long.
    #[error("an approver's name is 1 to {APPROVER_NAME_MAX} characters")]
    Name,
    /// A Slack team or user id that is not one.
    #[error("a Slack team or user id is capital letters and digits")]
    Slack,
    /// A webhook URL that is not `https`.
    #[error("a webhook's URL is https, with a host")]
    WebhookUrl,
    /// No way to reach it: no Slack, no webhook.
    #[error("an approver is reached through Slack, a webhook, or both")]
    Unreachable,
}

impl ApproverRecord {
    /// Checks the approver: a name, well-formed Slack ids, an `https` webhook, and at least one
    /// way to reach them (a link alone has nowhere to go: it is sent through Slack or the
    /// webhook).
    ///
    /// # Errors
    ///
    /// [`ApproverError`].
    pub fn validate(&self) -> Result<(), ApproverError> {
        let name = self.name.trim();
        if name.is_empty() || self.name.chars().count() > APPROVER_NAME_MAX {
            return Err(ApproverError::Name);
        }
        if let Some(slack) = &self.slack
            && !(is_slack_id(&slack.workspace) && is_slack_id(&slack.user))
        {
            return Err(ApproverError::Slack);
        }
        if let Some(webhook) = &self.webhook
            && (webhook.url.scheme() != "https" || webhook.url.host().is_none())
        {
            return Err(ApproverError::WebhookUrl);
        }
        if self.slack.is_none() && self.webhook.is_none() {
            return Err(ApproverError::Unreachable);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> ApproverRecord {
        ApproverRecord {
            id: ApproverId::parse("example-lead").unwrap(),
            name: "Example Lead".to_owned(),
            slack: Some(SlackIdentity {
                workspace: "T0EXAMPLE".to_owned(),
                user: "U0APPROVER".to_owned(),
            }),
            webhook: Some(WebhookTarget {
                url: Url::parse("https://approvals.example.com/hook").unwrap(),
                secret: SecretName::parse("example-webhook").unwrap(),
            }),
            link: true,
        }
    }

    #[test]
    fn ids_are_slugs_and_not_the_reserved_words() {
        assert!(ApproverId::parse("example-lead").is_ok());
        for bad in ["owner", "none", "", "Lead", "-a"] {
            assert_eq!(
                ApproverId::parse(bad),
                Err(InvalidApproverId(bad.to_owned()))
            );
        }
    }

    #[test]
    fn approvers_round_trip() {
        for approver in [
            Approver::Owner,
            Approver::Nobody,
            Approver::External(ApproverId::parse("a").unwrap()),
        ] {
            assert_eq!(Approver::parse(&approver.as_str()), Ok(approver.clone()));
            assert_eq!(approver.to_string(), approver.as_str());
        }
        assert!(Approver::parse("Owner").is_err());
    }

    #[test]
    fn an_agent_hears_a_role_only() {
        let external = Approver::External(ApproverId::parse("example-lead").unwrap());
        assert_eq!(external.role(), "the approver");
        assert_eq!(Approver::Owner.role(), "the owner");
    }

    #[test]
    fn records_are_checked() {
        assert_eq!(record().validate(), Ok(()));
        let mut r = record();
        r.name = " ".to_owned();
        assert_eq!(r.validate(), Err(ApproverError::Name));
        let mut r = record();
        r.slack = Some(SlackIdentity {
            workspace: "T0EXAMPLE".to_owned(),
            user: "someone".to_owned(),
        });
        assert_eq!(r.validate(), Err(ApproverError::Slack));
        let mut r = record();
        if let Some(webhook) = &mut r.webhook {
            webhook.url = Url::parse("http://approvals.example.com/hook").unwrap();
        }
        assert_eq!(r.validate(), Err(ApproverError::WebhookUrl));
        let mut r = record();
        r.slack = None;
        r.webhook = None;
        assert_eq!(r.validate(), Err(ApproverError::Unreachable));
        let mut r = record();
        r.webhook = None;
        assert_eq!(r.validate(), Ok(()), "Slack alone reaches them");
    }
}
