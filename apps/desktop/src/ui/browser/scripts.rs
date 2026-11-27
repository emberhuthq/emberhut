//! The scripts the live view's page gets ahead of its own, in a web view: the stream's own
//! floating buttons hidden, the picture's shape kept while nobody drives, and while this device
//! drives, the clipboard bridged to this device's and ⌘ shortcuts sent to the browser as Ctrl
//! (it runs on Linux). [`for_session`] picks them for a session; the web view runs them in the
//! main frame, before the page's own scripts.
//!
//! The clipboard is bridged only while this device drives (watching, the page has no business
//! with it; the reference client bridges it in both modes), and only for messages that carry
//! the bridge's [`Token`]: the bridge's script holds it, and runs in the main frame alone, so a
//! frame inside the page can't post to the clipboard.

use emberhut_api::browser::BrowserSession;

/// The clipboard bridge: the page's `navigator.clipboard` answered with this device's
/// clipboard, text only. A read posts `{"clipboard":"read","id":n,"token":…}` and waits for
/// [`clipboard_answer`]; a write posts `{"clipboard":"write","text":…,"token":…}`. Permission
/// queries for the clipboard say granted, so the stream doesn't wait on a prompt that never
/// comes.
#[must_use]
pub(crate) fn clipboard(token: &Token) -> String {
    let token = serde_json::to_string(&token.0).unwrap_or_else(|_| "\"\"".to_owned());
    format!(
        r#"(() => {{
  const token = {token};
  const waiting = new Map(); let n = 0;
  window.__emberhutClipboard = (id, text) => {{ const r = waiting.get(id); waiting.delete(id); if (r) r(text); }};
  const post = (m) => window.ipc && window.ipc.postMessage(JSON.stringify(Object.assign(m, {{ token }})));
  const readText = () => new Promise((resolve) => {{ const id = ++n; waiting.set(id, resolve); post({{ clipboard: "read", id }}); }});
  const writeText = (t) => {{ post({{ clipboard: "write", text: String(t) }}); return Promise.resolve(); }};
  const clip = navigator.clipboard;
  if (!clip) return;
  clip.readText = readText;
  clip.writeText = writeText;
  clip.read = async () => {{ const t = await readText(); return t ? [new ClipboardItem({{ "text/plain": new Blob([t], {{ type: "text/plain" }}) }})] : []; }};
  clip.write = async (items) => {{ for (const it of items || []) if (it.types.includes("text/plain")) return writeText(await (await it.getType("text/plain")).text()); }};
  const query = navigator.permissions && navigator.permissions.query && navigator.permissions.query.bind(navigator.permissions);
  if (query) navigator.permissions.query = (d) => d && /^clipboard-/.test(d.name) ? Promise.resolve({{ state: "granted", onchange: null }}) : query(d);
}})();"#
    )
}

/// The secret the clipboard bridge's messages carry: 16 random bytes in hex, new for each web
/// view.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Token(String);

impl Token {
    /// A new token; `None` when the system has no randomness (then there is no bridge).
    #[must_use]
    pub(crate) fn new() -> Option<Self> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut bytes = [0_u8; 16];
        if let Err(error) = getrandom::getrandom(&mut bytes) {
            log::warn!("no randomness for the live view's clipboard: it isn't bridged ({error})");
            return None;
        }
        Some(Self(bytes.iter().fold(
            String::with_capacity(32),
            |mut hex, byte| {
                hex.push(char::from(HEX[usize::from(byte >> 4)]));
                hex.push(char::from(HEX[usize::from(byte & 0xf)]));
                hex
            },
        )))
    }
}

impl std::fmt::Debug for Token {
    /// Never the token itself.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(…)")
    }
}

/// Only the page: the stream's floating keyboard button and its settings drawer's handle (one
/// stray swipe from the edge opens it) stay hidden.
pub(crate) const STREAM_ONLY: &str = r#"(() => {
  const s = document.createElement("style");
  s.textContent = ".virtual-keyboard-button, .keyboard-toggle-button, .toggle-handle { display: none !important; }";
  const put = () => (document.head || document.documentElement).appendChild(s);
  if (document.documentElement) put(); else document.addEventListener("DOMContentLoaded", put);
})();"#;

/// While this device drives: ⌘ shortcuts reach the browser as Ctrl (⌘L, ⌘T, ⌘W, ⌘R, ⌘F, ⌘A,
/// ⌘C, ⌘Z…), sent ahead of the stream's own keyboard as whole press-and-release sequences:
/// macOS never sends the release of a key pressed with ⌘, so passing those through would
/// leave keys held on the server. ⌘V runs this device's own paste, whose text the stream takes
/// and pastes with Ctrl+V. ⌘Q, ⌘H, ⌘M, ⌘ with the backtick key, ⌘, (comma), ⌘Tab, ⌘Esc and ⌃⌘
/// ones are left to this app and the system (kept from the stream, not cancelled).
pub(crate) const COMMAND_AS_CONTROL: &str = r#"(() => {
  const keep = new Set(["q", "h", "m", "`", ",", "Tab", "Escape"]);
  const send = (t, type, init) => t.dispatchEvent(new KeyboardEvent(type, Object.assign({ bubbles: true, cancelable: true, composed: true }, init)));
  const ctrl = { key: "Control", code: "ControlLeft", keyCode: 17, which: 17 };
  const press = (t, e) => {
    const k = { key: e.key.length === 1 ? e.key.toLowerCase() : e.key, code: e.code, keyCode: e.keyCode, which: e.keyCode, shiftKey: e.shiftKey, altKey: e.altKey };
    send(t, "keydown", { ...ctrl, ctrlKey: true }); send(t, "keydown", { ...k, ctrlKey: true });
    send(t, "keyup", { ...k, ctrlKey: true }); send(t, "keyup", { ...ctrl, ctrlKey: false });
  };
  const handle = (e) => {
    if (!e.isTrusted) return;
    if (e.key === "Meta") { e.preventDefault(); e.stopImmediatePropagation(); return; }
    if (!e.metaKey) return;
    if (keep.has(e.key) || e.ctrlKey) { e.stopImmediatePropagation(); return; }
    if (e.key.toLowerCase() === "v") { e.stopImmediatePropagation(); if (e.type === "keyup") e.preventDefault(); return; }
    e.preventDefault(); e.stopImmediatePropagation();
    if (e.type !== "keydown" || e.repeat) return;
    press(e.target || document, e);
  };
  window.addEventListener("keydown", handle, true);
  window.addEventListener("keyup", handle, true);
})();"#;

/// Watching while nobody drives: the stream tells a viewer the screen's size only while
/// someone drives, and without it the picture is squeezed to the window's shape. This hands
/// the page `width` × `height` itself, unless the stream does first.
#[must_use]
pub(crate) fn screen_size(width: u32, height: u32) -> String {
    format!(
        r#"(() => {{
  let transport = null;
  Object.defineProperty(window, "selkiesTransport", {{ configurable: true, get: () => transport, set: (t) => {{
    transport = t;
    if (!t) return;
    let handler = null, told = false;
    Object.defineProperty(t, "onmessage", {{ configurable: true, get: () => handler, set: (fn) => {{
      handler = fn && ((e) => {{ if (typeof e.data === "string" && e.data.includes('"stream_resolution"')) told = true; return fn.call(t, e); }});
      if (fn) for (const ms of [1000, 2500, 5000, 9000]) setTimeout(() => {{
        if (!told && handler) fn.call(t, {{ data: JSON.stringify({{ type: "stream_resolution", width: {width}, height: {height}, displayId: "primary" }}) }});
      }}, ms);
    }} }});
  }} }});
}})();"#
    )
}

/// The page's answer to clipboard read `id`: `text`, as a JavaScript string literal.
#[must_use]
pub(crate) fn clipboard_answer(id: u64, text: &str) -> String {
    let literal = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned());
    format!("window.__emberhutClipboard && window.__emberhutClipboard({id}, {literal})")
}

/// What the page posts to the app.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "clipboard", rename_all = "lowercase")]
pub(crate) enum PageMessage {
    /// It wants this device's clipboard's text, for read `id`.
    Read {
        /// The read.
        id: u64,
    },
    /// It copied `text`.
    Write {
        /// What it copied.
        text: String,
    },
}

/// A message as the bridge posts it: what it says, and the bridge's token.
#[derive(serde::Deserialize)]
struct Posted {
    token: String,
    #[serde(flatten)]
    message: PageMessage,
}

impl PageMessage {
    /// A message the page posted, if it is one of these and carries `token` (the bridge's
    /// script posted it, not something else in the page).
    #[must_use]
    pub(crate) fn parse(body: &str, token: &Token) -> Option<Self> {
        let posted: Posted = serde_json::from_str(body)
            .inspect_err(|error| log::debug!("a message from the live view didn't read: {error}"))
            .ok()?;
        if posted.token != token.0 {
            log::info!("a message from the live view without the bridge's token was dropped");
            return None;
        }
        Some(posted.message)
    }
}

/// The scripts for `session`'s page, in the order they run: the clipboard bridge (with
/// `bridge`'s token) only while this device drives.
#[must_use]
pub(crate) fn for_session(session: &BrowserSession, bridge: Option<&Token>) -> Vec<String> {
    let mut scripts = vec![STREAM_ONLY.to_owned()];
    if session.is_control() {
        scripts.extend(bridge.map(clipboard));
        scripts.push(COMMAND_AS_CONTROL.to_owned());
    } else if let Some(screen) = session.screen {
        scripts.push(screen_size(screen.width, screen.height));
    }
    scripts
}

#[cfg(test)]
mod tests {
    use emberhut_api::browser::{BrowserScreen, StreamMode};

    use super::*;

    fn session(mode: StreamMode, screen: Option<BrowserScreen>) -> BrowserSession {
        BrowserSession {
            url: Some("https://example.org/".to_owned()),
            user: "viewer".to_owned(),
            password: "example".to_owned(),
            mode,
            screen,
        }
    }

    #[test]
    fn driving_turns_command_into_control_and_watching_keeps_the_shape() {
        let screen = Some(BrowserScreen {
            width: 2880,
            height: 1800,
        });
        let token = Token::new().unwrap();
        let control = for_session(&session(StreamMode::Control, screen), Some(&token));
        assert_eq!(
            control,
            [
                STREAM_ONLY.to_owned(),
                clipboard(&token),
                COMMAND_AS_CONTROL.to_owned()
            ]
        );
        let view = for_session(&session(StreamMode::View, screen), Some(&token));
        assert_eq!(view.len(), 2, "no clipboard while watching");
        assert!(view[1].contains("width: 2880, height: 1800"));
        assert_eq!(
            for_session(&session(StreamMode::View, None), Some(&token)),
            [STREAM_ONLY]
        );
        assert_eq!(
            for_session(&session(StreamMode::Control, None), None),
            [STREAM_ONLY, COMMAND_AS_CONTROL],
            "no token, no bridge"
        );
    }

    #[test]
    fn the_apps_own_command_keys_are_left_to_it() {
        // The keys left to this app with ⌘, as the page names them.
        for key in ["q", "h", "m", "`", ",", "Tab", "Escape"] {
            let literal = format!("\"{key}\"");
            assert!(COMMAND_AS_CONTROL.contains(&literal), "{key}");
        }
    }

    #[test]
    fn the_page_talks_clipboard_only_with_the_bridges_token() {
        let token = Token::new().unwrap();
        let other = Token::new().unwrap();
        assert_ne!(token, other);
        let script = clipboard(&token);
        assert!(script.contains(&format!("const token = \"{}\";", token.0)));
        let read = format!(r#"{{"clipboard":"read","id":3,"token":"{}"}}"#, token.0);
        assert_eq!(
            PageMessage::parse(&read, &token),
            Some(PageMessage::Read { id: 3 })
        );
        assert_eq!(PageMessage::parse(&read, &other), None, "another token");
        let write = format!(
            r#"{{"clipboard":"write","text":"hi","token":"{}"}}"#,
            token.0
        );
        assert_eq!(
            PageMessage::parse(&write, &token),
            Some(PageMessage::Write {
                text: "hi".to_owned()
            })
        );
        assert_eq!(
            PageMessage::parse(r#"{"clipboard":"write","text":"hi"}"#, &token),
            None,
            "no token"
        );
        assert_eq!(PageMessage::parse(r#"{"other":1}"#, &token), None);
        assert_eq!(format!("{token:?}"), "Token(…)");
        assert_eq!(
            clipboard_answer(3, "a \"b\"\n"),
            r#"window.__emberhutClipboard && window.__emberhutClipboard(3, "a \"b\"\n")"#
        );
    }
}
