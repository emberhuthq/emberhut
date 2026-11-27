//! A new party, in a sheet over the window: the topic, the cast (ready-made personas, the
//! user's own from earlier parties, or new ones; 2 to 5), each one's model, and the caps with
//! the most they could cost. Start is there with a topic and two personas; the server's reason
//! shows when it refuses. A persona opens in an editor of its own over the sheet: its name, who
//! they are and how they talk, and its model.

use emberhut_api::party::{MAX_CAST, MIN_CAST, PartyModel, Persona};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Div, ElementId, Entity, FontWeight, SharedString, Stateful, Window, div, px,
};

use super::PartyWindow;
use super::look::{glyph, initial};
use crate::store::Store;
use crate::store::party::draft::{CastKey, Draft, Editing, Member, TOKEN_CAPS};
use crate::store::party::words;
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::palette::Pal;

/// The sheet over the window: the editor when a persona is being edited, else the form.
pub(super) fn sheet(
    view: &PartyWindow,
    p: &Pal,
    cx: &mut Context<'_, PartyWindow>,
) -> Option<Stateful<Div>> {
    let party = view.store.read(cx).party();
    let draft = party.draft().cloned()?;
    let roster = party.roster();
    let models = roster.models().to_vec();
    let offered = Offered {
        presets: draft
            .offered(roster.presets())
            .into_iter()
            .cloned()
            .collect(),
        recent: draft
            .offered(roster.recent())
            .into_iter()
            .cloned()
            .collect(),
    };
    let names: Vec<String> = draft
        .cast()
        .iter()
        .map(|member| roster.model_name(&member.model))
        .collect();
    let card = match draft.editing.clone() {
        Some(editing) => editor(view, &editing, &models, p, cx),
        None => form(view, &draft, &offered, &models, &names, p, cx),
    };
    // The backdrop and the card both take the pointer, so a click on either never reaches the
    // list or the party's controls drawn under the sheet.
    Some(
        div()
            .id("party-sheet-backdrop")
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(p.text.opacity(0.18))
            .child(card),
    )
}

/// The personas the Add menu offers: those not in the cast yet.
struct Offered {
    presets: Vec<Persona>,
    recent: Vec<Persona>,
}

/// A sheet's frame: its bar (Cancel, the title, the confirming button) over its body.
fn frame(
    p: &Pal,
    title: impl Into<SharedString>,
    (cancel, confirm): (Button, Button),
    body: Div,
    scroll: &gpui_kit::ScrollHandle,
) -> Stateful<Div> {
    div()
        .id("party-sheet-card")
        .occlude()
        .w(px(520.))
        .max_h(px(640.))
        .flex()
        .flex_col()
        .rounded(px(14.))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .shadow_lg()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(10.))
                .border_b_1()
                .border_color(p.border)
                .child(cancel)
                .child(
                    div()
                        .flex_1()
                        .text_center()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.into()),
                )
                .child(confirm),
        )
        .child(
            div()
                .id("party-sheet-body")
                .flex_1()
                .overflow_y_scroll()
                .track_scroll(scroll)
                .px(px(18.))
                .py(px(14.))
                .child(body),
        )
}

/// A titled part of a form, with a note under it.
fn section(p: &Pal, title: impl Into<SharedString>, rows: Div, note: Option<&str>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .pb(px(16.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.muted)
                .child(title.into()),
        )
        .child(rows)
        .children(note.map(|note| {
            div()
                .text_size(px(11.5))
                .text_color(p.muted)
                .child(note.to_owned())
        }))
}

/// The form.
fn form(
    view: &PartyWindow,
    draft: &Draft,
    offered: &Offered,
    models: &[PartyModel],
    model_names: &[String],
    p: &Pal,
    cx: &mut Context<'_, PartyWindow>,
) -> Stateful<Div> {
    let topic = view.topic.read(cx).value().to_string();
    let cancel = Button::new("party-cancel")
        .ghost()
        .small()
        .label("Cancel")
        .on_click(cx.listener(|this, _, _, cx| {
            this.store
                .update(cx, |store, cx| store.show_new_party(false, cx));
        }));
    let start = Button::new("party-start")
        .primary()
        .small()
        .label("Start")
        .disabled(!draft.can_start(&topic))
        .on_click(cx.listener(|this, _, _, cx| {
            let topic = this.topic.read(cx).value().to_string();
            this.store
                .update(cx, |store, cx| store.start_party(&topic, cx));
        }));
    let mut cast = div().flex().flex_col().gap(px(4.));
    for (member, model) in draft.cast().iter().zip(model_names) {
        cast = cast.child(cast_row(member, model, p, cx));
    }
    if draft.has_room() {
        cast = cast.child(add_menu(&view.store, offered, p, cx));
    }
    let body = div()
        .flex()
        .flex_col()
        .child(section(
            p,
            "Topic or opening line",
            div().child(Textarea::new(&view.topic)),
            None,
        ))
        .child(section(
            p,
            format!("Cast · {} of {MIN_CAST} to {MAX_CAST}", draft.cast().len()),
            cast,
            Some(
                "Click one to change their role or model. Opus talks best; Sonnet and Haiku \
                 cost less.",
            ),
        ))
        .child(section(p, "Limits", limits(draft, models, p, cx), None))
        .children(draft.problem.clone().map(|problem| {
            div()
                .text_size(px(12.5))
                .text_color(p.danger)
                .child(problem)
        }));
    frame(p, "New party", (cancel, start), body, &view.form_scroll)
}

/// A cast member's row: its initial, name, model and role; a click edits it, − removes it.
fn cast_row(
    member: &Member,
    model: &str,
    p: &Pal,
    cx: &mut Context<'_, PartyWindow>,
) -> AnyElement {
    let key = member.key;
    div()
        .id(ElementId::Name(format!("party-cast-{key:?}").into()))
        // One line for the member: its name, model and role.
        .button_named(a11y::line(&[&member.name, model, &member.role]))
        .hint("Edits the persona")
        .flex()
        .items_start()
        .gap(px(10.))
        .px(px(8.))
        .py(px(6.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(|d| d.bg(p.hover))
        .child(initial(&member.name, p.accent, 26.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::MEDIUM)
                                .child(member.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(p.muted)
                                .child(model.to_owned()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted)
                        .line_clamp(2)
                        .child(member.role.clone()),
                ),
        )
        .child(
            div()
                .id(ElementId::Name(format!("party-cast-remove-{key:?}").into()))
                .button_named(format!("Remove {}", member.name))
                .p(px(3.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(glyph(IconName::CircleMinus, p.muted, 15.))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.store
                        .update(cx, |store, cx| store.change_draft(|d| d.remove(key), cx));
                })),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            this.edit_persona(Some(key), window, cx);
        }))
        .into_any_element()
}

/// Add a persona: the ready-made ones and the user's own not in the cast, `Name · role`, and a
/// new one.
fn add_menu(
    store: &Entity<Store>,
    offered: &Offered,
    p: &Pal,
    cx: &mut Context<'_, PartyWindow>,
) -> impl IntoElement {
    let (presets, recent) = (offered.presets.clone(), offered.recent.clone());
    let (store, this) = (store.clone(), cx.entity().downgrade());
    div().pt(px(2.)).text_color(p.accent).child(
        Button::new("party-add")
            .ghost()
            .small()
            .icon(Icon::new(IconName::CirclePlus))
            .label("Add a persona")
            .dropdown_menu(move |menu: PopupMenu, _, _| {
                let mut menu = menu;
                if !presets.is_empty() {
                    menu = menu.label("Ready-made");
                    for persona in &presets {
                        menu = menu.item(add_item(&store, persona));
                    }
                }
                if !recent.is_empty() {
                    if !presets.is_empty() {
                        menu = menu.separator();
                    }
                    menu = menu.label("Yours");
                    for persona in &recent {
                        menu = menu.item(add_item(&store, persona));
                    }
                }
                if !presets.is_empty() || !recent.is_empty() {
                    menu = menu.separator();
                }
                let this = this.clone();
                menu.item(
                    PopupMenuItem::new("New persona…")
                        .icon(Icon::new(IconName::SquarePen))
                        .on_click(move |_, window, cx| {
                            if let Err(error) = this.update(cx, |this, cx| {
                                this.edit_persona(None, window, cx);
                            }) {
                                log::debug!("the Party window closed: {error}");
                            }
                        }),
                )
            }),
    )
}

/// The Add menu's item for `persona`.
fn add_item(store: &Entity<Store>, persona: &Persona) -> PopupMenuItem {
    let (store, chosen) = (store.clone(), persona.clone());
    PopupMenuItem::new(format!(
        "{} · {}",
        persona.name,
        words::short_role(&persona.role)
    ))
    .on_click(move |_, _, cx| {
        let chosen = chosen.clone();
        store.update(cx, |store, cx| store.change_draft(|d| d.add(&chosen), cx));
    })
}

/// The caps: turns (2 to 60, by 2), the token cap, and the most they could cost.
fn limits(draft: &Draft, models: &[PartyModel], p: &Pal, cx: &mut Context<'_, PartyWindow>) -> Div {
    let step =
        |id: &'static str, glyph_name: IconName, up: bool, cx: &mut Context<'_, PartyWindow>| {
            Button::new(id)
                .small()
                .icon(Icon::new(glyph_name))
                .accessibility_label(if up { "More turns" } else { "Fewer turns" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.store
                        .update(cx, |store, cx| store.change_draft(|d| d.step_turns(up), cx));
                }))
        };
    let turns = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .child(format!("Up to {} turns", draft.max_turns)),
        )
        .child(step("party-turns-down", IconName::Minus, false, cx))
        .child(step("party-turns-up", IconName::Plus, true, cx));
    let mut caps = div().flex().gap(px(6.));
    for cap in TOKEN_CAPS {
        let chosen = draft.max_tokens == cap;
        let button = Button::new(ElementId::Name(format!("party-cap-{cap}").into()))
            .small()
            .label(words::tokens(cap))
            .toggled(chosen)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.store.update(cx, |store, cx| {
                    store.change_draft(|d| d.max_tokens = cap, cx);
                });
            }));
        caps = caps.child(if chosen {
            button.primary()
        } else {
            button.outline()
        });
    }
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(turns)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().child("Token cap"))
                .child(caps),
        )
        .children(draft.estimate(models).map(|estimate| {
            div().text_size(px(11.5)).text_color(p.muted).child(format!(
                "At most about {} at these caps",
                words::cost(estimate)
            ))
        }))
}

/// The persona being edited: its name, its role and its model; Done needs a name.
fn editor(
    view: &PartyWindow,
    editing: &Editing,
    models: &[PartyModel],
    p: &Pal,
    cx: &mut Context<'_, PartyWindow>,
) -> Stateful<Div> {
    let name = view.name.read(cx).value().trim().to_owned();
    let title = if name.is_empty() {
        "New persona".to_owned()
    } else {
        name.clone()
    };
    let cancel = Button::new("persona-cancel")
        .ghost()
        .small()
        .label("Cancel")
        .on_click(cx.listener(|this, _, _, cx| {
            this.store
                .update(cx, |store, cx| store.change_draft(|d| d.editing = None, cx));
        }));
    let done = Button::new("persona-done")
        .primary()
        .small()
        .label("Done")
        .disabled(name.is_empty())
        .on_click(cx.listener(|this, _, _, cx| {
            let name = this.name.read(cx).value().to_string();
            let role = this.role.read(cx).value().to_string();
            this.store.update(cx, |store, cx| {
                store.change_draft(|d| d.save(&name, &role), cx);
            });
        }));
    let mut choices = div().flex().flex_col().gap(px(2.));
    for model in models {
        let chosen = model.id == editing.model;
        let id = model.id.clone();
        choices = choices.child(
            div()
                .id(ElementId::Name(
                    format!("persona-model-{}", model.id).into(),
                ))
                .button_named(a11y::line(&[&model.name, &model.note]))
                .aria_selected(chosen)
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(8.))
                .py(px(6.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .when(chosen, |d| d.bg(p.accent_soft))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .child(model.name.clone())
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(p.muted)
                                .child(model.note.clone()),
                        ),
                )
                .when(chosen, |d| d.child(glyph(IconName::Check, p.accent, 15.)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let id = id.clone();
                    this.store.update(cx, |store, cx| {
                        store.change_draft(
                            |d| {
                                if let Some(editing) = &mut d.editing {
                                    editing.model = id;
                                }
                            },
                            cx,
                        );
                    });
                })),
        );
    }
    let body = div()
        .flex()
        .flex_col()
        .child(section(
            p,
            "Name",
            div().child(Input::new(&view.name)),
            None,
        ))
        .child(section(
            p,
            "Role",
            div().child(Textarea::new(&view.role)),
            Some("e.g. \"A blunt startup investor who has seen a thousand pitches.\""),
        ))
        .child(section(p, "Model", choices, None));
    frame(p, title, (cancel, done), body, &view.form_scroll)
}

impl PartyWindow {
    /// Opens the editor on cast member `key` (its name and role in the fields), or on a new
    /// persona.
    pub(super) fn edit_persona(
        &mut self,
        key: Option<CastKey>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let member = key.and_then(|key| {
            self.store
                .read(cx)
                .party()
                .draft()
                .and_then(|draft| draft.member(key).cloned())
        });
        let (name, role) = member.map_or_default(|member| (member.name, member.role));
        self.name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.role
            .update(cx, |input, cx| input.set_value(role, window, cx));
        self.store
            .update(cx, |store, cx| store.change_draft(|d| d.edit(key), cx));
        self.name.update(cx, |input, cx| input.focus(window, cx));
    }
}
