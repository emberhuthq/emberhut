//! The assistant's own WhatsApp number (`wa_group_reply`, `wa_groups`, `wa_chat_set`,
//! `wa_link`, `wa_switch`) and sending as the owner through WhatsApp Web (`whatsapp_send`).

use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use super::{NoArgs, Tool, ToolContext, ToolError, ToolOutput};

/// Answers a message that asked the assistant, in that chat.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaGroupReply;

/// `wa_group_reply`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WaGroupReplyArgs {
    /// The chat id the event gave (or its exact name).
    #[garde(length(min = 1, max = 300))]
    group: String,
    /// Plain, short, in their language.
    #[garde(length(min = 1, max = 3000))]
    text: String,
    /// The message's id (left out: the latest one that asked).
    #[garde(length(max = 300))]
    quoted: Option<String>,
}

impl Tool for WaGroupReply {
    type Args = WaGroupReplyArgs;

    fn name(&self) -> &'static str {
        "wa_group_reply"
    }

    fn description(&self) -> &'static str {
        "Answer in a WhatsApp group (or an allowed DM) as the assistant, from its own number. Only \
         for a message that asked you (an event \"whatsapp\": an @mention, a reply to you, or it \
         starts with your name), once per message, within 30 minutes. group: the chat id the event \
         gave (or its exact name); quoted: that message's id (left out: the latest one that asked). \
         text: plain, short, in their language; no markdown, no links in your first messages. \
         Refused outside the allow-list, unprompted, faster than one per 10 s in a chat, or while \
         the kill switch is off. Every message is logged and shown in the room."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: WaGroupReplyArgs,
    ) -> Result<ToolOutput, ToolError> {
        let said = ctx
            .app
            .messaging()
            .whatsapp()
            .reply(&args.group, &args.text, args.quoted.as_deref())
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(said))
    }
}

/// The number's state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaGroups;

impl Tool for WaGroups {
    type Args = NoArgs;

    fn name(&self) -> &'static str {
        "wa_groups"
    }

    fn description(&self) -> &'static str {
        "The assistant's own WhatsApp: on or off (kill switch), linked or not and its number, \
         every chat it knows (allowed, pending the user's OK, denied, left) with its room and \
         allowed topics, and the last 10 messages it sent."
    }

    async fn call(&self, ctx: &ToolContext, _: NoArgs) -> Result<ToolOutput, ToolError> {
        let state = ctx
            .app
            .messaging()
            .whatsapp()
            .state()
            .await
            .map_err(ToolError::failed)?;
        serde_json::to_string(&state)
            .map(ToolOutput::text)
            .map_err(ToolError::failed)
    }
}

/// A chat's settings.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaChatSet;

/// A field that may be left out, given, or `null`: left out is `None`, `null` is
/// `Some(None)`.
#[expect(
    clippy::option_option,
    reason = "left out, null and a value are three different asks of wa_chat_set"
)]
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

/// `wa_chat_set`'s arguments.
#[expect(
    clippy::option_option,
    reason = "left out (unchanged), null (cleared) and a value differ for project and topics"
)]
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WaChatSetArgs {
    /// A group's id or name, or a person's phone number for DMs.
    #[garde(length(min = 1, max = 300))]
    chat: String,
    /// A DM's name (needed to allow a DM by number).
    #[garde(length(max = 200))]
    name: Option<String>,
    /// Ask the user to allow it; only their Allow puts it on the list.
    #[garde(skip)]
    allow: Option<bool>,
    /// Take it off the list.
    #[garde(skip)]
    remove: Option<bool>,
    /// The project whose room gets its asks (null: the WhatsApp room).
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    #[garde(skip)]
    project: Option<Option<String>>,
    /// What the user said the assistant may talk about there, only from their own words; null
    /// clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, max_length = 300)]
    #[garde(custom(at_most_300))]
    topics: Option<Option<String>>,
}

#[expect(
    clippy::ref_option,
    clippy::option_option,
    clippy::trivially_copy_pass_by_ref,
    reason = "garde hands a custom rule references to the field as it is and to its context"
)]
fn at_most_300(topics: &Option<Option<String>>, (): &()) -> garde::Result {
    match topics {
        Some(Some(topics)) if topics.chars().count() > 300 => {
            Err(garde::Error::new("length is greater than 300"))
        }
        _ => Ok(()),
    }
}

impl Tool for WaChatSet {
    type Args = WaChatSetArgs;

    fn name(&self) -> &'static str {
        "wa_chat_set"
    }

    fn description(&self) -> &'static str {
        "Settings of one chat on the assistant's own WhatsApp. chat: a group's id or name, or a \
         person's phone number for DMs. allow: true asks the user (Allow / Don't) and only their \
         Allow puts it on the list; a DM by number needs name too. remove: true takes it off the \
         list. project: the project whose room gets its asks (null: WhatsApp groups). topics: what \
         the user said the assistant may talk about there, only from their own words; null clears it."
    }

    async fn call(&self, ctx: &ToolContext, args: WaChatSetArgs) -> Result<ToolOutput, ToolError> {
        let mut raw = Map::new();
        raw.insert("chat".to_owned(), Value::String(args.chat));
        if let Some(name) = args.name {
            raw.insert("name".to_owned(), Value::String(name));
        }
        if let Some(allow) = args.allow {
            raw.insert("allow".to_owned(), Value::Bool(allow));
        }
        if let Some(remove) = args.remove {
            raw.insert("remove".to_owned(), Value::Bool(remove));
        }
        if let Some(project) = args.project {
            raw.insert(
                "project".to_owned(),
                project.map_or(Value::Null, Value::String),
            );
        }
        if let Some(topics) = args.topics {
            raw.insert(
                "topics".to_owned(),
                topics.map_or(Value::Null, Value::String),
            );
        }
        let said = ctx
            .app
            .messaging()
            .whatsapp()
            .chat_set(&raw)
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(said))
    }
}

/// Links the number.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaLink;

/// `wa_link`'s arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WaLinkArgs {
    /// The number, with its country code.
    #[garde(length(max = 40))]
    phone: Option<String>,
    /// A one-time QR page instead of a pairing code.
    #[garde(skip)]
    qr: Option<bool>,
    /// The task the user is shown the code on.
    #[garde(skip)]
    task_id: Option<i64>,
}

impl Tool for WaLink {
    type Args = WaLinkArgs;

    fn name(&self) -> &'static str {
        "wa_link"
    }

    fn description(&self) -> &'static str {
        "Link the assistant's own WhatsApp number (set up in WhatsApp Business on the user's \
         phone): asks WhatsApp for a pairing code for phone (with country code) and shows it to \
         them on the task (task_id) with a push. qr: true gives a one-time QR page instead. Only \
         when they ask; it never relinks by itself."
    }

    async fn call(&self, ctx: &ToolContext, args: WaLinkArgs) -> Result<ToolOutput, ToolError> {
        let link = ctx
            .app
            .messaging()
            .whatsapp()
            .link(args.task_id, args.phone.as_deref(), args.qr == Some(true))
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(link.text))
    }
}

/// The kill switch, off only: it is the owner's stop for the assistant's number, and what the
/// assistant reads (a group's messages) may ask it to turn replies back on, so only the owner's
/// signed-in `POST /api/whatsapp` does that.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaSwitch;

/// `wa_switch`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WaSwitchArgs {
    /// false stops every reply at once; true is refused (only the user turns it back on, in
    /// the app).
    #[garde(skip)]
    on: bool,
}

impl Tool for WaSwitch {
    type Args = WaSwitchArgs;

    fn name(&self) -> &'static str {
        "wa_switch"
    }

    fn description(&self) -> &'static str {
        "The kill switch of the assistant's own WhatsApp: on: false stops every reply at once \
         (queued ones too). Only the user turns it back on, in the app; on: true is refused."
    }

    async fn call(&self, ctx: &ToolContext, args: WaSwitchArgs) -> Result<ToolOutput, ToolError> {
        if args.on {
            return Err(ToolError::refused(
                "Not turned on: only the user turns the assistant's WhatsApp replies back on, \
                 in the app. Tell them if they want it on.",
            ));
        }
        let said = ctx
            .app
            .messaging()
            .whatsapp()
            .switch(false, "the assistant, for the user")
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(said))
    }
}

/// One WhatsApp message as the owner, through WhatsApp Web.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WhatsAppSendTool;

/// `whatsapp_send`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct WhatsAppSendArgs {
    /// The chat id (`wa:12` from `messages_chats` / `messages_search`; `wa:self` is the user's own
    /// chat).
    #[garde(length(min = 1, max = 300))]
    chat: String,
    /// Exactly what the user approved, plain text.
    #[garde(length(min = 1, max = 4000))]
    text: String,
    /// The task the message belongs to: the OK and the log go there.
    #[garde(skip)]
    task_id: Option<i64>,
}

impl Tool for WhatsAppSendTool {
    type Args = WhatsAppSendArgs;

    fn name(&self) -> &'static str {
        "whatsapp_send"
    }

    fn description(&self) -> &'static str {
        "Send the user's WhatsApp message through WhatsApp Web in the server's browser. chat: the \
         chat id (wa:12 from messages_chats / messages_search; wa:self is their own Message \
         yourself chat). text: exactly what they approved, plain text. It opens the chat (never \
         guessing when names clash), types the text and stops: the user is asked to Send it or \
         Cancel. Their Send it sends that one message by itself, checks the tick and logs it on the \
         task (you're told; don't call again for it). Their clear go in this chat (\"send it\", \
         \"do it\") as their next message after you showed them the exact text (and who it's for) \
         is the OK too: then the call sends right away. One message per OK; a changed text is a new \
         ask. The first time, WhatsApp Web needs linking: it hands the browser to the user to scan."
    }

    async fn call(
        &self,
        ctx: &ToolContext,
        args: WhatsAppSendArgs,
    ) -> Result<ToolOutput, ToolError> {
        let said = ctx
            .app
            .messaging()
            .whatsapp_send()
            .send(&args.chat, &args.text, args.task_id, Some(ctx.room))
            .await;
        Ok(ToolOutput::text(said))
    }
}
