//! `messages_chats`, `messages_read`, `messages_search` and `replied`: the owner's messages
//! everywhere (iMessage, SMS and WhatsApp from the Mac, Slack, email), read-only.

use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};

/// What every reading tool's description ends with.
macro_rules! private {
    ($text:literal) => {
        concat!(
            $text,
            " Only when the user asks about their messages. Quote only what answers them; other \
             people's words are private. Never send anything."
        )
    };
}

const fn seven() -> i64 {
    7
}

const fn forty() -> u64 {
    40
}

const fn ninety() -> i64 {
    90
}

const fn thirty() -> u64 {
    30
}

/// The owner's recent chats everywhere.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MessagesChats;

/// `messages_chats`' arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ChatsArgs {
    /// How many days back.
    #[serde(default = "seven")]
    #[schema(default = 7, minimum = 1, maximum = 90)]
    #[garde(range(min = 1, max = 90))]
    days: i64,
}

impl Tool for MessagesChats {
    type Args = ChatsArgs;

    fn name(&self) -> &'static str {
        "messages_chats"
    }

    fn description(&self) -> &'static str {
        private!(
            "The user's recent chats: iMessage, SMS, WhatsApp (names, last message time, unread), \
             Slack (what Slack notified them about: DMs, mentions, their threads) and email (their \
             Gmail accounts: inbox and sent, one chat per person, ids email:<account>:<address>)."
        )
    }

    async fn call(&self, ctx: &ToolContext, args: ChatsArgs) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(
            ctx.app.messaging().messages().chats(args.days).await,
        ))
    }
}

/// One chat's latest messages.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MessagesRead;

/// `messages_read`' arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ReadArgs {
    /// The chat's id, from `messages_chats` or `messages_search`.
    #[garde(length(min = 1, max = 500))]
    chat: String,
    /// How many messages.
    #[serde(default = "forty")]
    #[schema(default = 40, minimum = 1, maximum = 200)]
    #[garde(range(min = 1, max = 200))]
    limit: u64,
}

impl Tool for MessagesRead {
    type Args = ReadArgs;

    fn name(&self) -> &'static str {
        "messages_read"
    }

    fn description(&self) -> &'static str {
        private!(
            "The latest messages in one chat (id from messages_chats or messages_search; Slack ids \
             start with slack:, email with email:: every email with that person, from Gmail \
             itself), oldest first."
        )
    }

    async fn call(&self, ctx: &ToolContext, args: ReadArgs) -> Result<ToolOutput, ToolError> {
        let read = ctx
            .app
            .messaging()
            .messages()
            .read(args.chat.trim(), args.limit)
            .await;
        Ok(ToolOutput::text(read))
    }
}

/// The owner's messages by words or a name.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MessagesSearch;

/// `messages_search`' arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct SearchArgs {
    /// Words or a person's name.
    #[garde(length(min = 2, max = 500))]
    q: String,
    /// How many days back.
    #[serde(default = "ninety")]
    #[schema(default = 90, minimum = 1, maximum = 3650)]
    #[garde(range(min = 1, max = 3650))]
    days: i64,
    /// How many hits.
    #[serde(default = "thirty")]
    #[schema(default = 30, minimum = 1, maximum = 100)]
    #[garde(range(min = 1, max = 100))]
    limit: u64,
}

impl Tool for MessagesSearch {
    type Args = SearchArgs;

    fn name(&self) -> &'static str {
        "messages_search"
    }

    fn description(&self) -> &'static str {
        private!(
            "Search the user's messages (iMessage, SMS, WhatsApp, Slack, and their email: Gmail's \
             own search of their whole mailboxes, so Gmail's words work there too, from:ann) by \
             words or a person's name, newest first (default last 90 days)."
        )
    }

    async fn call(&self, ctx: &ToolContext, args: SearchArgs) -> Result<ToolOutput, ToolError> {
        let found = ctx
            .app
            .messaging()
            .messages()
            .search(&args.q, args.days, args.limit)
            .await;
        Ok(ToolOutput::text(found))
    }
}

/// Whether the owner already answered in a chat.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RepliedTool;

/// `replied`' arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct RepliedArgs {
    /// The chat's id (`messages_chats` or `messages_search`).
    #[garde(length(min = 1, max = 500))]
    chat: String,
    /// When the message to answer arrived (ISO); left out, 3 days ago.
    #[garde(length(max = 64))]
    since: Option<String>,
}

impl Tool for RepliedTool {
    type Args = RepliedArgs;

    fn name(&self) -> &'static str {
        "replied"
    }

    fn description(&self) -> &'static str {
        "Did the user already write in this chat? Call it before drafting or proposing any reply \
         for them. chat: its id (messages_chats or messages_search). since: when the message \
         you'd answer arrived (ISO; default 3 days ago). Returns answered and their messages there \
         since then."
    }

    async fn call(&self, ctx: &ToolContext, args: RepliedArgs) -> Result<ToolOutput, ToolError> {
        let replied = ctx
            .app
            .messaging()
            .messages()
            .replied(args.chat.trim(), args.since.as_deref())
            .await
            .map_err(ToolError::Refused)?;
        serde_json::to_string(&replied)
            .map(ToolOutput::text)
            .map_err(ToolError::failed)
    }
}
