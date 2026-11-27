//! A reader of server-sent events (`text/event-stream`), from any [`BufRead`].
//!
//! It follows the HTML standard's parsing rules for what emberhut's streams use: `data:` lines
//! (several join with `\n`), `event:` and `id:` fields, `:` comments, an event ending at a blank
//! line, and `\n` or `\r\n` line ends, a leading byte-order mark dropped. An event whose
//! data is empty isn't dispatched. `retry:` and unknown fields are ignored. Comments are
//! returned too, so a caller can act on the server's pings. A lone `\r` as a line end, which the
//! standard also allows, isn't read as one.

use std::error::Error;
use std::fmt;
use std::io::{self, BufRead, Read as _};

/// The longest line read (without its line end), so a stream that never ends a line can't take
/// all memory.
const MAX_LINE: u64 = 4 << 20;

/// What the stream said: an event or a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SseItem {
    /// An event.
    Event(SseEvent),
    /// A comment line (the server's `: ping`), without its `:` and the space after it.
    Comment(String),
}

/// One event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SseEvent {
    /// Its type, from an `event:` field; none for the default (`message`).
    pub event: Option<String>,
    /// Its data: every `data:` line, joined with `\n`.
    pub data: String,
    /// Its id, from an `id:` field.
    pub id: Option<String>,
}

/// Reads [`SseItem`]s from a stream, until it ends. An event still open at the end is dropped,
/// as the standard says.
#[derive(Debug)]
pub struct SseReader<R> {
    reader: R,
    line: Vec<u8>,
    /// The event being read: a comment can come between its lines.
    pending: SseEvent,
    /// No line has been read yet (it may start with a byte-order mark).
    first: bool,
    has_data: bool,
}

impl<R: BufRead> SseReader<R> {
    /// A reader of this stream.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            line: Vec::new(),
            pending: SseEvent::default(),
            has_data: false,
            first: true,
        }
    }

    /// The next item, or none at the end of the stream.
    ///
    /// # Errors
    ///
    /// The stream failed, a line wasn't UTF-8, or a line was longer than 4 MiB.
    pub fn next_item(&mut self) -> io::Result<Option<SseItem>> {
        loop {
            let Some(mut line) = next_line(&mut self.reader, &mut self.line)? else {
                return Ok(None);
            };
            if std::mem::take(&mut self.first) {
                line = line.strip_prefix('\u{feff}').unwrap_or(line);
            }
            if line.is_empty() {
                let event = std::mem::take(&mut self.pending);
                self.has_data = false;
                if !event.data.is_empty() {
                    return Ok(Some(SseItem::Event(event)));
                }
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                None => (line, ""),
            };
            match field {
                "" => return Ok(Some(SseItem::Comment(value.to_owned()))),
                "data" => {
                    if self.has_data {
                        self.pending.data.push('\n');
                    }
                    self.pending.data.push_str(value);
                    self.has_data = true;
                }
                "event" => self.pending.event = Some(value.to_owned()),
                "id" => self.pending.id = Some(value.to_owned()),
                _ => {}
            }
        }
    }
}

/// The next line of `reader` without its line end, read into `buffer`; none at the end.
fn next_line<'a>(
    reader: &mut impl BufRead,
    buffer: &'a mut Vec<u8>,
) -> io::Result<Option<&'a str>> {
    buffer.clear();
    let read = reader
        .by_ref()
        .take(MAX_LINE + 2)
        .read_until(b'\n', buffer)?;
    if read == 0 {
        return Ok(None);
    }
    if buffer.ends_with(b"\n") {
        buffer.pop();
    }
    if buffer.ends_with(b"\r") {
        buffer.pop();
    }
    if buffer.len() as u64 > MAX_LINE {
        return Err(malformed("an event-stream line longer than 4 MiB"));
    }
    std::str::from_utf8(buffer).map(Some).map_err(malformed)
}

/// The mark inside an error of the event stream's own format, which [`is_malformed`] finds:
/// the stream underneath may fail with any kind of error, `InvalidData` included.
#[derive(Debug)]
struct Malformed(Box<dyn Error + Send + Sync>);

impl fmt::Display for Malformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Error for Malformed {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}

/// An error of the stream's format (`InvalidData`, with the mark inside).
fn malformed(error: impl Into<Box<dyn Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, Malformed(error.into()))
}

/// Whether `error` is the event stream's own (a line that isn't UTF-8, or too long), not the
/// stream underneath failing.
pub(crate) fn is_malformed(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(<dyn Error + Send + Sync>::is::<Malformed>)
}

impl<R: BufRead> Iterator for SseReader<R> {
    type Item = io::Result<SseItem>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_item().transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(stream: &str) -> Vec<SseItem> {
        SseReader::new(stream.as_bytes())
            .collect::<io::Result<_>>()
            .unwrap()
    }

    fn data(text: &str) -> SseItem {
        SseItem::Event(SseEvent {
            data: text.to_owned(),
            ..SseEvent::default()
        })
    }

    #[test]
    fn reads_the_live_stream() {
        let stream = "data: {\"text\":\"\",\"doing\":\"Reading\"}\n\n: ping\n\ndata: null\n\n";
        assert_eq!(
            items(stream),
            [
                data(r#"{"text":"","doing":"Reading"}"#),
                SseItem::Comment("ping".to_owned()),
                data("null")
            ]
        );
    }

    #[test]
    fn joins_data_lines() {
        assert_eq!(items("data: one\ndata:two\ndata\n\n"), [data("one\ntwo\n")]);
    }

    #[test]
    fn reads_event_and_id_fields() {
        let event = SseEvent {
            event: Some("step".to_owned()),
            data: "x".to_owned(),
            id: Some("7".to_owned()),
        };
        assert_eq!(
            items("event: step\nid: 7\nretry: 100\nunknown: y\ndata: x\n\n"),
            [SseItem::Event(event)]
        );
    }

    #[test]
    fn takes_crlf_line_ends() {
        assert_eq!(
            items("data: a\r\n\r\n:ping\r\n"),
            [data("a"), SseItem::Comment("ping".to_owned())]
        );
    }

    #[test]
    fn an_event_without_data_is_not_sent() {
        assert_eq!(items("event: nothing\n\n\n\ndata: a\n\n"), [data("a")]);
        assert_eq!(items("data:\n\n"), [], "empty data isn't dispatched");
        assert_eq!(items("data:\ndata:\n\n"), [data("\n")]);
    }

    #[test]
    fn drops_a_leading_byte_order_mark() {
        let stream = "\u{feff}data: a\n\n\u{feff}data: b\n\n";
        assert_eq!(
            items(stream),
            [data("a")],
            "only the stream's first line loses it"
        );
    }

    #[test]
    fn takes_a_line_of_the_longest_length() {
        let longest = format!(
            "data: {}\n\n",
            "a".repeat(usize::try_from(MAX_LINE).unwrap() - 6)
        );
        let items = items(&longest);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn an_unfinished_event_at_the_end_is_dropped() {
        assert_eq!(items("data: a\n\ndata: half"), [data("a")]);
        assert_eq!(items(""), []);
    }

    #[test]
    fn a_comment_inside_an_event_keeps_it() {
        assert_eq!(
            items("data: a\n: ping\ndata: b\n\n"),
            [SseItem::Comment("ping".to_owned()), data("a\nb")]
        );
    }

    #[test]
    fn keeps_a_space_that_isnt_the_first() {
        assert_eq!(items("data:  two spaces\n\n"), [data(" two spaces")]);
    }

    #[test]
    fn refuses_lines_that_are_not_utf8() {
        let mut reader = SseReader::new(&b"data: \xff\n\n"[..]);
        assert_eq!(
            reader.next_item().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn its_own_errors_are_marked() {
        let mut reader = SseReader::new(&b"data: \xff\n\n"[..]);
        assert!(is_malformed(&reader.next_item().unwrap_err()));
        let underneath = io::Error::new(io::ErrorKind::InvalidData, "a broken stream");
        assert!(!is_malformed(&underneath));
    }

    #[test]
    fn refuses_endless_lines() {
        let long = vec![b'a'; usize::try_from(MAX_LINE).unwrap() + 2];
        let mut reader = SseReader::new(&long[..]);
        assert_eq!(
            reader.next_item().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
