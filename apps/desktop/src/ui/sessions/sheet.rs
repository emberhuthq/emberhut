//! New session: a sheet over the Sessions screen with the machine (Linux PC or Mac), the folder
//! (up to six one-tap picks, or a browser: the user's projects, recent folders, Documents to
//! drill into, a path typed), its own git worktree (a new branch, named or not, or one that
//! exists), the Claude account (Most room, or one with its room), the model and the effort;
//! Cancel and Open (once a folder is picked, and a branch when it is an existing one). Open
//! shows the new session's draft page, where its first message opens it.

use emberhut_api::runner::actions::Effort;
use emberhut_api::runner::new_session::Folder;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Div, ElementId, Entity, FocusHandle, FontWeight, Role, SharedString, Window, actions, div,
    px,
};

use super::marks::spinner;
use crate::store::Store;
use crate::store::sessions::sheet::{
    Listing, SessionSheet, accounts_for, machines, quick_folders, room_line,
};
use crate::store::sessions::words::{MODELS, machine_place, model_name, short_path};
use crate::store::{Page, sessions::places::SessionKey};
use crate::ui::a11y::{self, Accessible as _};
use crate::ui::column;
use crate::ui::controls::tab_stop;
use crate::ui::palette::{Pal, pal};

actions!(
    session_sheet,
    [
        /// Closes the New session sheet.
        CloseSheet,
    ]
);

/// The sheet's key context.
const CONTEXT: &str = "NewSessionSheet";

/// Binds the sheet's keys: Esc closes it.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([gpui_kit::KeyBinding::new(
        "escape",
        CloseSheet,
        Some(CONTEXT),
    )]);
}

/// What closes the sheet.
pub(crate) type OnClose = Box<dyn Fn(&mut Window, &mut App)>;

/// The sheet.
pub(crate) struct NewSessionView {
    store: Entity<Store>,
    sheet: Entity<SessionSheet>,
    branch: Entity<InputState>,
    typed: Entity<InputState>,
    focus: FocusHandle,
    close: OnClose,
}

impl NewSessionView {
    /// The sheet over `store`, starting on the choices kept from the last one.
    pub(crate) fn new(
        store: Entity<Store>,
        close: OnClose,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let sheet = store.update(cx, |store, cx| store.new_session_sheet(cx));
        cx.observe(&sheet, |_, _, cx| cx.notify()).detach();
        let branch = cx.new(|cx| InputState::new(window, cx).placeholder("Branch name (optional)"));
        cx.subscribe(&branch, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                this.sheet
                    .update(cx, |sheet, cx| sheet.set_branch(&text, cx));
            }
        })
        .detach();
        let typed = cx.new(|cx| InputState::new(window, cx).placeholder("A folder's path"));
        cx.subscribe(&typed, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let path = input.read(cx).value().trim().to_owned();
                if !path.is_empty() {
                    this.sheet.update(cx, |sheet, cx| sheet.set_cwd(&path, cx));
                }
            }
        })
        .detach();
        Self {
            store,
            sheet,
            branch,
            typed,
            focus: cx.focus_handle(),
            close,
        }
    }

    /// Gives the sheet the keyboard (Esc closes it).
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    /// Opens the draft of the session set up, over the screen.
    fn open(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let setup = self.sheet.read(cx).setup().clone();
        if !setup.can_open() {
            return;
        }
        let key: SessionKey = self
            .store
            .update(cx, |store, cx| store.draft_session(setup, cx));
        column::push(Page::Session(key), window, cx);
        (self.close)(window, cx);
    }

    fn machine_row(&self, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let now = self.sheet.read(cx).setup().machine.clone();
        let mut row = div().flex().gap(px(6.));
        let state = self.store.read(cx).state();
        for machine in machines(state) {
            let sheet = self.sheet.clone();
            let label = machine_place(state, &machine);
            let button = Button::new(ElementId::Name(format!("machine-{machine}").into()))
                .label(label)
                .toggled(now.as_str() == machine)
                .small()
                .on_click({
                    let machine = machine.clone();
                    move |_, _, cx| {
                        sheet.update(cx, |sheet, cx| sheet.set_machine(&machine, cx));
                    }
                });
            row = row.child(if now.as_str() == machine {
                button.primary()
            } else {
                button.ghost()
            });
        }
        field(p, "Hut", row)
    }

    fn folder_rows(&self, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let read = self.sheet.read(cx);
        let cwd = read.setup().cwd.clone();
        let sheet = self.sheet.clone();
        let choose = tab_stop(
            div()
                .id("folder-choose")
                .button_named(format!(
                    "Folder: {}",
                    if cwd.is_empty() {
                        "choose one".to_owned()
                    } else {
                        short_path(&cwd)
                    }
                ))
                .border_1()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(p.card)
                .cursor_pointer()
                .child(div().text_size(px(13.)).child("Folder"))
                .child(div().flex_1())
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.5))
                        .text_color(p.muted)
                        .child(if cwd.is_empty() {
                            "Choose…".to_owned()
                        } else {
                            short_path(&cwd)
                        }),
                )
                .on_click(move |_, _, cx| sheet.update(cx, |sheet, cx| sheet.browse(None, cx))),
            p,
        );
        let mut rows = div().flex().flex_col().gap(px(4.)).child(choose);
        if let Listing::Ready(list) = read.roots() {
            for (i, folder) in quick_folders(list, &cwd).into_iter().enumerate() {
                rows = rows.child(self.folder_pick(p, ("quick", i), folder));
            }
        }
        field(p, "Folder", rows)
    }

    fn folder_pick(&self, p: &Pal, id: (&'static str, usize), folder: &Folder) -> impl IntoElement {
        let (sheet, path) = (self.sheet.clone(), folder.path.clone());
        tab_stop(
            div()
                .id(ElementId::NamedInteger(id.0.into(), id.1 as u64))
                .button_named(a11y::line(&[folder.label(), &short_path(&folder.path)]))
                .border_1()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(4.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(crate::ui::tasks::marks::glyph(
                    gpui_kit::assets::IconName::Folder,
                    p.muted,
                    13.,
                ))
                .child(div().text_size(px(13.)).child(folder.label().to_owned()))
                .child(div().flex_1())
                .child(
                    div()
                        .truncate()
                        .text_size(px(11.5))
                        .text_color(p.faint)
                        .child(short_path(&folder.path)),
                )
                .on_click(move |_, _, cx| sheet.update(cx, |sheet, cx| sheet.set_cwd(&path, cx))),
            p,
        )
    }

    fn worktree_rows(&self, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let read = self.sheet.read(cx);
        let setup = read.setup();
        let sheet = self.sheet.clone();
        let mut rows = div().flex().flex_col().gap(px(6.)).child(
            Switch::new("worktree")
                .checked(setup.worktree)
                .label("Git worktree")
                .on_click(move |on, _, cx| {
                    let on = *on;
                    sheet.update(cx, |sheet, cx| sheet.set_worktree(on, cx));
                }),
        );
        if setup.worktree {
            let mut kinds = div().flex().gap(px(6.));
            for (existing, label) in [(false, "New branch"), (true, "Existing branch")] {
                let sheet = self.sheet.clone();
                let button = Button::new(ElementId::Name(format!("branch-kind-{existing}").into()))
                    .label(label)
                    .toggled(setup.existing == existing)
                    .small()
                    .on_click(move |_, _, cx| {
                        sheet.update(cx, |sheet, cx| sheet.set_existing(existing, cx));
                    });
                kinds = kinds.child(if setup.existing == existing {
                    button.primary()
                } else {
                    button.ghost()
                });
            }
            rows = rows.child(kinds);
            if setup.existing {
                let branches = read.branches().map_or_default(|b| b.branches.clone());
                let label: SharedString = if setup.branch.is_empty() {
                    "Choose…".into()
                } else {
                    setup.branch.clone().into()
                };
                let sheet = self.sheet.clone();
                rows = rows.child(
                    Button::new("branch-pick")
                        .label(label)
                        .small()
                        .ghost()
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            for branch in &branches {
                                let (sheet, branch) = (sheet.clone(), branch.clone());
                                menu = menu.item(PopupMenuItem::new(branch.clone()).on_click(
                                    move |_, _, cx| {
                                        sheet.update(cx, |sheet, cx| sheet.set_branch(&branch, cx));
                                    },
                                ));
                            }
                            menu
                        }),
                );
            } else {
                rows = rows.child(Input::new(&self.branch).small());
            }
        }
        let note = if setup.worktree {
            "Its own checkout under ~/Documents/worktrees; the folder's own stays as it is."
        } else {
            "It works in the folder itself."
        };
        field(p, "Worktree", rows.child(hint(p, note)))
    }

    fn account_row(&self, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let read = self.store.read(cx);
        let setup = self.sheet.read(cx).setup();
        let zone = jiff::tz::TimeZone::system();
        let accounts: Vec<(String, String)> = read.state().map_or_default(|state| {
            accounts_for(state, &setup.machine)
                .into_iter()
                .map(|a| (a.id.clone(), room_line(a, &zone)))
                .collect()
        });
        let label: SharedString = setup
            .account
            .as_ref()
            .and_then(|id| {
                accounts
                    .iter()
                    .find(|(a, _)| a == id)
                    .map(|(_, l)| l.clone())
            })
            .unwrap_or_else(|| "Most room".to_owned())
            .into();
        let sheet = self.sheet.clone();
        let picker = Button::new("account-pick")
            .label(label)
            .small()
            .ghost()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                let most = sheet.clone();
                menu = menu.item(PopupMenuItem::new("Most room").on_click(move |_, _, cx| {
                    most.update(cx, |s, cx| s.set_account(None, cx));
                }));
                for (id, line) in &accounts {
                    let (sheet, id) = (sheet.clone(), id.clone());
                    menu = menu.item(PopupMenuItem::new(line.clone()).on_click(move |_, _, cx| {
                        let id = id.clone();
                        sheet.update(cx, |sheet, cx| sheet.set_account(Some(id), cx));
                    }));
                }
                menu
            });
        field(
            p,
            "Claude account",
            div().flex().flex_col().gap(px(4.)).child(picker).child(hint(
                p,
                "Most room picks the account with the most left, and it moves to another by itself if that one runs out.",
            )),
        )
    }

    fn model_row(&self, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let setup = self.sheet.read(cx).setup();
        let (sheet, effort_sheet) = (self.sheet.clone(), self.sheet.clone());
        let model = Button::new("model-pick")
            .label(model_name(&setup.model).to_owned())
            .small()
            .ghost()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for (id, name) in MODELS {
                    let sheet = sheet.clone();
                    menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                        sheet.update(cx, |sheet, cx| sheet.set_model(id, cx));
                    }));
                }
                menu
            });
        let effort = Button::new("effort-pick")
            .label(setup.effort.name().to_owned())
            .small()
            .ghost()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                for effort in Effort::ALL {
                    let sheet = effort_sheet.clone();
                    let name = effort.name().to_owned();
                    menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                        let effort = effort.clone();
                        sheet.update(cx, |sheet, cx| sheet.set_effort(effort, cx));
                    }));
                }
                menu
            });
        field(
            p,
            "Model and effort",
            div().flex().gap(px(6.)).child(model).child(effort),
        )
    }

    /// The folder browser, in place of the form while it is open.
    fn browser(&self, p: &Pal, cx: &Context<'_, Self>) -> Div {
        let read = self.sheet.read(cx);
        let machine = machine_place(self.store.read(cx).state(), read.setup().machine.as_str());
        let Some((path, listing)) = read.browsing() else {
            return div();
        };
        let sheet = self.sheet.clone();
        let title = if path.is_empty() {
            format!("Folder on the {machine}")
        } else {
            folder_name(path)
        };
        let mut body = div().flex().flex_col().gap(px(6.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    Button::new("browse-back")
                        .label("Back")
                        .small()
                        .ghost()
                        .on_click(move |_, _, cx| sheet.update(cx, SessionSheet::browse_back)),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(13.5))
                        .child(title),
                ),
        );
        match listing {
            Listing::Loading => body = body.child(spinner(p, "browse-loading", 16., cx)),
            Listing::Failed => {
                body = body.child(hint(p, &format!("The {machine}'s runner didn't answer.")));
            }
            Listing::Ready(list) if path.is_empty() => {
                if !list.projects.is_empty() {
                    body = body.child(heading(p, "Your projects"));
                    for (i, folder) in list.projects.iter().enumerate() {
                        body = body.child(self.folder_pick(p, ("project", i), folder));
                    }
                }
                if !list.recent.is_empty() {
                    body = body.child(heading(p, "Recent"));
                    for (i, folder) in list.recent.iter().enumerate() {
                        body = body.child(self.folder_pick(p, ("recent", i), folder));
                    }
                }
                body = body.child(heading(p, "Browse"));
                let documents = list
                    .documents
                    .clone()
                    .unwrap_or_else(|| "~/Documents".to_owned());
                let sheet = self.sheet.clone();
                body = body.child(
                    Button::new("browse-documents")
                        .label("Documents")
                        .small()
                        .ghost()
                        .on_click(move |_, _, cx| {
                            let documents = documents.clone();
                            sheet.update(cx, |s, cx| s.browse(Some(documents), cx));
                        }),
                );
                body = body
                    .child(heading(p, "Or type it"))
                    .child(Input::new(&self.typed).small());
            }
            Listing::Ready(list) => {
                let (sheet, chosen) = (self.sheet.clone(), path.clone());
                body = body
                    .child(
                        Button::new("use-folder")
                            .label("Use this folder")
                            .small()
                            .primary()
                            .on_click(move |_, _, cx| {
                                sheet.update(cx, |s, cx| s.set_cwd(&chosen, cx));
                            }),
                    )
                    .child(hint(p, &short_path(path)));
                if !list.folders.is_empty() {
                    body = body.child(heading(p, "Inside"));
                    for (i, folder) in list.folders.iter().enumerate() {
                        body = body.child(self.inside(p, i, folder));
                    }
                }
            }
        }
        body
    }
}

impl NewSessionView {
    /// A folder inside the one browsed: a click browses into it.
    fn inside(&self, p: &Pal, i: usize, folder: &Folder) -> impl IntoElement {
        let (sheet, inside) = (self.sheet.clone(), folder.path.clone());
        tab_stop(
            div()
                .id(ElementId::NamedInteger("inside".into(), i as u64))
                .button_named(folder.label().to_owned())
                .hint("Opens the folder")
                .border_1()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(4.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .child(crate::ui::tasks::marks::glyph(
                    if folder.git == Some(true) {
                        gpui_kit::assets::IconName::GitBranch
                    } else {
                        gpui_kit::assets::IconName::Folder
                    },
                    p.muted,
                    13.,
                ))
                .child(div().text_size(px(13.)).child(folder.label().to_owned()))
                .on_click(move |_, _, cx| {
                    let inside = inside.clone();
                    sheet.update(cx, |s, cx| s.browse(Some(inside), cx));
                }),
            p,
        )
    }
}

/// A folder's last part.
fn folder_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_owned()
}

/// A labelled part of the form.
fn field(p: &Pal, label: &'static str, body: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .py(px(6.))
        .child(heading(p, label))
        .child(body)
}

fn heading(p: &Pal, text: &'static str) -> Div {
    div()
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.muted)
        .child(text)
}

fn hint(p: &Pal, text: &str) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(p.faint)
        .child(text.to_owned())
}

impl Render for NewSessionView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let can_open = self.sheet.read(cx).setup().can_open();
        let browsing = self.sheet.read(cx).browse_depth() > 0;
        let body = if browsing {
            self.browser(&p, cx)
        } else {
            div()
                .flex()
                .flex_col()
                .child(self.machine_row(&p, cx))
                .child(self.folder_rows(&p, cx))
                .child(self.worktree_rows(&p, cx))
                .child(self.account_row(&p, cx))
                .child(self.model_row(&p, cx))
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(p.text.opacity(0.18))
            .child(
                div()
                    .id("new-session-sheet")
                    .read_as(Role::Dialog, "New session")
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|this, _: &CloseSheet, window, cx| {
                        (this.close)(window, cx);
                    }))
                    .w(px(480.))
                    .max_w_full()
                    .max_h(px(620.))
                    .flex()
                    .flex_col()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.surface)
                    .shadow_lg()
                    .child(
                        div()
                            .px(px(18.))
                            .pt(px(16.))
                            .pb(px(8.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(15.))
                            .child("New session"),
                    )
                    .child(
                        div()
                            .id("new-session-body")
                            .flex_1()
                            .overflow_y_scroll()
                            .px(px(18.))
                            .pb(px(12.))
                            .child(body),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .px(px(14.))
                            .py(px(10.))
                            .border_t_1()
                            .border_color(p.border)
                            .child(Button::new("new-session-cancel").label("Cancel").on_click(
                                cx.listener(|this, _, window, cx| {
                                    (this.close)(window, cx);
                                }),
                            ))
                            .child(
                                Button::new("new-session-open")
                                    .primary()
                                    .label("Open")
                                    .disabled(!can_open)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.open(window, cx)),
                                    ),
                            ),
                    ),
            )
    }
}
