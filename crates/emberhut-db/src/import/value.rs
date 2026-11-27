//! The archive's row format: a JSON object with the columns in table order, each value keeping
//! its SQLite storage class.
//!
//! - integers are JSON integers;
//! - reals are the shortest decimal that reads back to the same `f64`, always with a fraction
//!   or an exponent (`1.0`, `1e300`, `-0.0`); an infinity is `{"real": "inf"}` or
//!   `{"real": "-inf"}`;
//! - text that is UTF-8 is a JSON string, other text `{"text_bytes": "<base64>"}`;
//! - blobs are `{"blob": "<base64>"}`;
//! - NULL is `null`.
//!
//! The JSON is written deterministically (column order, no whitespace), so the same row is
//! always the same text.

use std::fmt::{self, Write as _};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::de::{self, Deserialize, Deserializer, MapAccess, Visitor};

/// One value, in its SQLite storage class.
#[derive(Clone, Debug, PartialEq)]
pub enum SqlValue {
    /// NULL.
    Null,
    /// An integer.
    Integer(i64),
    /// A real number (never NaN: SQLite stores NaN as NULL).
    Real(f64),
    /// Text that is valid UTF-8.
    Text(String),
    /// Text that is not valid UTF-8, which SQLite allows: its bytes.
    TextBytes(Vec<u8>),
    /// A blob.
    Blob(Vec<u8>),
}

impl SqlValue {
    /// The storage class as SQLite's `typeof()` names it.
    #[must_use]
    pub const fn storage_class(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Integer(_) => "integer",
            Self::Real(_) => "real",
            Self::Text(_) | Self::TextBytes(_) => "text",
            Self::Blob(_) => "blob",
        }
    }

    /// The text, when the value is UTF-8 text.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    /// The integer, when the value is one.
    #[must_use]
    pub const fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// Whether the value is NULL.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    fn write_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Integer(n) => {
                // Writing to a String cannot fail.
                write!(out, "{n}").ok();
            }
            Self::Real(x) => match serde_json::Number::from_f64(*x) {
                Some(number) => out.push_str(&number.to_string()),
                None if x.is_sign_negative() => out.push_str(r#"{"real":"-inf"}"#),
                None => out.push_str(r#"{"real":"inf"}"#),
            },
            Self::Text(text) => push_json_string(out, text),
            Self::TextBytes(bytes) => {
                out.push_str(r#"{"text_bytes":""#);
                out.push_str(&STANDARD.encode(bytes));
                out.push_str(r#""}"#);
            }
            Self::Blob(bytes) => {
                out.push_str(r#"{"blob":""#);
                out.push_str(&STANDARD.encode(bytes));
                out.push_str(r#""}"#);
            }
        }
    }

    fn from_json(value: serde_json::Value) -> Result<Self, RowFormatError> {
        use serde_json::Value;
        match value {
            Value::Null => Ok(Self::Null),
            Value::String(text) => Ok(Self::Text(text)),
            Value::Number(number) => {
                if let Some(n) = number.as_i64() {
                    Ok(Self::Integer(n))
                } else if number.is_f64() {
                    number
                        .as_f64()
                        .map(Self::Real)
                        .ok_or_else(|| RowFormatError::new("a number that is not a real"))
                } else {
                    Err(RowFormatError::new("an integer out of range"))
                }
            }
            Value::Object(object) if object.len() == 1 => {
                let Some((tag, Value::String(text))) = object.into_iter().next() else {
                    return Err(RowFormatError::new("a tagged value that is not a string"));
                };
                match tag.as_str() {
                    "real" if text == "inf" => Ok(Self::Real(f64::INFINITY)),
                    "real" if text == "-inf" => Ok(Self::Real(f64::NEG_INFINITY)),
                    "text_bytes" => Ok(Self::TextBytes(decode(&text)?)),
                    "blob" => Ok(Self::Blob(decode(&text)?)),
                    _ => Err(RowFormatError::new(format!("an unknown tag {tag:?}"))),
                }
            }
            _ => Err(RowFormatError::new("a value of no storage class")),
        }
    }
}

fn decode(text: &str) -> Result<Vec<u8>, RowFormatError> {
    STANDARD
        .decode(text)
        .map_err(|error| RowFormatError::new(format!("bad base64: {error}")))
}

fn push_json_string(out: &mut String, text: &str) {
    // Serialising a string to JSON cannot fail.
    match serde_json::to_string(text) {
        Ok(quoted) => out.push_str(&quoted),
        Err(error) => log::error!("a string could not be written as JSON: {error}"),
    }
}

/// A row: its columns' names and values, in table order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    columns: Vec<(String, SqlValue)>,
}

impl Row {
    /// An empty row.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the next column.
    pub fn push(&mut self, name: impl Into<String>, value: SqlValue) {
        self.columns.push((name.into(), value));
    }

    /// The value of the column called `name`, if the row has it.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&SqlValue> {
        self.columns
            .iter()
            .find(|(column, _)| column == name)
            .map(|(_, value)| value)
    }

    /// Sets the column called `name` to NULL, if the row has it; whether it had a value.
    pub fn scrub(&mut self, name: &str) -> bool {
        self.columns
            .iter_mut()
            .find(|(column, _)| column == name)
            .is_some_and(|(_, value)| !std::mem::replace(value, SqlValue::Null).is_null())
    }

    /// The columns, in order.
    pub fn columns(&self) -> impl Iterator<Item = (&str, &SqlValue)> {
        self.columns
            .iter()
            .map(|(name, value)| (name.as_str(), value))
    }

    /// The row as the archive writes it.
    ///
    /// ```
    /// use emberhut_db::import::{Row, SqlValue};
    ///
    /// let mut row = Row::new();
    /// row.push("id", SqlValue::Integer(41));
    /// row.push("text", SqlValue::Text("Buy milk".into()));
    /// row.push("t", SqlValue::Real(2.0));
    /// row.push("files", SqlValue::Null);
    /// assert_eq!(row.to_json(), r#"{"id":41,"text":"Buy milk","t":2.0,"files":null}"#);
    /// ```
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(16 * self.columns.len() + 2);
        out.push('{');
        for (i, (name, value)) in self.columns.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            push_json_string(&mut out, name);
            out.push(':');
            value.write_json(&mut out);
        }
        out.push('}');
        out
    }

    /// The row the archive's `text` holds.
    ///
    /// # Errors
    ///
    /// [`RowFormatError`] when the text is not a row in the archive's format.
    pub fn from_json(text: &str) -> Result<Self, RowFormatError> {
        serde_json::from_str(text).map_err(|error| RowFormatError::new(error.to_string()))
    }
}

impl<'de> Deserialize<'de> for Row {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(RowVisitor)
    }
}

struct RowVisitor;

impl<'de> Visitor<'de> for RowVisitor {
    type Value = Row;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an archived row")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Row, A::Error> {
        let mut row = Row::new();
        while let Some((name, value)) = map.next_entry::<String, serde_json::Value>()? {
            let value = SqlValue::from_json(value).map_err(de::Error::custom)?;
            row.push(name, value);
        }
        Ok(row)
    }
}

/// The text is not a row in the archive's format.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not an archived row: {0}")]
pub struct RowFormatError(String);

impl RowFormatError {
    pub(crate) fn new(why: impl Into<String>) -> Self {
        Self(why.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: SqlValue) -> SqlValue {
        let mut row = Row::new();
        row.push("v", value);
        let text = row.to_json();
        let back = Row::from_json(&text).unwrap();
        assert_eq!(back.to_json(), text, "the same text again");
        back.get("v").unwrap().clone()
    }

    #[test]
    fn every_storage_class_comes_back_as_it_was() {
        for value in [
            SqlValue::Null,
            SqlValue::Integer(0),
            SqlValue::Integer(i64::MIN),
            SqlValue::Integer(i64::MAX),
            SqlValue::Real(1.0),
            SqlValue::Real(0.1),
            SqlValue::Real(1e300),
            SqlValue::Real(-2.5e-310),
            SqlValue::Real(f64::INFINITY),
            SqlValue::Real(f64::NEG_INFINITY),
            SqlValue::Text(String::new()),
            SqlValue::Text("ein \"Zitat\"\n\u{1F600}".into()),
            SqlValue::TextBytes(vec![0x66, 0xFF, 0x00]),
            SqlValue::Blob(vec![]),
            SqlValue::Blob(vec![0, 1, 2, 255]),
        ] {
            assert_eq!(round_trip(value.clone()), value);
        }
    }

    #[test]
    fn a_real_keeps_its_bits_even_the_sign_of_zero() {
        for x in [-0.0_f64, 0.0, 0.1 + 0.2, f64::MIN_POSITIVE, f64::MAX, 1e16] {
            let SqlValue::Real(back) = round_trip(SqlValue::Real(x)) else {
                panic!("{x} came back as something else");
            };
            assert_eq!(back.to_bits(), x.to_bits(), "{x}");
        }
    }

    #[test]
    fn a_whole_real_stays_a_real_and_an_integer_an_integer() {
        let mut row = Row::new();
        row.push("a", SqlValue::Real(3.0));
        row.push("b", SqlValue::Integer(3));
        assert_eq!(row.to_json(), r#"{"a":3.0,"b":3}"#);
        let back = Row::from_json(&row.to_json()).unwrap();
        assert_eq!(back, row);
    }

    #[test]
    fn text_that_is_not_utf8_and_a_blob_are_told_apart() {
        let mut row = Row::new();
        row.push("t", SqlValue::TextBytes(vec![0xC3]));
        row.push("b", SqlValue::Blob(vec![0xC3]));
        assert_eq!(
            row.to_json(),
            r#"{"t":{"text_bytes":"ww=="},"b":{"blob":"ww=="}}"#
        );
    }

    #[test]
    fn column_order_is_kept() {
        let back = Row::from_json(r#"{"z":1,"a":2,"m":null}"#).unwrap();
        let names: Vec<_> = back.columns().map(|(name, _)| name).collect();
        assert_eq!(names, ["z", "a", "m"]);
    }

    #[test]
    fn what_is_not_the_format_is_refused() {
        for text in [
            "[]",
            r#"{"a":true}"#,
            r#"{"a":[1]}"#,
            r#"{"a":{"real":"nan"}}"#,
            r#"{"a":{"blob":"%%"}}"#,
            r#"{"a":{"blob":"AA==","x":"y"}}"#,
            r#"{"a":18446744073709551615}"#,
        ] {
            assert!(Row::from_json(text).is_err(), "{text}");
        }
    }

    #[test]
    fn scrubbing_says_whether_there_was_a_value() {
        let mut row = Row::new();
        row.push("token", SqlValue::Text("secret".into()));
        row.push("none", SqlValue::Null);
        assert!(row.scrub("token"));
        assert!(!row.scrub("none"));
        assert!(!row.scrub("missing"));
        assert_eq!(row.get("token"), Some(&SqlValue::Null));
    }
}
