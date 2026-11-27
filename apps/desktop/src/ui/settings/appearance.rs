//! Appearance: light or dark (or the system's), the sidebar's agents (shown or hidden, above
//! or below the rooms), the colour presets, a custom colour typed as hex, and a preview of what
//! the colour paints. A typed colour is previewed by the server
//! (which holds it to WCAG AA) once the typing stops; Apply or Return uses it.

use std::time::Duration;

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    Div, ElementId, Entity, FontWeight, Hsla, Stateful, Task, Window, div, px, relative,
};

use super::parts::{note, page_body, problem_line, section};
use crate::color::normalize_hex;
use crate::store::approvals::shown::visible;
use crate::store::approvals::words::agent_line;
use crate::store::settings::appearance::{NOT_A_HEX, contrast_line, preset_name, to_preview};
use crate::store::{AgentsPlace, Store};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column::{self, Column};
use crate::ui::palette::Pal;
use crate::ui::widgets::dot;
use emberhut_api::ids::RoomId;
use emberhut_api::theme::{Appearance, DEFAULT_PRESET, ThemePage, Tokens};
use gpui_kit::component::switch::Switch;

/// The Show agents switch, for tests to find.
pub(crate) const SHOW_AGENTS: &str = "show-agents-switch";
/// The Below rooms segment, for tests to find.
pub(crate) const AGENTS_BELOW: &str = "agents-below";
/// The Above rooms segment, for tests to find.
pub(crate) const AGENTS_ABOVE: &str = "agents-above";
/// A hidden agent's row, before its room's id, for tests to find.
pub(crate) const HIDDEN_AGENT: &str = "hidden-agent";

/// How long after the last key a typed colour is previewed.
const PREVIEW_AFTER: Duration = Duration::from_millis(350);

/// The Appearance page.
pub(crate) struct AppearancePane {
    store: Entity<Store>,
    /// The custom colour, as typed.
    hex: Entity<InputState>,
    /// What was wrong with the colour typed.
    note: Option<&'static str>,
    /// The preview waiting for the typing to stop.
    pending_preview: Option<Task<()>>,
}

impl AppearancePane {
    /// The page over `store`, its field holding the custom colour in use.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let custom = store.read(cx).theme().page().choice.accent.clone();
        let hex = cx.new(|cx| {
            let input = InputState::new(window, cx).placeholder("#3366FF");
            match custom {
                Some(custom) => input.default_value(custom),
                None => input,
            }
        });
        cx.subscribe_in(
            &hex,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.apply(window, cx),
                InputEvent::Change => this.typed(cx),
                _ => {}
            },
        )
        .detach();
        Self {
            store,
            hex,
            note: None,
            pending_preview: None,
        }
    }

    /// The colour changed: its preview once the typing stops (or none, for one that isn't).
    fn typed(&mut self, cx: &mut Context<'_, Self>) {
        self.note = None;
        let typed = self.hex.read(cx).value().to_string();
        let current = self.store.read(cx).theme().page().choice.accent.clone();
        let Some(hex) = to_preview(&typed, current.as_deref()) else {
            self.pending_preview = None;
            self.store.update(cx, Store::clear_accent_preview);
            cx.notify();
            return;
        };
        self.pending_preview = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PREVIEW_AFTER).await;
            let asked = this.update(cx, |this, cx| {
                this.store.update(cx, |store, cx| {
                    store.custom_accent(Some(hex), true, cx);
                });
            });
            if let Err(error) = asked {
                log::debug!("the appearance page closed before its preview: {error}");
            }
        }));
        cx.notify();
    }

    /// Uses the typed colour, or says it isn't one.
    fn apply(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let typed = self.hex.read(cx).value().to_string();
        let Some(hex) = normalize_hex(&typed) else {
            self.note = Some(NOT_A_HEX);
            cx.notify();
            return;
        };
        self.pending_preview = None;
        self.note = None;
        if hex != typed {
            self.hex
                .update(cx, |input, cx| input.set_value(hex.clone(), window, cx));
        }
        self.store
            .update(cx, |store, cx| store.custom_accent(Some(hex), false, cx));
    }

    /// Back to the preset's own colour.
    fn reset(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.pending_preview = None;
        self.note = None;
        self.hex
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.store
            .update(cx, |store, cx| store.custom_accent(None, false, cx));
    }

    /// System, Light or Dark.
    fn modes(&self, chosen: &Appearance, p: &Pal) -> Div {
        let mut modes = div()
            .flex()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(9.))
            .bg(p.surface_raised);
        for (id, appearance, label) in [
            ("mode-system", Appearance::System, "System"),
            ("mode-light", Appearance::Light, "Light"),
            ("mode-dark", Appearance::Dark, "Dark"),
        ] {
            let store = self.store.clone();
            modes = modes.child(segment(id, label, *chosen == appearance, p).on_click(
                move |_, _, cx| {
                    store.update(cx, |store, cx| {
                        store.pick_theme(None, Some(appearance.clone()), cx);
                    });
                },
            ));
        }
        modes
    }

    /// The presets, the chosen one marked.
    fn presets(&self, page: &ThemePage, p: &Pal) -> Div {
        let mut presets = div().flex().flex_wrap().gap(px(8.));
        for preset in &page.presets {
            let on = page.choice.preset == preset.id && page.choice.accent.is_none();
            let swatch = Pal::token(if p.dark {
                &preset.dark.accent
            } else {
                &preset.light.accent
            });
            let (store, id) = (self.store.clone(), preset.id.clone());
            presets = presets.child(
                div()
                    .id(ElementId::Name(format!("preset-{}", preset.id).into()))
                    // The chosen one is read as selected, and Paper as the default.
                    .button_named(a11y::preset(&preset.name, preset.id == DEFAULT_PRESET))
                    .aria_selected(on)
                    .accessibility_id(format!("preset {}", preset.id))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .border_1()
                    .border_color(if on { p.accent } else { p.border })
                    .when(on, |d| d.bg(p.accent_soft))
                    .child(dot(swatch, 12.))
                    .child(div().text_size(px(12.5)).child(preset.name.clone()))
                    .on_click(move |_, _, cx| {
                        store.update(cx, |store, cx| store.pick_theme(Some(id.clone()), None, cx));
                    }),
            );
        }
        presets
    }

    /// The custom colour: its swatch (what it really comes out as, once previewed), the field,
    /// Apply, the contrast, and the way back to the preset's own.
    fn custom(&self, page: &ThemePage, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let theme = self.store.read(cx).theme();
        let preview = theme.preview().cloned();
        let typed = self.hex.read(cx).value().to_string();
        let swatch = preview.as_ref().map_or_else(
            || normalize_hex(&typed).map_or(p.accent, |hex| Pal::token(&hex)),
            |preview| Pal::token(&p.tokens_of(preview).accent),
        );
        let measured = preview
            .as_ref()
            .or_else(|| page.choice.accent.is_some().then_some(page));
        let line = measured.map(|page| contrast_line(page, p.dark));
        let valid = normalize_hex(&typed).is_some();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .id("custom-swatch")
                            .size(px(22.))
                            .rounded_full()
                            .bg(swatch)
                            .border_1()
                            .border_color(p.border),
                    )
                    .child(div().w(px(150.)).child(Input::new(&self.hex)))
                    .child(
                        Button::new("accent-apply")
                            .primary()
                            .label("Apply")
                            .disabled(!valid)
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    ),
            )
            .children(line.map(|line| note(p, line)))
            .when(page.choice.accent.is_some(), |d| {
                d.child(
                    div().child(
                        Button::new("accent-reset")
                            .ghost()
                            .small()
                            .label(format!("Use {}'s own colour", preset_name(page)))
                            .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx))),
                    ),
                )
            })
    }
}

impl Render for AppearancePane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = crate::ui::palette::pal(cx);
        let theme = self.store.read(cx).theme();
        let page = theme.page().clone();
        let preview = theme.preview().cloned();
        let problem = self
            .note
            .map(str::to_owned)
            .or_else(|| theme.problem().map(str::to_owned));
        let tokens = preview.as_ref().map(|preview| p.tokens_of(preview).clone());
        let content = div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(section(
                &p,
                "Appearance",
                None,
                vec![self.modes(&page.choice.appearance, &p).into_any_element()],
                Some("System follows this device's light or dark setting.".to_owned()),
            ))
            .child(self.sidebar(&p, cx))
            .child(section(
                &p,
                "Colour",
                None,
                vec![self.presets(&page, &p).into_any_element()],
                Some("What waits on you, buttons, links and progress take it.".to_owned()),
            ))
            .child(section(
                &p,
                "Custom colour",
                None,
                vec![self.custom(&page, &p, cx).into_any_element()],
                Some(
                    "Any hex colour. Text stays readable on it and with it (WCAG AA, 4.5:1): \
                     one too light or too dark for a mode is adjusted there only as far as it \
                     needs. Kept on the server, so your other devices follow the same choice."
                        .to_owned(),
                ),
            ))
            .child(section(
                &p,
                "Preview",
                None,
                vec![painted(&p, tokens.as_ref()).into_any_element()],
                None,
            ))
            .children(problem.map(|problem| problem_line(&p, problem)));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(page_body("appearance-scroll", content))
    }
}

/// What the colour paints: a Needs you label, a count, a decide-by chip on the soft accent, a
/// filled button, a progress bar, and the state colours. With `tokens` (a custom colour's
/// preview) the accent's three colours are those.
fn painted(p: &Pal, tokens: Option<&Tokens>) -> Div {
    let (accent, soft, on_accent) = tokens.map_or((p.accent, p.accent_soft, p.on_accent), |t| {
        (
            Pal::token(&t.accent),
            Pal::token(&t.accent_soft),
            Pal::token(&t.on_accent),
        )
    });
    let state = |glyph: IconName, label: &'static str, color: Hsla| {
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(12.))
            .text_color(color)
            .child(Icon::new(glyph).size(px(13.)).text_color(color))
            .child(label)
    };
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .py(px(4.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(accent)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(14.))
                .child(Icon::new(IconName::Hand).size(px(15.)).text_color(accent))
                .child("Needs you"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(20.))
                        .rounded_full()
                        .bg(accent)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(on_accent)
                        .child("2"),
                )
                .child(
                    div()
                        .px(px(8.))
                        .py(px(3.))
                        .rounded_full()
                        .bg(soft)
                        .text_color(accent)
                        .text_size(px(12.5))
                        .child("Decide by today"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .px(px(12.))
                        .py(px(4.))
                        .rounded(px(7.))
                        .bg(accent)
                        .text_color(on_accent)
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Answer"),
                ),
        )
        .child(
            div()
                .h(px(6.))
                .w_full()
                .rounded_full()
                .bg(p.border)
                .child(div().h_full().w(relative(0.6)).rounded_full().bg(accent)),
        )
        .child(
            div()
                .flex()
                .gap(px(14.))
                .child(state(IconName::CircleCheck, "Done", p.success))
                .child(state(IconName::CircleDot, "Running", p.info))
                .child(state(IconName::Clock, "Late", p.warn))
                .child(state(IconName::CircleX, "Failed", p.danger)),
        )
}

impl AppearancePane {
    /// The sidebar's agents: shown or hidden, and above or below the rooms. Kept on this
    /// computer only, like the sidebar's width. While they are hidden, each agent is listed
    /// here to open its room, so none is out of reach.
    fn sidebar(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Div {
        let store = self.store.read(cx);
        let shown = store.agents_shown();
        let place = store.agents_place();
        let hidden: Vec<(RoomId, String, String, bool)> = if shown {
            Vec::new()
        } else {
            store
                .agent_rooms()
                .iter()
                .map(|own| {
                    (
                        own.agent.room,
                        visible(&own.agent.name).into_owned(),
                        agent_line(own.agent),
                        own.agent.enabled,
                    )
                })
                .collect()
        };
        let switch_store = self.store.clone();
        let show =
            labelled(
                p,
                "Show agents",
                if shown {
                    "Each agent's room, in the sidebar"
                } else {
                    "Hidden from the sidebar"
                },
            )
            .child(Switch::new(SHOW_AGENTS).checked(shown).on_click(
                move |on, _, cx| {
                    switch_store.update(cx, |store, cx| store.show_agents(*on, cx));
                },
            ));
        let mut places = div()
            .flex()
            .flex_none()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(9.))
            .bg(p.surface_raised)
            .when(!shown, |d| d.opacity(0.5));
        for (id, chosen, label) in [
            (AGENTS_BELOW, AgentsPlace::Below, "Below rooms"),
            (AGENTS_ABOVE, AgentsPlace::Above, "Above rooms"),
        ] {
            let segment = segment(id, label, place == chosen, p);
            let segment = if shown {
                let store = self.store.clone();
                segment.on_click(move |_, _, cx| {
                    store.update(cx, |store, cx| store.place_agents(chosen, cx));
                })
            } else {
                segment.hint("Shown agents only: turn on Show agents first")
            };
            // Seen by the headless tests (a no-op outside them).
            places = places.child(segment.test_support());
        }
        let place_row = labelled(
            p,
            "Where agents sit",
            match place {
                AgentsPlace::Below => "After your rooms, projects, sessions and goals",
                AgentsPlace::Above => "Right under Needs you and Tasks",
            },
        )
        .child(places);
        let mut rows = vec![show.into_any_element(), place_row.into_any_element()];
        for (room, name, line, enabled) in hidden {
            let store = self.store.clone();
            rows.push(hidden_agent(store, (room, name, line, enabled), p).into_any_element());
        }
        let footer = if shown {
            "Your rooms come first; the agents' rooms follow, or sit above them."
        } else {
            "Hidden agents stay here and in Find (⌘K): open one to go to its room."
        };
        section(p, "Sidebar", None, rows, Some(footer.to_owned()))
    }
}

/// A setting's row: its title and a line under it at the left; its control goes at the right.
fn labelled(p: &Pal, title: &'static str, line: &'static str) -> Div {
    div().flex().items_center().gap(px(12.)).child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(2.))
            .child(div().text_size(px(13.)).child(title))
            .child(div().text_size(px(12.)).text_color(p.muted).child(line)),
    )
}

/// A hidden agent's row: its name and whom it works for; a click opens its room and shows the
/// chat.
fn hidden_agent(
    store: Entity<Store>,
    (room, name, line, enabled): (RoomId, String, String, bool),
    p: &Pal,
) -> impl IntoElement {
    div()
        .id(ElementId::Name(format!("{HIDDEN_AGENT}-{room}").into()))
        .button_named(format!("Open {name}'s room, {line}"))
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(6.))
        .py(px(5.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .child(
            Icon::new(IconName::Bot)
                .size(px(14.))
                .text_color(if enabled { p.accent } else { p.faint }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(13.)).truncate().child(name))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .truncate()
                        .child(line),
                ),
        )
        .child(
            Icon::new(IconName::ChevronRight)
                .size(px(12.))
                .text_color(p.faint),
        )
        .on_click(move |_, window, cx| {
            store.update(cx, |store, cx| store.switch_room(room, cx));
            if let Some(column) = column::of(window, cx) {
                column.update(cx, Column::close);
            }
        })
        // Seen by the headless tests (a no-op outside them).
        .test_support()
}

/// One of the Mode switch's segments.
fn segment(id: &'static str, label: &'static str, on: bool, p: &Pal) -> Stateful<Div> {
    div()
        .id(id)
        .button_named(label)
        .aria_selected(on)
        .px(px(12.))
        .py(px(5.))
        .rounded(px(7.))
        .cursor_pointer()
        .text_size(px(12.5))
        .when(on, |d| {
            d.bg(p.surface)
                .border_1()
                .border_color(p.border)
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!on, |d| d.text_color(p.muted))
        .child(label)
}
