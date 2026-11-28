//! Pairing a runner with a server: a one-time code from the server's app (Machines, "Add a hut")
//! is traded for the runner's name and token; the runner keeps the server's address in its
//! settings and the token in its token file, and the server keeps the address it calls the
//! runner at.

use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::config::{CONFIG_FILE, Config, is_runner_name, write_private};
use crate::wire;

/// What pairing takes.
#[derive(Debug, Clone, Default)]
pub struct PairAsk {
    /// The server's address.
    pub server: String,
    /// The one-time code.
    pub code: String,
    /// The name to ask for (the settings' name when not given).
    pub name: Option<String>,
    /// Where the machine is, in words.
    pub place: Option<String>,
    /// The address the server calls the runner at (the first listen address when not given).
    pub advertise: Option<String>,
}

/// What pairing did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paired {
    /// The name the server gave.
    pub name: String,
    /// The server's address.
    pub server: String,
    /// The address the server calls the runner at.
    pub url: String,
}

/// Why pairing didn't work, in words for a person.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct PairError(pub String);

fn ok_url(s: &str) -> bool {
    url::Url::parse(s).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
}

/// Trades the code for a name and token, then writes the settings and token file.
///
/// # Errors
/// A wrong address or name, a server that doesn't answer or refuses, or files that can't be written.
pub async fn pair(c: &Config, ask: &PairAsk, http: &reqwest::Client) -> Result<Paired, PairError> {
    let server = ask.server.trim().trim_end_matches('/').to_owned();
    if !ok_url(&server) {
        return Err(PairError(format!(
            "the server's address must start with http:// or https:// (got {server})"
        )));
    }
    let name = ask.name.clone().unwrap_or_else(|| c.name.clone());
    if !is_runner_name(&name) {
        return Err(PairError(format!(
            "name {name:?}: letters, digits, - and _ only"
        )));
    }
    let url = ask
        .advertise
        .clone()
        .unwrap_or_else(|| c.advertised())
        .trim_end_matches('/')
        .to_owned();
    if !ok_url(&url) {
        return Err(PairError(format!(
            "the runner's address must start with http:// or https:// (got {url})"
        )));
    }
    let mut body = json!({ "code": ask.code.trim(), "name": name, "url": url, "platform": std::env::consts::OS, "version": crate::version() });
    if let (Some(p), Value::Object(b)) = (&ask.place, &mut body) {
        b.insert("place".to_owned(), json!(p));
    }
    let r = http
        .post(format!("{server}{}", wire::PAIR))
        .json(&body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| PairError(format!("the server at {server} didn't answer ({e})")))?;
    let status = r.status();
    let answer: Value = r.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let why = answer
            .get("error")
            .and_then(Value::as_str)
            .map_or_else(|| format!("the server said {status}"), str::to_owned);
        return Err(PairError(why));
    }
    let token = answer
        .get("token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| PairError("the server's answer had no token".to_owned()))?;
    let given = answer
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| is_runner_name(n))
        .map_or(name, str::to_owned);
    let fail = |what: &str, e: std::io::Error| PairError(format!("cannot write {what}: {e}"));
    if let Some(dir) = c.token_file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail("the token's folder", e))?;
    }
    write_private(&c.token_file, format!("{token}\n").as_bytes())
        .map_err(|e| fail(&c.token_file.display().to_string(), e))?;
    let file = c.home.join(CONFIG_FILE);
    let mut settings: Map<String, Value> = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    settings.insert("name".to_owned(), json!(given));
    settings.insert("server".to_owned(), json!(server));
    if ask.advertise.is_some() {
        settings.insert("advertise".to_owned(), json!(url));
    }
    std::fs::create_dir_all(&c.home).map_err(|e| fail("the runner's home", e))?;
    let mut text = serde_json::to_string_pretty(&Value::Object(settings)).unwrap_or_default();
    text.push('\n');
    write_private(&file, text.as_bytes()).map_err(|e| fail(&file.display().to_string(), e))?;
    Ok(Paired {
        name: given,
        server,
        url,
    })
}
