//! Small serde helpers for the server's habits.

/// For `skip_serializing_if`: the server writes some flags only when they are true.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
pub(crate) fn is_false(flag: &bool) -> bool {
    !*flag
}

/// For `deserialize_with` on a defaulted field: `null` reads as the default.
pub(crate) fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    let value: Option<T> = serde::Deserialize::deserialize(deserializer)?;
    Ok(value.unwrap_or_default())
}

/// For `deserialize_with` on an optional text id: an empty string reads as none.
pub(crate) fn empty_as_none<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: From<String>,
{
    let text: Option<String> = serde::Deserialize::deserialize(deserializer)?;
    Ok(text.filter(|text| !text.is_empty()).map(T::from))
}

/// For `deserialize_with` on an optional text read loosely: a string, or a number as its digits. An empty string, zero, `null` or anything else is none.
pub(crate) fn loose_text<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(text) if !text.is_empty() => Some(text),
        serde_json::Value::Number(number) if number.as_f64() != Some(0.0) => {
            Some(number.to_string())
        }
        _ => None,
    })
}
