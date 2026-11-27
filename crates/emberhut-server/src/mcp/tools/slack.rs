//! `slack_file` and `slack_send`: a file in a Slack message, fetched; and one message as the
//! owner, only after the owner's OK on the exact text.

use emberhut_api::mcp::Content;
use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::messaging::slack::SlackFileError;

/// A file in a Slack message.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SlackFileTool;

/// `slack_file`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct SlackFileArgs {
    /// The file's id, F….
    #[garde(length(min = 1, max = 64))]
    file: String,
}

impl Tool for SlackFileTool {
    type Args = SlackFileArgs;

    fn name(&self) -> &'static str {
        "slack_file"
    }

    fn description(&self) -> &'static str {
        "A file in a Slack message (shown as \"[file: name, kind; slack_file F…]\"): fetched with \
         the user's Slack token and saved here; returns its path (read it: a PDF, a document) and \
         a picture to look at. Only when the user asks about their messages. Quote only what \
         answers them; other people's words are private. Never send anything."
    }

    async fn call(&self, ctx: &ToolContext, args: SlackFileArgs) -> Result<ToolOutput, ToolError> {
        match ctx.app.messaging().slack().file(args.file.trim()).await {
            Ok((file, image)) => {
                let info = serde_json::to_string(&file).map_err(ToolError::failed)?;
                let mut content = Vec::new();
                if let Some(image) = image {
                    content.push(Content::Image {
                        data: image.data,
                        mime_type: image.mime_type,
                    });
                }
                content.push(Content::text(info));
                Ok(ToolOutput {
                    content,
                    is_error: false,
                })
            }
            Err(SlackFileError::Refused(why)) => {
                Ok(ToolOutput::text(format!("Couldn't fetch it: {why}")))
            }
            Err(error) => Err(ToolError::failed(error)),
        }
    }
}

/// One Slack message as the owner.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SlackSend;

/// `slack_send`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct SlackSendArgs {
    /// The chat id from `messages_chats` / `messages_search` (`slack:Workspace:Name`).
    #[garde(length(min = 1, max = 500))]
    chat: String,
    /// Exactly what the user approved, plain text.
    #[garde(length(min = 1, max = 4000))]
    text: String,
    /// The task the message belongs to: the OK and the log go there.
    #[garde(skip)]
    task_id: Option<i64>,
}

impl Tool for SlackSend {
    type Args = SlackSendArgs;

    fn name(&self) -> &'static str {
        "slack_send"
    }

    fn description(&self) -> &'static str {
        "Send the user's Slack message, posted as them (their user token) in a workspace where \
         their Slack app is installed. chat: the chat id from messages_chats / messages_search \
         (slack:Workspace:Name, slack:Workspace:#channel). text: exactly what they approved, plain \
         text (no markdown, links in brackets, *bold*, _italic_, backticks or bullets: refused). \
         The first call sends nothing: the user is asked to Send it or Cancel with the chat and the \
         exact text. Their Send it posts that one message by itself and logs it on the task (you're \
         told; don't call again for it). Their clear go in this chat (\"send it\", \"do it\") as \
         their next message after you showed them the exact text (and who it's for) is the OK too: \
         then the call sends right away. One message per OK; a changed text is a new ask; never a \
         resend."
    }

    async fn call(&self, ctx: &ToolContext, args: SlackSendArgs) -> Result<ToolOutput, ToolError> {
        let said = ctx
            .app
            .messaging()
            .slack()
            .send(&args.chat, &args.text, args.task_id, Some(ctx.room))
            .await;
        Ok(ToolOutput::text(said))
    }
}
