//! The assistant's own WhatsApp number: `GET /api/whatsapp` (its state), `POST /api/whatsapp`
//! (the kill switch and linking, for the app and the daemon's command line), and
//! `POST /api/whatsapp/in` (what the WhatsApp daemon tells the server its number sees).

use serde::{Deserialize, Serialize};

/// One chat the number knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WhatsAppChat {
    /// The group's or the person's id.
    pub id: String,
    /// `group` or `dm`.
    pub kind: String,
    /// Its name (a DM with no name: `+<number>`).
    pub name: String,
    /// `pending` (the owner was asked), `allowed`, `denied` or `left`.
    pub status: String,
    /// The room its asks go to.
    pub room: String,
    /// What the owner allowed the assistant to talk about there.
    pub topics: Option<String>,
    /// The task the owner was asked on, as `task <id>`.
    pub task: Option<String>,
}

/// One of the last messages the number sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WhatsAppSentLine {
    /// The chat's name.
    pub chat: Option<String>,
    /// When.
    pub at: Option<String>,
    /// What it said.
    pub text: Option<String>,
}

/// The number's state: the kill switch, the connection, the chats and the last messages sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WhatsAppState {
    /// Replies are on (the kill switch).
    pub on: bool,
    /// The daemon's connection: `open`, `linking`, `unlinked`, `logged_out`, `not started`...
    pub state: String,
    /// The linked number.
    pub number: Option<String>,
    /// What went wrong last.
    pub error: Option<String>,
    /// A pairing code or QR is out now.
    pub linking: bool,
    /// Every chat, the allowed ones first.
    pub chats: Vec<WhatsAppChat>,
    /// The last 10 messages sent, newest first.
    pub sent: Vec<WhatsAppSentLine>,
}

/// Linking's answer: what is needed, the pairing code, the QR page, or why not; and always the
/// line for the assistant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WhatsAppLink {
    /// What is needed first (`phone`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need: Option<String>,
    /// The pairing code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The one-time QR page's address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qr: Option<String>,
    /// Why linking didn't start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The line for the assistant.
    pub text: String,
}

/// `POST /api/whatsapp`: `kind` is `on`, `off` (the kill switch), `link` (a pairing code for
/// `phone`), `qr` (a QR page instead), `link-task` (the task linking is asked on), or anything
/// else for the state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WhatsAppRequest {
    /// What to do.
    #[serde(default)]
    pub kind: Option<String>,
    /// Who flipped the switch, for the log.
    #[serde(default)]
    pub by: Option<String>,
    /// The task linking is asked on.
    #[serde(default)]
    pub task: Option<f64>,
    /// The number to link, with its country code.
    #[serde(default)]
    pub phone: Option<String>,
}

/// `{ ok, text }`: the switch was flipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WhatsAppSwitched {
    /// Always `true`.
    pub ok: bool,
    /// What it means now.
    pub text: String,
}

/// `POST /api/whatsapp`'s answer, by its `kind`. Its schema is any of the four shapes (they
/// overlap: a switched answer is also an `ok`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WhatsAppAnswer {
    /// `on` / `off`.
    Switched(WhatsAppSwitched),
    /// The state.
    State(WhatsAppState),
    /// `link` / `qr`.
    Link(WhatsAppLink),
    /// `link-task`: `{ ok: true }`.
    Ok(crate::error::OkReply),
}

#[cfg(feature = "openapi")]
impl utoipa::PartialSchema for WhatsAppAnswer {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{AnyOfBuilder, Schema};
        use utoipa::openapi::{Ref, RefOr};
        RefOr::T(Schema::AnyOf(
            AnyOfBuilder::new()
                .item(Ref::from_schema_name("WhatsAppSwitched"))
                .item(Ref::from_schema_name("WhatsAppState"))
                .item(Ref::from_schema_name("WhatsAppLink"))
                .item(Ref::from_schema_name("OkReply"))
                .description(Some(
                    "`on` / `off`: switched; `link` / `qr`: the link; `link-task`: ok; else the state",
                ))
                .build(),
        ))
    }
}

#[cfg(feature = "openapi")]
impl utoipa::ToSchema for WhatsAppAnswer {
    fn schemas(
        schemas: &mut Vec<(
            String,
            utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
        )>,
    ) {
        use utoipa::PartialSchema as _;
        schemas.push(("WhatsAppSwitched".to_owned(), WhatsAppSwitched::schema()));
        schemas.push(("WhatsAppState".to_owned(), WhatsAppState::schema()));
        WhatsAppState::schemas(schemas);
        schemas.push(("WhatsAppLink".to_owned(), WhatsAppLink::schema()));
        schemas.push(("OkReply".to_owned(), crate::error::OkReply::schema()));
    }
}

/// What the daemon tells the server (`POST /api/whatsapp/in`): `kind` `status` (its
/// connection), `group` (the number was added to a group, or it was renamed), `removed` (it
/// was taken out of one) or `message` (a message in a chat).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WhatsAppEvent {
    /// `status`, `group`, `removed` or `message`.
    pub kind: String,
    /// The connection's state (`status`).
    #[serde(default)]
    pub state: Option<String>,
    /// The linked number (`status`).
    #[serde(default)]
    pub number: Option<String>,
    /// What went wrong (`status`).
    #[serde(default)]
    pub error: Option<String>,
    /// The pairing code or QR out now (`status`): `{ code, qr }`.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub linking: Option<serde_json::Value>,
    /// The chat's id.
    #[serde(default)]
    pub chat: Option<String>,
    /// The group's name.
    #[serde(default)]
    pub name: Option<String>,
    /// Who added the number (`group`).
    #[serde(default)]
    pub by: Option<String>,
    /// It was only renamed (`group`).
    #[serde(default)]
    pub renamed: Option<bool>,
    /// The chat is a group (`message`).
    #[serde(default)]
    pub group: Option<bool>,
    /// The message's id.
    #[serde(default)]
    pub id: Option<String>,
    /// The sender's id.
    #[serde(default)]
    pub sender: Option<String>,
    /// The sender's phone id, when the sender's id is another kind.
    #[serde(default)]
    pub sender_pn: Option<String>,
    /// The sender's name.
    #[serde(default)]
    pub who: Option<String>,
    /// The number sent it.
    #[serde(default)]
    pub from_me: Option<bool>,
    /// It mentions the number.
    #[serde(default)]
    pub mentioned: Option<bool>,
    /// It replies to one of the number's messages.
    #[serde(default)]
    pub reply_to_me: Option<bool>,
    /// What it says.
    #[serde(default)]
    pub text: Option<String>,
    /// When it was sent.
    #[serde(default)]
    pub at: Option<String>,
    /// The message it quotes: `{ text }`.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub quoted: Option<serde_json::Value>,
}

/// What the server did with the daemon's news: `{ ok }` for `status`, `group` and `removed`;
/// for a message, `{ dropped }` (not from an allowed chat, nothing kept) or `{ kept }`, with
/// `asked` when it woke the assistant, `off` when the kill switch kept it from that, and
/// `capped` when too many asks came in the last hour.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WhatsAppInReply {
    /// Taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// Why the message was dropped unread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped: Option<String>,
    /// The message was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept: Option<bool>,
    /// It woke the assistant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked: Option<bool>,
    /// The kill switch is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off: Option<bool>,
    /// Too many asks in the last hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capped: Option<bool>,
}

impl WhatsAppInReply {
    /// `{ ok: true }`.
    #[must_use]
    pub fn ok() -> Self {
        Self {
            ok: Some(true),
            ..Self::default()
        }
    }

    /// `{ dropped: why }`.
    #[must_use]
    pub fn dropped(why: &str) -> Self {
        Self {
            dropped: Some(why.to_owned()),
            ..Self::default()
        }
    }

    /// `{ kept: true }`.
    #[must_use]
    pub fn kept() -> Self {
        Self {
            kept: Some(true),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answers_keep_their_shapes() {
        let ok = serde_json::to_value(WhatsAppAnswer::Ok(crate::error::OkReply::OK)).unwrap();
        assert_eq!(ok, serde_json::json!({ "ok": true }));
        let link = WhatsAppLink {
            need: Some("phone".to_owned()),
            text: "Need the number first.".to_owned(),
            ..WhatsAppLink::default()
        };
        assert_eq!(
            serde_json::to_value(&link).unwrap(),
            serde_json::json!({ "need": "phone", "text": "Need the number first." })
        );
        let mut asked = WhatsAppInReply::kept();
        asked.asked = Some(true);
        assert_eq!(
            serde_json::to_value(&asked).unwrap(),
            serde_json::json!({ "kept": true, "asked": true })
        );
        let event: WhatsAppEvent = serde_json::from_str(
            r#"{"kind":"message","senderPn":"15550100101@s.whatsapp.net","replyToMe":true}"#,
        )
        .unwrap();
        assert_eq!(
            event.sender_pn.as_deref(),
            Some("15550100101@s.whatsapp.net")
        );
        assert_eq!(event.reply_to_me, Some(true));
    }
}
