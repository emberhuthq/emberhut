//! An approval request's page: its exact payload, nothing summarised away (the agent and who
//! it works for, what it is for, where it goes, a payment's amount with its currency, the text
//! word for word in a fixed-width selectable block, each file's name, size and the start of its
//! SHA-256), who decides it, and how it ended once it has.
//!
//! When it waits on the user: Approve, asked first under the payload, and Reject, each sending
//! the payload hash of the request on screen; a request that changed in between decides nothing
//! and shows again as it is, with a line that says so. A waiting request, or an approved one
//! not used yet, can be withdrawn, asked first too. A request that can't be read says why, with
//! Retry, or that it isn't there.

use emberhut_api::approvals::{Approval, ApprovalState, Decision};
use emberhut_api::ids::ApprovalId;
use gpui_kit::TestSupportExt as _;
use gpui_kit::base::SelectableText;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, Entity, FontWeight, Role, ScrollHandle, div, px};
use jiff::Timestamp;

use super::parts::{ONLY_YOU, chips, tint};
use crate::store::approvals::lists::waits_on_you;
use crate::store::approvals::shown::visible;
use crate::store::approvals::words::{
    agent_name, amount_text, approve_question, approver_role, file_line, kind_name, outcome_lines,
    payload_foot, withdraw_question,
};
use crate::store::approvals::{Ask, Notice, Unread};
use crate::store::{Page, Store};
use crate::ui::a11y::Accessible as _;
use crate::ui::column;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::widgets::scroller;

/// The page of request `id`.
pub(crate) fn page(
    store: &Entity<Store>,
    id: ApprovalId,
    scroll: &ScrollHandle,
    p: &Pal,
    cx: &App,
) -> Div {
    let read = store.read(cx);
    let body = div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .px(px(24.))
        .py(px(18.))
        .max_w(px(720.));
    let body = match read.approval(id) {
        Some(approval) => body.children(sections(store, read, approval, p)),
        None => body.child(nothing(store, id, read.approvals().unread(id), p)),
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(scroller("approval-scroll", scroll).child(body))
}

/// The page's sections, top to bottom.
fn sections(store: &Entity<Store>, read: &Store, approval: &Approval, p: &Pal) -> Vec<AnyElement> {
    let now = Timestamp::now();
    let feed = read.approvals();
    let mut parts = Vec::new();
    if let Some(notice) = feed.notice(approval.id) {
        parts.push(notice_line(approval.id, notice, p).into_any_element());
    }
    parts.push(payload(read, approval, now, p).into_any_element());
    if approval.state != ApprovalState::Pending {
        parts.push(outcome(approval, now, p).into_any_element());
    }
    if waits_on_you(approval) {
        parts.push(decide(store, read, approval, p).into_any_element());
    } else if approval.state == ApprovalState::Pending {
        parts.push(
            note(
                "This waits on its approver, not on you: you see it so you know what is asked.",
                p,
            )
            .into_any_element(),
        );
    }
    if let Some(task) = approval.task {
        parts.push(task_link(read, task, p).into_any_element());
    }
    if can_withdraw(approval) {
        parts.push(withdraw(store, read, approval, p).into_any_element());
    }
    parts
}

/// Whether the request can still be taken back: while it waits, and once approved until it is
/// used (an imported one never can).
fn can_withdraw(approval: &Approval) -> bool {
    approval.state == ApprovalState::Pending
        || (approval.state == ApprovalState::Approved && !approval.imported)
}

/// Nothing to show yet: still looking, the request isn't there, or why it couldn't be read,
/// with Retry.
fn nothing(store: &Entity<Store>, id: ApprovalId, unread: Option<&Unread>, p: &Pal) -> AnyElement {
    match unread {
        None => div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_color(p.muted)
            .child(Spinner::new().small().color(p.muted))
            .child("Looking…")
            .into_any_element(),
        Some(Unread::Gone) => div()
            .id("approval-gone")
            .announced("This request isn't there")
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .py(px(40.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("This request isn't there"),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .child("The server has no request with this number."),
            )
            .into_any_element(),
        Some(Unread::Failed(why)) => {
            let store = store.clone();
            div()
                .id("approval-unread")
                .announced(why.clone())
                .flex()
                .flex_col()
                .items_start()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(p.danger)
                        .child(why.clone()),
                )
                .child(
                    Button::new("approval-retry")
                        .small()
                        .label("Retry")
                        .on_click(move |_, _, cx| {
                            store.update(cx, |store, cx| store.retry_approval(id, cx));
                        }),
                )
                .into_any_element()
        }
    }
}

/// The line that says what became of the user's last answer.
fn notice_line(id: ApprovalId, notice: &Notice, p: &Pal) -> impl IntoElement {
    div()
        .id(ElementId::Name(format!("approval-{id}-notice").into()))
        .announced(notice.text.clone())
        .p(px(10.))
        .rounded(px(8.))
        .text_size(px(12.5))
        .when(notice.failed, |d| {
            d.bg(p.danger.opacity(0.1)).text_color(p.danger)
        })
        .when(!notice.failed, |d| d.bg(p.hover).text_color(p.text))
        .child(notice.text.clone())
}

/// A small muted line.
fn note(text: &'static str, p: &Pal) -> Div {
    div().text_size(px(11.5)).text_color(p.muted).child(text)
}

/// A label over its value.
fn field(label: &'static str, value: impl IntoElement, p: &Pal) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(label),
        )
        .child(value)
}

/// Text the user can select and copy, as it is but for what would hide in it, written out
/// ([`visible`]).
fn plain(id: ApprovalId, part: &str, text: &str) -> SelectableText {
    SelectableText::new(
        ElementId::Name(format!("approval-{id}-{part}").into()),
        visible(text).into_owned(),
    )
}

/// The exact payload: its chips and one line, the agent, where it goes, the amount, the text,
/// the files, who decides, and the foot with the payload's own hash.
fn payload(read: &Store, approval: &Approval, now: Timestamp, p: &Pal) -> Div {
    let id = approval.id;
    let agents = read.agents();
    let agent = agents.iter().find(|row| row.id == approval.agent);
    let who = match agent {
        Some(agent) => format!(
            "{} · works for {}",
            agent_name(approval, agents),
            agent.principal
        ),
        None => agent_name(approval, agents).into_owned(),
    };
    let mut card = div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(14.))
        .rounded(px(10.))
        .border_1()
        .border_color(if waits_on_you(approval) {
            tint(approval, p).opacity(0.6)
        } else {
            p.border
        })
        .bg(p.card)
        .child(chips(approval, p))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(plain(id, "summary", &approval.summary)),
        )
        .child(field(
            "Agent",
            div().text_size(px(13.)).child(plain(id, "agent", &who)),
            p,
        ))
        .child(field(
            "What",
            div().text_size(px(13.)).child(kind_name(&approval.kind)),
            p,
        ))
        .child(field(
            "To",
            div()
                .text_size(px(13.))
                .child(plain(id, "target", &approval.target)),
            p,
        ));
    if let Some(cents) = approval.amount_cents {
        let amount = amount_text(cents, approval.amount_currency.as_deref());
        card = card.child(field(
            "Amount",
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(plain(id, "amount", &amount)),
            p,
        ));
    }
    card = text_and_files(card, approval, p);
    card.child(field(
        "Decides",
        div().text_size(px(13.)).child(approver_role(approval)),
        p,
    ))
    .when(
        approval.kind.is_fixed_core() && waits_on_you(approval),
        |d| {
            d.child(div().text_size(px(12.)).text_color(p.danger).child(format!(
                "{ONLY_YOU}: payments, passwords and codes, secrets, account security and \
                     anything in your name are never anyone else's to approve."
            )))
        },
    )
    .child(div().text_size(px(11.)).text_color(p.faint).child(plain(
        id,
        "foot",
        &payload_foot(approval, now),
    )))
}

/// `card` with the request's exact text, word for word in a fixed-width selectable block, and
/// each of its files' lines.
fn text_and_files(mut card: Div, approval: &Approval, p: &Pal) -> Div {
    let id = approval.id;
    if let Some(text) = &approval.text {
        card = card.child(field(
            "Exact text",
            div()
                .id(ElementId::Name(format!("approval-{id}-text-block").into()))
                .read_as(Role::Document, format!("Exact text: {}", visible(text)))
                .p(px(10.))
                .rounded(px(8.))
                .bg(p.surface_raised)
                .border_1()
                .border_color(p.border)
                .text_size(px(12.5))
                .font_family("monospace")
                .child(plain(id, "text", text)),
            p,
        ));
    }
    if !approval.files.is_empty() {
        let label = if approval.files.len() == 1 {
            "File"
        } else {
            "Files"
        };
        let mut lines = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(12.))
            .font_family("monospace");
        for (at, file) in approval.files.iter().enumerate() {
            lines = lines.child(plain(id, &format!("file-{at}"), &file_line(file)));
        }
        card = card.child(field(label, lines, p));
    }
    card
}

/// How it ended, or where it stands.
fn outcome(approval: &Approval, now: Timestamp, p: &Pal) -> Div {
    let lines = outcome_lines(approval, now).join("\n");
    div()
        .text_size(px(12.5))
        .text_color(p.muted)
        .child(plain(approval.id, "outcome", &lines))
}

/// Approve (asked first) and Reject, each on the payload hash shown; the question under the
/// payload once asked.
fn decide(store: &Entity<Store>, read: &Store, approval: &Approval, p: &Pal) -> Div {
    let feed = read.approvals();
    let id = approval.id;
    let busy = feed.busy().is_some();
    let (ask, reject) = (store.clone(), store.clone());
    let (hash_ask, hash_reject) = (approval.payload_hash.clone(), approval.payload_hash.clone());
    let mut buttons = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(
            Button::new("approval-approve")
                .small()
                .primary()
                .label("Approve")
                .disabled(busy)
                .on_click(move |_, _, cx| {
                    let ask_this = Ask::Approve {
                        payload_hash: hash_ask.clone(),
                    };
                    ask.update(cx, |store, cx| store.ask_approval(Some((id, ask_this)), cx));
                }),
        )
        .child(
            Button::new("approval-reject")
                .small()
                .danger()
                .label("Reject")
                .disabled(busy)
                .on_click(move |_, _, cx| {
                    let hash = hash_reject.clone();
                    reject.update(cx, |store, cx| {
                        store.decide_approval(id, Decision::Reject, hash, cx);
                    });
                }),
        );
    if feed.busy() == Some(id) {
        buttons = buttons.child(
            div()
                .id("approval-busy")
                .announced("Sending…")
                .child(Spinner::new().small().color(p.muted)),
        );
    }
    let mut column = div().flex().flex_col().gap(px(8.)).child(buttons);
    // The question shows only while the request on screen is the one it was asked on: a
    // payload that changed under it needs a new yes.
    if let Some(Ask::Approve { payload_hash }) = feed.asking(id)
        && *payload_hash == approval.payload_hash
    {
        // Seen by the headless tests (a no-op outside them).
        column = column.child(approve_dialog(store, read, approval, p).test_support());
    }
    column.child(note(
        "Approving covers exactly what is shown here, once. A changed word, file or target \
         needs a new OK.",
        p,
    ))
}

/// The question before an Approve: who may then do what, for how much and where; Approve and
/// Cancel.
fn approve_dialog(
    store: &Entity<Store>,
    read: &Store,
    approval: &Approval,
    p: &Pal,
) -> gpui_kit::Stateful<Div> {
    let (title, words) = approve_question(approval, read.agents());
    let (yes, no) = (store.clone(), store.clone());
    let (id, hash) = (approval.id, approval.payload_hash.clone());
    div()
        .id("approval-question")
        .read_as(Role::AlertDialog, title.clone())
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(8.))
        .bg(tint(approval, p).opacity(0.08))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(div().text_size(px(12.)).text_color(p.muted).child(words))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    Button::new("approval-confirm")
                        .small()
                        .primary()
                        .label("Approve")
                        .on_click(move |_, _, cx| {
                            let hash = hash.clone();
                            yes.update(cx, |store, cx| {
                                store.decide_approval(id, Decision::Approve, hash, cx);
                            });
                        }),
                )
                .child(
                    Button::new("approval-cancel")
                        .small()
                        .ghost()
                        .label("Cancel")
                        .on_click(move |_, _, cx| {
                            no.update(cx, |store, cx| store.ask_approval(None, cx));
                        }),
                ),
        )
}

/// The task whose step asked: a click opens its page.
fn task_link(read: &Store, task: emberhut_api::ids::TaskId, p: &Pal) -> impl IntoElement {
    let name = format!("From {}", read.task_key(task));
    tab_stop(
        div()
            .id("approval-task")
            .button_named(name.clone())
            .hint("Opens its page")
            .border_1()
            .rounded(px(6.))
            .text_size(px(13.))
            .text_color(p.accent)
            .cursor_pointer()
            .child(name)
            .on_click(move |_, window, cx| column::push(Page::Task(task), window, cx)),
        p,
    )
}

/// Withdraw, asked first: nothing the request covers goes out (an approval not used yet is
/// taken back).
fn withdraw(store: &Entity<Store>, read: &Store, approval: &Approval, p: &Pal) -> Div {
    let feed = read.approvals();
    let id = approval.id;
    let ask = store.clone();
    let mut column = div().flex().flex_col().gap(px(8.)).child(
        div().child(
            Button::new("approval-withdraw")
                .small()
                .ghost()
                .label("Withdraw request")
                .tooltip("Withdraw this request")
                .disabled(feed.busy().is_some())
                .on_click(move |_, _, cx| {
                    ask.update(cx, |store, cx| {
                        store.ask_approval(Some((id, Ask::Withdraw)), cx);
                    });
                }),
        ),
    );
    if feed.asking(id) == Some(&Ask::Withdraw) {
        let (title, words) = withdraw_question();
        let (yes, no) = (store.clone(), store.clone());
        column = column.child(
            div()
                .id("approval-withdraw-question")
                .read_as(Role::AlertDialog, title.clone())
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(p.danger.opacity(0.08))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().text_size(px(12.)).text_color(p.muted).child(words))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            Button::new("approval-withdraw-confirm")
                                .small()
                                .danger()
                                .label("Withdraw")
                                .on_click(move |_, _, cx| {
                                    yes.update(cx, |store, cx| store.withdraw_approval(id, cx));
                                }),
                        )
                        .child(
                            Button::new("approval-withdraw-cancel")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .on_click(move |_, _, cx| {
                                    no.update(cx, |store, cx| store.ask_approval(None, cx));
                                }),
                        ),
                ),
        );
    }
    column
}
