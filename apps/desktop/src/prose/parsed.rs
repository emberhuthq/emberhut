//! A message's text read once for the chat to draw: its blocks with their task keys and bare
//! addresses linked, its quotes' words for Copy, its tables measured, or the event lines of a
//! message the server wrote, and which way each part of it reads. The chat keeps one for each message it holds ([`ParsedCache`]),
//! read again only when the message's text changes, so a frame only lays out what was read.
//!
//! What is read is bounded, whatever the server sends: the first
//! [`MOST_TEXT`](blocks::MOST_TEXT) bytes, [`MOST_BLOCKS`] blocks and [`MOST_CELLS`] table
//! cells. A message cut so shows all of its text (up to [`MOST_WHOLE`] bytes) as plain text
//! when the user asks for it.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use emberhut_api::chat::ChatMessage;
use emberhut_api::ids::MessageId;

use super::blocks::{self, Block};
use super::direction::{self, Direction, Run};
use super::events::{self, EventLine};
use super::tables::Table;
use super::{copy, links};

/// The most blocks a message is drawn in.
pub(crate) const MOST_BLOCKS: usize = 80;

/// The most table cells a message draws, all its tables together.
pub(crate) const MOST_CELLS: usize = 400;

/// The most of a message's text shown as plain text when all of it is asked for, in bytes.
pub(crate) const MOST_WHOLE: usize = 256 * 1024;

/// One part of a message as the chat draws it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Drawn {
    /// Markdown that reads from the left throughout, linked.
    Markdown(Arc<str>),
    /// Markdown with parts that read from the right: its runs of one direction, each linked.
    Directed(Vec<Run>),
    /// A quote: its markdown, linked, the words its Copy copies, and which way it reads.
    Quote {
        /// What is drawn.
        shown: Arc<str>,
        /// What Copy puts on the clipboard: its words only.
        words: String,
        /// Which way it reads (its first letter's).
        direction: Direction,
    },
    /// A table, its cells linked.
    Table(Table),
    /// A picture alone on its line.
    Picture {
        /// Where it is: a path on the server or a web address.
        url: String,
        /// What it shows.
        alt: String,
    },
}

/// A message's text, read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Parsed {
    /// The text read (the message's, trimmed), which the message menu reads when it opens.
    text: Rc<str>,
    /// Whether it is the user's own (drawn as one run of inline markdown).
    mine: bool,
    /// Whether it is an event line of the server's (summed up, not drawn as blocks).
    event: bool,
    /// What is drawn, in order.
    pub(crate) blocks: Vec<Drawn>,
    /// Which way the text's first letter reads, which lays out the user's own words.
    pub(crate) direction: Direction,
    /// The whole text as plain text, up to [`MOST_WHOLE`] bytes, when the blocks leave some
    /// of it out ("Show all").
    pub(crate) whole: Option<Arc<str>>,
    /// An event message's event lines.
    pub(crate) event_lines: Vec<EventLine>,
    /// An event message's first line that isn't blank, for a run with no event lines.
    pub(crate) first_line: String,
}

impl Parsed {
    /// `message`'s text, read.
    #[must_use]
    pub(crate) fn of(message: &ChatMessage) -> Self {
        let text: Rc<str> = Rc::from(message.text.trim());
        let mut parsed = Self {
            text: Rc::clone(&text),
            mine: message.mine,
            event: message.event,
            blocks: Vec::new(),
            direction: Direction::default(),
            whole: None,
            event_lines: Vec::new(),
            first_line: String::new(),
        };
        if message.event {
            parsed.event_lines = events::lines([message.text.as_str()]);
            parsed.first_line = message
                .text
                .lines()
                .find(|line| !line.trim().is_empty())
                .map_or_else(String::new, |line| events::cut(line.trim()));
            return parsed;
        }
        let read = blocks::read_part(&text);
        let mut cut = read.len() < text.len();
        parsed.direction = direction::of_first_letter(read);
        if message.mine {
            if !read.is_empty() {
                parsed
                    .blocks
                    .push(Drawn::Markdown(links::linked(read).into()));
            }
        } else {
            let mut all = blocks::blocks(read, true);
            if all.len() > MOST_BLOCKS {
                all.truncate(MOST_BLOCKS);
                cut = true;
            }
            let mut cells = MOST_CELLS;
            parsed.blocks = all
                .into_iter()
                .map(|block| drawn(block, &mut cells))
                .collect();
        }
        if cut {
            let whole = &text[..text.floor_char_boundary(MOST_WHOLE)];
            parsed.whole = Some(Arc::from(whole));
        }
        parsed
    }

    /// The text read: the message's, trimmed.
    #[must_use]
    pub(crate) fn text(&self) -> &Rc<str> {
        &self.text
    }

    /// What Copy on quote `index` (its place among the message's quotes) copies.
    #[must_use]
    pub(crate) fn quote_words(&self, index: usize) -> Option<&str> {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                Drawn::Quote { words, .. } => Some(words.as_str()),
                Drawn::Markdown(_)
                | Drawn::Directed(_)
                | Drawn::Table(_)
                | Drawn::Picture { .. } => None,
            })
            .nth(index)
    }

    /// Whether it is `message`'s text as it is now.
    fn reads(&self, message: &ChatMessage) -> bool {
        self.mine == message.mine
            && self.event == message.event
            && *self.text == *message.text.trim()
    }
}

/// A block as it is drawn, its table taking from the `cells` left.
fn drawn(block: Block, cells: &mut usize) -> Drawn {
    match block {
        Block::Markdown(markdown) => {
            let runs = direction::runs(&markdown);
            if let [Run::LeftToRight(_)] = runs.as_slice() {
                Drawn::Markdown(links::linked(&markdown).into())
            } else {
                Drawn::Directed(
                    runs.into_iter()
                        .map(|run| run.map_markdown(links::linked))
                        .collect(),
                )
            }
        }
        Block::Quote(quote) => Drawn::Quote {
            shown: links::linked(&quote).into(),
            words: copy::plain(&quote),
            direction: direction::of_first_letter(&quote),
        },
        Block::Table(rows) => {
            let table = Table::new(&rows, *cells);
            *cells -= table.cells();
            Drawn::Table(table)
        }
        Block::Image { url, alt } => Drawn::Picture { url, alt },
    }
}

/// The messages read, by id: each read again only when its text changes.
#[derive(Debug, Default)]
pub(crate) struct ParsedCache(HashMap<MessageId, Rc<Parsed>>);

impl ParsedCache {
    /// `message` read, from the cache when its text is the same.
    pub(crate) fn get(&mut self, message: &ChatMessage) -> Rc<Parsed> {
        if let Some(parsed) = self
            .0
            .get(&message.id)
            .filter(|parsed| parsed.reads(message))
        {
            return Rc::clone(parsed);
        }
        let parsed = Rc::new(Parsed::of(message));
        self.0.insert(message.id, Rc::clone(&parsed));
        parsed
    }

    /// `message` as it was read last, if it was.
    #[must_use]
    pub(crate) fn read(&self, message: MessageId) -> Option<&Rc<Parsed>> {
        self.0.get(&message)
    }

    /// Keeps only the messages `held` says are still held.
    pub(crate) fn keep(&mut self, held: impl Fn(MessageId) -> bool) {
        self.0.retain(|&id, _| held(id));
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::*;
    use crate::fixtures::message_at;
    use crate::wire::{TASK_KEY_PREFIX, TASK_LINK};

    fn said(id: i64, extra: serde_json::Value) -> ChatMessage {
        message_at(id, "2026-11-25T08:00:00Z", extra)
    }

    #[test]
    fn a_quotes_copy_takes_its_words_only() {
        let text = format!(
            "Two drafts:\n\n> Hey, the **ads** are ready, {TASK_KEY_PREFIX}4.\n\nOr:\n\n> Done, see [the folder](https://example.com/ads)"
        );
        let parsed = Parsed::of(&said(1, json!({ "text": text })));
        assert_eq!(
            parsed.quote_words(0),
            Some(format!("Hey, the ads are ready, {TASK_KEY_PREFIX}4.").as_str())
        );
        assert_eq!(
            parsed.quote_words(1),
            Some("Done, see the folder (https://example.com/ads)")
        );
        assert_eq!(parsed.quote_words(2), None);
        let Drawn::Quote { shown, .. } = &parsed.blocks[1] else {
            panic!("a quote")
        };
        assert!(shown.contains(&format!("]({TASK_LINK}4)")), "{shown}");
    }

    #[test]
    fn the_users_words_are_one_run_of_markdown_and_an_event_its_lines() {
        let mine = Parsed::of(&said(
            2,
            json!({"mine": true, "text": "> not a quote\n| a |"}),
        ));
        assert_eq!(
            mine.blocks,
            [Drawn::Markdown("> not a quote\n| a |".into())]
        );
        let event = Parsed::of(&said(
            3,
            json!({"event": true, "text": "\n- [mail] From the bank"}),
        ));
        assert_eq!(event.blocks, []);
        assert_eq!(event.event_lines.len(), 1);
        assert_eq!(event.first_line, "- [mail] From the bank");
    }

    #[test]
    fn arabic_parts_are_read_once_with_their_direction_and_links() {
        let text = format!("مرحبا {TASK_KEY_PREFIX}4\n\nThe end.\n\n> اقتباس");
        let parsed = Parsed::of(&said(10, json!({ "text": text })));
        let Drawn::Directed(runs) = &parsed.blocks[0] else {
            panic!("runs of one direction: {:?}", parsed.blocks)
        };
        let Run::RightToLeft(parts) = &runs[0] else {
            panic!("from the right first")
        };
        assert!(parts[0].markdown.contains(&format!("]({TASK_LINK}4)")));
        assert_eq!(runs[1], Run::LeftToRight("The end.".into()));
        let Drawn::Quote { direction, .. } = &parsed.blocks[1] else {
            panic!("a quote")
        };
        assert!(direction.is_rtl());
        let mine = Parsed::of(&said(11, json!({"mine": true, "text": "أهلا Emberhut"})));
        assert!(mine.direction.is_rtl());
        let english = Parsed::of(&said(12, json!({"text": "Hello."})));
        assert_eq!(english.blocks, [Drawn::Markdown("Hello.".into())]);
        assert!(!english.direction.is_rtl());
    }

    #[test]
    fn a_message_is_read_again_only_when_its_text_changes() {
        let mut cache = ParsedCache::default();
        let first = said(1, json!({"text": "Hello"}));
        let read = cache.get(&first);
        assert!(Rc::ptr_eq(&read, &cache.get(&first)), "read once");
        let changed = said(1, json!({"text": "Hello again"}));
        let again = cache.get(&changed);
        assert!(!Rc::ptr_eq(&read, &again));
        assert_eq!(&**again.text(), "Hello again");
        cache.keep(|id| id != MessageId::new(1));
        assert_eq!(cache.read(MessageId::new(1)), None);
    }

    #[test]
    fn a_long_message_is_cut_and_shows_whole_on_asking() {
        let long = "A line of words.\n".repeat(blocks::MOST_TEXT / 8);
        let parsed = Parsed::of(&said(4, json!({ "text": long })));
        let whole = parsed.whole.as_deref().unwrap();
        assert_eq!(whole, long.trim());
        let blocks = "> a quote\n\nwords\n\n".repeat(MOST_BLOCKS);
        let parsed = Parsed::of(&said(5, json!({ "text": blocks })));
        assert_eq!(parsed.blocks.len(), MOST_BLOCKS);
        assert!(parsed.whole.is_some());
        let short = Parsed::of(&said(6, json!({"text": "Short."})));
        assert_eq!(short.whole, None);
    }

    #[test]
    fn tables_share_a_budget_of_cells() {
        let row = format!("|{}|\n", " x |".repeat(20));
        let table = row.repeat(100);
        let text = format!("{table}\nBetween.\n\n{table}");
        let parsed = Parsed::of(&said(7, json!({ "text": text })));
        let cells: usize = parsed
            .blocks
            .iter()
            .map(|block| match block {
                Drawn::Table(table) => table.cells(),
                _ => 0,
            })
            .sum();
        assert!(cells <= MOST_CELLS, "{cells}");
    }

    /// Text built to make a scan go back over itself: brackets that never close, links that
    /// never end, bars, backticks and stars.
    #[test]
    fn pathological_text_is_read_in_a_bounded_time() {
        const SIZE: usize = 100 * 1024;
        let fill = |unit: &str| unit.repeat(SIZE / unit.len());
        let texts = [
            fill("["),
            fill("!["),
            fill("[a]("),
            fill("]("),
            fill("[a"),
            fill("[a] (b"),
            fill("|"),
            fill("| a "),
            fill("`"),
            fill("*a"),
            fill("> ["),
            fill("[a](b c"),
            format!("{}{}", fill("["), fill(")")),
        ];
        let started = Instant::now();
        for text in &texts {
            let parsed = Parsed::of(&said(8, json!({ "text": text })));
            let mine = Parsed::of(&said(9, json!({"mine": true, "text": text })));
            assert!(parsed.blocks.len() <= MOST_BLOCKS && mine.blocks.len() <= 1);
            // What the menu and Copy read, from the whole text.
            let pasted = copy::for_pasting(text);
            let words = copy::plain(text);
            let found = copy::web_links(text);
            let linked = links::linked(text);
            assert!(!pasted.is_empty() && !words.is_empty() && !linked.is_empty());
            assert!(found.len() <= text.len());
        }
        let took = started.elapsed();
        assert!(
            took < Duration::from_secs(5),
            "{took:?} for {} texts of 100 KB",
            texts.len()
        );
    }
}
