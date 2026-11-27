//! `gmail_connect`: a one-time link that signs one of the owner's Gmail accounts in, read-only.

use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};

/// Signs a Gmail account in.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GmailConnect;

/// `gmail_connect`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct GmailConnectArgs {
    /// A short name for the account (home, work): lowercase letters and digits.
    #[schema(pattern = "^[a-z0-9]{2,20}$")]
    #[garde(length(min = 2, max = 20))]
    account: String,
    /// The address the user signs in with (it's refused if they pick another).
    #[garde(length(max = 320))]
    email: Option<String>,
}

impl Tool for GmailConnect {
    type Args = GmailConnectArgs;

    fn name(&self) -> &'static str {
        "gmail_connect"
    }

    fn description(&self) -> &'static str {
        "Sign one of the user's Gmail accounts in to be read here (read-only: gmail.readonly), or \
         again when messages_chats says Google no longer takes its sign-in. account: a short name \
         for it (personal, work): its mail reads as \"Email\" (personal) or \"Email Work\". email: \
         the address they sign in with (it's refused if they pick another). Returns a one-time \
         link (a day): open it in their browser for that account so they sign in and allow it on \
         Google's screen; it saves the sign-in on the server (never in a chat) and its mail comes \
         with the messages."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: GmailConnectArgs,
    ) -> Result<ToolOutput, ToolError> {
        let link = ctx
            .app
            .messaging()
            .gmail()
            .connect_link(&args.account, args.email.as_deref())
            .await;
        Ok(ToolOutput::text(match link {
            Ok(link) => format!("{link} (works once, for a day)"),
            Err(error) => format!("Can't yet: {error}."),
        }))
    }
}
