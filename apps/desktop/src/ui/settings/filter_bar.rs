//! The filter bar over Settings: Show (one part of the page) and State (Needs a look, Off,
//! Fine). A filter on is a filled chip with its value and an ✕ (a click takes it off), the
//! other a chip that opens its choices; Clear when both are on.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, Div, ElementId, Entity, FontWeight, SharedString, div, px};

use crate::store::Store;
use crate::store::working::settings_filter::{HealthPick, SettingsFilter, SettingsPart};
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;

/// One filter of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Facet {
    Show,
    State,
}

impl Facet {
    fn name(self) -> &'static str {
        match self {
            Self::Show => "Show",
            Self::State => "State",
        }
    }

    /// The value of the filter when it is on.
    fn value(self, filter: SettingsFilter) -> Option<String> {
        match self {
            Self::Show => filter.part.map(SettingsPart::name),
            Self::State => filter.health.map(|pick| pick.name().to_owned()),
        }
    }

    fn cleared(self, filter: SettingsFilter) -> SettingsFilter {
        match self {
            Self::Show => SettingsFilter {
                part: None,
                ..filter
            },
            Self::State => SettingsFilter {
                health: None,
                ..filter
            },
        }
    }

    /// Its choices, each the filter it makes.
    fn choices(self, filter: SettingsFilter) -> Vec<(String, SettingsFilter)> {
        match self {
            Self::Show => SettingsPart::ALL
                .iter()
                .map(|&part| {
                    let next = SettingsFilter {
                        part: Some(part),
                        ..filter
                    };
                    (part.name(), next)
                })
                .collect(),
            Self::State => HealthPick::ALL
                .iter()
                .map(|&pick| {
                    let next = SettingsFilter {
                        health: Some(pick),
                        ..filter
                    };
                    (pick.name().to_owned(), next)
                })
                .collect(),
        }
    }
}

/// Sets the filter.
fn set(store: &Entity<Store>, filter: SettingsFilter, cx: &mut App) {
    store.update(cx, |store, cx| store.set_settings_filter(filter, cx));
}

/// The bar.
pub(super) fn bar(store: &Entity<Store>, p: &Pal, cx: &App) -> Div {
    let filter = store.read(cx).settings_filter();
    let mut bar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .px(px(20.))
        .py(px(6.))
        .border_b_1()
        .border_color(p.border);
    for facet in [Facet::Show, Facet::State] {
        let store = store.clone();
        if let Some(value) = facet.value(filter) {
            let next = facet.cleared(filter);
            bar = bar.child(tab_stop(
                div()
                    .id(ElementId::Name(
                        format!("settings-filter-on-{}", facet.name()).into(),
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
                    .child(value)
                    .child(Icon::new(IconName::X).size(px(11.)).text_color(p.on_accent))
                    .on_click(move |_, _, cx| set(&store, next, cx)),
                p,
            ));
        } else {
            let options = facet.choices(filter);
            let label: SharedString = facet.name().into();
            bar = bar.child(
                Button::new(ElementId::Name(
                    format!("settings-filter-{}", facet.name()).into(),
                ))
                .label(label)
                .small()
                .ghost()
                .dropdown_caret(true)
                .dropdown_menu(move |mut menu, _, _| {
                    for (name, next) in &options {
                        let (store, next) = (store.clone(), *next);
                        menu = menu.item(
                            PopupMenuItem::new(name.clone())
                                .on_click(move |_, _, cx| set(&store, next, cx)),
                        );
                    }
                    menu
                }),
            );
        }
    }
    if filter.count() > 1 {
        let store = store.clone();
        bar = bar.child(tab_stop(
            div()
                .id("settings-filter-clear")
                .border_1()
                .rounded(px(6.))
                .px(px(6.))
                .text_size(px(12.))
                .text_color(p.accent)
                .cursor_pointer()
                .child("Clear")
                .on_click(move |_, _, cx| set(&store, SettingsFilter::default(), cx)),
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
        let filter = SettingsFilter {
            part: Some(SettingsPart::Skills),
            health: None,
        };
        let states = Facet::State.choices(filter);
        assert_eq!(states.len(), 3);
        assert_eq!(states[0].0, "Needs a look");
        assert_eq!(
            states[0].1.part,
            Some(SettingsPart::Skills),
            "the rest stays"
        );
        assert_eq!(Facet::Show.cleared(filter), SettingsFilter::default());
        assert_eq!(Facet::Show.value(filter).as_deref(), Some("Skills"));
        assert_eq!(Facet::State.value(filter), None);
    }
}
