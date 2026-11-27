//! The assistant's hands on the owner's Mac, through the Mac helper: one app at a time in the
//! background (its window read as elements with refs, pressed, filled and its menus picked by
//! accessibility, keys sent to that app only, a picture of that window only; the owner's mouse
//! never moves and the app they have in front stays there), and, only with the owner's OK or
//! when the Mac has been idle a while, the whole screen (`mac_screen`), with a bar on the Mac
//! saying the assistant is driving and a Stop button.
//!
//! The rules: plain dev and system tools never ask, and
//! Terminal is only looked at; any other app is asked once ("Let the assistant use Pages on
//! your Mac?") and remembered; on a job the owner OK'd no app asks. A final step goes by the
//! step gate (the job's OK, or a card); Return (or a typed new line) in an app that sends as
//! the owner is a final step, and on the whole screen so are Return and a click that doesn't
//! say what it presses; never a password or a code; Chrome, and any browser built on Chromium
//! (known by the app the helper finds, not the words it was asked for), goes through the
//! browser extension. Every step is logged on its task.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use emberhut_core::browser::{
    MAC_IDLE_MINUTES, is_enter_key, is_final, is_secret_label, mac_app_is_chromium,
    mac_app_pre_allowed, mac_app_sends, mac_app_view_only, mask,
};
use emberhut_db::{SettingKey, SettingStore};
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use crate::services::browser::Act;
use crate::services::browser::desk::{Card, Gated, Step};
use crate::services::browser::surface::{BrowserLog, Kit, Who};
use crate::services::mac::{MacError, MacHelper};

/// The apps the owner allowed, once each, for every task.
const ALLOWED: SettingKey<Vec<String>> = SettingKey::new("macApps");

/// What the agent is told when the helper doesn't answer.
const UNREACHED: &str = "The Mac helper isn't answering (it's off, or the Mac is asleep or away \
                         from the network). Nothing was done.";

/// What the agent is told for Chrome (or a browser built on Chromium).
const CHROME: &str = "Chrome goes through the browser extension: use the browser tools with \
                      browser \"chrome:<profile>\" (chrome_pair lists them).";

/// How long a helper call may take.
const WAIT: Duration = Duration::from_secs(60);

/// What a Mac tool is given, every tool's fields together.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HandsArgs {
    /// The app: its name, bundle id or pid.
    pub app: Option<String>,
    /// Open it in front.
    pub visible: Option<bool>,
    /// Which window.
    pub window: Option<String>,
    /// Part of the window, by ref.
    pub scope: Option<String>,
    /// A picture of the window too.
    pub picture: Option<bool>,
    /// The most elements to list.
    pub limit: Option<i64>,
    /// An element's ref.
    pub at: Option<String>,
    /// What the element is, in a few words.
    pub element: Option<String>,
    /// Another of its actions, or the screen's action.
    pub action: Option<String>,
    /// Text to type.
    pub text: Option<String>,
    /// A key to press.
    pub key: Option<String>,
    /// A menu path.
    pub path: Option<String>,
    /// The screen's x.
    pub x: Option<f64>,
    /// The screen's y.
    pub y: Option<f64>,
    /// Pixels to scroll.
    pub dy: Option<f64>,
    /// Why (the whole screen).
    pub reason: Option<String>,
    /// For how long (the whole screen).
    pub minutes: Option<f64>,
}

/// The Mac tools' service. Cheap to clone.
#[derive(Clone)]
pub struct HandsService {
    inner: Arc<Inner>,
}

struct Inner {
    helper: Arc<dyn MacHelper>,
    kit: Kit,
    settings: SettingStore,
    logs: BrowserLog,
    /// `task:what` → when it was asked.
    asked: Mutex<HashMap<String, Timestamp>>,
}

impl fmt::Debug for HandsService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandsService").finish_non_exhaustive()
    }
}

/// A helper's answer's `text`.
fn text(answer: &Map<String, Value>) -> String {
    answer
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A value as JavaScript writes it in a template (`undefined` when missing).
fn js(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// `{ key: value, … }` with what isn't given left out, as JSON leaves `undefined` out.
fn body(fields: &[(&str, Option<Value>)]) -> Value {
    let mut map = Map::new();
    for (key, value) in fields {
        if let Some(value) = value {
            map.insert((*key).to_owned(), value.clone());
        }
    }
    Value::Object(map)
}

impl HandsService {
    /// The service over the Mac `helper`, logging in `data_dir`'s `mac/logs`.
    #[must_use]
    pub fn new(
        helper: Arc<dyn MacHelper>,
        kit: Kit,
        settings: SettingStore,
        data_dir: &Path,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                helper,
                kit,
                settings,
                logs: BrowserLog::mac(data_dir),
                asked: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn asked(&self) -> std::sync::MutexGuard<'_, HashMap<String, Timestamp>> {
        self.inner
            .asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    async fn hands(
        &self,
        path: &str,
        body: Value,
        wait: Duration,
    ) -> Result<Map<String, Value>, String> {
        self.inner
            .helper
            .hands(path, body, wait)
            .await
            .map_err(|error| match error {
                MacError::NotSetUp => {
                    "No Mac helper is set up on this server. Nothing was done.".to_owned()
                }
                MacError::Unreachable(_) => UNREACHED.to_owned(),
                MacError::Refused(why) => why,
            })
    }

    /// A line on task `task`'s Mac log, with a picture (base64).
    async fn note(&self, task: Option<i64>, line: &str, picture: Option<&str>) {
        let Some(task) = task else { return };
        let bytes = picture.and_then(|p| base64::engine::general_purpose::STANDARD.decode(p).ok());
        let now = self.inner.kit.clock.now();
        match self.inner.logs.write(task, line, bytes.as_deref(), "", now) {
            Ok(made) => {
                for (path, title) in made {
                    self.inner
                        .kit
                        .desk
                        .attach(task, path.display().to_string(), title)
                        .await;
                }
            }
            Err(error) => tracing::warn!(%error, task, "the task's Mac log was not written"),
        }
    }

    /// What the owner chose on `task` since the ask kept under `what`.
    async fn okd(&self, task: i64, what: &str) -> bool {
        let at = self.asked().get(&format!("{task}:{what}")).copied();
        match at {
            Some(at) => {
                self.inner.kit.desk.chose_since(task, at).await.as_deref() == Some("Go ahead")
            }
            None => false,
        }
    }

    /// Asks the owner once about `what` on `task`; a second call while it waits says so.
    async fn ask_once(
        &self,
        task: i64,
        what: &str,
        question: String,
        action: String,
        brief: Vec<String>,
    ) -> String {
        let key = format!("{task}:{what}");
        let at = self.asked().get(&key).copied();
        if let Some(at) = at {
            match self.inner.kit.desk.chose_since(task, at).await {
                None => return "Still waiting for the owner's answer (it's under Needs you). Stop for now.".to_owned(),
                Some(said) if said != "Go ahead" => {
                    self.asked().remove(&key);
                    return format!("The owner said {said}: not done. Don't ask again unless they bring it up.");
                }
                Some(_) => {}
            }
        }
        self.asked().insert(key, self.inner.kit.clock.now());
        self.inner
            .kit
            .desk
            .ask(
                task,
                Card {
                    question,
                    options: vec!["Go ahead".to_owned(), "Stop".to_owned()],
                    action: action.chars().take(90).collect(),
                    brief,
                    kind: "approve",
                },
            )
            .await;
        "Paused: the owner was asked first (it's under Needs you). When they say Go ahead, call the \
         same tool again. Stop for now."
            .to_owned()
    }

    /// The app as the helper names it, allowed; or what the agent is told.
    async fn app(&self, args: &HandsArgs, task: Option<i64>) -> Result<String, String> {
        let want = args.app.clone().unwrap_or_default().trim().to_owned();
        if want.to_lowercase().contains("chrome") {
            return Err(CHROME.to_owned());
        }
        let which = self.hands("/which", json!({ "app": want }), WAIT).await?;
        let name = js(which.get("name"));
        // The app the helper found (a pid, a bundle id or another name for it), not the words
        // the agent used: Chrome is never driven from here, however it is asked for.
        let bundle = which
            .get("bundle")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if mac_app_is_chromium(&name, bundle) {
            return Err(CHROME.to_owned());
        }
        let allowed = self
            .inner
            .settings
            .get_or(&ALLOWED, Vec::new())
            .await
            .unwrap_or_default();
        if allowed.contains(&name) || mac_app_pre_allowed(&name) {
            return Ok(name);
        }
        let Some(t) = task else {
            return Err(format!(
                "{name} hasn't been allowed on the owner's Mac yet: use it for a task (task_id) so \
                 they can say yes once."
            ));
        };
        let what = format!("app:{name}");
        let granted = if self.okd(t, &what).await {
            Some("their Go ahead".to_owned())
        } else {
            self.inner.kit.gate.job_grant(t).await
        };
        if let Some(by) = granted {
            let mut allowed = allowed;
            allowed.push(name.clone());
            allowed.dedup();
            if let Err(error) = self.inner.settings.set(&ALLOWED, &allowed).await {
                tracing::warn!(%error, "the allowed Mac apps were not kept");
            }
            self.asked().remove(&format!("{t}:{what}"));
            self.note(
                Some(t),
                &format!("Allowed {name} on the Mac ({by}); remembered for every task"),
                None,
            )
            .await;
            return Ok(name);
        }
        Err(self
            .ask_once(
                t,
                &what,
                format!(
                    "Let the assistant use {name} on your Mac? It works in the background: your \
                     mouse doesn't move and the app you're in stays in front. Once allowed, it can \
                     use {name} for any task; final steps (send, delete, pay) still ask you."
                ),
                format!("Let the assistant use {name} on your Mac"),
                vec![
                    format!("App: {name}"),
                    "In the background, without your mouse or keyboard".to_owned(),
                    "Asked once per app".to_owned(),
                ],
            )
            .await)
    }

    /// A step that can't be undone, by what it is: through the step gate, else asked.
    async fn final_step(
        &self,
        task: Option<i64>,
        what: &str,
        app: &str,
        typed: Option<&str>,
    ) -> Option<String> {
        if !is_final(what) {
            return None;
        }
        self.gated(task, what, app, typed, "looks like a final step")
            .await
    }

    /// A step through the step gate (the job's OK, or a card), whatever it is: `None` when it
    /// may go, else what the agent is told. `why` says why it needs the OK, for a step with no
    /// task.
    async fn gated(
        &self,
        task: Option<i64>,
        what: &str,
        app: &str,
        typed: Option<&str>,
        why: &str,
    ) -> Option<String> {
        let Some(t) = task else {
            return Some(format!(
                "\"{what}\" {why}: do it for a task (task_id) so the owner can OK it."
            ));
        };
        let composed = typed.map(|t| vec![t.to_owned()]);
        let page = if mac_app_sends(app) {
            format!("{app} message")
        } else {
            app.to_owned()
        };
        let gated = self
            .inner
            .kit
            .gate
            .step(Step {
                task: t,
                what: what.to_owned(),
                page,
                question: format!(
                    "The assistant is about to do \"{what}\" in {app} on your Mac. Go ahead?"
                ),
                action: format!("OK \"{what}\" in {app}"),
                brief: vec![format!("App: {app}"), format!("Step: \"{what}\"")],
                composed,
            })
            .await;
        match gated {
            Gated::Through(by) => {
                self.note(Some(t), &format!("{app}: \"{what}\" ({by})"), None)
                    .await;
                None
            }
            Gated::Wait(text) => Some(text),
        }
    }

    /// Runs Mac tool `name` for `who`. A failure is said in the answer.
    pub async fn act(&self, name: &str, args: &HandsArgs, who: &Who) -> Act {
        match self.act_on(name, args, who).await {
            Ok(act) => act,
            Err(said) => Act {
                text: format!("Mac: {}", said.split('\n').next().unwrap_or_default()),
                image: None,
            },
        }
    }

    #[expect(clippy::too_many_lines, reason = "one arm per tool")]
    async fn act_on(&self, name: &str, args: &HandsArgs, who: &Who) -> Result<Act, String> {
        let task = who.task;
        let said = |text: String| Ok(Act { text, image: None });
        if name == "mac_apps" {
            return said(text(&self.hands("/apps", json!({}), WAIT).await?));
        }
        if name == "mac_screen" {
            return self.screen(args, who).await;
        }
        let app = match self.app(args, task).await {
            Ok(app) => app,
            Err(why) => return said(why),
        };
        let look_only = || {
            format!(
                "{app} is only looked at on the Mac (mac_app_snapshot): the assistant never \
                 presses, types or picks menus there."
            )
        };
        let with = |more: &[(&str, Option<Value>)]| {
            let mut fields = vec![("app", Some(json!(app)))];
            fields.extend(more.iter().cloned());
            body(&fields)
        };
        let at = args.at.clone().map(Value::from);
        match name {
            "mac_app_open" => {
                let visible = args.visible.unwrap_or(false);
                let r = self
                    .hands("/open", with(&[("hidden", Some(json!(!visible)))]), WAIT)
                    .await?;
                self.note(
                    task,
                    &format!("Opened {app}{}", if visible { "" } else { " (hidden)" }),
                    None,
                )
                .await;
                said(text(&r))
            }
            "mac_app_snapshot" => {
                let view = mac_app_view_only(&app);
                let r = self
                    .hands(
                        "/snapshot",
                        with(&[
                            ("window", args.window.clone().map(Value::from)),
                            ("scope", args.scope.clone().map(Value::from)),
                            (
                                "picture",
                                Some(json!(!view && args.picture.unwrap_or(false))),
                            ),
                            ("limit", args.limit.map(Value::from)),
                        ]),
                        WAIT,
                    )
                    .await?;
                let image = r
                    .get("image")
                    .and_then(Value::as_str)
                    .filter(|i| !i.is_empty())
                    .map(str::to_owned);
                if image.is_some() {
                    self.note(task, &format!("Looked at {app}"), image.as_deref())
                        .await;
                } else if view {
                    self.note(task, &format!("Looked at {app}"), None).await;
                }
                Ok(Act {
                    text: if view { mask(&text(&r)) } else { text(&r) },
                    image: if view {
                        None
                    } else {
                        image.and_then(|i| base64::engine::general_purpose::STANDARD.decode(i).ok())
                    },
                })
            }
            "mac_app_click" => {
                if mac_app_view_only(&app) {
                    return said(look_only());
                }
                let d = self
                    .hands("/describe", with(&[("ref", at.clone())]), WAIT)
                    .await?;
                let truthy = |v: Option<&Value>| {
                    v.is_some_and(|v| !(v.is_null() || v == &json!("") || v == &json!(false)))
                };
                let what = if truthy(d.get("label")) {
                    js(d.get("label"))
                } else if let Some(e) = args.element.clone().filter(|e| !e.is_empty()) {
                    e
                } else {
                    js(d.get("kind"))
                };
                let mut wait = self.final_step(task, &what, &app, None).await;
                if wait.is_none()
                    && let Some(element) =
                        args.element.clone().filter(|e| !e.is_empty() && *e != what)
                {
                    wait = self.final_step(task, &element, &app, None).await;
                }
                if let Some(wait) = wait {
                    self.note(
                        task,
                        &format!("Paused before \"{what}\": asked the owner"),
                        None,
                    )
                    .await;
                    return said(wait);
                }
                let r = self
                    .hands(
                        "/press",
                        with(&[
                            ("ref", at.clone()),
                            ("action", args.action.clone().map(Value::from)),
                        ]),
                        WAIT,
                    )
                    .await?;
                self.note(
                    task,
                    &format!("{app}: pressed {} \"{what}\"", js(d.get("kind"))),
                    None,
                )
                .await;
                said(text(&r))
            }
            "mac_app_type" => {
                if mac_app_view_only(&app) {
                    return said(look_only());
                }
                let typed = args.text.clone().unwrap_or_default();
                let key = args.key.clone().unwrap_or_default().trim().to_owned();
                if at.is_some() {
                    let d = self
                        .hands("/describe", with(&[("ref", at.clone())]), WAIT)
                        .await?;
                    let secret = d.get("secret").is_some_and(|s| s == &json!(true));
                    if secret || is_secret_label(&js(d.get("label"))) {
                        return said(
                            "That field wants a password or a code: the assistant never types \
                             those. Ask the owner to type it themselves (task_ask)."
                                .to_owned(),
                        );
                    }
                }
                let lower = key.to_lowercase();
                let enter = is_enter_key(&key);
                // A new line typed into an app that sends is Return pressed there.
                let line = typed.contains(['\n', '\r']);
                if (enter || line) && mac_app_sends(&app) {
                    let typed = (!typed.is_empty()).then_some(typed.as_str());
                    if let Some(wait) = self
                        .final_step(task, &format!("send in {app}"), &app, typed)
                        .await
                    {
                        self.note(
                            task,
                            &format!("Paused before Return in {app}: asked the owner"),
                            None,
                        )
                        .await;
                        return said(wait);
                    }
                }
                if matches!(
                    lower.as_str(),
                    "cmd+delete" | "cmd+backspace" | "command+delete" | "command+backspace"
                ) && let Some(wait) = self
                    .final_step(task, &format!("delete ({key})"), &app, None)
                    .await
                {
                    return said(wait);
                }
                let mut output = String::new();
                if !typed.is_empty() && at.is_some() {
                    output = match self
                        .hands(
                            "/set",
                            with(&[("ref", at.clone()), ("text", Some(json!(typed)))]),
                            WAIT,
                        )
                        .await
                    {
                        Ok(r) => text(&r),
                        Err(why) if why.contains("can't be set directly") => text(
                            &self
                                .hands(
                                    "/keys",
                                    with(&[("ref", at.clone()), ("text", Some(json!(typed)))]),
                                    WAIT,
                                )
                                .await?,
                        ),
                        Err(why) => return Err(why),
                    };
                } else if !typed.is_empty() {
                    output = text(
                        &self
                            .hands("/keys", with(&[("text", Some(json!(typed)))]), WAIT)
                            .await?,
                    );
                }
                if !key.is_empty() {
                    let pressed = text(
                        &self
                            .hands(
                                "/keys",
                                with(&[("key", Some(json!(key))), ("ref", at.clone())]),
                                WAIT,
                            )
                            .await?,
                    );
                    output = if output.is_empty() {
                        pressed
                    } else {
                        format!("{output} {pressed}")
                    };
                }
                if output.is_empty() {
                    return said("Nothing to type: give text, key or both.".to_owned());
                }
                let shown = if typed.is_empty() {
                    String::new()
                } else {
                    format!(" «{}»", typed.chars().take(80).collect::<String>())
                };
                let then = if key.is_empty() {
                    String::new()
                } else {
                    format!(" then {key}")
                };
                self.note(task, &format!("{app}: typed{shown}{then}"), None)
                    .await;
                said(output)
            }
            "mac_app_menu" => {
                if mac_app_view_only(&app) {
                    return said(look_only());
                }
                let path = args.path.clone().unwrap_or_default();
                let last = path
                    .rsplit('>')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if path.contains('>')
                    && let Some(wait) = self.final_step(task, &last, &app, None).await
                {
                    self.note(
                        task,
                        &format!("Paused before {path}: asked the owner"),
                        None,
                    )
                    .await;
                    return said(wait);
                }
                let r = self
                    .hands("/menu", with(&[("path", Some(json!(path)))]), WAIT)
                    .await?;
                if text(&r).starts_with("Picked") {
                    self.note(task, &format!("{app}: picked {path}"), None)
                        .await;
                }
                said(text(&r))
            }
            other => said(format!("No tool {other}")),
        }
    }

    /// The whole screen: a start goes through on the job's OK, or when the Mac has been idle a
    /// while (the helper says how long), else it asks once; never right after the owner stopped
    /// it.
    #[expect(clippy::too_many_lines, reason = "one arm per screen action")]
    async fn screen(&self, args: &HandsArgs, who: &Who) -> Result<Act, String> {
        let task = who.task;
        let action = args.action.clone().unwrap_or_default();
        let me = |more: &[(&str, Option<Value>)]| {
            let mut fields = vec![("who", Some(json!(who.driver)))];
            fields.extend(more.iter().cloned());
            body(&fields)
        };
        let said = |text: String| Ok(Act { text, image: None });
        match action.as_str() {
            "start" => {
                let Some(t) = task else {
                    return said("Driving the whole Mac needs a task (task_id), so the owner can say Go ahead.".to_owned());
                };
                let reason: String = args
                    .reason
                    .clone()
                    .unwrap_or_default()
                    .trim()
                    .chars()
                    .take(70)
                    .collect();
                let minutes = args
                    .minutes
                    .filter(|m| *m != 0.0 && m.is_finite())
                    .unwrap_or(15.0)
                    .clamp(1.0, 60.0);
                if reason.is_empty() {
                    return said("reason: what you'll do on the Mac, in a few words (it shows in the bar on its screen).".to_owned());
                }
                let health = self
                    .hands("/health", json!({}), Duration::from_secs(8))
                    .await
                    .unwrap_or_default();
                let idle = health.get("idle").and_then(Value::as_f64);
                let stopped = health.get("stopped").and_then(Value::as_f64);
                let by = if self.okd(t, "screen").await {
                    Some("their Go ahead".to_owned())
                } else if stopped.is_some_and(|s| s < 600.0) {
                    None
                } else if let Some(g) = self.inner.kit.gate.job_grant(t).await {
                    Some(g)
                } else {
                    idle.filter(|i| *i >= f64::from(MAC_IDLE_MINUTES) * 60.0)
                        .map(|i| format!("the Mac idle for {} minutes", (i / 60.0).floor()))
                };
                let Some(by) = by else {
                    let mut text = self
                        .ask_once(
                            t,
                            "screen",
                            format!(
                                "Let the assistant use your whole Mac for up to {minutes} minutes: \
                                 {reason}? It moves the real mouse and types. A bar at the top says \
                                 it is driving, with a Stop button, and touching the mouse or \
                                 keyboard stops it. Your OK covers the rest of this job."
                            ),
                            "Let the assistant drive your Mac".to_owned(),
                            vec![
                                format!("For: {reason}"),
                                format!("Up to {minutes} minutes"),
                                "Real mouse and keyboard, Stop on screen".to_owned(),
                                format!("Without asking when your Mac is idle {MAC_IDLE_MINUTES}+ minutes"),
                            ],
                        )
                        .await;
                    if stopped.is_some_and(|s| s < 600.0) {
                        text.push_str(" (They stopped the assistant on the screen a few minutes ago, so this one asks.)");
                    }
                    return said(text);
                };
                self.asked().remove(&format!("{t}:screen"));
                let started = self
                    .hands(
                        "/screen/start",
                        me(&[
                            ("reason", Some(json!(reason))),
                            ("minutes", Some(json!(minutes))),
                        ]),
                        WAIT,
                    )
                    .await?;
                self.note(
                    task,
                    &format!("Started driving the whole Mac ({minutes} min, {by}): {reason}"),
                    None,
                )
                .await;
                said(text(&started))
            }
            "stop" => {
                let r = self.hands("/screen/stop", me(&[]), WAIT).await?;
                self.note(task, "Stopped driving the Mac", None).await;
                said(text(&r))
            }
            "shot" => {
                let r = self.hands("/screen/shot", me(&[]), WAIT).await?;
                Ok(Act {
                    text: text(&r),
                    image: r
                        .get("image")
                        .and_then(Value::as_str)
                        .and_then(|i| base64::engine::general_purpose::STANDARD.decode(i).ok()),
                })
            }
            _ => {
                let element = args.element.clone().unwrap_or_default();
                if action == "type" && is_secret_label(&element) {
                    return said(
                        "Never a password or a code: the owner types those themselves.".to_owned(),
                    );
                }
                if let Some(wait) = self.screen_step(task, &action, &element, args).await {
                    return said(wait);
                }
                let r = self
                    .hands(
                        "/screen/act",
                        me(&[
                            ("action", Some(json!(action))),
                            ("x", args.x.map(Value::from)),
                            ("y", args.y.map(Value::from)),
                            ("text", args.text.clone().map(Value::from)),
                            ("key", args.key.clone().map(Value::from)),
                            ("dy", args.dy.map(Value::from)),
                        ]),
                        WAIT,
                    )
                    .await?;
                let mut line = format!("Screen: {action}");
                if !element.is_empty() {
                    add(&mut line, format_args!(" \"{element}\""));
                }
                if let (Some(x), Some(y)) = (args.x, args.y) {
                    add(&mut line, format_args!(" at {x},{y}"));
                }
                if let Some(t) = args.text.clone().filter(|t| !t.is_empty()) {
                    add(
                        &mut line,
                        format_args!(" «{}»", t.chars().take(60).collect::<String>()),
                    );
                }
                if let Some(k) = args.key.clone().filter(|k| !k.is_empty()) {
                    add(&mut line, format_args!(" {k}"));
                }
                self.note(task, &line, None).await;
                said(text(&r))
            }
        }
    }

    /// Before a step on the whole screen: a click names what it presses (`element`), checked
    /// as a final step; a click that doesn't goes by the step gate, as nothing says what it
    /// presses. Return (with any modifier) or a typed new line may send in whatever app is in
    /// front, so it is a send, with the owner's OK each time. `None`: it may go.
    async fn screen_step(
        &self,
        task: Option<i64>,
        action: &str,
        element: &str,
        args: &HandsArgs,
    ) -> Option<String> {
        const MAC: &str = "the Mac";
        if action.contains("click") {
            let wait = if element.is_empty() {
                let at = match (args.x, args.y) {
                    (Some(x), Some(y)) => format!(" at {x},{y}"),
                    _ => String::new(),
                };
                let what = format!("{action}{at} on the screen");
                self.gated(task, &what, MAC, None, "says nothing of what it presses")
                    .await
            } else {
                self.final_step(task, element, MAC, None).await
            };
            if wait.is_some() {
                return wait;
            }
        }
        let typed = args.text.as_deref().unwrap_or_default();
        let key = args.key.as_deref().unwrap_or_default();
        let enter = is_enter_key(key) || (action == "type" && typed.contains(['\n', '\r']));
        if !enter {
            return None;
        }
        let what = if is_enter_key(key) {
            format!("send with {key} on the screen")
        } else {
            "send with a new line typed on the screen".to_owned()
        };
        let typed = (!typed.is_empty()).then_some(typed);
        self.gated(
            task,
            &what,
            MAC,
            typed,
            "may send what is written in the app in front",
        )
        .await
    }
}

/// Appends `part` to `line` (writing to a `String` never fails).
fn add(line: &mut String, part: fmt::Arguments<'_>) {
    use std::fmt::Write as _;
    let _written = line.write_fmt(part);
}
