//! `send_file`: a file on this machine, shown in the chat with the assistant's reply.

use std::path::Path;

use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::uploads::Attachment;

/// Sends the owner a file with the reply.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SendFile;

/// `send_file`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct SendFileArgs {
    /// The file's absolute path on this machine.
    #[garde(length(min = 1, max = 4096))]
    path: String,
    /// The task it belongs to (not used yet).
    #[serde(default)]
    #[garde(skip)]
    task_id: Option<i64>,
}

impl Tool for SendFile {
    type Args = SendFileArgs;

    fn name(&self) -> &'static str {
        "send_file"
    }

    fn description(&self) -> &'static str {
        "Show the owner a file in the chat: a picture, chart, screenshot, PDF or any document \
         on this machine. It goes out with your reply (call it before you finish). Use an \
         absolute path. Files the server keeps private (its secrets, key and token files, its \
         database) and hidden files or folders are never sent."
    }

    async fn call(&self, ctx: &ToolContext, args: SendFileArgs) -> Result<ToolOutput, ToolError> {
        let _ = args.task_id;
        let path = Path::new(&args.path);
        if !path.is_absolute() {
            return Ok(ToolOutput::error(
                "Not sent: give the file's absolute path on this machine.",
            ));
        }
        let uploads = ctx.app.uploads();
        let file = match uploads.keep(path).await {
            Ok(file) => file,
            Err(error) => return Ok(ToolOutput::error(format!("Not sent: {}", reason(&error)))),
        };
        let name = file.name.clone();
        let path = uploads.path_of(&file);
        ctx.app
            .talk()
            .send_file(ctx.room, Attachment { file, path });
        Ok(ToolOutput::text(format!(
            "Attached {name}; it goes out with your reply."
        )))
    }
}

/// An upload error and its first cause, for the agent.
fn reason(error: &dyn std::error::Error) -> String {
    match error.source() {
        Some(cause) => format!("{error}: {cause}"),
        None => error.to_string(),
    }
}
