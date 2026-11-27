//! Settings' front door: a big search field, focused as Settings opens; the groups as chips
//! that open their pages; and under them Suggested (a handful of plain verbs worked out from
//! how things stand), or, once something is typed, every setting that matches, its matched
//! letters marked and where it lives at the right. ↑ and ↓ move the highlight, Return opens
//! the highlighted row's page, Esc clears the field (a second Esc lets go of it). Nothing here
//! changes a setting: a row only opens where the setting is.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Escape, Input, MoveDown, MoveUp};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, ElementId, FontWeight, HighlightStyle, Role, SharedString, StyledText, div,
    px, relative,
};
use std::ops::Range;

use super::SettingsView;
use super::parts::{note, problem_line};
use crate::store::settings::entries::{Entry, Group};
use crate::store::settings::now::Now;
use crate::store::settings::search::{Found, search};
use crate::store::settings::suggest::suggestions;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::window::{CloseReport, RestoreReport};

/// The search field's placeholder.
pub(super) const PLACEHOLDER: &str = "What do you want to change?";

/// The id of the front door's list of rows.
pub(super) const ROWS: &str = "settings-rows";

/// A row of the front door's list: a suggestion or a search result.
#[derive(Clone, Debug)]
pub(super) struct Row {
    /// The setting it opens.
    pub(super) entry: &'static Entry,
    /// Its title, and the parts of it to mark.
    pub(super) title: (String, Vec<Range<usize>>),
    /// Its one line, and the parts of it to mark.
    pub(super) line: (String, Vec<Range<usize>>),
    /// Its icon.
    pub(super) icon: IconName,
}

/// What the list under the chips shows.
#[derive(Clone, Debug)]
pub(super) enum Listing {
    /// Nothing typed: the suggestions.
    Suggested(Vec<Row>),
    /// The settings that match what is typed.
    Found(Vec<Row>),
    /// Nothing matches: the closest few, if any.
    Nothing(Vec<Row>),
}

impl Listing {
    /// What shows for `query` now.
    #[must_use]
    pub(super) fn of(query: &str, now: &Now) -> Self {
        if query.trim().is_empty() {
            return Self::Suggested(
                suggestions(now)
                    .into_iter()
                    .map(|row| Row {
                        entry: row.entry,
                        title: (row.title, Vec::new()),
                        line: (row.line, Vec::new()),
                        icon: row.icon,
                    })
                    .collect(),
            );
        }
        let found: Found = search(query, now);
        let rows = found
            .rows()
            .iter()
            .map(|hit| Row {
                entry: hit.entry,
                title: (hit.entry.title.to_owned(), hit.title_marks.clone()),
                line: (hit.line.clone(), hit.line_marks.clone()),
                icon: hit.entry.icon,
            })
            .collect();
        if found.hits.is_empty() {
            Self::Nothing(rows)
        } else {
            Self::Found(rows)
        }
    }

    /// Its rows, in order.
    #[must_use]
    pub(super) fn rows(&self) -> &[Row] {
        match self {
            Self::Suggested(rows) | Self::Found(rows) | Self::Nothing(rows) => rows,
        }
    }
}

/// `text` with `marks` lit in the accent.
fn marked(text: String, marks: &[Range<usize>], p: &Pal) -> StyledText {
    let lit = HighlightStyle {
        background_color: Some(p.accent.opacity(0.28)),
        ..HighlightStyle::default()
    };
    StyledText::new(SharedString::from(text))
        .with_highlights(marks.iter().map(|range| (range.clone(), lit)))
}

impl SettingsView {
    /// The search field: ↑ and ↓ move the highlight, Return opens (the field's own event),
    /// Esc clears it.
    fn field(&self, p: &Pal, cx: &mut Context<'_, Self>) -> AnyElement {
        div()
            .id("settings-search")
            .key_context("SettingsSearch")
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                cx.stop_propagation();
                this.step(-1, cx);
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                cx.stop_propagation();
                this.step(1, cx);
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                cx.stop_propagation();
                this.escape(window, cx);
            }))
            // The window binds Esc to the report's Restore and Close too, ahead of the
            // field's own: while typing here, Esc is the search's.
            .capture_action(cx.listener(|this, _: &RestoreReport, window, cx| {
                cx.stop_propagation();
                this.escape(window, cx);
            }))
            .capture_action(cx.listener(|this, _: &CloseReport, window, cx| {
                cx.stop_propagation();
                this.escape(window, cx);
            }))
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(10.))
            .border_1()
            .border_color(p.border)
            .bg(p.card)
            .child(
                Icon::new(IconName::Search)
                    .size(px(18.))
                    .text_color(p.muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(17.))
                    .child(Input::new(&self.search).appearance(false)),
            )
            .test_support()
            .into_any_element()
    }

    /// The front door's content.
    pub(super) fn front_door(&self, now: &Now, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let listing = Listing::of(&self.query, now);
        let pick = self.pick.min(listing.rows().len().saturating_sub(1));
        let field = self.field(p, cx);
        let chips = div().flex().flex_wrap().gap(px(8.)).children(
            Group::ALL
                .into_iter()
                .filter(|group| group.shown(now))
                .map(|group| chip(group, group.badge(now), p, cx)),
        );
        let (heading, rows) = match &listing {
            Listing::Suggested(rows) => (Some("Suggested"), rows.as_slice()),
            Listing::Found(rows) | Listing::Nothing(rows) => (None, rows.as_slice()),
        };
        let list = (!rows.is_empty()).then(|| {
            div()
                .id(ROWS)
                .role(Role::ListBox)
                .aria_label("Settings")
                .flex()
                .flex_col()
                .p(px(4.))
                .rounded(px(12.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .children(
                    rows.iter()
                        .enumerate()
                        .map(|(index, row)| list_row(index, index == pick, row, p, cx)),
                )
                .test_support()
        });
        let nothing = matches!(listing, Listing::Nothing(_)).then(|| {
            div()
                .id("settings-nothing")
                .text_size(px(13.))
                .text_color(p.muted)
                .child("Nothing called that. Try a group above.")
                .test_support()
        });
        let problem = self
            .store
            .read(cx)
            .action_problem()
            .map(|problem| problem_line(p, problem.to_owned()));
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(field)
            .child(chips)
            .children(nothing)
            .children(heading.map(|heading| {
                div()
                    .px(px(4.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.muted)
                    .child(heading)
            }))
            .children(list)
            .child(note(p, "↑↓ to move · Return to open · or pick a group above").px(px(4.)))
            .children(problem)
    }
}

/// A row of the list: its icon, its title and line with the matches marked, where it lives;
/// lit when highlighted. A click opens it.
fn list_row(
    index: usize,
    picked: bool,
    row: &Row,
    p: &Pal,
    cx: &mut Context<'_, SettingsView>,
) -> AnyElement {
    let entry = row.entry;
    let (title, title_marks) = row.title.clone();
    let (line, line_marks) = row.line.clone();
    let label = a11y::line(&[&title, &line, &entry.path()]);
    div()
        .id(ElementId::Name(format!("settings-row-{}", entry.id).into()))
        .role(Role::ListBoxOption)
        .aria_selected(picked)
        .aria_label(label)
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .cursor_pointer()
        .when(picked, |d| d.bg(p.accent.opacity(0.14)))
        .when(!picked, |d| d.hover(|d| d.bg(p.hover)))
        .child(Icon::new(row.icon).size(px(16.)).text_color(p.muted))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .child(marked(title, &title_marks, p)),
                )
                .child(div().text_size(px(12.)).text_color(p.muted).child(marked(
                    line,
                    &line_marks,
                    p,
                ))),
        )
        .child(
            div()
                // Where it lives wraps in a third of the row, leaving the title its room.
                .max_w(relative(0.34))
                .text_right()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(entry.path()),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            this.pick = index;
            this.go(entry, window, cx);
        }))
        .test_support()
        .into_any_element()
}

/// A group's chip: it opens the group's page; what needs a look there counted on it.
fn chip(
    group: Group,
    badge: Option<usize>,
    p: &Pal,
    cx: &mut Context<'_, SettingsView>,
) -> AnyElement {
    let name = group.name();
    tab_stop(
        div()
            .id(ElementId::Name(
                format!("settings-chip-{}", group.id()).into(),
            ))
            .button_named(match badge {
                Some(count) => format!("{name}, {count} need a look"),
                None => name.to_owned(),
            })
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(12.))
            .py(px(5.))
            .rounded_full()
            .border_1()
            .bg(p.card)
            .text_size(px(13.))
            .cursor_pointer()
            .hover(|d| d.bg(p.hover))
            .active(|d| d.bg(p.accent.opacity(0.16)))
            .child(name)
            .children(badge.map(|count| {
                div()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::BOLD)
                    .text_color(p.accent)
                    .child(count.to_string())
            }))
            .child(
                Icon::new(IconName::ChevronRight)
                    .size(px(11.))
                    .text_color(p.faint),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.open_group(group, window, cx))),
        p,
    )
    .border_color(p.border)
    .test_support()
    .into_any_element()
}
