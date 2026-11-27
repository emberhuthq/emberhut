//! The chips under the assistant's answers: found in an answer's text when it is stored
//! ([`emberhut_core::refs`]), kept with the message, and put back on its chat line wherever the
//! chat is shown (the state's `chat`, `GET /api/history`).

use emberhut_api::chat::{ChatMessage, MessageRef, RefKind};
use emberhut_core::MessageId;
use emberhut_core::refs::{LinkKind, LinkRef, link_refs};
use emberhut_db::{DbError, MessageRefStore};

use crate::convert::message_from_api;

/// An answer's chips as the chat shows them; none when its text has no link worth one.
#[must_use]
pub fn refs_of(text: &str) -> Vec<MessageRef> {
    link_refs(text).into_iter().map(to_api).collect()
}

fn to_api(chip: LinkRef) -> MessageRef {
    let kind = match chip.kind {
        LinkKind::PullRequest => RefKind::Pr,
        LinkKind::Ticket => RefKind::Ticket,
        LinkKind::Report => RefKind::Report,
    };
    MessageRef {
        kind,
        label: chip.label,
        task: None,
        title: Some(chip.title),
        url: Some(chip.url),
        name: None,
    }
}

/// Keeps `message`'s chips, found in its `text`; nothing when it has none.
///
/// # Errors
///
/// [`DbError`].
pub async fn keep(store: &MessageRefStore, message: MessageId, text: &str) -> Result<(), DbError> {
    let refs = refs_of(text);
    if refs.is_empty() {
        return Ok(());
    }
    match serde_json::to_string(&refs) {
        Ok(json) => store.set(message, json).await,
        Err(error) => {
            tracing::warn!(%error, "an answer's chips could not be written");
            Ok(())
        }
    }
}

/// Puts each line's kept chips back on it.
///
/// # Errors
///
/// [`DbError`].
pub async fn fill(store: &MessageRefStore, lines: &mut [ChatMessage]) -> Result<(), DbError> {
    let ids: Vec<MessageId> = lines.iter().map(|line| message_from_api(line.id)).collect();
    let kept = store.of_messages(&ids).await?;
    if kept.is_empty() {
        return Ok(());
    }
    for line in lines {
        if let Some(json) = kept.get(&message_from_api(line.id)) {
            match serde_json::from_str::<Vec<MessageRef>>(json) {
                Ok(refs) => line.refs = refs,
                Err(error) => tracing::warn!(%error, "an answer's chips could not be read"),
            }
        }
    }
    Ok(())
}
