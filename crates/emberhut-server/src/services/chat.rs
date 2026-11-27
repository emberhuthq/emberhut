//! A room's messages as the chat shows them, in the state's `chat` and in `GET /api/history`:
//! each with its files and, on a reply, the quote of the message it answers.

use std::collections::{BTreeMap, HashMap};

use emberhut_api::chat::{ChatMessage, Quote};
use emberhut_core::MessageId;
use emberhut_core::message::{Author, Message, age_label, preview};
use emberhut_core::upload::{FileKind, StoredFile};
use jiff::Timestamp;

use crate::convert::{message_to_api, timestamp_to_api};
use crate::services::uploads::chat_file;

/// Characters of a quoted message shown in a reply's bubble.
const QUOTE_LENGTH: usize = 160;

/// A page of messages as the chat shows them. `quoted` holds the messages the page's replies
/// quote (a reply whose message is gone shows no quote), and `files` every file of the page's
/// messages and of the quoted ones.
pub(crate) fn chat_lines(
    page: &[Message],
    quoted: &[Message],
    files: &BTreeMap<MessageId, Vec<StoredFile>>,
    now: Timestamp,
) -> Vec<ChatMessage> {
    let by_id: HashMap<MessageId, &Message> =
        quoted.iter().map(|message| (message.id, message)).collect();
    page.iter()
        .map(|message| ChatMessage {
            id: message_to_api(message.id),
            mine: message.author == Author::User,
            event: message.author == Author::Event,
            text: message.text.clone(),
            time: age_label(message.created_at, now),
            at: timestamp_to_api(message.created_at),
            files: files
                .get(&message.id)
                .map_or_default(|files| files.iter().map(chat_file).collect()),
            reply: message
                .reply_to
                .and_then(|id| by_id.get(&id))
                .map(|quoted| quote(quoted, files)),
            refs: Vec::new(),
        })
        .collect()
}

/// A reply's quote of `quoted`: who said it, when, its text
/// on one line cut to about 160 characters, and its first picture or video.
fn quote(quoted: &Message, files: &BTreeMap<MessageId, Vec<StoredFile>>) -> Quote {
    Quote {
        id: message_to_api(quoted.id),
        mine: quoted.author == Author::User,
        at: Some(timestamp_to_api(quoted.created_at)),
        text: preview(&one_line(&quoted.text), QUOTE_LENGTH),
        file: files
            .get(&quoted.id)
            .and_then(|files| files.iter().find(|file| file.kind != FileKind::File))
            .map(chat_file),
    }
}

/// `text` on one line: every run of whitespace a single space, none at the ends.
pub(crate) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use emberhut_core::{FileId, RoomId};
    use jiff::SignedDuration;

    use super::*;

    fn message(id: i64, author: Author, text: &str, reply_to: Option<i64>) -> Message {
        Message {
            id: MessageId::new(id),
            room: RoomId::MAIN,
            author,
            text: text.to_owned(),
            created_at: Timestamp::from_second(1_800_000_000).unwrap(),
            reply_to: reply_to.map(MessageId::new),
        }
    }

    fn stored(id: i64, name: &str) -> StoredFile {
        StoredFile {
            id: FileId::new(id),
            stored_name: format!("0000000{id}-{name}"),
            name: name.to_owned(),
            kind: FileKind::of(name),
            size: 1,
            created_at: Timestamp::from_second(1_800_000_000).unwrap(),
        }
    }

    #[test]
    fn a_reply_quotes_one_line_and_the_first_picture_or_video() {
        let long = format!("first line\n\n  {}", "word ".repeat(60));
        let quoted = message(1, Author::Assistant, &long, None);
        let reply = message(2, Author::User, "yes", Some(1));
        let files = BTreeMap::from([(
            quoted.id,
            vec![
                stored(1, "notes.pdf"),
                stored(2, "a b.png"),
                stored(3, "clip.mov"),
            ],
        )]);
        let now = quoted.created_at + SignedDuration::from_mins(5);
        let lines = chat_lines(&[quoted.clone(), reply], &[quoted], &files, now);
        assert_eq!(lines[0].files.len(), 3);
        assert_eq!(lines[0].files[1].url, "/uploads/00000002-a%20b.png");
        assert_eq!(lines[0].files[1].path, None, "a chat line shows no path");
        assert_eq!(lines[0].time, "5m");
        let quote = lines[1].reply.as_ref().unwrap();
        assert!(!quote.mine);
        assert!(quote.text.starts_with("first line word word"));
        assert!(quote.text.ends_with('…'));
        assert!(quote.text.chars().count() <= 161);
        assert_eq!(quote.file.as_ref().unwrap().name, "a b.png");
        assert_eq!(lines[1].files, [] as [emberhut_api::chat::ChatFile; 0]);
    }

    #[test]
    fn a_quote_of_files_alone_has_no_picture() {
        let quoted = message(1, Author::User, "see", None);
        let reply = message(2, Author::Assistant, "ok", Some(1));
        let files = BTreeMap::from([(quoted.id, vec![stored(1, "notes.pdf")])]);
        let now = quoted.created_at;
        let lines = chat_lines(&[reply], &[quoted], &files, now);
        let quote = lines[0].reply.as_ref().unwrap();
        assert!(quote.mine);
        assert_eq!(quote.file, None);
    }

    #[test]
    fn lines_are_one_line() {
        assert_eq!(one_line("one\n two\t three "), "one two three");
    }
}
