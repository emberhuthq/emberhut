//! The pending approval requests, every agent's or one agent's alone: what waits on the user
//! first, each with Approve… (which opens the request whole, its question asked under its exact
//! payload: Approve and Reject are only there); then what waits on an approver, read-only with
//! who that is. A row opens the request's page.

use emberhut_api::agents::AgentRow;
use emberhut_api::approvals::Approval;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Div, ElementId, Entity, FontWeight, ScrollHandle, div, px};
use jiff::Timestamp;

use super::parts::{chips, tint};
use crate::prose::direction;
use crate::store::approvals::lists::Groups;
use crate::store::approvals::shown::visible;
use crate::store::approvals::words::{amount_text, approver_role, kind_name, row_facts};
use crate::store::approvals::{AgentKey, Ask};
use crate::store::{Page, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::widgets::scroller;

/// The page: every agent's requests, or `filter`'s agent's alone.
pub(crate) fn page(
    store: &Entity<Store>,
    filter: Option<AgentKey>,
    scroll: &ScrollHandle,
    p: &Pal,
    cx: &App,
) -> Div {
    let read = store.read(cx);
    let feed = read.approvals();
    let agent = filter.and_then(|key| feed.agent_of(key));
    let name = agent.map(|id| {
        read.agents()
            .iter()
            .find(|row| row.id == *id)
            .map_or_else(|| id.to_string(), |row| visible(&row.name).into_owned())
    });
    let groups = Groups::of(read.pending_approvals(), agent);
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(18.))
        .px(px(24.))
        .py(px(18.))
        .max_w(px(760.));
    if groups.is_empty() {
        body = body.child(nothing(feed.has_read(), feed.problem(), name.as_deref(), p));
    } else {
        let now = Timestamp::now();
        let agents = read.agents();
        if !groups.yours.is_empty() {
            let mut section = div().flex().flex_col().gap(px(8.)).child(section_head(
                "Waiting on you",
                groups.yours.len(),
                p.accent,
            ));
            for approval in &groups.yours {
                section = section.child(row(store, read, approval, agents, now, p));
            }
            body = body.child(section);
        }
        if !groups.others.is_empty() {
            let mut section = div().flex().flex_col().gap(px(8.)).child(section_head(
                "Waiting on others",
                groups.others.len(),
                p.muted,
            ));
            for approval in &groups.others {
                section = section.child(row(store, read, approval, agents, now, p));
            }
            body = body.child(
                section.child(div().text_size(px(11.5)).text_color(p.muted).child(
                    "Their approver decides these, from Slack, a link or their own app. You \
                     see them so you know what is asked.",
                )),
            );
        }
        if let Some(problem) = feed.problem() {
            body = body.child(
                div()
                    .id("approvals-problem")
                    .announced(problem.to_owned())
                    .text_size(px(12.))
                    .text_color(p.danger)
                    .child(problem.to_owned()),
            );
        }
    }
    let title = name.map_or_else(
        || "Approvals".to_owned(),
        |name| format!("{name}: approvals"),
    );
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(head(&title, p))
        .child(scroller("approvals-scroll", scroll).child(body))
}

/// The page's title bar.
fn head(title: &str, p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(52.))
        .px(px(24.))
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(15.))
                .truncate()
                .child(title.to_owned()),
        )
}

/// A group's name with its count.
fn section_head(name: &str, count: usize, tint: gpui_kit::Hsla) -> Div {
    div()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(tint)
        .child(format!("{name} · {count}"))
}

/// Nothing waits: still looking, why the list didn't come, or that nothing waits (for the
/// agent named).
fn nothing(read: bool, problem: Option<&str>, agent: Option<&str>, p: &Pal) -> AnyElement {
    if let Some(problem) = problem {
        return div()
            .id("approvals-problem")
            .announced(problem.to_owned())
            .text_size(px(13.))
            .text_color(p.muted)
            .child(problem.to_owned())
            .into_any_element();
    }
    if !read {
        return div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_color(p.muted)
            .child(Spinner::new().small().color(p.muted))
            .child("Looking…")
            .into_any_element();
    }
    let line = agent.map_or_else(
        || {
            "When an agent asks before it posts, sends, merges, deploys or pays, the request \
             shows here with exactly what it would do."
                .to_owned()
        },
        |name| format!("When {name} asks before it acts, the request shows here with exactly what it would do."),
    );
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .py(px(40.))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Nothing waits for an OK"),
        )
        .child(div().text_size(px(12.5)).text_color(p.muted).child(line))
        .into_any_element()
}

/// One request: what it is for, the agent's line, where it goes, a payment's amount, its
/// facts; who decides it when that isn't the user, else Approve and Reject. A click on it
/// opens its page.
fn row(
    store: &Entity<Store>,
    read: &Store,
    approval: &Approval,
    agents: &[AgentRow],
    now: Timestamp,
    p: &Pal,
) -> Div {
    let yours = crate::store::approvals::lists::waits_on_you(approval);
    let id = approval.id;
    let amount = approval
        .amount_cents
        .map(|cents| amount_text(cents, approval.amount_currency.as_deref()));
    let facts = row_facts(approval, agents, now);
    let spoken = a11y::line(&[
        &kind_name(&approval.kind),
        &visible(&approval.summary),
        &format!("to {}", visible(&approval.target)),
        amount.as_deref().unwrap_or_default(),
        &facts,
    ]);
    let open = div()
        .id(ElementId::Name(format!("approval-{id}").into()))
        .button_named(spoken)
        .hint("Opens it with its exact payload")
        .border_1()
        .rounded(px(6.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .cursor_pointer()
        .child(chips(approval, p))
        .child(
            div()
                .text_size(px(13.5))
                .font_weight(FontWeight::SEMIBOLD)
                .line_clamp(3)
                .child(direction::marked(&visible(&approval.summary))),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted)
                .line_clamp(2)
                .child(format!("To {}", visible(&approval.target))),
        )
        .when_some(amount, |d, amount| {
            d.child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(amount),
            )
        })
        .child(div().text_size(px(11.)).text_color(p.faint).child(facts))
        .when(!yours, |d| {
            d.child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(format!("Waits on {}", approver_role(approval))),
            )
        })
        .on_click(move |_, window, cx| column::push(Page::Approval(id), window, cx));
    let feed = read.approvals();
    let notice = feed.notice(id).map(|notice| {
        div()
            .id(ElementId::Name(format!("approval-{id}-notice").into()))
            .announced(notice.text.clone())
            .text_size(px(11.5))
            .text_color(if notice.failed { p.danger } else { p.muted })
            .child(notice.text.clone())
    });
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(px(10.))
        .border_1()
        .border_color(if yours {
            tint(approval, p).opacity(0.6)
        } else {
            p.border
        })
        .bg(p.card)
        // Seen by the headless tests (a no-op outside them).
        .child(tab_stop(open, p).test_support())
        .when(yours, |d| {
            d.child(approve(store, approval, feed.busy().is_some(), p))
        })
        .children(notice)
}

/// Approve…, which opens the request whole with its question under the payload: a decision is
/// made only there, on everything the request covers, never on a row's clamped lines. Named
/// with the request, so a screen reader moving by buttons tells the rows' apart.
fn approve(store: &Entity<Store>, approval: &Approval, busy: bool, p: &Pal) -> Div {
    let id = approval.id;
    let (store, payload_hash) = (store.clone(), approval.payload_hash.clone());
    let name = format!(
        "Approve…: {}, {}",
        kind_name(&approval.kind),
        visible(&approval.summary)
    );
    let button = div()
        .id(ElementId::Name(format!("approval-{id}-approve").into()))
        .button_named(name)
        .hint("Opens it whole and asks first")
        .border_1()
        .px(px(10.))
        .py(px(3.))
        .rounded(px(6.))
        .bg(p.accent)
        .text_color(p.on_accent)
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .child("Approve…")
        .when(busy, |d| d.opacity(0.5))
        .when(!busy, |d| {
            d.cursor_pointer()
                .hover(|d| d.opacity(0.9))
                .on_click(move |_, window, cx| {
                    let ask = Ask::Approve {
                        payload_hash: payload_hash.clone(),
                    };
                    store.update(cx, |store, cx| store.ask_approval(Some((id, ask)), cx));
                    column::push(Page::Approval(id), window, cx);
                })
        });
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        // Seen by the headless tests (a no-op outside them).
        .child(tab_stop(button, p).test_support())
        .when(busy, |d| d.child(Spinner::new().small().color(p.muted)))
}
