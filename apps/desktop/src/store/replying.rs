//! Replying to one message: its quote waits above the composer until the next message is sent
//! (which carries it as `replyTo`), the user drops it, or another room opens (a reply is to a
//! message of the room left: [`Store::switch_draft`] drops it).

use emberhut_api::chat::{ChatMessage, FileKind, Quote};
use gpui_kit::Context;

use super::Store;

/// How much of a message its quote keeps, in characters (the server's quotes are as long).
const QUOTED: usize = 160;

/// The quote a reply to `message` carries: who said it and when, its first words, and its
/// first picture or video.
#[must_use]
pub(crate) fn quote_of(message: &ChatMessage) -> Quote {
    Quote {
        id: message.id,
        mine: message.mine,
        at: Some(message.at.clone()),
        text: message.text.trim().chars().take(QUOTED).collect(),
        file: message
            .files
            .iter()
            .find(|file| matches!(file.kind, FileKind::Image | FileKind::Video))
            .cloned(),
    }
}

impl Store {
    /// The quote waiting above the composer.
    #[must_use]
    pub(crate) fn replying(&self) -> Option<&Quote> {
        self.replying.as_ref()
    }

    /// Reply: `message` (of the open room) is quoted above the composer.
    pub(crate) fn reply_to(&mut self, message: &ChatMessage, cx: &mut Context<'_, Self>) {
        self.replying = Some(quote_of(message));
        cx.notify();
    }

    /// The X on the quote: the next message isn't a reply after all.
    pub(crate) fn stop_replying(&mut self, cx: &mut Context<'_, Self>) {
        self.replying = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::message_at;

    #[test]
    fn a_quote_keeps_who_when_the_first_words_and_a_picture() {
        let message = message_at(
            7,
            "2026-11-25T08:00:00Z",
            json!({"mine": true, "text": format!("  {}", "x".repeat(200)),
                "files": [{"name": "a.pdf", "url": "/uploads/a.pdf", "kind": "file"},
                          {"name": "b.png", "url": "/uploads/b.png", "kind": "image"}]}),
        );
        let quote = quote_of(&message);
        assert_eq!(quote.id, message.id);
        assert!(quote.mine);
        assert_eq!(quote.text.chars().count(), QUOTED);
        assert_eq!(quote.file.map(|file| file.name).as_deref(), Some("b.png"));
    }
}
