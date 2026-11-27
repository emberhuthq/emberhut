//! `secret_link`: a one-time page where the user types secrets in, so they never pass through
//! the chat.

use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::secret_links::SecretLinkError;

/// Makes a one-time secret link and answers its address.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SecretLink;

/// `secret_link`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct SecretLinkArgs {
    /// The secrets' file names, lowercase letters, digits and dashes (`slack-user-token`).
    #[garde(length(min = 1, max = 6))]
    #[schema(min_items = 1, max_items = 6)]
    names: Vec<SecretNameArg>,
    /// Why they are needed, shown on the page.
    #[garde(length(max = 2000))]
    why: String,
}

/// A secret's file name: lowercase letters, digits and dashes.
///
/// The server checks it (and refuses the reserved ones) when it makes the link.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[schema(pattern = "^[a-z0-9-]+$")]
pub(crate) struct SecretNameArg(String);

impl Tool for SecretLink {
    type Args = SecretLinkArgs;

    fn name(&self) -> &'static str {
        "secret_link"
    }

    fn description(&self) -> &'static str {
        "A one-time private page where the user types secrets (API keys, tokens) straight into \
         the server: never ask for a secret in chat. names: the secret file names, lowercase \
         with dashes (slack-user-token). Returns the link to send the user: it works once, for \
         24 hours."
    }

    async fn call(&self, ctx: &ToolContext, args: SecretLinkArgs) -> Result<ToolOutput, ToolError> {
        let names: Vec<&str> = args.names.iter().map(|name| name.0.as_str()).collect();
        match ctx.app.secret_links().create(&names, &args.why).await {
            Ok(link) => Ok(ToolOutput::text(link.url)),
            Err(SecretLinkError::Invalid(invalid)) => Err(ToolError::Invalid(invalid.to_string())),
            Err(error) => Err(ToolError::failed(error)),
        }
    }
}
