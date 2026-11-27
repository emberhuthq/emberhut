//! Builds the wire dialect's tables (see `src/dialect.rs`) from the JSON file named by
//! `EMBERHUT_WIRE_DIALECT`, or empty tables when it is unset.
//!
//! The file is `{ "words": [[server, ours], ...], "keys": [...], "headers": [...],
//! "prefixes": [...] }`, every part optional: each pair is the server's spelling, then the
//! contract's.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

use serde_json::Value;

/// The variable naming the dialect file.
const VAR: &str = "EMBERHUT_WIRE_DIALECT";

/// The tables a dialect file has, in the order the generated code lists them.
const PARTS: [&str; 4] = ["words", "keys", "headers", "prefixes"];

fn main() -> Result<(), String> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed={VAR}");
    let tables = match env::var_os(VAR).filter(|path| !path.is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            read(&path)?
        }
        None => vec![Vec::new(); PARTS.len()],
    };
    let mut code = String::from("Table {\n");
    for (part, pairs) in PARTS.iter().zip(&tables) {
        writeln!(code, "    {part}: &[").map_err(|error| error.to_string())?;
        for (server, ours) in pairs {
            writeln!(code, "        ({server:?}, {ours:?}),").map_err(|error| error.to_string())?;
        }
        code.push_str("    ],\n");
    }
    code.push_str("}\n");
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("cargo sets OUT_DIR")?);
    fs::write(out.join("dialect.rs"), code).map_err(|error| error.to_string())
}

/// The dialect file's tables, in [`PARTS`]' order.
fn read(path: &Path) -> Result<Vec<Vec<(String, String)>>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("{VAR}: can't read {}: {error}", path.display()))?;
    let file: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{VAR}: {} isn't JSON: {error}", path.display()))?;
    let object = file
        .as_object()
        .ok_or_else(|| format!("{VAR}: {} isn't a JSON object", path.display()))?;
    if let Some(unknown) = object.keys().find(|key| !PARTS.contains(&key.as_str())) {
        return Err(format!("{VAR}: `{unknown}` isn't one of {PARTS:?}"));
    }
    PARTS
        .iter()
        .map(|&part| {
            let Some(pairs) = object.get(part) else {
                return Ok(Vec::new());
            };
            let pairs = pairs
                .as_array()
                .ok_or_else(|| format!("{VAR}: `{part}` isn't a list"))?;
            pairs.iter().map(|pair| read_pair(part, pair)).collect()
        })
        .collect()
}

/// One `[server, ours]` pair of `part`: two different, non-empty strings, header names in
/// lower case.
fn read_pair(part: &str, pair: &Value) -> Result<(String, String), String> {
    let bad = || format!("{VAR}: `{part}` has {pair}, not a pair of different non-empty strings");
    let [Value::String(server), Value::String(ours)] = pair.as_array().ok_or_else(bad)?.as_slice()
    else {
        return Err(bad());
    };
    if server.is_empty() || ours.is_empty() || server == ours {
        return Err(bad());
    }
    if part == "headers" {
        let name = |name: &str| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        };
        if !name(server) || !name(ours) {
            return Err(format!("{VAR}: `headers` has {pair}, not two header names"));
        }
        return Ok((server.to_ascii_lowercase(), ours.to_ascii_lowercase()));
    }
    Ok((server.clone(), ours.clone()))
}
