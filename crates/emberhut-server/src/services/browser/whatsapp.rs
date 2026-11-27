//! WhatsApp Web in the server's own browser, for the messaging area's `whatsapp_send`: open
//! the owner's chat (a person by phone number, WhatsApp's own link; a group or a hidden number
//! by its exact name, once, never a guess), type a draft line by line (Shift+Enter between
//! lines; Enter would send) and read it back; and, once the owner said send, press Send and
//! wait for its tick. No unofficial library or API, nothing read from WhatsApp's storage. When
//! WhatsApp Web isn't linked, the browser is handed to the owner to scan its code.
//!
//! The messaging area's `WhatsAppWeb` seam is these two calls: [`BrowserService::whatsapp_draft`]
//! and [`BrowserService::whatsapp_send`], their errors one for one.

use std::time::Duration;

use serde_json::{Value, json};

use super::engine::{PageQuery, TabId};
use super::surface::Who;
use super::{BrowserArgs, BrowserService};

/// WhatsApp Web's address.
pub const WHATSAPP_WEB: &str = "https://web.whatsapp.com";

/// The page's state: `linked` (the chat list), `login` (its code or phone-number page), or `""`.
pub const STATE: &str = r##"() => document.querySelector("#pane-side, [aria-label='Chat list']") ? "linked" : (/scan to log in|log in with phone number|link with phone number|use whatsapp on your computer/i.test(document.body?.innerText ?? "") ? "login" : "")"##;

/// Focuses the chat search box: whether there is one.
pub const SEARCH: &str = r##"() => { const s = document.querySelector("#side [contenteditable='true'], #side input[type='text'], [aria-label='Search input textbox']"); s?.focus(); s?.click(); return !!s; }"##;

/// The chats listed whose title is exactly the argument: how many, and the one clicked when
/// there is exactly one.
pub const PICK: &str = r##"(name) => { const hits = [...document.querySelectorAll("#pane-side span[title]")].filter((s) => s.getAttribute("title") === name); if (hits.length === 1) hits[0].click(); return hits.length; }"##;

/// Focuses the chat's composer: whether it's there.
pub const COMPOSER: &str = r#"() => { const c = document.querySelector("footer [contenteditable='true'][role='textbox'], footer div[contenteditable='true']"); c?.focus(); c?.click(); return !!c; }"#;

/// The composer's text.
pub const TYPED: &str = r#"() => document.querySelector("footer [contenteditable='true'][role='textbox'], footer div[contenteditable='true']")?.innerText ?? """#;

/// The open chat's name, as its header shows it.
pub const HEADER: &str =
    r##"() => (document.querySelector("#main header span[dir='auto']")?.innerText ?? "").trim()"##;

/// Presses the composer's Send: whether it was there.
pub const SEND: &str = r#"() => { const b = document.querySelector("footer button[aria-label='Send'], footer span[data-icon='send'], footer span[data-icon='wds-ic-send-filled']"); b?.click(); return !!b; }"#;

/// The tick on the owner's newest message holding the argument (`msg-dblcheck`, `msg-check`,
/// or `""` while it's sending).
pub const TICK: &str = r##"(snip) => { const mine = [...document.querySelectorAll("#main [data-id^='true_'], #main div.message-out")].filter((m) => m.innerText.includes(snip)); return mine.at(-1)?.querySelector("[data-icon*='check']")?.getAttribute("data-icon") ?? ""; }"##;

/// A chat to open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaChat {
    /// Its name.
    pub name: String,
    /// A person's number; none for a group or a hidden number.
    pub phone: Option<String>,
    /// The owner's own "message yourself" chat.
    pub me: bool,
}

/// Why WhatsApp Web didn't type or send.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WaError {
    /// No browser is set up.
    #[error("WhatsApp Web isn't available on this server (no browser is set up for it)")]
    NotAvailable,
    /// Not linked: the owner was handed the browser; what the hand-off said.
    #[error("WhatsApp Web isn't linked yet. {0}")]
    NotLinked(String),
    /// The page didn't load.
    #[error("WhatsApp Web didn't load (no chat list, no login page).")]
    NotLoaded,
    /// More or fewer than one chat by that name.
    #[error("{0}")]
    NotOne(String),
    /// The chat didn't open.
    #[error("{0}")]
    NotOpened(String),
    /// The text read back differs from what was typed: cleared, nothing kept.
    #[error(
        "WhatsApp Web changed the text while typing it (it reads back different), so it wasn't kept."
    )]
    Changed,
    /// Something else failed; its words.
    #[error("{0}")]
    Failed(String),
}

/// How a sent message shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaTick {
    /// Two ticks.
    Delivered,
    /// One tick.
    Sent,
    /// No tick yet.
    Unconfirmed,
}

/// The text as WhatsApp keeps it: `\r` gone, no-break spaces plain, each line's end trimmed.
fn same(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\u{a0}', " ")
        .split('\n')
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

impl BrowserService {
    async fn run(&self, tab: TabId, function: &'static str, arg: Value) -> Result<Value, WaError> {
        self.engine()
            .page(tab, PageQuery::Run { function, arg })
            .await
            .map_err(|e| WaError::Failed(e.to_string()))
    }

    /// Waits for the page to show its chat list or its login page.
    async fn wa_state(&self, tab: TabId, tries: u32) -> String {
        for _ in 0..tries {
            if let Ok(Value::String(state)) = self.run(tab, STATE, Value::Null).await
                && !state.is_empty()
            {
                return state;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        String::new()
    }

    /// WhatsApp Web's tab: one already on it, or a new one, brought to the front.
    async fn wa_tab(&self) -> Result<TabId, WaError> {
        let engine = self.engine();
        let tabs = engine.tabs().await.map_err(|_| WaError::NotAvailable)?;
        let tab = if let Some(t) = tabs.iter().find(|t| t.url.starts_with(WHATSAPP_WEB)) {
            t.id
        } else {
            let tab = engine
                .new_tab(None)
                .await
                .map_err(|e| WaError::Failed(e.to_string()))?;
            engine
                .goto(tab, format!("{WHATSAPP_WEB}/"), Duration::from_secs(45))
                .await
                .map_err(|e| WaError::Failed(e.to_string()))?;
            tab
        };
        drop(engine.select_tab(tab).await);
        self.own().current = Some(tab);
        Ok(tab)
    }

    /// Opens `chat` and types `text` there as a draft, read back to be exactly it: the chat's
    /// name as its header shows it ("Message yourself" for the owner's own). Nothing is sent.
    ///
    /// # Errors
    ///
    /// [`WaError`]: no browser, not linked (the browser is handed to the owner for `task`),
    /// not one chat by that name, the chat didn't open, the text changed.
    #[expect(clippy::too_many_lines, reason = "one flow, step by step")]
    pub async fn whatsapp_draft(
        &self,
        chat: &WaChat,
        text: &str,
        task: Option<i64>,
    ) -> Result<String, WaError> {
        self.restore().await;
        if let Some(busy) = self.own_busy() {
            return Err(WaError::Failed(busy));
        }
        let text = same(text);
        let engine = std::sync::Arc::clone(self.engine());
        let tab = self.wa_tab().await?;
        let mut state = self.wa_state(tab, 60).await;
        if state == "login" {
            drop(engine.navigate(tab, super::engine::Nav::Reload).await);
            state = self.wa_state(tab, 60).await;
        }
        if state != "linked" {
            if state == "login" {
                let args = BrowserArgs {
                    reason: Some(
                        "Link WhatsApp: scan the code from WhatsApp, Linked devices, or tap Log in \
                         with phone number"
                            .to_owned(),
                    ),
                    browser: Some(crate::services::chrome::OWN_BROWSER.to_owned()),
                    ..BrowserArgs::default()
                };
                let handed = self
                    .act("browser_handoff", &args, &Who::assistant(task))
                    .await;
                return Err(WaError::NotLinked(handed.text));
            }
            return Err(WaError::NotLoaded);
        }
        let failed = |e: super::engine::EngineError| WaError::Failed(e.to_string());
        if let Some(phone) = chat.phone.as_deref().filter(|p| !p.is_empty()) {
            engine
                .goto(
                    tab,
                    format!("{WHATSAPP_WEB}/send?phone={phone}"),
                    Duration::from_secs(45),
                )
                .await
                .map_err(failed)?;
            self.wa_state(tab, 60).await;
        } else {
            if self.run(tab, SEARCH, Value::Null).await? != json!(true) {
                return Err(WaError::NotLoaded);
            }
            engine
                .press(tab, "Control+A".to_owned())
                .await
                .map_err(failed)?;
            engine
                .press(tab, "Backspace".to_owned())
                .await
                .map_err(failed)?;
            engine
                .insert_text(tab, chat.name.clone())
                .await
                .map_err(failed)?;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let hits = self
                .run(tab, PICK, json!(chat.name))
                .await?
                .as_u64()
                .unwrap_or(0);
            if hits != 1 {
                drop(engine.press(tab, "Escape".to_owned()).await);
                let many = if hits > 0 {
                    format!("{hits} chats")
                } else {
                    "No chat".to_owned()
                };
                return Err(WaError::NotOne(format!(
                    "{many} named \"{}\" in WhatsApp Web, so it can't tell which one: ask the \
                     owner; never guess.",
                    chat.name
                )));
            }
        }
        let mut opened = false;
        for _ in 0..20 {
            if self.run(tab, COMPOSER, Value::Null).await? == json!(true) {
                opened = true;
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        if !opened {
            let why = if chat.phone.is_some() {
                " (that number may not be on WhatsApp)"
            } else {
                ""
            };
            return Err(WaError::NotOpened(format!(
                "WhatsApp Web didn't open the chat with {}{why}.",
                chat.name
            )));
        }
        let header = if chat.me {
            String::new()
        } else {
            self.run(tab, HEADER, Value::Null)
                .await
                .map_or_default(|v| v.as_str().unwrap_or_default().to_owned())
        };
        let to = if chat.me {
            "Message yourself".to_owned()
        } else if header.is_empty() {
            chat.name.clone()
        } else {
            header
        };
        engine
            .press(tab, "Control+A".to_owned())
            .await
            .map_err(failed)?;
        engine
            .press(tab, "Backspace".to_owned())
            .await
            .map_err(failed)?;
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                engine
                    .press(tab, "Shift+Enter".to_owned())
                    .await
                    .map_err(failed)?;
            }
            if !line.is_empty() {
                engine
                    .insert_text(tab, line.to_owned())
                    .await
                    .map_err(failed)?;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let typed = same(
            self.run(tab, TYPED, Value::Null)
                .await?
                .as_str()
                .unwrap_or_default(),
        );
        if typed != text {
            drop(self.run(tab, COMPOSER, Value::Null).await);
            drop(engine.press(tab, "Control+A".to_owned()).await);
            drop(engine.press(tab, "Backspace".to_owned()).await);
            return Err(WaError::Changed);
        }
        Ok(to)
    }

    /// Presses Send on the draft just typed, and waits up to 30 s for its tick.
    ///
    /// # Errors
    ///
    /// [`WaError`]: no browser, or no Send button (the message didn't go out).
    pub async fn whatsapp_send(&self, text: &str) -> Result<WaTick, WaError> {
        let tab = self.wa_tab().await?;
        if self.run(tab, SEND, Value::Null).await? != json!(true) {
            return Err(WaError::Failed(
                "WhatsApp Web shows no Send button: nothing was sent.".to_owned(),
            ));
        }
        let text = same(text);
        let snip: String = text
            .split('\n')
            .next()
            .unwrap_or_default()
            .chars()
            .take(60)
            .collect();
        for _ in 0..30 {
            let tick = self
                .run(tab, TICK, json!(snip))
                .await
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            if tick.contains("dblcheck") {
                return Ok(WaTick::Delivered);
            }
            if tick.contains("check") {
                return Ok(WaTick::Sent);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Ok(WaTick::Unconfirmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_is_compared_as_whatsapp_keeps_it() {
        assert_eq!(same(" hi \u{a0}\r\nthere  \n"), "hi\nthere");
    }
}
