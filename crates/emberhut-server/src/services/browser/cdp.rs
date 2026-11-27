//! The real browser engine: Chrome's `DevTools` protocol (CDP), spoken over a [`CdpLink`].
//!
//! - [`DirectLink`]: a Chrome the server reaches itself, by its `DevTools` address
//!   (`EMBERHUT_BROWSER_CDP`, `http://127.0.0.1:9223`): the browser's WebSocket, one flat session per
//!   tab.
//! - The extension's link ([`crate::services::chrome`]): the owner's own Chrome, where the
//!   extension relays each command to `chrome.debugger` in the assistant's own tabs only.
//!
//! The page is read as its accessibility tree (`Accessibility.getFullAXTree`); each element shown
//! gets a ref (`e12`) kept per tab, which the steps use to find it again (`DOM.resolveNode`).
//! Clicks and keys are real input events; a file goes into a field as a `File` the page builds
//! from the bytes (so nothing is copied to the browser's machine); a download is fetched in the
//! page with its own login and read back in pieces.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use base64::Engine as _;
use emberhut_core::browser::TOKEN_SOURCE;
use futures_util::future::BoxFuture;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot};
use tokio_tungstenite::tungstenite::Message;

use super::engine::{
    BrowserEngine, ElementQuery, EngineError, EngineFuture, EngineResult, FileAsk, FilePayload,
    FileVia, Nav, PageQuery, Tab, TabId,
};

/// How long one command may take.
const COMMAND_WAIT: Duration = Duration::from_secs(30);

/// A tab as a link knows it: its key (a target id, or the extension's tab id) and where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// The link's own key for it.
    pub key: String,
    /// Its address.
    pub url: String,
    /// Its title.
    pub title: String,
}

/// An event from a tab.
#[derive(Clone, Debug, PartialEq)]
pub struct CdpEvent {
    /// The tab's key.
    pub key: String,
    /// The event (`Page.loadEventFired`).
    pub method: String,
    /// Its parameters.
    pub params: Value,
}

/// How a CDP engine reaches its browser's tabs.
pub trait CdpLink: fmt::Debug + Send + Sync {
    /// Connected now.
    fn connected(&self) -> bool;

    /// The open tabs, in order; the one on screen first when the browser says.
    fn targets(&self) -> BoxFuture<'_, EngineResult<Vec<Target>>>;

    /// A new tab.
    fn create(&self, url: Option<String>) -> BoxFuture<'_, EngineResult<Target>>;

    /// Closes a tab.
    fn close(&self, key: String) -> BoxFuture<'_, EngineResult<()>>;

    /// Brings a tab to the front.
    fn activate(&self, key: String) -> BoxFuture<'_, EngineResult<()>>;

    /// A command to a tab's page.
    fn send(
        &self,
        key: String,
        method: &'static str,
        params: Value,
    ) -> BoxFuture<'_, EngineResult<Value>>;

    /// A browser-wide command about a tab (its window); none where the link has none.
    fn window(
        &self,
        key: String,
        method: &'static str,
        params: Value,
    ) -> BoxFuture<'_, EngineResult<Value>> {
        let _ = (key, method, params);
        Box::pin(async { Err(EngineError::Failed("not here".to_owned())) })
    }

    /// The tabs' events.
    fn events(&self) -> broadcast::Receiver<CdpEvent>;
}

/// Each tab's refs: ref → the element's backend node id, and back.
#[derive(Debug, Default)]
struct Refs {
    by_ref: HashMap<String, i64>,
    by_node: HashMap<i64, String>,
}

#[derive(Debug, Default)]
struct Book {
    by_key: HashMap<String, TabId>,
    by_id: HashMap<TabId, String>,
    next_tab: TabId,
    next_ref: u64,
    refs: HashMap<TabId, Refs>,
    ready: HashSet<TabId>,
    file_ask: Option<(TabId, i64, bool)>,
    dialogs: Vec<String>,
    listening: bool,
}

/// The CDP engine over a link.
pub struct CdpEngine {
    link: Arc<dyn CdpLink>,
    /// Hold the page's own file picker back for the owner's app (the server's own browser).
    catch_files: bool,
    book: Mutex<Book>,
    owner: AtomicBool,
    me: Weak<Self>,
}

impl fmt::Debug for CdpEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CdpEngine")
            .field("link", &self.link)
            .finish_non_exhaustive()
    }
}

/// Pages can't tell an agent is attached: their console goes nowhere (some sites watch it to
/// close their own tab while a browser is driven).
const QUIET: &str = r#"(() => { const quiet = () => {}; for (const k of ["log", "info", "warn", "error", "debug", "dir", "dirxml", "table", "trace", "clear", "group", "groupCollapsed", "groupEnd", "count", "countReset", "time", "timeLog", "timeEnd", "timeStamp", "assert", "profile", "profileEnd"]) try { Object.defineProperty(console, k, { value: quiet, writable: true, configurable: true }); } catch {} })()"#;

/// The keys on the page; `mark` tags the elements showing them for a picture's mask.
const FIND: &str = r#"({ src, mark }) => {
  const one = new RegExp(src), all = new RegExp(src, "g"), values = new Set(), hits = new Set();
  const about = /token|secret|api.?key|access.?key|private.?key|signing|credential|bearer|webhook|client.?id|password/i;
  const long = (v) => /^[A-Za-z0-9+/=_.-]{20,}$/.test(v) && /\d/.test(v) && /[A-Za-z]/.test(v);
  for (const f of document.querySelectorAll("input, textarea")) {
    const v = (f.value ?? "").trim();
    if (!v) continue;
    const said = [f.type, f.name, f.id, f.autocomplete, f.getAttribute("aria-label"), f.placeholder, ...[...(f.labels ?? [])].map((l) => l.innerText)].join(" ");
    const tokens = v.match(all) ?? [];
    if (tokens.length) tokens.forEach((t) => values.add(t));
    else if (long(v) && (f.type === "password" || about.test(said))) values.add(v);
    else continue;
    hits.add(f);
  }
  const walk = document.createTreeWalker(document.body ?? document.documentElement, NodeFilter.SHOW_TEXT);
  for (let n = walk.nextNode(); n; n = walk.nextNode()) {
    const t = n.nodeValue ?? "";
    if (!one.test(t)) continue;
    for (const m of t.match(all) ?? []) values.add(m);
    if (n.parentElement) hits.add(n.parentElement);
  }
  if (mark) for (const el of hits) {
    const r = el.getBoundingClientRect(), box = document.createElement("div");
    box.setAttribute("data-emberhut-mask", "");
    box.style.cssText = `position:fixed;left:${r.left}px;top:${r.top}px;width:${r.width}px;height:${r.height}px;background:#000;z-index:2147483647;pointer-events:none`;
    document.documentElement.appendChild(box);
  }
  return [...values];
}"#;

const UNMASK: &str =
    r#"() => document.querySelectorAll("[data-emberhut-mask]").forEach((e) => e.remove())"#;

/// The text written in the boxes around an element, nearest first, quoted mail and signatures
/// left out.
const WRITTEN: &str = r#"function () {
  const el = this, out = [];
  const read = (f) => {
    if (f.tagName === "TEXTAREA" || f.tagName === "INPUT") return String(f.value ?? "");
    const c = f.cloneNode(true);
    c.querySelectorAll(".gmail_quote, .gmail_signature, .gmail_extra, blockquote, [data-smartmail], #divRplyFwdMsg, #appendonsend ~ *").forEach((x) => x.remove());
    const box = document.createElement("div"); box.style.cssText = "position:fixed;left:-99999px;top:0;white-space:pre-wrap"; box.appendChild(c);
    document.body.appendChild(box); const t = box.innerText ?? ""; box.remove();
    return t;
  };
  const boxes = "textarea, [contenteditable='true'], [contenteditable=''], [role='textbox']";
  if (el.matches?.(boxes)) out.push(read(el));
  for (let n = el.parentElement, i = 0; n && i < 14 && !out.some((x) => x.trim()); n = n.parentElement, i++)
    for (const f of n.querySelectorAll(boxes)) if (!(f.closest("[contenteditable]") && f.closest("[contenteditable]") !== f)) out.push(read(f));
  return out.map((x) => x.replace(/\u200b/g, "").trim()).filter(Boolean).slice(0, 6);
}"#;

/// What has the focus, through open shadow roots and same-origin frames: `{ kind, form, label }`
/// ([`PageQuery::FocusedField`]). A search field is known by its type, its role, a search
/// landmark around it, or (a one-line field) its name, label or placeholder.
const FOCUSED_FIELD: &str = r#"() => {
  const say = (kind, form, label) => ({ kind, form: form || "", label: String(label || "").trim().slice(0, 80) });
  let el = document.activeElement;
  for (;;) {
    if (el && el.shadowRoot && el.shadowRoot.activeElement) { el = el.shadowRoot.activeElement; continue; }
    if (el && (el.tagName === "IFRAME" || el.tagName === "FRAME")) {
      let doc = null;
      try { doc = el.contentDocument; } catch (e) { doc = null; }
      if (!doc) return say("unknown");
      el = doc.activeElement;
      continue;
    }
    break;
  }
  if (!el || el === el.ownerDocument.body || el === el.ownerDocument.documentElement) return say("none");
  const type = (el.getAttribute("type") || "").toLowerCase();
  const role = (el.getAttribute("role") || "").toLowerCase();
  const f = el.form || el.closest("form");
  const b = f && f.querySelector("button[type=submit], input[type=submit], button:not([type])");
  const form = f ? String((b && (b.innerText || b.value)) || "submit").trim().slice(0, 80) : "";
  const label = el.isContentEditable ? el.getAttribute("aria-label") : (el.innerText || el.value || el.getAttribute("aria-label") || el.title);
  const line = el.tagName === "INPUT" && !["button", "submit", "reset", "checkbox", "radio", "file", "image", "range", "color"].includes(type);
  const named = [el.getAttribute("aria-label"), el.getAttribute("placeholder"), el.getAttribute("name")].join(" ");
  if (type === "search" || role === "searchbox" || el.closest("[role=search], search") || (line && /\bsearch\b/i.test(named))) return say("search", form, label);
  if (el.tagName === "TEXTAREA" || el.isContentEditable || role === "textbox") return say("composer", form, label);
  if (line) return say(f ? "field" : "composer", form, label);
  return say("control", form, label);
}"#;

const CODE_EDITOR: &str = r#"function (text) {
  const root = this.closest?.(".CodeMirror, .cm-editor, .monaco-editor, .ace_editor");
  const monaco = root?.classList.contains("monaco-editor") && globalThis.monaco?.editor?.getEditors?.()
    .find((e) => root.contains(e.getDomNode?.()) || e.getDomNode?.()?.contains(root));
  if (monaco && monaco.getModel()) {
    monaco.focus(); monaco.pushUndoStop();
    monaco.executeEdits("emberhut", [{ range: monaco.getModel().getFullModelRange(), text, forceMoveMarkers: true }]);
    monaco.pushUndoStop();
    return "set";
  }
  const input = root?.querySelector("textarea, [contenteditable=true], [contenteditable='']");
  input?.focus();
  return input ? "keys" : null;
}"#;

const KEY_AT: &str = r#"function (src) {
  const el = this;
  const isField = (e) => e && /^(INPUT|TEXTAREA)$/.test(e.tagName);
  const secretField = (e) => e.type === "password" || /current-password|new-password|one-time-code/.test(e.autocomplete ?? "");
  if (isField(el)) {
    if (secretField(el)) return { why: "password" };
    return { value: String(el.value ?? "").trim() };
  }
  const text = String(el.innerText ?? el.textContent ?? "").trim();
  if (text && !/\s/.test(text)) return { value: text };
  const found = new Set(text.match(new RegExp(src, "g")) ?? []);
  for (const f of el.querySelectorAll?.("input, textarea") ?? []) if (!secretField(f) && String(f.value ?? "").trim()) found.add(String(f.value).trim());
  if (found.size === 1) return { value: [...found][0] };
  return { why: found.size ? "several" : /[•●]{3}/.test(text) ? "hidden" : "none" };
}"#;

const FILE_KIND: &str = r#"function () {
  const e = this;
  const input = e.tagName === "INPUT" ? e : e.tagName === "LABEL" ? e.control : null;
  const labelOf = (i) => (i.getAttribute("aria-label") || i.labels?.[0]?.innerText || i.title || "").trim().slice(0, 80);
  if (input) return [input.type !== "file" ? "other" : input.disabled ? "disabled" : "field", labelOf(input)];
  return [e.querySelectorAll("input[type=file]").length === 1 ? "inner" : "button", ""];
}"#;

/// Sets a `File` made from base64 bytes on a file field (the element, its label's field, or the
/// one field inside it), as a page would get it: then `input` and `change`.
const SET_FILE: &str = r#"function (name, type, data) {
  const e = this;
  const input = e.tagName === "INPUT" ? e : e.tagName === "LABEL" ? e.control : e.querySelector("input[type=file]");
  if (!input) return "";
  const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
  const t = new DataTransfer(); t.items.add(new File([bytes], name, { type }));
  input.files = t.files;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
  return input.files?.[0]?.name ?? "";
}"#;

const FILL: &str = r#"function (text) {
  const e = this;
  e.scrollIntoView?.({ block: "center" }); e.focus?.();
  if (e.isContentEditable) {
    document.getSelection()?.selectAllChildren(e);
    document.execCommand("insertText", false, text);
    return true;
  }
  const proto = e.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype : e.tagName === "SELECT" ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  const set = Object.getOwnPropertyDescriptor(proto, "value")?.set;
  if (set) set.call(e, text); else e.value = text;
  e.dispatchEvent(new Event("input", { bubbles: true }));
  e.dispatchEvent(new Event("change", { bubbles: true }));
  return true;
}"#;

const FETCH_IN_PAGE: &str = r#"async ({ href, max }) => {
  try {
    const r = await fetch(href, { credentials: "include" });
    if (!r.ok) return { error: `the site answered ${r.status} ${r.statusText}`.trim() };
    if (Number(r.headers.get("content-length") || 0) > max) return { error: "big", size: Number(r.headers.get("content-length")) };
    const blob = await r.blob(), key = `emberhut-download-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    if (blob.size > max) return { error: "big", size: blob.size };
    globalThis[Symbol.for(key)] = blob;
    return { key, size: blob.size, type: r.headers.get("content-type") || blob.type || "", cd: r.headers.get("content-disposition") || "", url: r.url };
  } catch (e) { return { error: String(e?.message ?? e) }; }
}"#;

const PIECE_IN_PAGE: &str = r#"async ({ k, at, n }) => {
  const blob = globalThis[Symbol.for(k)];
  if (!blob) throw new Error("the page changed while downloading");
  const r = await new Promise((ok, no) => { const f = new FileReader(); f.onload = () => ok(String(f.result)); f.onerror = () => no(f.error); f.readAsDataURL(blob.slice(at, at + n)); });
  return r.slice(r.indexOf(",") + 1);
}"#;

impl CdpEngine {
    /// An engine over `link`; `catch_files`: the page's own file picker is held back for the
    /// owner's app (the server's own browser).
    #[must_use]
    pub fn new(link: Arc<dyn CdpLink>, catch_files: bool) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            link,
            catch_files,
            book: Mutex::new(Book {
                next_tab: 1,
                next_ref: 1,
                ..Book::default()
            }),
            owner: AtomicBool::new(false),
            me: me.clone(),
        })
    }

    fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn id_of(&self, key: &str) -> TabId {
        let mut book = self.book();
        if let Some(id) = book.by_key.get(key) {
            return *id;
        }
        let id = book.next_tab;
        book.next_tab += 1;
        book.by_key.insert(key.to_owned(), id);
        book.by_id.insert(id, key.to_owned());
        id
    }

    fn key_of(&self, tab: TabId) -> EngineResult<String> {
        self.book()
            .by_id
            .get(&tab)
            .cloned()
            .ok_or_else(|| EngineError::TabClosed("The tab has been closed.".to_owned()))
    }

    /// Listens to the link's events, once: dialogs, the page's file picker, closed tabs.
    fn listen(&self) {
        {
            let mut book = self.book();
            if book.listening {
                return;
            }
            book.listening = true;
        }
        let mut events = self.link.events();
        let me = self.me.clone();
        tokio::spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let Some(engine) = me.upgrade() else { break };
                engine.on_event(event).await;
            }
            if let Some(engine) = me.upgrade() {
                engine.book().listening = false;
            }
        });
    }

    async fn on_event(&self, event: CdpEvent) {
        match event.method.as_str() {
            "Page.javascriptDialogOpening" => {
                if self.owner.load(Ordering::SeqCst) {
                    self.book().dialogs.push(event.key);
                } else {
                    drop(
                        self.link
                            .send(
                                event.key,
                                "Page.handleJavaScriptDialog",
                                json!({ "accept": false }),
                            )
                            .await,
                    );
                }
            }
            "Page.fileChooserOpened" if self.catch_files => {
                let node = event.params.get("backendNodeId").and_then(Value::as_i64);
                let multiple =
                    event.params.get("mode").and_then(Value::as_str) == Some("selectMultiple");
                if let Some(node) = node {
                    let tab = self.id_of(&event.key);
                    self.book().file_ask = Some((tab, node, multiple));
                }
            }
            "Inspector.detached" | "Target.detachedFromTarget" => {
                let mut book = self.book();
                if let Some(id) = book.by_key.get(&event.key).copied() {
                    book.ready.remove(&id);
                }
            }
            _ => {}
        }
    }

    /// The tab's domains on, once.
    async fn prepare(&self, tab: TabId) -> EngineResult<String> {
        let key = self.key_of(tab)?;
        self.listen();
        if self.book().ready.contains(&tab) {
            return Ok(key);
        }
        self.link
            .send(key.clone(), "Page.enable", json!({}))
            .await?;
        drop(self.link.send(key.clone(), "DOM.enable", json!({})).await);
        drop(
            self.link
                .send(
                    key.clone(),
                    "Page.addScriptToEvaluateOnNewDocument",
                    json!({ "source": QUIET }),
                )
                .await,
        );
        if self.catch_files {
            drop(
                self.link
                    .send(
                        key.clone(),
                        "Page.setInterceptFileChooserDialog",
                        json!({ "enabled": true }),
                    )
                    .await,
            );
        }
        self.book().ready.insert(tab);
        Ok(key)
    }

    async fn send(&self, tab: TabId, method: &'static str, params: Value) -> EngineResult<Value> {
        let key = self.prepare(tab).await?;
        tokio::time::timeout(COMMAND_WAIT, self.link.send(key, method, params))
            .await
            .map_err(|_| EngineError::Timeout(format!("{method} took longer than 30 s")))?
    }

    /// Runs `function` on the page with `arg`: its value.
    async fn evaluate(&self, tab: TabId, function: &str, arg: &Value) -> EngineResult<Value> {
        let expression = format!("({function})({arg})");
        let got = self
            .send(
                tab,
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            )
            .await?;
        result_value(&got)
    }

    /// The element by its ref, as a remote object id.
    async fn object(&self, tab: TabId, at: &str) -> EngineResult<String> {
        let at = at.trim_start_matches("ref=");
        let node = self
            .book()
            .refs
            .get(&tab)
            .and_then(|r| r.by_ref.get(at).copied())
            .ok_or_else(|| EngineError::NoRef(at.to_owned()))?;
        self.object_of(tab, node)
            .await
            .map_err(|_| EngineError::NoRef(at.to_owned()))
    }

    async fn object_of(&self, tab: TabId, node: i64) -> EngineResult<String> {
        let got = self
            .send(tab, "DOM.resolveNode", json!({ "backendNodeId": node }))
            .await?;
        got.pointer("/object/objectId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| EngineError::Failed("no such element".to_owned()))
    }

    /// Calls `function` (a `function () { this … }`) on an element with `args`.
    async fn call_on(
        &self,
        tab: TabId,
        object: String,
        function: &str,
        args: Vec<Value>,
    ) -> EngineResult<Value> {
        let arguments: Vec<Value> = args.into_iter().map(|v| json!({ "value": v })).collect();
        let got = self
            .send(
                tab,
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object,
                    "functionDeclaration": function,
                    "arguments": arguments,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await?;
        result_value(&got)
    }

    async fn on_element(
        &self,
        tab: TabId,
        at: &str,
        function: &str,
        args: Vec<Value>,
    ) -> EngineResult<Value> {
        let object = self.object(tab, at).await?;
        self.call_on(tab, object, function, args).await
    }

    /// Waits for an event on `tab` (one of `methods`), at most `wait`.
    async fn wait_event(
        rx: &mut broadcast::Receiver<CdpEvent>,
        key: &str,
        methods: &[&str],
        wait: Duration,
    ) -> Option<CdpEvent> {
        let until = tokio::time::Instant::now() + wait;
        loop {
            match tokio::time::timeout_at(until, rx.recv()).await {
                Ok(Ok(event)) if event.key == key && methods.contains(&event.method.as_str()) => {
                    return Some(event);
                }
                Ok(Ok(_) | Err(broadcast::error::RecvError::Lagged(_))) => {}
                Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => return None,
            }
        }
    }

    /// The accessibility tree, written as lines with refs.
    async fn tree(&self, tab: TabId, scope: Option<&str>) -> EngineResult<Option<String>> {
        let got = self
            .send(tab, "Accessibility.getFullAXTree", json!({}))
            .await?;
        let nodes = got
            .get("nodes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let by_id: HashMap<String, &Value> = nodes
            .iter()
            .filter_map(|n| {
                n.get("nodeId")
                    .and_then(Value::as_str)
                    .map(|id| (id.to_owned(), n))
            })
            .collect();
        let root = nodes
            .iter()
            .find(|n| n.get("parentId").is_none())
            .or(nodes.first());
        let Some(root) = root else {
            return Ok(Some(String::new()));
        };
        let start: &Value = match scope {
            None => root,
            Some("main") => match nodes.iter().find(|n| role(n) == "main") {
                Some(node) => node,
                None => return Ok(None),
            },
            Some(at) => {
                let at = at.trim_start_matches("ref=");
                let backend = self
                    .book()
                    .refs
                    .get(&tab)
                    .and_then(|r| r.by_ref.get(at).copied());
                match backend.and_then(|b| {
                    nodes
                        .iter()
                        .find(|n| n.get("backendDOMNodeId").and_then(Value::as_i64) == Some(b))
                }) {
                    Some(node) => node,
                    None => return Ok(None),
                }
            }
        };
        let mut lines = Vec::new();
        let mut book = self.book();
        let Book { refs, next_ref, .. } = &mut *book;
        let refs = refs.entry(tab).or_default();
        let top = role(start) == "RootWebArea";
        render(start, &by_id, 0, top, refs, next_ref, &mut lines);
        Ok(Some(lines.join("\n")))
    }

    async fn screenshot_now(&self, tab: TabId, quality: u8) -> EngineResult<Option<Vec<u8>>> {
        let marked = self
            .evaluate(tab, FIND, &json!({ "src": TOKEN_SOURCE, "mark": true }))
            .await;
        if marked.is_err() {
            return Ok(None);
        }
        let shot = self
            .send(
                tab,
                "Page.captureScreenshot",
                json!({ "format": "jpeg", "quality": quality }),
            )
            .await;
        drop(self.evaluate(tab, UNMASK, &Value::Null).await);
        let data = shot?
            .get("data")
            .and_then(Value::as_str)
            .map_or_default(str::to_owned);
        Ok(base64::engine::general_purpose::STANDARD.decode(data).ok())
    }

    async fn set_on(&self, tab: TabId, object: String, file: &FilePayload) -> EngineResult<String> {
        let data = base64::engine::general_purpose::STANDARD.encode(&file.bytes);
        let got = self
            .call_on(
                tab,
                object,
                SET_FILE,
                vec![json!(file.name), json!(file.mime), json!(data)],
            )
            .await?;
        Ok(got.as_str().unwrap_or_default().to_owned())
    }

    async fn click_at(&self, tab: TabId, at: &str) -> EngineResult<()> {
        let at = at.trim_start_matches("ref=");
        let node = self
            .book()
            .refs
            .get(&tab)
            .and_then(|r| r.by_ref.get(at).copied())
            .ok_or_else(|| EngineError::NoRef(at.to_owned()))?;
        drop(
            self.send(
                tab,
                "DOM.scrollIntoViewIfNeeded",
                json!({ "backendNodeId": node }),
            )
            .await,
        );
        let quads = self
            .send(tab, "DOM.getContentQuads", json!({ "backendNodeId": node }))
            .await;
        let quad = quads
            .ok()
            .and_then(|q| {
                q.get("quads")
                    .and_then(Value::as_array)
                    .and_then(|q| q.first().cloned())
            })
            .and_then(|q| {
                q.as_array()
                    .map(|p| p.iter().filter_map(Value::as_f64).collect::<Vec<_>>())
            })
            .filter(|p| p.len() == 8);
        let Some(p) = quad else {
            let object = self
                .object_of(tab, node)
                .await
                .map_err(|_| EngineError::NoRef(at.to_owned()))?;
            self.call_on(tab, object, "function () { this.click(); }", Vec::new())
                .await?;
            return Ok(());
        };
        let x = (p[0] + p[2] + p[4] + p[6]) / 4.0;
        let y = (p[1] + p[3] + p[5] + p[7]) / 4.0;
        for (kind, buttons) in [("mouseMoved", 0), ("mousePressed", 1), ("mouseReleased", 0)] {
            let mut event =
                json!({ "type": kind, "x": x, "y": y, "button": "left", "buttons": buttons });
            if kind != "mouseMoved" {
                event["clickCount"] = json!(1);
            }
            self.send(tab, "Input.dispatchMouseEvent", event).await?;
        }
        Ok(())
    }
}

/// A node's role.
fn role(node: &Value) -> &str {
    node.pointer("/role/value")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn name_of(node: &Value) -> &str {
    node.pointer("/name/value")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// Roles that only hold others: their children go up a level.
fn transparent(node: &Value) -> bool {
    let r = role(node);
    node.get("ignored").and_then(Value::as_bool) == Some(true)
        || matches!(r, "none" | "presentation" | "InlineTextBox" | "LineBreak")
        || (r == "generic" && name_of(node).is_empty())
}

fn render(
    node: &Value,
    by_id: &HashMap<String, &Value>,
    depth: usize,
    top: bool,
    refs: &mut Refs,
    next_ref: &mut u64,
    out: &mut Vec<String>,
) {
    let children: Vec<&Value> = node
        .get("childIds")
        .and_then(Value::as_array)
        .map_or_default(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().and_then(|id| by_id.get(id).copied()))
                .collect()
        });
    if top || transparent(node) {
        for child in children {
            render(child, by_id, depth, false, refs, next_ref, out);
        }
        return;
    }
    let pad = "  ".repeat(depth);
    let r = role(node);
    let name = name_of(node).trim();
    if r == "StaticText" {
        if !name.is_empty() {
            out.push(format!("{pad}- text: {}", quote_text(name)));
        }
        return;
    }
    let mut line = format!("{pad}- {r}");
    if !name.is_empty() {
        add(
            &mut line,
            format_args!(" \"{}\"", name.replace('"', "\\\"")),
        );
    }
    for property in node
        .get("properties")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = property
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let value = property.pointer("/value/value");
        match (key, value) {
            ("level", Some(v)) => add(&mut line, format_args!(" [level={v}]")),
            ("checked", Some(v)) if v != &json!("false") && v != &json!(false) => {
                line.push_str(" [checked]");
            }
            ("disabled", Some(v)) if v == &json!(true) => line.push_str(" [disabled]"),
            ("expanded", Some(v)) => add(&mut line, format_args!(" [expanded={v}]")),
            ("selected", Some(v)) if v == &json!(true) => line.push_str(" [selected]"),
            ("hidden", Some(v)) if v == &json!(true) => line.push_str(" [aria-hidden]"),
            _ => {}
        }
    }
    if let Some(backend) = node.get("backendDOMNodeId").and_then(Value::as_i64) {
        let at = refs.by_node.get(&backend).cloned().unwrap_or_else(|| {
            let at = format!("e{next_ref}");
            *next_ref += 1;
            refs.by_node.insert(backend, at.clone());
            refs.by_ref.insert(at.clone(), backend);
            at
        });
        add(&mut line, format_args!(" [ref={at}]"));
    }
    let value = node
        .pointer("/value/value")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .filter(|v| !v.is_empty() && v != name);
    let has_children = !children.is_empty()
        && children
            .iter()
            .any(|c| !(role(c) == "StaticText" && name_of(c).trim() == name));
    match value {
        Some(value) => add(&mut line, format_args!(": {}", quote_text(&value))),
        None if has_children => line.push(':'),
        None => {}
    }
    out.push(line);
    if has_children {
        for child in children {
            if role(child) == "StaticText" && name_of(child).trim() == name {
                continue;
            }
            render(child, by_id, depth + 1, false, refs, next_ref, out);
        }
    }
}

fn quote_text(text: &str) -> String {
    let one = text.replace('\n', " ");
    if one.contains(':') || one.starts_with(['-', '"', '[']) {
        format!("\"{}\"", one.replace('"', "\\\""))
    } else {
        one
    }
}

/// A CDP evaluate's value, or the page's exception as a failure.
fn result_value(got: &Value) -> EngineResult<Value> {
    if let Some(error) = got.get("exceptionDetails") {
        let text = error
            .pointer("/exception/description")
            .and_then(Value::as_str)
            .or_else(|| error.get("text").and_then(Value::as_str))
            .unwrap_or("the page's script failed");
        return Err(EngineError::Failed(
            text.split('\n').next().unwrap_or_default().to_owned(),
        ));
    }
    Ok(got.pointer("/result/value").cloned().unwrap_or(Value::Null))
}

/// A key or chord as CDP key events: modifiers, the key, its code, its key code, its text.
fn key_events(chord: &str) -> (i64, String, String, i64, Option<String>) {
    let parts: Vec<&str> = chord.split('+').collect();
    let (mods, key) = parts.split_at(parts.len().saturating_sub(1));
    let mut modifiers = 0;
    for m in mods {
        modifiers |= match m.to_lowercase().as_str() {
            "alt" | "option" => 1,
            "control" | "ctrl" => 2,
            "meta" | "cmd" | "command" | "controlormeta" => 4,
            "shift" => 8,
            _ => 0,
        };
    }
    let key = key.first().copied().unwrap_or_default();
    let named: Option<(&str, &str, i64, Option<&str>)> = match key {
        "Enter" | "Return" => Some(("Enter", "Enter", 13, Some("\r"))),
        "Tab" => Some(("Tab", "Tab", 9, None)),
        "Escape" | "Esc" => Some(("Escape", "Escape", 27, None)),
        "Backspace" => Some(("Backspace", "Backspace", 8, None)),
        "Delete" => Some(("Delete", "Delete", 46, None)),
        "ArrowUp" | "Up" => Some(("ArrowUp", "ArrowUp", 38, None)),
        "ArrowDown" | "Down" => Some(("ArrowDown", "ArrowDown", 40, None)),
        "ArrowLeft" | "Left" => Some(("ArrowLeft", "ArrowLeft", 37, None)),
        "ArrowRight" | "Right" => Some(("ArrowRight", "ArrowRight", 39, None)),
        "Home" => Some(("Home", "Home", 36, None)),
        "End" => Some(("End", "End", 35, None)),
        "PageUp" => Some(("PageUp", "PageUp", 33, None)),
        "PageDown" => Some(("PageDown", "PageDown", 34, None)),
        "Space" | " " => Some((" ", "Space", 32, Some(" "))),
        "Backslash" | "\\" => Some(("\\", "Backslash", 220, Some("\\"))),
        _ => None,
    };
    if let Some((k, code, vk, text)) = named {
        let text = text.filter(|_| modifiers & 0b0110 == 0).map(str::to_owned);
        return (modifiers, k.to_owned(), code.to_owned(), vk, text);
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => {
            let upper = c.to_ascii_uppercase();
            let code = if upper.is_ascii_alphabetic() {
                format!("Key{upper}")
            } else if upper.is_ascii_digit() {
                format!("Digit{upper}")
            } else {
                String::new()
            };
            let text = (modifiers & 0b0110 == 0).then(|| c.to_string());
            (
                modifiers,
                c.to_string(),
                code,
                i64::from(u32::from(upper)),
                text,
            )
        }
        _ => (modifiers, key.to_owned(), key.to_owned(), 0, None),
    }
}

impl BrowserEngine for CdpEngine {
    fn connected(&self) -> bool {
        self.link.connected()
    }

    fn tabs(&self) -> EngineFuture<'_, Vec<Tab>> {
        Box::pin(async move {
            let targets = self.link.targets().await?;
            let keys: HashSet<String> = targets.iter().map(|t| t.key.clone()).collect();
            {
                let mut book = self.book();
                let gone: Vec<(String, TabId)> = book
                    .by_key
                    .iter()
                    .filter(|(k, _)| !keys.contains(*k))
                    .map(|(k, id)| (k.clone(), *id))
                    .collect();
                for (key, id) in gone {
                    book.by_key.remove(&key);
                    book.by_id.remove(&id);
                    book.refs.remove(&id);
                    book.ready.remove(&id);
                }
            }
            Ok(targets
                .into_iter()
                .enumerate()
                .map(|(i, t)| Tab {
                    id: self.id_of(&t.key),
                    url: t.url,
                    title: t.title,
                    front: i == 0,
                })
                .collect())
        })
    }

    fn new_tab(&self, url: Option<String>) -> EngineFuture<'_, TabId> {
        Box::pin(async move {
            let target = self.link.create(url).await?;
            Ok(self.id_of(&target.key))
        })
    }

    fn close_tab(&self, tab: TabId) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            let key = self.key_of(tab)?;
            self.link.close(key.clone()).await?;
            let mut book = self.book();
            book.by_key.remove(&key);
            book.by_id.remove(&tab);
            book.refs.remove(&tab);
            book.ready.remove(&tab);
            Ok(())
        })
    }

    fn select_tab(&self, tab: TabId) -> EngineFuture<'_, ()> {
        Box::pin(async move { self.link.activate(self.key_of(tab)?).await })
    }

    fn goto(&self, tab: TabId, url: String, wait: Duration) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            let key = self.prepare(tab).await?;
            let mut events = self.link.events();
            let got = self
                .send(tab, "Page.navigate", json!({ "url": url }))
                .await?;
            if let Some(error) = got
                .get("errorText")
                .and_then(Value::as_str)
                .filter(|e| !e.is_empty())
            {
                return Err(EngineError::Failed(format!("{error} at {url}")));
            }
            if got.get("loaderId").is_some() {
                Self::wait_event(
                    &mut events,
                    &key,
                    &["Page.domContentEventFired", "Page.loadEventFired"],
                    wait,
                )
                .await;
            }
            Ok(())
        })
    }

    fn settle(&self, tab: TabId, wait: Duration) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            let until = tokio::time::Instant::now() + wait;
            while tokio::time::Instant::now() < until {
                let state = self
                    .evaluate(tab, "() => document.readyState", &Value::Null)
                    .await;
                if state.as_ref().ok().and_then(Value::as_str) == Some("complete") || state.is_err()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Ok(())
        })
    }

    fn navigate(&self, tab: TabId, nav: Nav) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            if nav == Nav::Reload {
                self.send(tab, "Page.reload", json!({})).await?;
                return Ok(());
            }
            let history = self
                .send(tab, "Page.getNavigationHistory", json!({}))
                .await?;
            let at = history
                .get("currentIndex")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let entries = history
                .get("entries")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let to = if nav == Nav::Back { at - 1 } else { at + 1 };
            let entry = usize::try_from(to).ok().and_then(|i| entries.get(i));
            if let Some(id) = entry.and_then(|e| e.get("id")).and_then(Value::as_i64) {
                self.send(tab, "Page.navigateToHistoryEntry", json!({ "entryId": id }))
                    .await?;
            }
            Ok(())
        })
    }

    fn info(&self, tab: TabId) -> EngineFuture<'_, Tab> {
        Box::pin(async move {
            let got = self
                .evaluate(tab, "() => [location.href, document.title]", &Value::Null)
                .await?;
            Ok(Tab {
                id: tab,
                url: got
                    .get(0)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                title: got
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                front: false,
            })
        })
    }

    fn snapshot(&self, tab: TabId, scope: Option<String>) -> EngineFuture<'_, Option<String>> {
        Box::pin(async move { self.tree(tab, scope.as_deref()).await })
    }

    fn page(&self, tab: TabId, query: PageQuery) -> EngineFuture<'_, Value> {
        Box::pin(async move {
            let (function, arg): (&str, Value) = match query {
                PageQuery::IsPdf => (
                    "() => document.contentType === \"application/pdf\"",
                    Value::Null,
                ),
                PageQuery::SecretValues => (
                    "() => [...document.querySelectorAll(\"input\")].filter((i) => i.type === \"password\" || /one-time-code|current-password|new-password/.test(i.autocomplete)).map((i) => i.value).filter((v) => v.length > 0)",
                    Value::Null,
                ),
                PageQuery::Keys => (FIND, json!({ "src": TOKEN_SOURCE, "mark": false })),
                PageQuery::FocusedField => (FOCUSED_FIELD, Value::Null),
                PageQuery::FocusedWritten => {
                    let got = self
                        .evaluate(
                            tab,
                            "() => document.activeElement ? true : false",
                            &Value::Null,
                        )
                        .await?;
                    if got != json!(true) {
                        return Ok(json!([]));
                    }
                    let found = self
                        .send(
                            tab,
                            "Runtime.evaluate",
                            json!({ "expression": "document.activeElement" }),
                        )
                        .await?;
                    let Some(object) = found.pointer("/result/objectId").and_then(Value::as_str)
                    else {
                        return Ok(json!([]));
                    };
                    return self
                        .call_on(tab, object.to_owned(), WRITTEN, Vec::new())
                        .await;
                }
                PageQuery::IsMac => ("() => /Mac|iP/.test(navigator.platform)", Value::Null),
                PageQuery::ScreenSize => ("() => [screen.width, screen.height]", Value::Null),
                PageQuery::FetchStart { href, max } => {
                    (FETCH_IN_PAGE, json!({ "href": href, "max": max }))
                }
                PageQuery::FetchPiece { key, at, n } => {
                    (PIECE_IN_PAGE, json!({ "k": key, "at": at, "n": n }))
                }
                PageQuery::FetchDrop { key } => {
                    ("(k) => { delete globalThis[Symbol.for(k)]; }", json!(key))
                }
                PageQuery::Run { function, arg } => (function, arg),
            };
            self.evaluate(tab, function, &arg).await
        })
    }

    fn element(&self, tab: TabId, at: String, query: ElementQuery) -> EngineFuture<'_, Value> {
        Box::pin(async move {
            let (function, args): (&str, Vec<Value>) = match query {
                ElementQuery::Label => (
                    "function () { return (this.innerText || this.value || this.getAttribute?.(\"aria-label\") || this.title || \"\").trim().slice(0, 80); }",
                    Vec::new(),
                ),
                ElementQuery::Field => (
                    "function () { const el = this; return `${el.type ?? \"\"} ${el.autocomplete ?? \"\"} ${el.name ?? \"\"} ${el.id ?? \"\"} ${el.getAttribute?.(\"aria-label\") ?? \"\"} ${el.placeholder ?? \"\"}`; }",
                    Vec::new(),
                ),
                ElementQuery::Written => (WRITTEN, Vec::new()),
                ElementQuery::CodeEditor(text) => (CODE_EDITOR, vec![json!(text)]),
                ElementQuery::KeyAt => (KEY_AT, vec![json!(TOKEN_SOURCE)]),
                ElementQuery::FileKind => (FILE_KIND, Vec::new()),
                ElementQuery::Link => (
                    "function () { return this.href || this.src || this.data || this.closest?.(\"a\")?.href || \"\"; }",
                    Vec::new(),
                ),
                ElementQuery::Disabled => ("function () { return !!this.disabled; }", Vec::new()),
            };
            self.on_element(tab, &at, function, args).await
        })
    }

    fn click(&self, tab: TabId, at: String, _force: bool) -> EngineFuture<'_, ()> {
        Box::pin(async move { self.click_at(tab, &at).await })
    }

    fn fill(&self, tab: TabId, at: String, text: String) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            self.on_element(tab, &at, FILL, vec![json!(text)])
                .await
                .map(drop)
        })
    }

    fn press(&self, tab: TabId, key: String) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            let (modifiers, key, code, vk, text) = key_events(&key);
            let mut down = json!({
                "type": if text.is_some() { "keyDown" } else { "rawKeyDown" },
                "modifiers": modifiers, "key": key, "code": code,
                "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk,
            });
            if let Some(text) = &text {
                down["text"] = json!(text);
                down["unmodifiedText"] = json!(text);
            }
            self.send(tab, "Input.dispatchKeyEvent", down).await?;
            self.send(
                tab,
                "Input.dispatchKeyEvent",
                json!({ "type": "keyUp", "modifiers": modifiers, "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk }),
            )
            .await?;
            Ok(())
        })
    }

    fn insert_text(&self, tab: TabId, text: String) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            self.send(tab, "Input.insertText", json!({ "text": text }))
                .await
                .map(drop)
        })
    }

    fn wheel(&self, tab: TabId, dy: f64) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            self.send(
                tab,
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseWheel", "x": 400, "y": 300, "deltaX": 0, "deltaY": dy }),
            )
            .await
            .map(drop)
        })
    }

    fn wait_for_text(&self, tab: TabId, text: String, wait: Duration) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            let until = tokio::time::Instant::now() + wait;
            loop {
                let found = self
                    .evaluate(
                        tab,
                        "(t) => !!document.body && document.body.innerText.includes(t)",
                        &json!(text),
                    )
                    .await?;
                if found == json!(true) {
                    return Ok(());
                }
                if tokio::time::Instant::now() >= until {
                    return Err(EngineError::Timeout(format!(
                        "\"{text}\" didn't show within {} s",
                        wait.as_secs()
                    )));
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
    }

    fn screenshot(&self, tab: TabId, quality: u8) -> EngineFuture<'_, Option<Vec<u8>>> {
        Box::pin(async move { self.screenshot_now(tab, quality).await })
    }

    fn set_file(
        &self,
        tab: TabId,
        at: String,
        via: FileVia,
        file: FilePayload,
    ) -> EngineFuture<'_, Option<String>> {
        Box::pin(async move {
            if via != FileVia::Picker {
                let object = self.object(tab, &at).await?;
                return self.set_on(tab, object, &file).await.map(Some);
            }
            let key = self.prepare(tab).await?;
            let mut events = self.link.events();
            self.send(
                tab,
                "Page.setInterceptFileChooserDialog",
                json!({ "enabled": true }),
            )
            .await?;
            self.click_at(tab, &at).await?;
            let opened = Self::wait_event(
                &mut events,
                &key,
                &["Page.fileChooserOpened"],
                Duration::from_secs(10),
            )
            .await;
            if !self.catch_files {
                drop(
                    self.send(
                        tab,
                        "Page.setInterceptFileChooserDialog",
                        json!({ "enabled": false }),
                    )
                    .await,
                );
            }
            {
                let mut book = self.book();
                if book.file_ask.is_some_and(|(t, _, _)| t == tab) {
                    book.file_ask = None;
                }
            }
            let Some(node) =
                opened.and_then(|e| e.params.get("backendNodeId").and_then(Value::as_i64))
            else {
                return Ok(None);
            };
            let object = self.object_of(tab, node).await?;
            self.set_on(tab, object, &file).await.map(Some)
        })
    }

    fn file_ask(&self) -> Option<FileAsk> {
        self.book()
            .file_ask
            .map(|(_, _, multiple)| FileAsk { multiple })
    }

    fn answer_file_ask(&self, file: FilePayload) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            let Some((tab, node, _)) = self.book().file_ask.take() else {
                return Err(EngineError::Failed(
                    "The page isn't asking for a file".to_owned(),
                ));
            };
            let object = self.object_of(tab, node).await?;
            self.set_on(tab, object, &file).await.map(drop)
        })
    }

    fn owner_drives(&self, owner: bool) {
        self.owner.store(owner, Ordering::SeqCst);
        if !owner {
            let left: Vec<String> = std::mem::take(&mut self.book().dialogs);
            let link = Arc::clone(&self.link);
            if !left.is_empty()
                && let Ok(runtime) = tokio::runtime::Handle::try_current()
            {
                runtime.spawn(async move {
                    for key in left {
                        drop(
                            link.send(
                                key,
                                "Page.handleJavaScriptDialog",
                                json!({ "accept": false }),
                            )
                            .await,
                        );
                    }
                });
            }
        }
    }

    fn show_controls(&self, show: bool) -> EngineFuture<'_, ()> {
        Box::pin(async move {
            let tabs = self.link.targets().await?;
            let Some(first) = tabs.first() else {
                return Ok(());
            };
            let got = self
                .link
                .window(first.key.clone(), "Browser.getWindowForTarget", json!({}))
                .await?;
            let Some(window) = got.get("windowId").cloned() else {
                return Ok(());
            };
            let state = if show { "normal" } else { "fullscreen" };
            if show {
                self.link
                    .window(
                        first.key.clone(),
                        "Browser.setWindowBounds",
                        json!({ "windowId": window, "bounds": { "windowState": "normal" } }),
                    )
                    .await?;
            } else {
                self.link
                    .window(
                        first.key.clone(),
                        "Browser.setWindowBounds",
                        json!({ "windowId": window, "bounds": { "windowState": state } }),
                    )
                    .await?;
            }
            Ok(())
        })
    }
}

// ── A Chrome the server reaches itself ──

type Waiting = oneshot::Sender<Result<Value, String>>;

#[derive(Debug, Default)]
struct DirectState {
    out: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    calls: HashMap<u64, Waiting>,
    sessions: HashMap<String, String>,
    targets: HashMap<String, String>,
}

/// The longest `/json/version` answer read: a few hundred bytes of JSON.
const VERSION_LIMIT: usize = 64 << 10;

/// A Chrome reached by its `DevTools` address: `http://host:port` (its WebSocket from
/// `/json/version`), or the browser's `ws://` address itself.
pub struct DirectLink {
    address: String,
    http: reqwest::Client,
    state: Arc<Mutex<DirectState>>,
    next: AtomicU64,
    events: broadcast::Sender<CdpEvent>,
    connecting: tokio::sync::Mutex<()>,
}

impl fmt::Debug for DirectLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectLink")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl DirectLink {
    /// The browser at `address`, its `DevTools` address read with `http`.
    #[must_use]
    pub fn new(http: reqwest::Client, address: &str) -> Arc<Self> {
        Arc::new(Self {
            address: address.trim_end_matches('/').to_owned(),
            http,
            state: Arc::new(Mutex::new(DirectState::default())),
            next: AtomicU64::new(1),
            events: broadcast::channel(256).0,
            connecting: tokio::sync::Mutex::new(()),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, DirectState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn unreachable(&self, why: impl fmt::Display) -> EngineError {
        EngineError::Unavailable(format!(
            "The server's browser isn't answering at {} ({why}): it isn't running, or \
             EMBERHUT_BROWSER_CDP is wrong.",
            self.address
        ))
    }

    /// The browser's own WebSocket address.
    async fn socket_url(&self) -> EngineResult<String> {
        if self.address.starts_with("ws://") || self.address.starts_with("wss://") {
            return Ok(self.address.clone());
        }
        let response = self
            .http
            .get(format!("{}/json/version", self.address))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| self.unreachable(e))?;
        let body = crate::http::read_body(response, VERSION_LIMIT)
            .await
            .map_err(|e| self.unreachable(e))?;
        let got: Value = serde_json::from_slice(&body.bytes).map_err(|e| self.unreachable(e))?;
        got.get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| self.unreachable("no webSocketDebuggerUrl"))
    }

    async fn connect(&self) -> EngineResult<()> {
        let _one = self.connecting.lock().await;
        if self.connected() {
            return Ok(());
        }
        let url = self.socket_url().await?;
        let (socket, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .map_err(|e| self.unreachable(e))?;
        let (mut sink, mut stream) = socket.split();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        self.state().out = Some(tx);
        tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if sink.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
        });
        let state = Arc::clone(&self.state);
        let events = self.events.clone();
        tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                direct_message(&state, &events, &value);
            }
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.out = None;
            state.sessions.clear();
            state.targets.clear();
            for (_, waiting) in state.calls.drain() {
                drop(waiting.send(Err("the browser went away".to_owned())));
            }
        });
        drop(
            self.call(
                None,
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
            )
            .await,
        );
        Ok(())
    }

    async fn call(
        &self,
        session: Option<String>,
        method: &str,
        params: Value,
    ) -> EngineResult<Value> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        {
            let mut state = self.state();
            let Some(out) = state.out.clone() else {
                return Err(self.unreachable("not connected"));
            };
            state.calls.insert(id, tx);
            drop(state);
            if out.send(message.to_string()).is_err() {
                return Err(self.unreachable("not connected"));
            }
        }
        match tokio::time::timeout(COMMAND_WAIT, rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error))) => Err(
                if error.contains("No target")
                    || error.contains("No session")
                    || error.contains("went away")
                {
                    EngineError::TabClosed("The tab has been closed.".to_owned())
                } else {
                    EngineError::Failed(error)
                },
            ),
            Ok(Err(_)) => Err(self.unreachable("the link dropped")),
            Err(_) => {
                self.state().calls.remove(&id);
                Err(EngineError::Timeout(format!(
                    "{method} took longer than 30 s"
                )))
            }
        }
    }

    async fn session(&self, key: &str) -> EngineResult<String> {
        self.connect().await?;
        if let Some(session) = self.state().sessions.get(key).cloned() {
            return Ok(session);
        }
        let got = self
            .call(
                None,
                "Target.attachToTarget",
                json!({ "targetId": key, "flatten": true }),
            )
            .await?;
        let session = got
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| EngineError::TabClosed("The tab has been closed.".to_owned()))?;
        let mut state = self.state();
        state.sessions.insert(key.to_owned(), session.clone());
        state.targets.insert(session.clone(), key.to_owned());
        Ok(session)
    }
}

fn direct_message(
    state: &Arc<Mutex<DirectState>>,
    events: &broadcast::Sender<CdpEvent>,
    value: &Value,
) {
    let mut state = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        if let Some(waiting) = state.calls.remove(&id) {
            let answer = match value.get("error") {
                Some(error) => Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("failed")
                    .to_owned()),
                None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
            };
            drop(waiting.send(answer));
        }
        return;
    }
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    if method == "Target.detachedFromTarget" || method == "Target.targetDestroyed" {
        let session = params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let target = params
            .get("targetId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(session) = &session
            && let Some(key) = state.targets.remove(session)
        {
            state.sessions.remove(&key);
            drop(events.send(CdpEvent {
                key,
                method: method.to_owned(),
                params: params.clone(),
            }));
        }
        if let Some(target) = target
            && let Some(session) = state.sessions.remove(&target)
        {
            state.targets.remove(&session);
        }
        return;
    }
    let key = value
        .get("sessionId")
        .and_then(Value::as_str)
        .and_then(|s| state.targets.get(s).cloned());
    if let Some(key) = key {
        drop(events.send(CdpEvent {
            key,
            method: method.to_owned(),
            params,
        }));
    }
}

impl CdpLink for DirectLink {
    fn connected(&self) -> bool {
        self.state().out.as_ref().is_some_and(|o| !o.is_closed())
    }

    fn targets(&self) -> BoxFuture<'_, EngineResult<Vec<Target>>> {
        Box::pin(async move {
            self.connect().await?;
            let got = self.call(None, "Target.getTargets", json!({})).await?;
            Ok(got
                .get("targetInfos")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
                .map(|t| Target {
                    key: t
                        .get("targetId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    url: t
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    title: t
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect())
        })
    }

    fn create(&self, url: Option<String>) -> BoxFuture<'_, EngineResult<Target>> {
        Box::pin(async move {
            self.connect().await?;
            let url = url.unwrap_or_else(|| "about:blank".to_owned());
            let got = self
                .call(None, "Target.createTarget", json!({ "url": url }))
                .await?;
            Ok(Target {
                key: got
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                url,
                title: String::new(),
            })
        })
    }

    fn close(&self, key: String) -> BoxFuture<'_, EngineResult<()>> {
        Box::pin(async move {
            self.connect().await?;
            self.call(None, "Target.closeTarget", json!({ "targetId": key }))
                .await
                .map(drop)
        })
    }

    fn activate(&self, key: String) -> BoxFuture<'_, EngineResult<()>> {
        Box::pin(async move {
            self.connect().await?;
            self.call(None, "Target.activateTarget", json!({ "targetId": key }))
                .await
                .map(drop)
        })
    }

    fn send(
        &self,
        key: String,
        method: &'static str,
        params: Value,
    ) -> BoxFuture<'_, EngineResult<Value>> {
        Box::pin(async move {
            let session = self.session(&key).await?;
            self.call(Some(session), method, params).await
        })
    }

    fn window(
        &self,
        key: String,
        method: &'static str,
        mut params: Value,
    ) -> BoxFuture<'_, EngineResult<Value>> {
        Box::pin(async move {
            self.connect().await?;
            if method == "Browser.getWindowForTarget" {
                params["targetId"] = json!(key);
            }
            self.call(None, method, params).await
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
}

/// Appends `part` to `line` (writing to a `String` never fails).
fn add(line: &mut String, part: fmt::Arguments<'_>) {
    use std::fmt::Write as _;
    let _written = line.write_fmt(part);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> reqwest::Client {
        crate::http::shared_client().unwrap()
    }

    /// A scripted browser's `DevTools` socket on `127.0.0.1`: one tab, `T1`, whose session is
    /// `S1`. It answers what the engine asks and records the input events.
    async fn devtools() -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let input = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&input);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let m: Value = serde_json::from_str(text.as_str()).unwrap();
                let method = m["method"].as_str().unwrap_or_default().to_owned();
                let result = match method.as_str() {
                    "Target.getTargets" => json!({ "targetInfos": [
                        { "targetId": "T1", "type": "page", "url": "https://example.com/", "title": "Example" },
                        { "targetId": "W1", "type": "service_worker", "url": "https://example.com/sw.js", "title": "" }
                    ] }),
                    "Target.attachToTarget" => json!({ "sessionId": "S1" }),
                    "Page.navigate" => json!({ "frameId": "F1", "loaderId": "L1" }),
                    "Runtime.evaluate" => {
                        let expression = m["params"]["expression"].as_str().unwrap_or_default();
                        let value = if expression.contains("location.href") {
                            json!(["https://example.com/next", "Next"])
                        } else {
                            Value::Null
                        };
                        json!({ "result": { "type": "object", "value": value } })
                    }
                    "Accessibility.getFullAXTree" => json!({ "nodes": [
                        { "nodeId": "1", "role": { "value": "RootWebArea" }, "childIds": ["2"], "backendDOMNodeId": 1 },
                        { "nodeId": "2", "parentId": "1", "role": { "value": "button" }, "name": { "value": "Go" }, "backendDOMNodeId": 7 }
                    ] }),
                    "DOM.getContentQuads" => json!({ "quads": [[0, 0, 10, 0, 10, 20, 0, 20]] }),
                    _ => json!({}),
                };
                if method == "Input.dispatchMouseEvent" {
                    seen.lock()
                        .unwrap()
                        .push(m["params"]["type"].as_str().unwrap_or_default().to_owned());
                }
                let mut answer = json!({ "id": m["id"], "result": result });
                if let Some(session) = m.get("sessionId") {
                    answer["sessionId"] = session.clone();
                }
                ws.send(Message::text(answer.to_string())).await.unwrap();
                if method == "Page.navigate" {
                    let loaded = json!({ "method": "Page.domContentEventFired", "params": {}, "sessionId": "S1" });
                    ws.send(Message::text(loaded.to_string())).await.unwrap();
                }
            }
        });
        (format!("ws://{address}/devtools/browser/one"), input)
    }

    #[tokio::test]
    async fn a_direct_link_drives_a_tab_over_the_browser_s_socket() {
        let (address, input) = devtools().await;
        let engine = CdpEngine::new(DirectLink::new(test_client(), &address), true);
        assert!(!engine.connected());
        let tabs = engine.tabs().await.unwrap();
        assert_eq!(tabs.len(), 1, "pages only");
        assert_eq!(
            (tabs[0].url.as_str(), tabs[0].front),
            ("https://example.com/", true)
        );
        let tab = tabs[0].id;
        engine
            .goto(
                tab,
                "https://example.com/next".to_owned(),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_eq!(engine.info(tab).await.unwrap().title, "Next");
        let tree = engine.snapshot(tab, None).await.unwrap().unwrap();
        assert_eq!(tree, "- button \"Go\" [ref=e1]");
        assert_eq!(
            engine.snapshot(tab, Some("main".to_owned())).await.unwrap(),
            None
        );
        engine.click(tab, "e1".to_owned(), false).await.unwrap();
        assert_eq!(
            *input.lock().unwrap(),
            ["mouseMoved", "mousePressed", "mouseReleased"]
        );
        assert_eq!(
            engine.click(tab, "e9".to_owned(), false).await,
            Err(EngineError::NoRef("e9".to_owned()))
        );
        assert!(engine.connected());
    }

    #[tokio::test]
    async fn an_unreachable_browser_says_so() {
        let engine = CdpEngine::new(DirectLink::new(test_client(), "http://127.0.0.1:9"), true);
        let said = engine.tabs().await.unwrap_err().to_string();
        assert!(
            said.starts_with("The server's browser isn't answering at http://127.0.0.1:9"),
            "{said}"
        );
    }

    #[test]
    fn keys_become_cdp_key_events() {
        assert_eq!(
            key_events("Enter"),
            (0, "Enter".into(), "Enter".into(), 13, Some("\r".into()))
        );
        let (mods, key, code, vk, text) = key_events("Control+a");
        assert_eq!(
            (mods, key.as_str(), code.as_str(), vk, text),
            (2, "a", "KeyA", 65, None)
        );
        let (mods, key, _, vk, _) = key_events("Control+Backslash");
        assert_eq!((mods, key.as_str(), vk), (2, "\\", 220));
        assert_eq!(key_events("Meta+Shift+x").0, 12);
    }

    #[test]
    fn the_tree_is_written_with_refs_and_values() {
        let nodes = json!([
            { "nodeId": "1", "role": { "value": "RootWebArea" }, "name": { "value": "Example" }, "childIds": ["2", "5"], "backendDOMNodeId": 1 },
            { "nodeId": "2", "parentId": "1", "role": { "value": "generic" }, "childIds": ["3", "4"], "backendDOMNodeId": 2 },
            { "nodeId": "3", "parentId": "2", "role": { "value": "heading" }, "name": { "value": "Sign in" }, "properties": [{ "name": "level", "value": { "value": 1 } }], "childIds": ["6"], "backendDOMNodeId": 3 },
            { "nodeId": "6", "parentId": "3", "role": { "value": "StaticText" }, "name": { "value": "Sign in" }, "backendDOMNodeId": 6 },
            { "nodeId": "4", "parentId": "2", "role": { "value": "textbox" }, "name": { "value": "Email" }, "value": { "value": "a@b.c" }, "backendDOMNodeId": 4 },
            { "nodeId": "5", "parentId": "1", "role": { "value": "button" }, "name": { "value": "Next" }, "backendDOMNodeId": 5, "ignored": false }
        ]);
        let nodes = nodes.as_array().cloned().unwrap_or_default();
        let by_id: HashMap<String, &Value> = nodes
            .iter()
            .map(|n| (n["nodeId"].as_str().unwrap_or_default().to_owned(), n))
            .collect();
        let mut refs = Refs::default();
        let mut next = 1;
        let mut out = Vec::new();
        render(&nodes[0], &by_id, 0, true, &mut refs, &mut next, &mut out);
        assert_eq!(
            out,
            [
                "- heading \"Sign in\" [level=1] [ref=e1]",
                "- textbox \"Email\" [ref=e2]: a@b.c",
                "- button \"Next\" [ref=e3]",
            ]
        );
        assert_eq!(refs.by_ref.get("e2"), Some(&4));
    }

    #[test]
    fn an_exception_in_the_page_is_a_failure() {
        let failed = result_value(
            &json!({ "exceptionDetails": { "text": "Uncaught", "exception": { "description": "Error: nope\n at x" } } }),
        );
        assert_eq!(failed, Err(EngineError::Failed("Error: nope".to_owned())));
        assert_eq!(
            result_value(&json!({ "result": { "value": 3 } })),
            Ok(json!(3))
        );
    }
}
