//! The actions pinned at the foot of a task's page: the first answer as the big button (⌘D; a
//! tick when pressed, a spinner while it is sent), the other answers beside it or under More,
//! or `I did it`, `Mark done`, `Unarchive`; then Not now (⌘L), Archive (⌃⌘A) and Talk. A
//! pressed action goes back from the page after a beat, while the toast offers Undo.
//!
//! An ask whose answers are on the page (its options, `decision.rs`) leaves them out of the
//! bar. A task with no question of its own whose action names another task (by its key) offers
//! that task's answers, and answers it.

use emberhut_api::ids::TaskId;
use emberhut_api::tasks::{Issue, TaskStatus};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{Div, SharedString, Window, div, px};

use super::{Part, TaskView};
use crate::store::{Store, asks, lookup};
use crate::ui::tasks::marks::glyph;
use crate::ui::tasks::row::parkable;

/// The big button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Primary {
    /// Back to to do.
    Unarchive,
    /// The first of the ask's answers.
    Answer(String),
    /// Something only the user could do, done (sent as the answer `Done`).
    DidIt,
    /// The user's own task, done.
    MarkDone,
}

impl Primary {
    /// Its words.
    fn label(&self) -> String {
        match self {
            Self::Unarchive => "Unarchive".to_owned(),
            Self::Answer(choice) => answer_label(choice),
            Self::DidIt => "I did it".to_owned(),
            Self::MarkDone => "Mark done".to_owned(),
        }
    }
}

/// The ask a task stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum AskOf<'a> {
    /// Its own (or none).
    Own,
    /// Another task's, which its action names and which still waits.
    Other(&'a Issue),
    /// Another task's that no longer waits: nothing to answer here.
    Gone,
}

impl<'a> AskOf<'a> {
    /// The ask `task` stands for, over the store's state.
    #[must_use]
    pub(super) fn of(store: &'a Store, task: &Issue) -> Self {
        let Some(needs) = store.needs() else {
            return Self::Own;
        };
        match needs.ask_of(task) {
            None => Self::Gone,
            Some(id) if id == task.id => Self::Own,
            Some(id) => store.task_of(id).map_or(Self::Gone, Self::Other),
        }
    }
}

/// What the bar offers for a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Offer {
    /// The task its answers answer: the task, or the one it stands for.
    pub(super) to: TaskId,
    /// The big button.
    pub(super) primary: Option<Primary>,
    /// The other answers.
    pub(super) others: Vec<String>,
    /// Not now puts it off.
    pub(super) not_now: bool,
    /// Archive puts it away.
    pub(super) archive: bool,
}

/// An answer as its button reads it: `Done` on something only the user can do is `I did it`
/// (the server still gets `Done`).
#[must_use]
fn answer_label(choice: &str) -> String {
    if choice.eq_ignore_ascii_case("done") {
        "I did it".to_owned()
    } else {
        choice.to_owned()
    }
}

/// What the bar offers for `task`, which stands for `ask`.
#[must_use]
pub(super) fn offer(task: &Issue, ask: AskOf<'_>) -> Offer {
    let own = task
        .decision
        .as_ref()
        .filter(|_| task.status == TaskStatus::Waiting);
    let in_page = own.is_some_and(|decision| !decision.options.is_empty());
    let (to, asked) = match ask {
        AskOf::Other(other) if own.is_none() => (other.id, other.decision.as_ref()),
        _ => (task.id, own),
    };
    let options = if in_page {
        Vec::new()
    } else {
        asked.map_or_else(Vec::new, |decision| decision.options.clone())
    };
    let primary = if task.status == TaskStatus::Canceled {
        Some(Primary::Unarchive)
    } else if let Some(first) = options.first() {
        Some(Primary::Answer(first.clone()))
    } else if task.artifacts.iter().any(|artifact| artifact.x.is_some()) {
        // An X post's replies close with Posted, on the reply used: no Done beside them.
        None
    } else if task.status == TaskStatus::Waiting
        && task.action.is_some()
        && !in_page
        && asked.is_none()
        && ask != AskOf::Gone
    {
        Some(Primary::DidIt)
    } else if !lookup::is_assistants(task) && task.status != TaskStatus::Done {
        Some(Primary::MarkDone)
    } else {
        None
    };
    Offer {
        to,
        primary,
        others: options.into_iter().skip(1).collect(),
        not_now: parkable(task),
        archive: task.is_open(),
    }
}

impl TaskView {
    /// The task shown and what its bar offers, while nothing was pressed yet.
    fn offered(&self, cx: &gpui_kit::App) -> Option<(TaskId, Offer)> {
        if self.tapped.is_some() {
            return None;
        }
        let store = self.store.read(cx);
        let task = store.task_of(self.shown?)?;
        let offer = offer(task, AskOf::of(store, task));
        Some((offer.to, offer))
    }

    /// Answers `task` with `choice`: an answer that sends puts the ask's draft on the
    /// clipboard, once the answer is taken.
    fn send_answer(&self, task: TaskId, choice: String, cx: &mut Context<'_, Self>) {
        let copied = self
            .store
            .read(cx)
            .task_of(task)
            .and_then(|task| task.decision.as_ref())
            .and_then(|decision| asks::copied_draft(&choice, decision));
        let taken = self
            .store
            .update(cx, |store, cx| store.decide(task, choice, cx));
        if let Some(copied) = copied.filter(|_| taken) {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(copied));
        }
    }

    /// The big button (⌘D).
    pub(crate) fn press_primary(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) {
        let Some((task, offer)) = self.offered(cx) else {
            return;
        };
        let Some(primary) = offer.primary else {
            return;
        };
        let Some(shown) = self.shown else {
            return;
        };
        match &primary {
            Primary::Answer(choice) => self.send_answer(task, choice.clone(), cx),
            Primary::DidIt => self.send_answer(task, "Done".to_owned(), cx),
            Primary::Unarchive => self
                .store
                .update(cx, |store, cx| store.unarchive(shown, cx)),
            Primary::MarkDone => self
                .store
                .update(cx, |store, cx| store.mark_done(shown, cx)),
        }
        self.leave(&primary.label(), cx);
    }

    /// Another of the ask's answers.
    fn answer(&mut self, choice: &str, cx: &mut Context<'_, Self>) {
        let Some((task, _)) = self.offered(cx) else {
            return;
        };
        self.send_answer(task, choice.to_owned(), cx);
        self.leave(&answer_label(choice), cx);
    }

    /// Not now (⌘L): put off a week.
    pub(crate) fn press_not_now(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) {
        if let (Some((_, offer)), Some(task)) = (self.offered(cx), self.shown)
            && offer.not_now
        {
            self.store.update(cx, |store, cx| store.not_now(task, cx));
            self.leave("Not now", cx);
        }
    }

    /// Archive (⌃⌘A): put away for good.
    pub(crate) fn press_archive(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) {
        if let (Some((_, offer)), Some(task)) = (self.offered(cx), self.shown)
            && offer.archive
        {
            self.store.update(cx, |store, cx| store.archive(task, cx));
            self.leave("Archive", cx);
        }
    }
}

/// A bordered button of the second row.
fn second(
    id: &'static str,
    label: impl Into<SharedString>,
    icon: IconName,
    p: &crate::ui::palette::Pal,
) -> Button {
    Button::new(id)
        .label(label)
        .icon(glyph(icon, p.muted, 14.))
        .small()
        .outline()
}

/// The bar.
pub(super) fn bar(view: &TaskView, part: &Part<'_>, cx: &Context<'_, TaskView>) -> Div {
    let p = part.p;
    let offer = offer(part.task, AskOf::of(part.read, part.task));
    let pressed = view.tapped.clone();
    let waits = pressed.is_some();
    let sending = part.read.answers().is_sending(part.task.id);
    let mut column = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(px(16.))
        .pt(px(10.))
        .pb(px(10.))
        .border_t_1()
        .border_color(p.border)
        .bg(p.surface_raised);
    if let Some(primary) = &offer.primary {
        let label = primary.label();
        let ticked = pressed.as_deref() == Some(label.as_str());
        let icon = if ticked {
            Some(IconName::CircleCheck)
        } else {
            match primary {
                Primary::Unarchive => Some(IconName::ArchiveRestore),
                Primary::DidIt | Primary::MarkDone => Some(IconName::CircleCheck),
                Primary::Answer(_) => None,
            }
        };
        let mut button = Button::new("task-primary")
            .label(label.clone())
            .primary()
            .w_full()
            .loading(sending)
            .disabled(waits)
            .tooltip(format!("{label} ({})", shortcut("D")))
            .on_click(cx.listener(|this, _, window, cx| this.press_primary(window, cx)));
        if let Some(icon) = icon {
            button = button.icon(glyph(icon, p.on_accent, 15.));
        }
        column = column.child(button);
    }
    let mut row = div().flex().gap(px(8.));
    if offer.others.len() == 1 {
        let choice = offer.others[0].clone();
        row = row.child(
            Button::new("task-answer-2")
                .label(answer_label(&choice))
                .small()
                .outline()
                .disabled(waits)
                .on_click(cx.listener(move |this, _, _, cx| this.answer(&choice, cx))),
        );
    } else if offer.others.len() > 1 {
        let others = offer.others.clone();
        let column_view = cx.entity();
        row = row.child(
            second("task-more", "More", IconName::Ellipsis, p)
                .disabled(waits)
                .dropdown_menu(move |mut menu, _, _| {
                    for choice in &others {
                        let (view, choice) = (column_view.clone(), choice.clone());
                        menu = menu.item(PopupMenuItem::new(answer_label(&choice)).on_click(
                            move |_, _, cx| view.update(cx, |this, cx| this.answer(&choice, cx)),
                        ));
                    }
                    menu
                }),
        );
    }
    if offer.not_now {
        row = row.child(
            second("task-not-now", "Not now", IconName::MoonStar, p)
                .disabled(waits)
                .tooltip(format!("Not now: next week ({})", shortcut("L")))
                .on_click(cx.listener(|this, _, window, cx| this.press_not_now(window, cx))),
        );
    }
    if offer.archive {
        row = row.child(
            second("task-archive", "Archive", IconName::Archive, p)
                .disabled(waits)
                .tooltip(format!(
                    "Archive: put it away for good ({})",
                    archive_shortcut()
                ))
                .on_click(cx.listener(|this, _, window, cx| this.press_archive(window, cx))),
        );
    }
    let id = part.task.id;
    row = row.child(
        second("task-talk", "Talk", IconName::MessageCircle, p)
            .tooltip(format!("Talk to {} about this", crate::brand::APP_NAME))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.store
                    .update(cx, |store, cx| store.talk_about_task(id, cx));
                crate::shell::talk_shown(window, cx);
            })),
    );
    column.child(row)
}

/// A shortcut with the primary modifier: `⌘D` on the Mac, `Ctrl+D` elsewhere.
fn shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

/// Archive's shortcut: `⌃⌘A` on the Mac, `Ctrl+Alt+A` elsewhere.
fn archive_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌃⌘A"
    } else {
        "Ctrl+Alt+A"
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixtures::issue;
    use crate::wire::{ASSISTANT_OWNER, TASK_KEY_PREFIX};

    #[test]
    fn an_ask_with_its_options_on_the_page_leaves_them_out_of_the_bar() {
        let task = issue(
            1,
            "waiting",
            json!({"owner": ASSISTANT_OWNER, "decision": {
            "question": "Which?", "options": ["Done", "Report B", "Leave it"]}}),
        );
        let offer = offer(&task, AskOf::Own);
        assert_eq!(offer.primary, None);
        assert_eq!(offer.others.len(), 0);
        assert!(offer.not_now && offer.archive);
    }

    #[test]
    fn a_task_standing_for_another_offers_and_answers_that_ask() {
        let task = issue(
            1,
            "waiting",
            json!({"owner": ASSISTANT_OWNER, "action": format!("One yes on {TASK_KEY_PREFIX}2 for both")}),
        );
        let other = issue(
            2,
            "waiting",
            json!({"decision": {"question": "Ship?", "options": ["Done", "Report B", "Leave it"]}}),
        );
        let offer = offer(&task, AskOf::Other(&other));
        assert_eq!(offer.to, TaskId::new(2));
        assert_eq!(offer.primary, Some(Primary::Answer("Done".into())));
        assert_eq!(
            offer.primary.as_ref().map(Primary::label).as_deref(),
            Some("I did it")
        );
        assert_eq!(offer.others, ["Report B", "Leave it"]);
        let answered = super::offer(&task, AskOf::Gone);
        assert_eq!(answered.primary, None, "the ask it stood for is answered");
        let thing = issue(
            3,
            "waiting",
            json!({"action": format!("Log in on {TASK_KEY_PREFIX}4")}),
        );
        let to_do = issue(4, "waiting", json!({"action": "Log in"}));
        let offered = super::offer(&thing, AskOf::Other(&to_do));
        assert_eq!(
            (offered.to, offered.primary),
            (TaskId::new(4), Some(Primary::DidIt))
        );
    }

    #[test]
    fn without_an_ask_it_is_done_or_unarchived() {
        let mine = issue(1, "todo", json!({"owner": "owner"}));
        assert_eq!(offer(&mine, AskOf::Own).primary, Some(Primary::MarkDone));
        let theirs = issue(2, "working", json!({"owner": ASSISTANT_OWNER}));
        let offered = offer(&theirs, AskOf::Own);
        assert_eq!(offered.primary, None);
        assert!(!offered.not_now && offered.archive);
        let action = issue(
            3,
            "waiting",
            json!({"owner": ASSISTANT_OWNER, "action": "Log in to the bank"}),
        );
        assert_eq!(offer(&action, AskOf::Own).primary, Some(Primary::DidIt));
        let archived = issue(4, "canceled", json!({}));
        let offered = offer(&archived, AskOf::Own);
        assert_eq!(offered.primary, Some(Primary::Unarchive));
        assert!(!offered.archive && !offered.not_now);
        assert_eq!(
            offer(&issue(5, "done", json!({"owner": "owner"})), AskOf::Own).primary,
            None
        );
    }
}
