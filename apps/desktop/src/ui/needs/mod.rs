//! Needs you: the asks waiting on the user, as a board (a column per project, or per when they
//! have to be decided) or a list in sections, narrowed by the bar's filters, then Later folded
//! after them. A card opens its task's page; its quick answers sit on it. Agents' pending
//! approval requests come first, as one row that opens their list.

mod bar;
mod board;
mod card;

pub(crate) use bar::segmented;
pub(crate) use card::decide_chip as chip;

use emberhut_api::ids::TaskId;
use emberhut_api::state::State;
use emberhut_api::work::WorkRow;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, ElementId, Entity, FontWeight, ScrollHandle, Stateful, Window, div, px,
};
use jiff::Timestamp;

use super::a11y::{self, Accessible as _};
use super::column;
use super::controls::tab_stop;
use super::palette::{Pal, pal};
use super::widgets::{icon, scroller};
use super::window::MainWindow;
use crate::store::lookup;
use crate::store::need_card::CardContext;
use crate::store::needs::Needs;
use crate::store::needs_view::{Grouping, Layout, NeedsFilter};
use crate::store::tasks::filter::When;
use crate::store::{Page, Store};

/// Why Later waits, under its title.
const LATER_NOTE: &str =
    "More than a month out, or in a paused project: not counted, so it waits quietly";
/// [`LATER_NOTE`] with the projects screens hidden: no paused projects to speak of.
const LATER_NOTE_NO_PROJECTS: &str = "More than a month out: not counted, so it waits quietly";

/// Why Later waits, as the page words it.
fn later_note(state: &State) -> &'static str {
    if lookup::shows_tasks(state) {
        LATER_NOTE_NO_PROJECTS
    } else {
        LATER_NOTE
    }
}

/// What the page shows over one state: the asks the filter keeps, how many it hides.
struct Shown<'a> {
    needs: Needs<'a>,
    state: &'a State,
    /// Every ask waiting on the user, Later included.
    all: Vec<&'a WorkRow>,
    /// The ones the filter keeps.
    kept: Vec<&'a WorkRow>,
    filter: NeedsFilter,
}

impl<'a> Shown<'a> {
    fn of(store: &'a Store) -> Option<Self> {
        let needs = store.needs()?;
        let state = store.state()?;
        let view = store.needs_view();
        let filter = view
            .filter()
            .shown(lookup::shows_tasks(state), view.layout());
        let all = needs.rows();
        let kept = all
            .iter()
            .copied()
            .filter(|row| filter.keeps(row, &needs, state))
            .collect();
        Some(Self {
            needs,
            state,
            all,
            kept,
            filter,
        })
    }

    /// Whether the server hides the projects screens: cards name their project only where the
    /// page is grouped by it.
    fn hides_projects(&self) -> bool {
        lookup::shows_tasks(self.state)
    }

    /// How many asks count (Later doesn't).
    fn total(&self) -> usize {
        self.all
            .iter()
            .filter(|row| !self.needs.is_later(row))
            .count()
    }

    /// The cards' words, `project` named where the cards aren't grouped by it, with tiers on
    /// the chip where they are grouped by project.
    fn cards(&self, project: bool, tiers: bool) -> CardContext<'_> {
        CardContext {
            state: self.state,
            needs: &self.needs,
            now: Timestamp::now(),
            project,
            tiers,
        }
    }
}

/// The page.
pub(crate) fn page(
    window: &mut MainWindow,
    scroll: &ScrollHandle,
    cx: &mut Context<'_, MainWindow>,
) -> Div {
    let p = pal(cx);
    let store_entity = window.store.clone();
    let store = window.store.read(cx);
    let shown = Shown::of(store);
    let view = store.needs_view();
    let (layout, grouping) = (view.layout(), view.grouping());
    // The approval requests come first, as one row opening their list: each is an agent's
    // exact payload to read whole before deciding, which a card here couldn't hold. Those
    // waiting on the user count with the asks, as on the sidebar.
    let approvals = store.approvals_summary();
    let head = bar::head(
        &p,
        &store_entity,
        shown.as_ref().map_or(0, Shown::total) + approvals.yours,
        layout,
    );
    let approvals_row = approvals.any().then(|| {
        div()
            .px(px(20.))
            .pt(px(14.))
            .max_w(px(900.))
            .child(super::approvals::needs_row(approvals, &p))
    });
    let mut page = div()
        .size_full()
        .flex()
        .flex_col()
        .child(head)
        .children(approvals_row);
    let Some(shown) = shown.filter(|shown| !shown.all.is_empty()) else {
        return if approvals.yours > 0 {
            page
        } else {
            page.child(empty(&p, true))
        };
    };
    page = page.child(bar::filters(
        &p,
        &store_entity,
        &shown,
        layout,
        grouping,
        cx,
    ));
    let body = if shown.kept.is_empty() {
        empty(&p, false).into_any_element()
    } else {
        match layout {
            Layout::Board => board::board(&p, &store_entity, &shown, grouping, cx),
            Layout::List => scroller("needs-scroll", scroll)
                .child(list(&p, &store_entity, &shown, grouping, cx))
                .into_any_element(),
        }
    };
    page.child(div().flex_1().min_h_0().flex().flex_col().child(body))
}

/// Nothing waits (`unfiltered`), or nothing the filter keeps.
fn empty(p: &Pal, unfiltered: bool) -> Div {
    let (title, line) = if unfiltered {
        (
            "Nothing needs you",
            format!(
                "When {} needs a decision, or something only you can do, it shows here.",
                crate::brand::APP_NAME
            ),
        )
    } else {
        (
            "Nothing here for this filter",
            "Take a filter off to see the rest.".to_owned(),
        )
    };
    div()
        .pt(px(80.))
        .px(px(24.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.))
        .text_color(p.muted)
        .child(icon(IconName::CircleCheck, p.success, 28.))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text)
                .child(title),
        )
        .child(div().text_size(px(12.5)).child(line))
}

/// The list: a section per project (or per time), then Later folded, then how many the filter
/// hides.
fn list(
    p: &Pal,
    store: &Entity<Store>,
    shown: &Shown<'_>,
    grouping: Grouping,
    cx: &Context<'_, MainWindow>,
) -> Div {
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(20.))
        .px(px(20.))
        .py(px(16.))
        .max_w(px(900.));
    let by_project = grouping == Grouping::Project;
    let sections: Vec<(String, bool, Vec<&WorkRow>)> = if by_project {
        shown
            .needs
            .by_project_of(shown.kept.clone())
            .into_iter()
            .map(|group| (group.name, group.project.is_some(), group.rows))
            .collect()
    } else {
        shown
            .needs
            .by_when_of(&shown.kept)
            .into_iter()
            .map(|(when, rows)| (when.name().to_owned(), when != When::ThisMonth, rows))
            .collect()
    };
    let cards = shown.cards(!by_project && !shown.hides_projects(), by_project);
    for (name, strong, rows) in sections {
        let mut section =
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(section_head(p, &name, rows.len(), strong));
        for row in rows {
            section = section.child(card::card(p, store, &cards.card(row), row, cx));
        }
        body = body.child(section);
    }
    let later = shown.needs.later_of(shown.kept.clone());
    if !later.is_empty() {
        body = body.child(later_fold(p, store, shown, &later, by_project, cx));
    }
    let hidden = shown.all.len() - shown.kept.len();
    if hidden > 0 {
        let store = store.clone();
        body = body.child(
            div()
                .id("needs-hidden")
                .button_named(format!("{hidden} more hidden by the filter. Show all"))
                .text_size(px(12.))
                .text_color(p.muted)
                .cursor_pointer()
                .child(format!("{hidden} more hidden by the filter · Show all"))
                .on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| {
                        store.set_needs_filter(NeedsFilter::default(), cx);
                    });
                }),
        );
    }
    body
}

/// A section's name with its count in a capsule, in the accent (softer for Other and This
/// month).
fn section_head(p: &Pal, name: &str, count: usize, strong: bool) -> Div {
    let tint = if strong {
        p.accent
    } else {
        p.accent.opacity(0.6)
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(13.))
                .text_color(tint)
                .child(name.to_owned()),
        )
        .child(
            div()
                .px(px(8.))
                .rounded_full()
                .bg(tint)
                .text_color(p.on_accent)
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child(count.to_string()),
        )
}

/// Later, after the sections: its title with its count and why it waits, which opens and
/// folds it; open, its cards (naming their project).
fn later_fold(
    p: &Pal,
    store: &Entity<Store>,
    shown: &Shown<'_>,
    later: &[&WorkRow],
    by_project: bool,
    cx: &Context<'_, MainWindow>,
) -> Div {
    let open = store.read(cx).is_later_open();
    let toggle = store.clone();
    let title = div()
        .id("later")
        // Read as Later and its count, open or folded.
        .button_named(format!("Later, {}", later.len()))
        .aria_expanded(open)
        .aria_value(a11y::folded(open))
        .flex()
        .items_center()
        .gap(px(8.))
        .cursor_pointer()
        .child(icon(
            if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            },
            p.muted,
            12.,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_size(px(13.))
                        .child(format!("Later · {}", later.len())),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(later_note(shown.state)),
                ),
        )
        .on_click(move |_, _, cx| toggle.update(cx, Store::toggle_later));
    let title = tab_stop(title.border_1().rounded(px(6.)), p);
    let mut column = div().flex().flex_col().gap(px(8.)).child(title);
    if open {
        // Not grouped by project, so each names its own (even with projects hidden, when the
        // page is grouped by project).
        let cards = shown.cards(by_project || !shown.hides_projects(), false);
        for row in later {
            column = column.child(card::card(p, store, &cards.card(row), row, cx));
        }
    }
    column
}

/// Opens `task`'s page in the column, over Needs you (Back returns here).
fn open_task(task: TaskId, window: &mut Window, cx: &mut gpui_kit::App) {
    column::push(Page::Task(task), window, cx);
}

/// `card`, made a link to the thread of `row` when the row has one and no task: a click opens
/// the thread in the column over Needs you (its session on a runner, else what it was asked
/// and said), and Back returns here.
fn thread_card(card: Stateful<Div>, store: &Entity<Store>, row: &WorkRow, p: &Pal) -> AnyElement {
    if row.task.is_some() || row.thread.is_none() {
        return card.into_any_element();
    }
    let (store, id) = (store.clone(), row.id.clone());
    tab_stop(card, p)
        .on_click(move |_, window, cx| {
            crate::ui::sessions::push(
                &store,
                crate::ui::sessions::Open::Place(
                    crate::store::sessions::places::SessionPlace::Work(id.clone()),
                ),
                window,
                cx,
            );
        })
        // Seen by the headless tests (a no-op outside them).
        .test_support()
        .into_any_element()
}

/// A card's id for the element tree.
fn card_id(prefix: &str, row: &WorkRow) -> ElementId {
    ElementId::Name(format!("{prefix}-{}", row.id).into())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::*;
    use crate::fixtures::{state, waiting};

    /// A card of `row` alone in a window, as [`thread_card`] makes it.
    struct Probe {
        store: Entity<Store>,
        row: WorkRow,
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let p = pal(cx);
            thread_card(
                div().id(card_id("need", &self.row)).size_full(),
                &self.store,
                &self.row,
                &p,
            )
        }
    }

    #[gpui_kit::test]
    fn a_card_for_a_thread_with_no_task_opens_the_thread(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        use gpui_kit::test::TestWindowExt as _;

        use crate::store::sessions::places::SessionPlace;

        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = std::sync::Arc::new(crate::backend::fake::FakeBackend::new([]))
            as std::sync::Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        let row = waiting(1, json!({"kind": "thread", "task": null, "thread": "t-1"}));
        store.update(cx, |store, cx| {
            store.show_state_for_tests(state(json!({"working": [row.clone()]})), cx);
        });
        let mut column = None;
        let window = cx.add_window(|window, cx| {
            column = Some(column::Column::new(store.clone(), window, cx));
            Probe {
                store: store.clone(),
                row: row.clone(),
            }
        });
        let column = column.unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(card_id("need", &row), cx);
        })
        .unwrap();
        cx.run_until_parked();
        let shown = column.read_with(cx, |column, _| column.current());
        let Some(Page::Session(key)) = shown else {
            panic!("the thread's page opens, not {shown:?}");
        };
        store.read_with(cx, |store, _| {
            assert_eq!(
                store.session_place(key),
                Some(&SessionPlace::Work(row.id.clone()))
            );
        });
    }

    #[test]
    fn later_says_why_it_waits() {
        let hidden = state(json!({}));
        assert_eq!(later_note(&hidden), LATER_NOTE_NO_PROJECTS);
        let shown = state(json!({"ui": {"hideProjects": false, "hideGoals": false}}));
        assert_eq!(later_note(&shown), LATER_NOTE);
    }

    #[test]
    fn the_head_counts_what_counts() {
        let lists = state(json!({
            "working": [waiting(1, json!({})), waiting(2, json!({"paused": true}))],
        }));
        let gone = HashSet::new();
        let needs = Needs::new(&lists, &gone, jiff::civil::date(2026, 11, 25));
        let all = needs.rows();
        let shown = Shown {
            needs,
            state: &lists,
            kept: all.clone(),
            all,
            filter: NeedsFilter::default(),
        };
        assert_eq!(shown.total(), 1, "Later counts nowhere");
    }
}
