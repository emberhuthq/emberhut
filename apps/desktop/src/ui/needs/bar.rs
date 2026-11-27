//! The bars over Needs you: the head with its count and the List / Board switch, and under it
//! the Project / Time grouping and the filters. Each filter on is a filled chip with its value
//! and an ✕; each other one opens its choices, only those the asks have (so a pick never comes
//! up empty); Clear from two. Project and area are offered only while projects show, when only
//! on the list.

use emberhut_api::state::State;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, ElementId, Entity, FontWeight, SharedString, div, px};

use super::{MainWindow, Shown};
use crate::store::Store;
use crate::store::needs_view::{Grouping, Layout, NeedsFilter, areas_of, projects_of};
use crate::store::tasks::filter::{OwnerPick, ProjectPick, When};
use crate::ui::a11y::Accessible as _;
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::glyph;

/// A segmented pick: each choice a small button, the one on filled.
pub(crate) fn segmented<T: Copy + PartialEq + 'static>(
    p: &Pal,
    id: &'static str,
    choices: &[(T, &'static str)],
    on: T,
    pick: impl Fn(T, &mut App) + Clone + 'static,
) -> Div {
    let mut row = div()
        .flex()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(7.))
        .bg(p.hover);
    for (i, &(choice, name)) in choices.iter().enumerate() {
        let picked = choice == on;
        let pick = pick.clone();
        row = row.child(
            div()
                .id(ElementId::Name(format!("{id}-{i}").into()))
                .button_named(name)
                .aria_selected(picked)
                .px(px(10.))
                .py(px(3.))
                .rounded(px(5.))
                .text_size(px(12.))
                .font_weight(if picked {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .when(picked, |d| d.bg(p.surface_raised))
                .text_color(if picked { p.text } else { p.muted })
                .cursor_pointer()
                .child(name)
                .on_click(move |_, _, cx| pick(choice, cx)),
        );
    }
    row
}

/// The head: the title, how many wait, and List / Board.
pub(super) fn head(p: &Pal, store: &Entity<Store>, total: usize, layout: Layout) -> Div {
    let store = store.clone();
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(52.))
        .px(px(20.))
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(15.))
                .child("Needs you"),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(12.))
                .text_color(p.muted)
                .child(format!("{total} waiting on you")),
        )
        .child(segmented(
            p,
            "needs-layout",
            &[(Layout::List, "List"), (Layout::Board, "Board")],
            layout,
            move |layout, cx| store.update(cx, |store, cx| store.set_needs_layout(layout, cx)),
        ))
}

/// One filter of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Facet {
    Project,
    Area,
    Owner,
    When,
}

impl Facet {
    fn name(self) -> &'static str {
        match self {
            Self::Project => "Project",
            Self::Area => "Area",
            Self::Owner => "Owner",
            Self::When => "Decide by",
        }
    }

    fn is_on(self, filter: &NeedsFilter) -> bool {
        match self {
            Self::Project => filter.project.is_some(),
            Self::Area => filter.area.is_some(),
            Self::Owner => filter.owner.is_some(),
            Self::When => filter.when.is_some(),
        }
    }

    fn cleared(self, filter: &NeedsFilter) -> NeedsFilter {
        let mut filter = filter.clone();
        match self {
            // An area belongs to its project: it goes with it.
            Self::Project => {
                filter.project = None;
                filter.area = None;
            }
            Self::Area => filter.area = None,
            Self::Owner => filter.owner = None,
            Self::When => filter.when = None,
        }
        filter
    }
}

/// A project's name for the bar.
fn project_name(state: &State, pick: ProjectPick) -> String {
    match pick {
        ProjectPick::NoProject => "No project".to_owned(),
        ProjectPick::Project(id) => state
            .projects
            .iter()
            .find(|project| project.id == id)
            .map_or_else(|| format!("Project {id}"), |project| project.name.clone()),
    }
}

/// The value an active filter's chip shows.
fn value(facet: Facet, filter: &NeedsFilter, state: &State) -> String {
    match facet {
        Facet::Project => filter
            .project
            .map_or_default(|pick| project_name(state, pick)),
        Facet::Area => filter.area.map_or_default(|id| {
            state
                .areas
                .iter()
                .find(|area| area.id == id)
                .map_or_else(|| format!("Area {id}"), |area| area.name.clone())
        }),
        Facet::Owner => filter.owner.map_or_default(OwnerPick::name),
        Facet::When => filter.when.map_or_default(|when| when.name().to_owned()),
    }
}

/// The choices a facet offers over `shown`'s asks, each the filter it makes.
fn choices(facet: Facet, filter: &NeedsFilter, shown: &Shown<'_>) -> Vec<(String, NeedsFilter)> {
    let with = |change: &dyn Fn(&mut NeedsFilter)| {
        let mut next = filter.clone();
        change(&mut next);
        next
    };
    match facet {
        Facet::Project => projects_of(&shown.all)
            .into_iter()
            .map(|pid| {
                let pick = pid.map_or(ProjectPick::NoProject, ProjectPick::Project);
                (
                    project_name(shown.state, pick),
                    with(&|f| {
                        f.project = Some(pick);
                        f.area = None;
                    }),
                )
            })
            .collect(),
        Facet::Area => areas_of(&shown.all, shown.state)
            .into_iter()
            .map(|id| {
                (
                    value(
                        Facet::Area,
                        &NeedsFilter {
                            area: Some(id),
                            ..NeedsFilter::default()
                        },
                        shown.state,
                    ),
                    with(&|f| f.area = Some(id)),
                )
            })
            .collect(),
        Facet::Owner => OwnerPick::ALL
            .iter()
            .map(|&pick| (pick.name(), with(&|f| f.owner = Some(pick))))
            .collect(),
        Facet::When => [When::Today, When::ThisWeek, When::ThisMonth, When::Later]
            .iter()
            .map(|&when| (when.name().to_owned(), with(&|f| f.when = Some(when))))
            .collect(),
    }
}

/// The bar under the head: the grouping, then the filters.
pub(super) fn filters(
    p: &Pal,
    store: &Entity<Store>,
    shown: &Shown<'_>,
    layout: Layout,
    grouping: Grouping,
    cx: &Context<'_, MainWindow>,
) -> Div {
    let picked = store.read(cx).needs_view().filter().clone();
    let filter = &shown.filter;
    let group_store = store.clone();
    let mut bar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .px(px(14.))
        .py(px(6.))
        .border_b_1()
        .border_color(p.border)
        .child(segmented(
            p,
            "needs-grouping",
            &[(Grouping::Project, "Project"), (Grouping::When, "Time")],
            grouping,
            move |grouping, cx| {
                group_store.update(cx, |store, cx| store.set_needs_grouping(grouping, cx));
            },
        ));
    let mut facets = Vec::new();
    if !shown.hides_projects() {
        facets.extend([Facet::Project, Facet::Area]);
    }
    facets.push(Facet::Owner);
    if layout == Layout::List {
        facets.push(Facet::When);
    }
    bar = facet_chips(bar, p, store, shown, &facets, &picked);
    if filter.count() > 1 {
        let store = store.clone();
        bar = bar.child(
            div()
                .id("needs-filter-clear")
                .button_named("Clear the filters")
                .px(px(6.))
                .text_size(px(12.))
                .text_color(p.accent)
                .cursor_pointer()
                .child("Clear")
                .on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| {
                        store.set_needs_filter(NeedsFilter::default(), cx);
                    });
                }),
        );
    }
    bar
}

/// The filters' chips: each on filled with its value and an ✕, each other one opening its
/// choices.
fn facet_chips(
    mut bar: Div,
    p: &Pal,
    store: &Entity<Store>,
    shown: &Shown<'_>,
    facets: &[Facet],
    picked: &NeedsFilter,
) -> Div {
    let filter = &shown.filter;
    for &facet in facets.iter().filter(|facet| facet.is_on(filter)) {
        let next = facet.cleared(picked);
        let store = store.clone();
        bar = bar.child(
            div()
                .id(ElementId::Name(
                    format!("needs-filter-on-{}", facet.name()).into(),
                ))
                .button_named(format!(
                    "{}: {}. Remove filter",
                    facet.name(),
                    value(facet, filter, shown.state)
                ))
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(10.))
                .py(px(4.))
                .rounded_full()
                .bg(p.accent)
                .text_color(p.on_accent)
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .child(value(facet, filter, shown.state))
                .child(glyph(IconName::X, p.on_accent, 11.))
                .on_click(move |_, _, cx| {
                    let next = next.clone();
                    store.update(cx, |store, cx| store.set_needs_filter(next, cx));
                }),
        );
    }
    for &facet in facets.iter().filter(|facet| !facet.is_on(filter)) {
        let options = choices(facet, picked, shown);
        if options.is_empty() {
            continue;
        }
        let store = store.clone();
        let label: SharedString = facet.name().into();
        bar = bar.child(
            Button::new(ElementId::Name(
                format!("needs-filter-{}", facet.name()).into(),
            ))
            .label(label)
            .small()
            .ghost()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for (name, next) in &options {
                    let (store, next) = (store.clone(), next.clone());
                    menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                        let next = next.clone();
                        store.update(cx, |store, cx| store.set_needs_filter(next, cx));
                    }));
                }
                menu
            }),
        );
    }
    bar
}
