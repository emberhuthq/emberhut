//! `mac_record` and `call_transcribe`: recording a call on the owner's Mac, and transcribing a
//! recording they share; the transcript comes back as a `call` event.

use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::calls::CallError;
use crate::services::mac::RecorderCommand;
use emberhut_core::RoomId;

/// What `mac_record` does.
#[derive(Clone, Copy, Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
enum RecordWhat {
    /// Start recording.
    Start,
    /// Stop: the audio goes to the server and is transcribed.
    Stop,
    /// Whether it records, and the last calls.
    Status,
}

/// Records a call on the owner's Mac.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MacRecord;

/// `mac_record`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct MacRecordArgs {
    /// start, stop or status.
    #[garde(skip)]
    action: RecordWhat,
    /// Who and what the call is ("Weekly with the design team").
    #[garde(length(max = 1000))]
    title: Option<String>,
    /// The task the transcript is for.
    #[garde(skip)]
    task_id: Option<i64>,
    /// A room's name, when the transcript is for another room than this one.
    #[garde(length(max = 200))]
    room: Option<String>,
}

/// The room a tool names (by name or number), else the caller's.
async fn room_of(ctx: &ToolContext, named: Option<&str>) -> Result<RoomId, ToolError> {
    match named.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => Ok(ctx
            .app
            .calls()
            .room_named(name)
            .await
            .map_err(ToolError::failed)?
            .unwrap_or(ctx.room)),
        None => Ok(ctx.room),
    }
}

/// A call tool's failure: `Calls: <its first line>`.
fn said(error: &CallError) -> ToolOutput {
    let text = error.to_string();
    ToolOutput::text(format!(
        "Calls: {}",
        text.lines().next().unwrap_or_default()
    ))
}

impl Tool for MacRecord {
    type Args = MacRecordArgs;

    fn name(&self) -> &'static str {
        "mac_record"
    }

    fn description(&self) -> &'static str {
        "Record a call on the user's Mac: their microphone and the Mac's sound as two tracks, so \
         the other side of a call (a huddle, Zoom, Meet, FaceTime) is heard even in headphones. \
         When the user says \"record this call\": start (title: who and what; task_id or room: \
         where the transcript goes, default this room). A red dot in the Mac's menu bar shows it. \
         stop when the user says so (or presses Stop): the audio goes to the server and is \
         transcribed there, on the server's own machine; the transcript comes back as a \"call\" \
         event. status: is it recording, and the last calls."
    }

    async fn call(&self, ctx: &ToolContext, args: MacRecordArgs) -> Result<ToolOutput, ToolError> {
        let calls = ctx.app.calls();
        let action = match args.action {
            RecordWhat::Status => {
                return calls
                    .status()
                    .await
                    .map(ToolOutput::text)
                    .map_err(ToolError::failed);
            }
            RecordWhat::Start => RecorderCommand::Start,
            RecordWhat::Stop => RecorderCommand::Stop,
        };
        let room = room_of(ctx, args.room.as_deref()).await?;
        match calls
            .record(action, args.title.as_deref(), args.task_id, room)
            .await
        {
            Ok(text) => Ok(ToolOutput::text(text.unwrap_or_else(|| "null".to_owned()))),
            Err(error) => Ok(said(&error)),
        }
    }
}

/// Transcribes a recording the owner shared.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CallTranscribe;

/// `call_transcribe`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct CallTranscribeArgs {
    /// The file: a chat upload's path, in the server's data folder.
    #[garde(length(min = 1, max = 4096))]
    path: String,
    /// Who and what the call is.
    #[garde(length(max = 1000))]
    title: String,
    /// The task the transcript is for.
    #[garde(skip)]
    task_id: Option<i64>,
    /// A room's name, when the transcript is for another room than this one.
    #[garde(length(max = 200))]
    room: Option<String>,
}

impl Tool for CallTranscribe {
    type Args = CallTranscribeArgs;

    fn name(&self) -> &'static str {
        "call_transcribe"
    }

    fn description(&self) -> &'static str {
        "Transcribe a recording of a call that the user shares (a voice memo dropped in the \
         chat: its upload's path) on the server, with its local speech-to-text. path: the file. \
         title: who and what. task_id or room: where the transcript goes (default this room). It \
         runs in the background (a minute or a few for an hour); the transcript comes back as a \
         \"call\" event. One file has no speaker labels: tell who said what from what they say."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: CallTranscribeArgs,
    ) -> Result<ToolOutput, ToolError> {
        let room = room_of(ctx, args.room.as_deref()).await?;
        match ctx
            .app
            .calls()
            .transcribe_file(&args.path, &args.title, args.task_id, room)
            .await
        {
            Ok(text) => Ok(ToolOutput::text(text)),
            Err(error) => Ok(said(&error)),
        }
    }
}
