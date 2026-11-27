//! A field of a change request that can be kept, cleared or set.
//!
//! The server reads such a field three ways: left out keeps what it has, `null` clears it, a
//! value sets it. [`Patch`] names the three, so a request can't confuse "keep" with "clear".
//! A field of this type takes `#[serde(default, skip_serializing_if = "Patch::is_keep")]`, so
//! `Keep` is left out of the JSON.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What a change does to one field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Patch<T> {
    /// Keep it as it is: the field is left out.
    #[default]
    Keep,
    /// Clear it: `null`.
    Clear,
    /// Set it to this.
    Set(T),
}

impl<T> Patch<T> {
    /// Whether it keeps the field (and so is left out of the request).
    #[must_use]
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }

    /// `Some(value)` sets, `None` clears.
    #[must_use]
    pub fn of(value: Option<T>) -> Self {
        value.map_or(Self::Clear, Self::Set)
    }
}

impl<T> From<Option<T>> for Patch<T> {
    /// As [`Patch::of`].
    fn from(value: Option<T>) -> Self {
        Self::of(value)
    }
}

impl<T: Serialize> Serialize for Patch<T> {
    /// `Set` writes the value; `Clear` (and `Keep`, which a request leaves out) write `null`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Set(value) => value.serialize(serializer),
            Self::Keep | Self::Clear => serializer.serialize_none(),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    /// A value reads as `Set`, `null` as `Clear`; a field left out takes the default, `Keep`.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(deserializer).map(Self::of)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Change {
        #[serde(default, skip_serializing_if = "Patch::is_keep")]
        day: Patch<String>,
    }

    #[test]
    fn a_patch_has_three_cases_on_the_wire() {
        let keep: Change = serde_json::from_str("{}").unwrap();
        assert_eq!(keep.day, Patch::Keep);
        assert_eq!(serde_json::to_string(&keep).unwrap(), "{}");
        let clear: Change = serde_json::from_str(r#"{"day":null}"#).unwrap();
        assert_eq!(clear.day, Patch::Clear);
        assert_eq!(serde_json::to_string(&clear).unwrap(), r#"{"day":null}"#);
        let set = Change {
            day: Patch::of(Some("2026-12-01".to_owned())),
        };
        assert_eq!(
            serde_json::to_string(&set).unwrap(),
            r#"{"day":"2026-12-01"}"#
        );
        assert_eq!(Patch::<u8>::from(None), Patch::Clear);
    }
}
