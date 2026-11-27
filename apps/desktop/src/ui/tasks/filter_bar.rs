//! The filter bar above the Tasks list: each filter on is a filled chip with its value and an
//! ✕ (a click takes it off), each other one a chip that opens its choices, and Clear when more
//! than one is on. Project is offered only while projects show.

use emberhut_api::state::State;
use emberhut_api::tasks::ProjectStatus;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, Entity, FontWeight, SharedString, div, px};

use super::marks::glyph;
use crate::store::Store;
use crate::store::tasks::filter::{OwnerPick, ProjectPick, StatusPick, TaskFilter, When};
use crate::ui::a11y::Accessible as _;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;

/// One filter of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Facet {
    Status,
    Owner,
    When,
    Project,
}

impl Facet {
    fn name(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Owner => "Owner",
            Self::When => "Decide by",
            Self::Project => "Project",
        }
    }

    fn is_on(self, filter: &TaskFilter) -> bool {
        match self {
            Self::Status => filter.status.is_some(),
            Self::Owner => filter.owner.is_some(),
            Self::When => filter.when.is_some(),
            Self::Project => filter.project.is_some(),
        }
    }

    fn cleared(self, filter: &TaskFilter) -> TaskFilter {
        let mut filter = filter.clone();
        match self {
            Self::Status => filter.status = None,
            Self::Owner => filter.owner = None,
            Self::When => filter.when = None,
            Self::Project => filter.project = None,
        }
        filter
    }
}

/// A project's name for the bar.
fn project_name(state: Option<&State>, pick: ProjectPick) -> String {
    match pick {
        ProjectPick::NoProject => "No project".to_owned(),
        ProjectPick::Project(id) => state
            .and_then(|state| state.projects.iter().find(|project| project.id == id))
            .map_or_else(|| format!("Project {id}"), |project| project.name.clone()),
    }
}

/// The value an active filter's chip shows.
fn value(facet: Facet, filter: &TaskFilter, state: Option<&State>) -> String {
    match facet {
        Facet::Status => filter.status.map_or_default(|s| s.name().to_owned()),
        Facet::Owner => filter.owner.map_or_default(OwnerPick::name),
        Facet::When => filter.when.map_or_default(|w| w.name().to_owned()),
        Facet::Project => filter
            .project
            .map_or_default(|pick| project_name(state, pick)),
    }
}

/// The choices a facet offers, each the filter it makes.
fn choices(facet: Facet, filter: &TaskFilter, state: Option<&State>) -> Vec<(String, TaskFilter)> {
    let with = |change: &dyn Fn(&mut TaskFilter)| {
        let mut next = filter.clone();
        change(&mut next);
        next
    };
    match facet {
        Facet::Status => StatusPick::ALL
            .iter()
            .map(|&pick| (pick.name().to_owned(), with(&|f| f.status = Some(pick))))
            .collect(),
        Facet::Owner => OwnerPick::ALL
            .iter()
            .map(|&pick| (pick.name(), with(&|f| f.owner = Some(pick))))
            .collect(),
        Facet::When => When::ALL
            .iter()
            .map(|&pick| (pick.name().to_owned(), with(&|f| f.when = Some(pick))))
            .collect(),
        Facet::Project => {
            let active = state.map_or_default(|state| {
                state
                    .projects
                    .iter()
                    .filter(|project| project.status == ProjectStatus::Active)
                    .map(|project| ProjectPick::Project(project.id))
                    .collect::<Vec<_>>()
            });
            active
                .into_iter()
                .chain([ProjectPick::NoProject])
                .map(|pick| (project_name(state, pick), with(&|f| f.project = Some(pick))))
                .collect()
        }
    }
}

/// The bar.
pub(crate) fn bar(store: &Entity<Store>, p: &Pal, cx: &gpui_kit::App) -> Div {
    let read = store.read(cx);
    let filter = read.task_filter();
    let state = read.state();
    let facets: &[Facet] = if read.hides_projects() {
        &[Facet::Status, Facet::Owner, Facet::When]
    } else {
        &[Facet::Project, Facet::Status, Facet::Owner, Facet::When]
    };
    let mut bar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .py(px(6.));
    for &facet in facets.iter().filter(|facet| facet.is_on(&filter)) {
        let next = facet.cleared(&filter);
        let store = store.clone();
        let shown = value(facet, &filter, state);
        bar = bar.child(tab_stop(
            div()
                .id(ElementId::Name(
                    format!("filter-on-{}", facet.name()).into(),
                ))
                .border_1()
                .button_named(format!("{}: {shown}. Remove filter", facet.name()))
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
                .child(shown)
                .child(glyph(IconName::X, p.on_accent, 11.))
                .on_click(move |_, _, cx| {
                    let next = next.clone();
                    store.update(cx, |store, cx| store.set_task_filter(next, cx));
                }),
            p,
        ));
    }
    for &facet in facets.iter().filter(|facet| !facet.is_on(&filter)) {
        let options = choices(facet, &filter, state);
        if options.is_empty() {
            continue;
        }
        let store = store.clone();
        let label: SharedString = facet.name().into();
        bar = bar.child(
            Button::new(ElementId::Name(format!("filter-{}", facet.name()).into()))
                .accessibility_label(format!("Filter by {}", facet.name().to_lowercase()))
                .label(label)
                .small()
                .ghost()
                .dropdown_caret(true)
                .dropdown_menu(move |mut menu, _, _| {
                    for (name, next) in &options {
                        let (store, next) = (store.clone(), next.clone());
                        menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                            move |_, _, cx| {
                                let next = next.clone();
                                store.update(cx, |store, cx| store.set_task_filter(next, cx));
                            },
                        ));
                    }
                    menu
                }),
        );
    }
    if filter.count() > 1 {
        let store = store.clone();
        bar = bar.child(tab_stop(
            div()
                .id("filter-clear")
                .border_1()
                .rounded(px(6.))
                .button_named("Clear filters")
                .px(px(6.))
                .text_size(px(12.))
                .text_color(p.accent)
                .cursor_pointer()
                .child("Clear")
                .on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| {
                        store.set_task_filter(TaskFilter::default(), cx);
                    });
                }),
            p,
        ));
    }
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_facet_offers_its_picks_and_clears_its_own() {
        let filter = TaskFilter {
            owner: Some(OwnerPick::Yours),
            ..TaskFilter::default()
        };
        let picks = choices(Facet::Status, &filter, None);
        assert_eq!(picks.len(), 4);
        assert_eq!(picks[0].0, "Needs you");
        assert_eq!(picks[0].1.owner, Some(OwnerPick::Yours), "the rest stays");
        assert_eq!(Facet::Owner.cleared(&filter), TaskFilter::default());
        assert_eq!(value(Facet::Owner, &filter, None), "Yours");
        let projects = choices(Facet::Project, &filter, None);
        assert_eq!(projects.len(), 1, "with no state: only No project");
        assert_eq!(projects[0].0, "No project");
    }
}
