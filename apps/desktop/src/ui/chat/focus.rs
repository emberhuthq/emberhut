//! A message started from outside the chat (a link, Talk in another window).

use gpui_kit::Window;
use gpui_kit::prelude::*;

use super::{ChatView, composer};

impl ChatView {
    /// Starts a message with `text` in the composer, focused, for the user to finish and send.
    /// Words the user was writing are kept: `text` goes after them, a blank line between, so a
    /// link from outside the app never takes a draft away.
    pub(crate) fn start_message(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let draft = self.input.read(cx).value().to_string();
        let text = joined(&draft, text);
        composer::fill(&self.input, &text, window, cx);
        // Setting the value says no change: the send button is told here.
        self.has_text = !text.trim().is_empty();
        self.focus_composer(window, cx);
    }
}

/// `added` after `draft`, a blank line between, when the draft has words of its own.
fn joined(draft: &str, added: &str) -> String {
    if draft.trim().is_empty() {
        added.to_owned()
    } else {
        format!("{}\n\n{added}", draft.trim_end())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::open_chat;
    use super::*;

    #[test]
    fn a_draft_is_kept_and_the_words_go_after_it() {
        assert_eq!(joined("", "Hello"), "Hello");
        assert_eq!(joined("  \n", "Hello"), "Hello");
        assert_eq!(
            joined("Half a thought\n", "Hello"),
            "Half a thought\n\nHello"
        );
    }

    #[gpui_kit::test]
    fn a_message_started_from_outside_keeps_the_draft(cx: &mut gpui_kit::TestAppContext) {
        let (_backend, _store, window) = open_chat(cx);
        window
            .update(cx, |view, window, cx| {
                view.input
                    .update(cx, |input, cx| input.set_value("My draft", window, cx));
                view.start_message("From a link", window, cx);
            })
            .unwrap();
        let value = window
            .update(cx, |view, _, cx| view.input.read(cx).value().to_string())
            .unwrap();
        assert_eq!(value, "My draft\n\nFrom a link");
    }
}
