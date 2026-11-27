//! The Scheduled page's filter bar: Room (the rooms the list has rows in), Kind and Runs. Each
//! filter on is a filled chip with its value and an ✕ (a click takes it off), each other one a
//! chip that opens its choices, as on Tasks; under the list, what the filter hides.

use emberhut_api::ids::RoomId;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, Entity, FontWeight, SharedString, div, px};

use crate::store::Store;
use crate::store::scheduled::filter::{KINDS, ScheduledFilter, kind_name};
use crate::store::scheduled::when::Runs;
use crate::ui::a11y::Accessible as _;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::tasks::marks::glyph;

/// One filter of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Facet {
    Room,
    Kind,
    Runs,
}

impl Facet {
    const ALL: [Self; 3] = [Self::Room, Self::Kind, Self::Runs];

    fn name(self) -> &'static str {
        match self {
            Self::Room => "Room",
            Self::Kind => "Kind",
            Self::Runs => "Runs",
        }
    }

    fn is_on(self, filter: &ScheduledFilter) -> bool {
        match self {
            Self::Room => filter.room.is_some(),
            Self::Kind => filter.kind.is_some(),
            Self::Runs => filter.runs.is_some(),
        }
    }

    fn cleared(self, filter: &ScheduledFilter) -> ScheduledFilter {
        let mut filter = filter.clone();
        match self {
            Self::Room => filter.room = None,
            Self::Kind => filter.kind = None,
            Self::Runs => filter.runs = None,
        }
        filter
    }
}

/// A room's name, from the state.
fn room_name(store: &Store, room: RoomId) -> String {
    store
        .state()
        .and_then(|state| state.rooms.iter().find(|row| row.id == room))
        .map_or_else(|| format!("Room {room}"), |row| row.name.clone())
}

/// The value an active filter's chip shows.
fn value(facet: Facet, filter: &ScheduledFilter, store: &Store) -> String {
    match facet {
        Facet::Room => filter.room.map_or_default(|room| room_name(store, room)),
        Facet::Kind => filter.kind.as_ref().map_or_default(kind_name),
        Facet::Runs => filter.runs.map_or_default(|runs| runs.name().to_owned()),
    }
}

/// The choices a facet offers, each the filter it makes.
fn choices(
    facet: Facet,
    filter: &ScheduledFilter,
    store: &Store,
) -> Vec<(String, ScheduledFilter)> {
    let with = |change: &dyn Fn(&mut ScheduledFilter)| {
        let mut next = filter.clone();
        change(&mut next);
        next
    };
    match facet {
        Facet::Room => {
            let mut rooms: Vec<RoomId> = store
                .scheduled()
                .items()
                .unwrap_or_default()
                .iter()
                .map(|item| item.room)
                .collect();
            rooms.sort_unstable();
            rooms.dedup();
            rooms
                .into_iter()
                .map(|room| (room_name(store, room), with(&|f| f.room = Some(room))))
                .collect()
        }
        Facet::Kind => KINDS
            .iter()
            .map(|kind| (kind_name(kind), with(&|f| f.kind = Some(kind.clone()))))
            .collect(),
        Facet::Runs => Runs::ALL
            .iter()
            .map(|&runs| (runs.name().to_owned(), with(&|f| f.runs = Some(runs))))
            .collect(),
    }
}

/// The bar.
pub(super) fn bar(store: &Entity<Store>, read: &Store, p: &Pal) -> Div {
    let filter = read.scheduled().filter().clone();
    let mut bar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .px(px(16.))
        .py(px(6.));
    for facet in Facet::ALL.into_iter().filter(|facet| facet.is_on(&filter)) {
        let next = facet.cleared(&filter);
        let store = store.clone();
        bar = bar.child(tab_stop(
            div()
                .id(ElementId::Name(
                    format!("scheduled-on-{}", facet.name()).into(),
                ))
                .button_named(format!(
                    "{}: {}. Remove filter",
                    facet.name(),
                    value(facet, &filter, read)
                ))
                .border_1()
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
                .child(value(facet, &filter, read))
                .child(glyph(IconName::X, p.on_accent, 11.))
                .on_click(move |_, _, cx| {
                    let next = next.clone();
                    store.update(cx, |store, cx| store.set_scheduled_filter(next, cx));
                }),
            p,
        ));
    }
    for facet in Facet::ALL.into_iter().filter(|facet| !facet.is_on(&filter)) {
        let options = choices(facet, &filter, read);
        if options.is_empty() {
            continue;
        }
        let store = store.clone();
        let label: SharedString = facet.name().into();
        bar = bar.child(
            Button::new(ElementId::Name(
                format!("scheduled-{}", facet.name()).into(),
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
                        store.update(cx, |store, cx| store.set_scheduled_filter(next, cx));
                    }));
                }
                menu
            }),
        );
    }
    bar
}

/// Under the list: how many rows the filter hides, and Show all.
pub(super) fn filtered_out(store: &Entity<Store>, hidden: usize, p: &Pal) -> impl IntoElement {
    let store = store.clone();
    tab_stop(
        div()
            .id("scheduled-show-all")
            .button_named(format!("{hidden} more hidden by the filter. Show all"))
            .border_1()
            .mx(px(12.))
            .mt(px(12.))
            .px(px(4.))
            .rounded(px(4.))
            .text_size(px(12.))
            .text_color(p.muted)
            .cursor_pointer()
            .hover(|d| d.text_color(p.accent))
            .child(format!("{hidden} more hidden by the filter · Show all"))
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| {
                    store.set_scheduled_filter(super::cleared(), cx);
                });
            }),
        p,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::backend::fake::FakeBackend;
    use emberhut_api::scheduled::ScheduledKind;

    #[test]
    fn a_facet_offers_its_picks_and_clears_its_own() {
        let store = Store::for_tests(Arc::new(FakeBackend::new([])));
        let filter = ScheduledFilter {
            kind: Some(ScheduledKind::Watch),
            ..ScheduledFilter::default()
        };
        let kinds = choices(Facet::Kind, &filter, &store);
        assert_eq!(kinds.len(), 5);
        assert_eq!(kinds[1].0, "Check-in");
        let runs = choices(Facet::Runs, &filter, &store);
        assert_eq!(runs[0].1.kind, Some(ScheduledKind::Watch), "the rest stays");
        assert!(
            choices(Facet::Room, &filter, &store).is_empty(),
            "no rows: no rooms to pick"
        );
        assert_eq!(Facet::Kind.cleared(&filter), ScheduledFilter::default());
        assert_eq!(value(Facet::Kind, &filter, &store), "Watch");
    }
}
