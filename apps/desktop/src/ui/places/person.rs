//! A person's page, as the assistant knows them: their note, what is going on with them now,
//! whether their messages wake the assistant at once, where it came from and when, and the
//! tasks that name them (each opening its page).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, FontWeight, div, px};

use super::{MainWindow, body, gone, section, title};
use crate::brand::APP_NAME;
use crate::prose::direction;
use crate::store::spotlight::PersonKey;
use crate::store::tasks::clock;
use crate::store::tasks::search::Found;
use crate::store::{Page, lookup};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::palette::Pal;
use crate::ui::reads;
use crate::ui::tasks::marks::{glyph, status_icon};

/// The page.
pub(super) fn page(
    window: &mut MainWindow,
    p: &Pal,
    key: PersonKey,
    cx: &Context<'_, MainWindow>,
) -> Div {
    let store = window.store.read(cx);
    let Some(person) = store.person(key) else {
        return gone(
            p,
            if store.places().people().is_some() {
                "Not known any more"
            } else {
                "Loading…"
            },
        );
    };
    let mut page = body().child(title(person.title.clone()));
    if !person.text.is_empty() {
        page = page.child(
            div()
                .pt(px(6.))
                .text_size(px(13.5))
                .when(reads::starts_right(&person.text), Styled::text_right)
                .child(person.text.clone()),
        );
    }
    if let Some(now) = &person.now {
        page = page.child(
            div()
                .flex()
                .gap(px(10.))
                .pt(px(8.))
                .text_size(px(13.))
                .child(
                    div()
                        .w(px(40.))
                        .flex_none()
                        .text_color(p.muted)
                        .child("Now"),
                )
                .child(
                    div()
                        .flex_1()
                        .when(reads::mostly_right(now), Styled::text_right)
                        .child(direction::marked(now)),
                ),
        );
    }
    if person.is_important() {
        page = page.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .pt(px(8.))
                .text_size(px(13.))
                .text_color(p.warn)
                .child(glyph(IconName::Bell, p.warn, 14.))
                .child(format!("Their messages wake {APP_NAME} at once")),
        );
    }
    let day = clock::instant(&person.updated).map(|at| {
        at.to_zoned_datetime(jiff::tz::TimeZone::system())
            .strftime("%-d %b")
            .to_string()
    });
    let footer = [Some(person.source.clone()).filter(|s| !s.is_empty()), day]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    if !footer.is_empty() {
        page = page.child(
            div()
                .pt(px(6.))
                .text_size(px(11.5))
                .text_color(p.faint)
                .child(footer),
        );
    }
    page.child(section(p, "Tasks that name them"))
        .child(tasks(p, store, key))
}

/// The tasks that name a person, each opening its page.
fn tasks(p: &Pal, store: &crate::store::Store, key: PersonKey) -> Div {
    let mut page = div().flex().flex_col();
    let found = store
        .places()
        .person_tasks(key)
        .map(|lookup| lookup.found());
    let line = |text: String| div().text_size(px(13.)).text_color(p.muted).child(text);
    match found {
        Some(Found::Tasks([])) => page.child(line("No task names them.".to_owned())),
        Some(Found::Tasks(tasks)) => {
            for task in tasks {
                let id = task.id;
                let project = store
                    .state()
                    .and_then(|state| lookup::project_name(state, task.pid));
                let under = [Some(task.key.clone()), project.map(str::to_owned)]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                page = page.child(
                    div()
                        .id(ElementId::Name(format!("person-task-{id}").into()))
                        // One line for the row: its title, its key and project.
                        .button_named(a11y::line(&[&task.title, &under]))
                        .hint("Shows its details")
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .py(px(6.))
                        .cursor_pointer()
                        .hover(|d| d.bg(p.hover))
                        .child(status_icon(p, task))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(direction::marked(&task.title)),
                                )
                                .child(div().text_size(px(11.5)).text_color(p.muted).child(under)),
                        )
                        .on_click(move |_, window, cx| column::push(Page::Task(id), window, cx)),
                );
            }
            page
        }
        Some(Found::Failed(reason)) => page.child(line(reason.to_owned())),
        Some(Found::Idle | Found::Loading) | None => page.child(line("Looking…".to_owned())),
    }
}
