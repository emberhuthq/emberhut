//! An agent's channels: the conversations it is in, outside the server.

use std::collections::HashSet;

use super::error::AgentError;
use crate::secret_link::SecretName;
use crate::wire::wire_enum;

/// The longest channel address, in bytes.
const ADDRESS_MAX: usize = 128;
/// The longest Slack id (a team's or a channel's), in bytes.
const SLACK_ID_MAX: usize = 32;

wire_enum! {
    /// Where a channel is.
    pub enum ChannelKind {
        /// A Slack channel or conversation, through the agent's own bot.
        Slack = "slack",
        /// A WhatsApp chat.
        WhatsApp = "whatsapp",
    }
}

/// One conversation an agent is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    /// Where it is.
    pub kind: ChannelKind,
    /// The Slack team's id (`T0EXAMPLE`); none for WhatsApp.
    pub workspace: Option<String>,
    /// The channel's id (`C0EXAMPLE`), or a WhatsApp chat's id.
    pub address: String,
    /// For Slack, the secret holding the bot token, read at every use; none for WhatsApp.
    pub token_secret: Option<SecretName>,
}

impl Channel {
    /// Checks the channel: Slack's with a team id, a channel id and its token's secret;
    /// WhatsApp's with a chat id only.
    ///
    /// # Errors
    ///
    /// Why not, in words.
    pub fn check(&self) -> Result<(), &'static str> {
        match self.kind {
            ChannelKind::Slack => {
                if !self.workspace.as_deref().is_some_and(is_slack_id) {
                    return Err("a Slack channel needs its team's id (capital letters and digits)");
                }
                if !is_slack_id(&self.address) {
                    return Err("a Slack channel's address is its id (capital letters and digits)");
                }
                if self.token_secret.is_none() {
                    return Err("a Slack channel needs the secret holding its bot token");
                }
            }
            ChannelKind::WhatsApp => {
                if self.workspace.is_some() || self.token_secret.is_some() {
                    return Err("a WhatsApp channel has no workspace and no token secret");
                }
                let fits = !self.address.is_empty()
                    && self.address.len() <= ADDRESS_MAX
                    && !self
                        .address
                        .chars()
                        .any(|c| c.is_whitespace() || c.is_control());
                if !fits {
                    return Err("a WhatsApp channel's address is its chat's id, without spaces");
                }
            }
        }
        Ok(())
    }

    /// Whether this is the channel a message from `kind`, `workspace` and `address` came from.
    #[must_use]
    pub fn is(&self, kind: ChannelKind, workspace: Option<&str>, address: &str) -> bool {
        self.kind == kind && self.workspace.as_deref() == workspace && self.address == address
    }
}

/// Whether `text` is a Slack id (a team's, a channel's or a user's): capital letters and
/// digits.
pub(crate) fn is_slack_id(text: &str) -> bool {
    (2..=SLACK_ID_MAX).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Checks an agent's whole list of channels: each one, and none twice.
///
/// # Errors
///
/// [`AgentError::Channel`] naming the first one refused.
pub fn validate_channels(channels: &[Channel]) -> Result<(), AgentError> {
    let mut seen = HashSet::new();
    for (index, channel) in channels.iter().enumerate() {
        channel
            .check()
            .map_err(|why| AgentError::Channel { index, why })?;
        if !seen.insert((
            channel.kind,
            channel.workspace.as_deref(),
            channel.address.as_str(),
        )) {
            return Err(AgentError::Channel {
                index,
                why: "listed twice",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slack() -> Channel {
        Channel {
            kind: ChannelKind::Slack,
            workspace: Some("T0EXAMPLE".to_owned()),
            address: "C0EXAMPLE".to_owned(),
            token_secret: Some(SecretName::parse("slack-example-bot").unwrap()),
        }
    }

    fn whatsapp() -> Channel {
        Channel {
            kind: ChannelKind::WhatsApp,
            workspace: None,
            address: "15550100102@s.whatsapp.net".to_owned(),
            token_secret: None,
        }
    }

    #[test]
    fn good_channels() {
        assert_eq!(validate_channels(&[slack(), whatsapp()]), Ok(()));
        assert_eq!(validate_channels(&[]), Ok(()));
    }

    #[test]
    fn slack_needs_its_parts() {
        let changes: [fn(&mut Channel); 4] = [
            |c| c.workspace = None,
            |c| c.workspace = Some("t0example".to_owned()),
            |c| c.address = "#general".to_owned(),
            |c| c.token_secret = None,
        ];
        for change in changes {
            let mut channel = slack();
            change(&mut channel);
            assert!(matches!(
                validate_channels(&[channel]),
                Err(AgentError::Channel { index: 0, .. })
            ));
        }
    }

    #[test]
    fn whatsapp_has_no_workspace_or_token() {
        let mut channel = whatsapp();
        channel.workspace = Some("T0EXAMPLE".to_owned());
        assert!(channel.check().is_err());
        let mut channel = whatsapp();
        channel.address = "a chat".to_owned();
        assert!(channel.check().is_err());
    }

    #[test]
    fn a_channel_listed_twice_is_refused() {
        assert_eq!(
            validate_channels(&[whatsapp(), slack(), slack()]),
            Err(AgentError::Channel {
                index: 2,
                why: "listed twice"
            })
        );
    }

    #[test]
    fn a_message_is_from_a_channel() {
        assert!(slack().is(ChannelKind::Slack, Some("T0EXAMPLE"), "C0EXAMPLE"));
        assert!(!slack().is(ChannelKind::Slack, Some("T0OTHER"), "C0EXAMPLE"));
        assert!(!slack().is(ChannelKind::WhatsApp, None, "C0EXAMPLE"));
    }

    #[test]
    fn kinds_have_wire_words() {
        assert_eq!(ChannelKind::WhatsApp.as_str(), "whatsapp");
        assert_eq!(ChannelKind::parse("slack"), Some(ChannelKind::Slack));
    }
}
