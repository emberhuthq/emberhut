//! Loading fixtures and comparing JSON.

use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A fixture as JSON.
pub(crate) fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A fixture read as `T`.
pub(crate) fn read<T: DeserializeOwned>(name: &str) -> T {
    from_value(fixture(name))
}

/// JSON read as `T`, with the error shown on failure.
pub(crate) fn from_value<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap_or_else(|e| panic!("{e}"))
}

/// `T` as JSON.
pub(crate) fn to_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or_else(|e| panic!("{e}"))
}

/// The fixture reads as `T` and `T` writes it back exactly: the whole shape is the type's.
pub(crate) fn assert_round_trip<T: DeserializeOwned + Serialize>(name: &str) -> T {
    let theirs = fixture(name);
    let read: T = from_value(theirs.clone());
    assert_eq!(to_value(&read), theirs, "{name} doesn't round-trip");
    read
}

/// The fixture reads as `T`, and `T` writes it back in the fixture's shape: every field
/// `T` writes is in the fixture with the same value, and every field of the fixture is written too,
/// except the paths in `not_modelled` (`issues[].body`: a key of every element of `issues`), which
/// the contract doesn't model yet. Writing is stable too.
pub(crate) fn assert_writes_shape<T: DeserializeOwned + Serialize>(
    name: &str,
    not_modelled: &[&str],
) -> T {
    let theirs = fixture(name);
    let read: T = from_value(theirs.clone());
    let ours = to_value(&read);
    assert_subset(&ours, &theirs, name);
    let mut missing = Vec::new();
    find_missing(&ours, &theirs, "", not_modelled, &mut missing);
    assert!(
        missing.is_empty(),
        "{name}: written without these keys the fixture has: {missing:#?}"
    );
    let again: T = from_value(ours.clone());
    assert_eq!(to_value(&again), ours, "{name}: writing isn't stable");
    read
}

/// The paths of `theirs` that `ours` lacks, skipping `not_modelled`.
fn find_missing(
    ours: &Value,
    theirs: &Value,
    path: &str,
    not_modelled: &[&str],
    missing: &mut Vec<String>,
) {
    match (ours, theirs) {
        (Value::Object(ours), Value::Object(theirs)) => {
            for (key, their) in theirs {
                let at = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if not_modelled.contains(&at.as_str()) {
                    continue;
                }
                match ours.get(key) {
                    Some(our) => find_missing(our, their, &at, not_modelled, missing),
                    None => missing.push(at),
                }
            }
        }
        (Value::Array(ours), Value::Array(theirs)) => {
            for (our, their) in ours.iter().zip(theirs) {
                find_missing(our, their, &format!("{path}[]"), not_modelled, missing);
            }
        }
        _ => {}
    }
}

/// Every key of `ours` is in `theirs` with the same value, recursively. Arrays match element
/// by element. Numbers compare by value (`1` and `1.0` are one JSON number).
pub(crate) fn assert_subset(ours: &Value, theirs: &Value, path: &str) {
    match (ours, theirs) {
        (Value::Object(ours), Value::Object(theirs)) => {
            for (key, value) in ours {
                let Some(their) = theirs.get(key) else {
                    panic!("{path}.{key}: written, but the fixture doesn't have it");
                };
                assert_subset(value, their, &format!("{path}.{key}"));
            }
        }
        (Value::Array(ours), Value::Array(theirs)) => {
            assert_eq!(ours.len(), theirs.len(), "{path}: lengths differ");
            for (i, (a, b)) in ours.iter().zip(theirs).enumerate() {
                assert_subset(a, b, &format!("{path}[{i}]"));
            }
        }
        (Value::Number(a), Value::Number(b)) => {
            assert!(a.as_f64() == b.as_f64(), "{path}: {a} is not {b}");
        }
        (a, b) => assert_eq!(a, b, "{path}"),
    }
}

/// Adds an unknown field to every object in the value, recursively.
pub(crate) fn with_unknown_fields(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for field in fields.values_mut() {
                with_unknown_fields(field);
            }
            fields.insert(
                "fromANewerServer".to_owned(),
                serde_json::json!({ "anything": [1, "two", null] }),
            );
        }
        Value::Array(items) => items.iter_mut().for_each(with_unknown_fields),
        _ => {}
    }
}
