//! `tell_owner`: an alert on the owner's phone that stands out.

use emberhut_core::RoomId;
use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};

/// The tool's name.
pub(crate) const TELL_OWNER: &str = "tell_owner";

/// Alerts the owner's phones.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TellOwner;

/// `tell_owner`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct TellOwnerArgs {
    /// The alert's title.
    #[garde(length(min = 1, max = 200))]
    title: String,
    /// Its text.
    #[serde(default)]
    #[garde(length(max = 4000))]
    detail: String,
}

impl Tool for TellOwner {
    type Args = TellOwnerArgs;

    fn name(&self) -> &'static str {
        TELL_OWNER
    }

    fn description(&self) -> &'static str {
        "An alert on the owner's phone that stands out: only when they must act now or \
         something broke. Your replies already reach their phone."
    }

    async fn call(&self, ctx: &ToolContext, args: TellOwnerArgs) -> Result<ToolOutput, ToolError> {
        // From a family member's room, the alert is theirs, and opens Main.
        let person = ctx
            .app
            .family()
            .store()
            .person_of(ctx.room)
            .await
            .map_err(ToolError::failed)?;
        let (room, title) = match person {
            Some(person) => (
                RoomId::MAIN,
                format!("From {}: {}", named(&person), args.title),
            ),
            None => (ctx.room, args.title.clone()),
        };
        let ready = ctx.app.talk().alert(room, &title, &args.detail).await;
        ctx.app
            .activity()
            .note(format!("Alerted the owner: {title}"))
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(if ready {
            "Sent."
        } else {
            "Push is not set up on the server; say it in the chat."
        }))
    }
}

/// A family member's name as a line says it: `sam` is `Sam`.
fn named(person: &str) -> String {
    let mut chars = person.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
