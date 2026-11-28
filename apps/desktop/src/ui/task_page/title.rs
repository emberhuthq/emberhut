//! A task's title at the top of its page, and its rename: Edit title (the pencil beside it)
//! puts the title in a field; Return or Save sends it, Esc or Cancel leaves it as it was. While
//! the server has it the field waits; a refusal (an empty or too long title) shows the
//! server's reason under it, and the field stays for another try.

use emberhut_api::ids::TaskId;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{Div, Entity, FontWeight, Window, div, px};

use super::{Part, TaskView};
use crate::store::tasks::rename::new_title;
use crate::ui::a11y::Accessible as _;
use crate::ui::reads::reads;

/// A rename under way.
#[derive(Debug, Default)]
pub(super) struct Renaming {
    /// The server has it: Save waits.
    busy: bool,
    /// Why the last try didn't take.
    problem: Option<String>,
}

/// The title's field, which saves on Return.
pub(super) fn field(window: &mut Window, cx: &mut Context<'_, TaskView>) -> Entity<InputState> {
    let field = cx.new(|cx| InputState::new(window, cx).placeholder("The task's title"));
    cx.subscribe_in(&field, window, |this, _, event: &InputEvent, _, cx| {
        if let InputEvent::PressEnter { .. } = event {
            this.save_title(cx);
        }
    })
    .detach();
    field
}

impl TaskView {
    /// Edit title: the field takes the title and the keyboard.
    pub(super) fn edit_title(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(title) = self.shown_title(cx) else {
            return;
        };
        self.title_field.update(cx, |field, cx| {
            field.set_value(title, window, cx);
            field.focus(window, cx);
        });
        self.renaming = Some(Renaming::default());
        cx.notify();
    }

    /// Leaves the title as it was.
    pub(super) fn cancel_title(&mut self, cx: &mut Context<'_, Self>) {
        self.renaming = None;
        cx.notify();
    }

    /// Sends the title typed, unless it is the task's (the field closes then).
    pub(super) fn save_title(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(task), Some(current)) = (self.shown, self.shown_title(cx)) else {
            return;
        };
        let Some(renaming) = self.renaming.as_mut().filter(|renaming| !renaming.busy) else {
            return;
        };
        let typed = self.title_field.read(cx).value().to_string();
        let Some(title) = new_title(&current, &typed) else {
            self.renaming = None;
            cx.notify();
            return;
        };
        renaming.busy = true;
        renaming.problem = None;
        let renamed = self
            .store
            .update(cx, |store, cx| store.rename_task(task, title, cx));
        cx.spawn(async move |this, cx| {
            let renamed = renamed.await;
            this.update(cx, |this, cx| {
                // The page may show another task by now: its own rename isn't touched.
                if this.shown == Some(task) {
                    this.renaming = match renamed {
                        Ok(_) => None,
                        Err(problem) => Some(Renaming {
                            busy: false,
                            problem: Some(problem),
                        }),
                    };
                }
                cx.notify();
            })
            .inspect_err(|error| log::debug!("the task's page is gone: {error}"))
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The title of the task shown.
    fn shown_title(&self, cx: &gpui_kit::App) -> Option<String> {
        let task: TaskId = self.shown?;
        Some(self.store.read(cx).task_of(task)?.title.clone())
    }
}

/// The title, with Edit title beside it; while it is renamed, its field, Save and Cancel.
pub(super) fn title(part: &Part<'_>, view: &TaskView, cx: &Context<'_, TaskView>) -> Div {
    let p = part.p;
    let Some(renaming) = &view.renaming else {
        return div()
            .flex()
            .items_start()
            .gap(px(6.))
            .pt(px(14.))
            .child(
                reads(&part.task.title)
                    .flex_1()
                    .min_w_0()
                    .text_size(px(18.))
                    .font_weight(FontWeight::BOLD),
            )
            .child(
                Button::new("task-edit-title")
                    .ghost()
                    .small()
                    .icon(IconName::Pencil)
                    .accessibility_label("Edit title")
                    .tooltip("Edit title")
                    .on_click(cx.listener(|this, _, window, cx| this.edit_title(window, cx))),
            );
    };
    let busy = renaming.busy;
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .pt(px(10.))
        .capture_action(cx.listener(|this, _: &Escape, _, cx| {
            cx.stop_propagation();
            this.cancel_title(cx);
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(16.))
                        .child(Input::new(&view.title_field)),
                )
                .child(
                    Button::new("task-title-save")
                        .primary()
                        .small()
                        .label("Save")
                        .loading(busy)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.save_title(cx))),
                )
                .child(
                    Button::new("task-title-cancel")
                        .ghost()
                        .small()
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_title(cx))),
                ),
        )
        .children(renaming.problem.clone().map(|problem| {
            div()
                .id("task-title-problem")
                .announced(problem.clone())
                .text_size(px(12.))
                .text_color(p.danger)
                .child(problem)
                .test_support()
        }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, size};
    use serde_json::json;

    use crate::backend::Error;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::fixtures::{issue, state, task_page};
    use crate::store::Store;
    use crate::ui::palette::Pal;
    use crate::ui::window::{self, MainWindow};

    const SELECT_ALL: &str = if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    };

    fn click(cx: &mut VisualTestContext, id: &'static str) {
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        });
        cx.run_until_parked();
    }

    fn shows(cx: &mut VisualTestContext, id: &'static str) -> bool {
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
    }

    /// With and without plans, Edit title puts the title in a field; a refusal keeps it up with
    /// the server's reason, and a title the server takes closes it.
    #[gpui_kit::test]
    fn edit_title_renames_the_task_or_says_why_not(cx: &mut TestAppContext) {
        for plans in [false, true] {
            cx.update(|cx| {
                gpui_kit::init(cx);
                cx.set_global(Pal::new(&crate::theme::built_in(), false));
                window::bind_keys(cx);
            });
            let backend = Arc::new(FakeBackend::new([]));
            backend.put_task(task_page(4, json!({})));
            backend.push_rename(Err(Error::Refused {
                status: Some(400),
                message: Some("a title is 1 to 200 characters".to_owned()),
            }));
            let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
            let store = cx.new(|_| Store::for_tests(shared));
            let (view, cx) = cx.add_window_view(|window, cx| {
                MainWindow::new(
                    store.clone(),
                    crate::ui::report::ReportSize::default(),
                    window,
                    cx,
                )
            });
            cx.simulate_resize(size(gpui_kit::px(1440.), gpui_kit::px(900.)));
            store.update(cx, |store, cx| {
                let ui = json!({"hideProjects": true, "hideGoals": true, "taskPlans": plans});
                let state = state(json!({"ui": ui, "issues": [issue(4, "todo", json!({}))]}));
                store.take_state_for_tests(state, cx);
            });
            view.update_in(cx, |main, window, cx| {
                main.open_task(emberhut_api::ids::TaskId::new(4), window, cx);
            });
            cx.run_until_parked();

            click(cx, "task-edit-title");
            cx.simulate_keystrokes(SELECT_ALL);
            cx.simulate_input("   ");
            click(cx, "task-title-save");
            assert!(
                shows(cx, "task-title-problem"),
                "plans {plans}: the reason shows"
            );

            cx.simulate_keystrokes(SELECT_ALL);
            cx.simulate_input("Bake rye");
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            let renames: Vec<Call> = backend
                .calls()
                .into_iter()
                .filter(|call| matches!(call, Call::RenameTask(..)))
                .collect();
            let task = emberhut_api::ids::TaskId::new(4);
            assert_eq!(
                renames,
                [
                    Call::RenameTask(task, String::new()),
                    Call::RenameTask(task, "Bake rye".to_owned())
                ],
                "plans {plans}"
            );
            assert!(
                shows(cx, "task-edit-title"),
                "plans {plans}: the field closed"
            );
            assert_eq!(
                store.read_with(cx, |store, _| store.task_of(task).unwrap().title.clone()),
                "Bake rye"
            );
        }
    }
}
