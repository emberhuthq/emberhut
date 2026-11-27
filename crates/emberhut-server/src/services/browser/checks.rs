//! The checks around a browser tool's step: a tab that reaches a blocked site is let go of, on
//! any surface, whatever took it there (a click, a key, a redirect); and Enter (with any
//! modifier) that may send what the focused field holds goes by the step gate first.

use emberhut_core::browser::{blocked_page, host_of, is_final, url_host};
use serde_json::Value;

use super::desk::{Gated, Step};
use super::engine::{EngineError, PageQuery, TabId};
use super::surface::Surface;
use super::{BrowserService, strings, value_text};

/// What the agent is told when the browser won't go to `url`, a blocked site.
pub(super) fn not_opened(url: &str) -> String {
    format!(
        "{} is a blocked site (the owner's passwords and accounts): the assistant doesn't open \
         it. If the task needs that site, browser_handoff so the owner does it.",
        shown_host(url)
    )
}

/// `url`'s host, or the address itself when it has none.
fn shown_host(url: &str) -> String {
    let host = host_of(url);
    if host.is_empty() {
        url.to_owned()
    } else {
        host
    }
}

/// What a press of Enter would send, by what has the focus.
#[derive(Debug, PartialEq, Eq)]
enum Sends {
    /// Nothing that needs the owner's OK: a search field, a form's plain field whose submit
    /// isn't a final step, a control that isn't one, or no focus.
    Nothing,
    /// The focused field's form, whose submit (its label) reads as a final step.
    Form(String),
    /// The focused button or link (its label), which reads as a final step.
    Control(String),
    /// What is written in a text box, a text area or an editor (a chat composer, a mail's
    /// body), or in a field the page doesn't let be read: a message or a post as the owner.
    Written,
}

/// What `focus` ([`PageQuery::FocusedField`]'s answer) sends with Enter. An answer that can't
/// be read is taken as a text box.
fn sends(focus: &Value) -> Sends {
    let read = |name: &str| focus.get(name).map_or_default(value_text);
    let form = read("form");
    let label = read("label");
    match read("kind").as_str() {
        "control" if is_final(&label) => Sends::Control(label),
        "field" | "control" if is_final(&form) => Sends::Form(form),
        "search" | "none" | "field" | "control" => Sends::Nothing,
        _ => Sends::Written,
    }
}

impl BrowserService {
    /// When the tab `tab` is on a blocked site: closes it, lets go of it, and says so.
    pub(super) async fn let_go_if_blocked(
        &self,
        on: &dyn Surface,
        tab: TabId,
        task: Option<i64>,
    ) -> Option<String> {
        let engine = on.engine();
        let url = engine.info(tab).await.ok()?.url;
        if !blocked_page(&url) {
            return None;
        }
        let host = shown_host(&url);
        if let Err(error) = engine.close_tab(tab).await {
            tracing::warn!(%error, "a tab on a blocked site was not closed");
        }
        on.use_tab(None);
        on.update(&mut |s| {
            s.url.clear();
            s.title.clear();
        });
        self.log1(
            on,
            task,
            &format!("Closed a tab that went to {host}, a blocked site"),
            None,
        )
        .await;
        Some(format!(
            "That tab went to {host}, a blocked site (the owner's passwords and accounts): the \
             assistant closed it. If the task needs that site, browser_handoff so the owner does \
             it."
        ))
    }

    /// Before `key` (Enter, with any modifier) is pressed in `tab`: a send through the step
    /// gate, by what has the focus. `None`: it may be pressed; else what the agent is told.
    pub(super) async fn before_enter(
        &self,
        on: &dyn Surface,
        tab: TabId,
        key: &str,
        task: Option<i64>,
    ) -> Result<Option<String>, EngineError> {
        let engine = on.engine();
        let focus = engine
            .page(tab, PageQuery::FocusedField)
            .await
            .unwrap_or(Value::Null);
        let sends = sends(&focus);
        // What it is (the step), what pressing it does, and why that needs the OK.
        let (what, pressing, why) = match &sends {
            Sends::Nothing => return Ok(None),
            Sends::Form(form) => (
                form.clone(),
                format!("Pressing {key} sends the form \"{form}\""),
                ", which looks like a final step",
            ),
            Sends::Control(label) => (
                label.clone(),
                format!("Pressing {key} on \"{label}\" clicks it"),
                ", which looks like a final step",
            ),
            Sends::Written => (
                format!("send what is written ({key})"),
                format!(
                    "Pressing {key} here can send what is written in this box (a message, a post \
                     or a comment) as the owner"
                ),
                "",
            ),
        };
        let Some(t) = task else {
            return Ok(Some(format!(
                "{pressing}{why}. Browse for a task (task_id) so the owner can OK it."
            )));
        };
        let info = engine.info(tab).await?;
        let composed = engine
            .page(tab, PageQuery::FocusedWritten)
            .await
            .ok()
            .map(|v| strings(&v));
        let host = url_host(&info.url).unwrap_or_default();
        let place = on.place();
        let gated = self
            .kit()
            .gate
            .step(Step {
                task: t,
                what: what.clone(),
                page: format!("{} {}", info.title, info.url),
                question: match &sends {
                    Sends::Written => format!(
                        "The browser is about to press {key} in a text box on {host}{place}, \
                         which sends what is written there. Go ahead?"
                    ),
                    Sends::Control(_) => format!(
                        "The browser is about to press \"{what}\" on {host}{place}. Go ahead?"
                    ),
                    Sends::Form(_) | Sends::Nothing => format!(
                        "The browser is about to send a form (\"{what}\") on {host}{place}. Go \
                         ahead?"
                    ),
                },
                action: format!("OK the browser's \"{what}\" step"),
                brief: if sends == Sends::Written {
                    vec![format!("Page: {host}"), format!("Key: {key}")]
                } else {
                    Vec::new()
                },
                composed,
            })
            .await;
        match gated {
            Gated::Wait(text) => {
                let shot = engine.screenshot(tab, 60).await.ok().flatten();
                self.log1(on, task, &format!("Paused before \"{what}\""), shot)
                    .await;
                Ok(Some(format!("{text} ({pressing}: press it again then.)")))
            }
            Gated::Through(by) => {
                self.log1(
                    on,
                    task,
                    &format!("Pressing {key}: \"{what}\" (OK'd: {by})"),
                    None,
                )
                .await;
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn enter_sends_a_final_form_a_final_control_or_what_a_text_box_holds() {
        let focus = |kind: &str, form: &str, label: &str| {
            sends(&json!({ "kind": kind, "form": form, "label": label }))
        };
        assert_eq!(focus("search", "Send", ""), Sends::Nothing);
        assert_eq!(focus("none", "", ""), Sends::Nothing);
        assert_eq!(focus("field", "Sign in", ""), Sends::Nothing);
        assert_eq!(
            focus("field", "Submit order", ""),
            Sends::Form("Submit order".to_owned())
        );
        assert_eq!(focus("control", "", "Next"), Sends::Nothing);
        assert_eq!(
            focus("control", "", "Send"),
            Sends::Control("Send".to_owned())
        );
        assert_eq!(focus("composer", "", ""), Sends::Written);
        assert_eq!(focus("composer", "Comment", ""), Sends::Written);
        assert_eq!(focus("unknown", "", ""), Sends::Written);
        assert_eq!(sends(&Value::Null), Sends::Written);
    }

    #[test]
    fn a_blocked_site_is_not_opened() {
        assert!(not_opened("https://my.1password.com/vault").starts_with("my.1password.com is"));
    }
}
