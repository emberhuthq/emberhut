//! An agent's conversations in its channels: what reaches it (`inbound`, through its bridge),
//! its answers (`agent_reply`) and the threads it reads (`agent_thread`), each only in a
//! conversation that is its own: one of its channels, or a thread it is in (`agent_threads`).
//!
//! Its answers are plain text posted with the bot token of its channel (for a direct message,
//! its channel's in that conversation's workspace): no blocks, buttons, mentions or formatting,
//! so nothing it posts can pass for an approval's buttons. Its files come only from its roots
//! (its own folder and its tasks' work folders). A disabled agent posts nothing.

use std::path::PathBuf;
use std::sync::Arc;

use emberhut_core::agents::{Agent, AgentId, Channel, ChannelKind};
use emberhut_core::{Clock, RoomId};
use emberhut_db::{AgentStore, DbError, MessageStore, ThreadPlace};

use super::scope::AgentRoots;
use super::slack::{AgentSlack, Place, ThreadMessage};
use crate::services::approvals::channels::{ChannelError, DmFile};
use crate::services::approvals::files::{self, FileRefused};
use crate::services::messaging::secrets::Secrets;
use crate::services::private::PrivatePaths;
use crate::services::talk::TalkService;
use crate::services::wake::WakeService;

/// The most messages `agent_thread` reads.
pub const THREAD_MAX: u32 = 100;

/// Why an agent's answer was not posted or its thread not read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConversationError {
    /// It may not do this; the text says why, for the agent.
    #[error("{0}")]
    Refused(String),
    /// A file it gave can't be sent.
    #[error(transparent)]
    File(#[from] FileRefused),
    /// Slack refused or could not be reached.
    #[error("Slack: {0}")]
    Slack(#[from] ChannelError),
    /// The database failed.
    #[error(transparent)]
    Db(#[from] DbError),
}

/// An answer to post: where, the words, and the absolute paths of its files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// The channel's id.
    pub channel: String,
    /// The thread to answer in; none: the conversation itself.
    pub thread: Option<String>,
    /// The words, plain text.
    pub text: String,
    /// Its files, read by the server.
    pub files: Vec<PathBuf>,
}

/// What [`AgentConversations`] is built from.
#[derive(Clone, Debug)]
pub struct ConversationParts {
    /// The agents, their channels and threads.
    pub agents: AgentStore,
    /// The rooms' messages.
    pub messages: MessageStore,
    /// The secrets directory: the bridges' tokens and the bots'.
    pub secrets: Secrets,
    /// Slack, as the agents' bots.
    pub slack: Arc<dyn AgentSlack>,
    /// The turns a message starts.
    pub talk: TalkService,
    /// The events a message waits as when its room is full.
    pub wake: WakeService,
    /// What never leaves the server.
    pub private: PrivatePaths,
    /// The folders an agent's files may come from.
    pub roots: AgentRoots,
    /// What time it is.
    pub clock: Arc<dyn Clock>,
}

/// An agent's conversations: inbound messages, its answers and the threads it reads. Cheap to
/// clone.
#[derive(Clone, Debug)]
pub struct AgentConversations {
    pub(super) inner: Arc<ConversationParts>,
}

impl AgentConversations {
    /// The service over `parts`.
    #[must_use]
    pub fn new(parts: ConversationParts) -> Self {
        Self {
            inner: Arc::new(parts),
        }
    }

    /// The agent whose room `room` is.
    ///
    /// # Errors
    ///
    /// The database's.
    pub async fn agent_of(&self, room: RoomId) -> Result<Agent, DbError> {
        self.inner.agents.by_room(room).await
    }

    /// Everything said in `room` after `since`, the events included, the newest `limit`.
    ///
    /// # Errors
    ///
    /// The database's.
    pub async fn said_since(
        &self,
        room: RoomId,
        since: jiff::Timestamp,
        limit: u64,
    ) -> Result<Vec<emberhut_core::message::Message>, DbError> {
        self.inner
            .messages
            .conversation_since(room, since, limit)
            .await
    }

    /// Posts `reply` as `agent`, in one of its conversations: its message's timestamp when it
    /// was words alone.
    ///
    /// # Errors
    ///
    /// [`ConversationError::Refused`] for a disabled agent, a conversation that isn't its own,
    /// or a channel it can't post in; [`ConversationError::File`], Slack's, or the database's.
    pub async fn reply(
        &self,
        agent: &Agent,
        reply: &Reply,
    ) -> Result<Option<String>, ConversationError> {
        if !agent.enabled {
            return Err(refused("The agent is disabled: nothing is posted."));
        }
        let (token, workspace) = self.token_for(&agent.id, &reply.channel).await?;
        let place = Place {
            channel: &reply.channel,
            thread: reply.thread.as_deref(),
        };
        let text = plain(&reply.text);
        let ts = if reply.files.is_empty() {
            Some(self.inner.slack.post(&token, place, &text).await?)
        } else {
            let roots = self.inner.roots.of(agent).await?;
            let read = files::read_all(&reply.files, &self.inner.private, Some(&roots)).await?;
            let mut upload = Vec::with_capacity(read.len());
            for file in &read {
                let Some(bytes) = files::bytes_if_unchanged(file).await else {
                    return Err(refused(&format!(
                        "{} changed while it was being sent: send it again.",
                        file.digest.name
                    )));
                };
                upload.push(DmFile {
                    name: file.digest.name.clone(),
                    bytes,
                });
            }
            self.inner
                .slack
                .upload(&token, place, &text, &upload)
                .await?;
            None
        };
        if let Some(thread) = reply.thread.as_deref().or(ts.as_deref()) {
            self.inner
                .agents
                .touch_thread(
                    &agent.id,
                    ThreadPlace {
                        channel: &reply.channel,
                        thread,
                        workspace: workspace.as_deref(),
                    },
                    Some(&agent.name),
                    self.inner.clock.now(),
                )
                .await?;
        }
        Ok(ts)
    }

    /// The last `limit` messages of `thread` in `channel`, a conversation of `agent`'s, oldest
    /// first.
    ///
    /// # Errors
    ///
    /// As [`AgentConversations::reply`].
    pub async fn thread(
        &self,
        agent: &Agent,
        channel: &str,
        thread: &str,
        limit: u32,
    ) -> Result<Vec<ThreadMessage>, ConversationError> {
        let (token, _) = self.token_for(&agent.id, channel).await?;
        let place = Place {
            channel,
            thread: Some(thread),
        };
        Ok(self
            .inner
            .slack
            .replies(&token, place, limit.clamp(1, THREAD_MAX))
            .await?)
    }

    /// The bot token `agent` posts in `channel` with, and that channel's Slack team: the token of
    /// its Slack channel `channel` is; or, for a conversation it is in that isn't one of its
    /// channels (a direct message), the token of its Slack channel in that conversation's
    /// workspace. None there: refused.
    async fn token_for(
        &self,
        agent: &AgentId,
        channel: &str,
    ) -> Result<(String, Option<String>), ConversationError> {
        let channels = self.inner.agents.channels(agent).await?;
        let slack: Vec<&Channel> = channels
            .iter()
            .filter(|known| known.kind == ChannelKind::Slack)
            .collect();
        let answered_by_its_bridge = channels
            .iter()
            .any(|known| known.kind.answered_by_its_bridge() && known.address == channel);
        if answered_by_its_bridge && !slack.iter().any(|known| known.address == channel) {
            return Err(refused(
                "agent_reply posts in Slack only: a WhatsApp chat or an app's chat is answered \
                 through its bridge's own tools.",
            ));
        }
        let chosen = if let Some(known) = slack.iter().find(|known| known.address == channel) {
            *known
        } else {
            let threads = self.inner.agents.threads(agent).await?;
            let mut in_it = threads
                .iter()
                .filter(|known| known.channel == channel)
                .peekable();
            if in_it.peek().is_none() {
                return Err(refused(&format!(
                    "{channel} isn't a conversation you're in: answer where the message came from."
                )));
            }
            let workspace = in_it.find_map(|known| known.workspace.clone());
            let Some(known) = workspace.and_then(|workspace| {
                slack
                    .iter()
                    .find(|known| known.workspace.as_deref() == Some(workspace.as_str()))
                    .copied()
            }) else {
                return Err(refused(
                    "You have no Slack channel in that conversation's workspace to post with.",
                ));
            };
            known
        };
        let Some(secret) = chosen.token_secret.as_ref() else {
            return Err(refused("You have no Slack channel to post with."));
        };
        let token = self.inner.secrets.read(secret.as_str()).await;
        if token.is_empty() {
            return Err(refused(
                "Your Slack bot's token isn't on this server: tell the owner.",
            ));
        }
        Ok((token, chosen.workspace.clone()))
    }
}

fn refused(text: &str) -> ConversationError {
    ConversationError::Refused(text.to_owned())
}

/// `text` as Slack shows it word for word: its control characters `&`, `<` and `>` escaped,
/// so it makes no mention, no `@channel` and no link of its own.
#[must_use]
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_makes_no_mention_or_link() {
        assert_eq!(
            plain("<!channel> see <https://x.example|here> & more"),
            "&lt;!channel&gt; see &lt;https://x.example|here&gt; &amp; more"
        );
        assert_eq!(plain("just words"), "just words");
    }
}
