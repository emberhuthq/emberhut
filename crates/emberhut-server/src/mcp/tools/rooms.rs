//! The rooms, across the assistant's sessions: handing something to another room
//! (`room_send`) and what was said in every room (`rooms`).

use emberhut_core::RoomId;
use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::rooms::RoomSendError;

/// Characters of a handed-over text the activity log keeps.
const LOGGED: usize = 80;

/// Hands something to another room.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RoomSend;

/// `room_send`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct RoomSendArgs {
    /// Main, or the room's name (or its number).
    #[garde(length(min = 1, max = 200))]
    room: String,
    /// What that room needs to know and do, in a few lines.
    #[garde(length(min = 3, max = 20_000))]
    text: String,
}

impl Tool for RoomSend {
    type Args = RoomSendArgs;

    fn name(&self) -> &'static str {
        "room_send"
    }

    fn description(&self) -> &'static str {
        "Hand something to another room (Main, or a project's room by its name): it lands \
         there as a note and wakes that room's session, which carries it on. For work or news \
         that belongs to that room, or (from a project room) something Main should bring up in \
         a brief. Say what it needs to know and do, in a few lines. Work the owner asked for \
         carries their request with it: that room does it end to end."
    }

    async fn call(&self, ctx: &ToolContext, args: RoomSendArgs) -> Result<ToolOutput, ToolError> {
        match ctx.app.rooms().send(ctx.room, &args.room, &args.text).await {
            Ok(room) => {
                let to = if room.id == RoomId::MAIN {
                    "Main".to_owned()
                } else {
                    format!("the {} room", room.name)
                };
                let shown = emberhut_core::message::preview(&args.text, LOGGED);
                ctx.app
                    .activity()
                    .note(format!("Handed to {to}: {shown}"))
                    .await
                    .map_err(ToolError::failed)?;
                Ok(ToolOutput::text(format!("Sent to {to}; it wakes with it.")))
            }
            Err(error @ (RoomSendError::NoRoom(_) | RoomSendError::SameRoom)) => {
                Ok(ToolOutput::error(format!("Not sent: {error}")))
            }
            Err(error) => Err(ToolError::failed(error)),
        }
    }
}

/// What was said in every room, and how each is now.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rooms;

/// `rooms`' arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct RoomsArgs {
    /// An ISO time; the last 24 hours when left out.
    #[garde(length(max = 64))]
    since: Option<String>,
    /// Only this room (its name).
    #[garde(length(max = 200))]
    room: Option<String>,
}

/// One room as `rooms` shows it now.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RoomNow {
    id: emberhut_api::ids::RoomId,
    name: String,
    unread: u32,
    needs: u32,
    ask: Option<String>,
    busy: bool,
    last_at: Option<emberhut_api::time::Timestamp>,
}

impl Tool for Rooms {
    type Args = RoomsArgs;

    fn name(&self) -> &'static str {
        "rooms"
    }

    fn description(&self) -> &'static str {
        "What was said in every room (the owner's messages and your answers, last 12 each) \
         since a time, and each room's state now (unread, waiting on the owner, working). For \
         the morning brief, the evening wrap, and questions across projects."
    }

    async fn call(&self, ctx: &ToolContext, args: RoomsArgs) -> Result<ToolOutput, ToolError> {
        let only = match args.room.as_deref().filter(|name| !name.trim().is_empty()) {
            Some(name) => match ctx
                .app
                .rooms()
                .find(name)
                .await
                .map_err(ToolError::failed)?
            {
                Some(room) => Some(room.id),
                None => return Ok(ToolOutput::text(format!("No room \"{name}\"."))),
            },
            None => None,
        };
        let now = ctx.app.clock().now();
        let since = match args.since.as_deref() {
            Some(since) => since
                .parse::<Timestamp>()
                .map_err(|_| ToolError::Invalid(format!("since: {since:?} is not an ISO time")))?,
            None => now
                .checked_sub(SignedDuration::from_hours(24))
                .unwrap_or(now),
        };
        let said = ctx
            .app
            .rooms()
            .said_since(since, only)
            .await
            .map_err(ToolError::failed)?;
        let said = if said.is_empty() {
            "Nothing said in any room since then.".to_owned()
        } else {
            said
        };
        let state = ctx
            .app
            .state()
            .state(RoomId::MAIN)
            .await
            .map_err(ToolError::failed)?;
        let rooms: Vec<RoomNow> = state
            .rooms
            .into_iter()
            .map(|row| RoomNow {
                id: row.id,
                name: row.name,
                unread: row.unread,
                needs: row.needs,
                ask: row.ask,
                busy: row.busy,
                last_at: row.last_at,
            })
            .collect();
        let now = serde_json::to_string_pretty(&rooms).map_err(ToolError::failed)?;
        Ok(ToolOutput::text(format!("{said}\n\nNow:\n{now}")))
    }
}
