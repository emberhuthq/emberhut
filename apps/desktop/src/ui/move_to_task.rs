//! Move to task…, on a server that keeps plans: a session's head, its rows' menus (Sessions,
//! the Tasks list's sessions with no task, a task page's sessions) and a task page's session
//! lines offer it. It brings a card over the window, as the spotlight does: the session's name,
//! a field (`Find a task: T-212 or its title`), and the open tasks it can move to, the latest
//! first. ↑ and ↓ move the pick, Return or a click moves the session there; Esc or a click
//! outside closes it. Once the server takes the move, the card closes and the task's page opens
//! over the column (Back returns); a refusal shows the server's reason on the card.

use emberhut_api::ids::TaskId;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, App, ClickEvent, Div, ElementId, Entity, EventEmitter, FocusHandle, FontWeight, Role,
    ScrollHandle, Stateful, Window, div, px,
};
use jiff::ZonedDateTime;

use super::a11y::{self, Accessible as _};
use super::palette::{Pal, pal};
use super::spotlight::task_hit;
use super::tasks::marks::glyph;
use super::window::MainWindow;
use crate::store::tasks::moving::Moving;
use crate::store::tasks::words;
use crate::store::{Page, Store};

/// The card's width.
const WIDTH: f32 = 560.;

/// The card's height at most.
const HEIGHT: f32 = 460.;

/// How far from the window's top the card sits.
const TOP: f32 = 70.;

/// The words of the action, on its buttons and menu items.
pub(crate) const LABEL: &str = "Move to task…";

/// Brings the picker up for a session, in the window it is dispatched in.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = tasks, no_json)]
pub(crate) struct MoveToTask {
    /// The session to move.
    pub(crate) moving: Moving,
}

/// What the picker asks of its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MoveEvent {
    /// Close it.
    Close,
    /// The session is on this task now: close it and open the task's page.
    Moved(TaskId),
}

/// The picker's card.
pub(crate) struct MoveToTaskView {
    store: Entity<Store>,
    moving: Moving,
    field: Entity<InputState>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    /// The task the arrows picked, by its place in the list.
    selected: usize,
    /// The pick the list was last scrolled to, so a new pick is brought into view once.
    shown_pick: Option<usize>,
    /// A move on its way: another pick waits for it.
    busy: bool,
    /// Why the last move didn't happen.
    problem: Option<String>,
}

impl EventEmitter<MoveEvent> for MoveToTaskView {}

impl MoveToTaskView {
    /// The picker for `moving` over `store`, its field taking the keyboard.
    pub(crate) fn new(
        store: Entity<Store>,
        moving: Moving,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let field = cx.new(|cx| {
            InputState::new(window, cx).placeholder(format!(
                "Find a task: {}212 or its title",
                crate::wire::TASK_KEY_PREFIX
            ))
        });
        field.update(cx, |field, cx| field.focus(window, cx));
        cx.subscribe_in(
            &field,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    this.selected = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    if let Some(task) = this.targets(cx).get(this.selected).copied() {
                        this.pick(task, cx);
                    }
                }
                _ => {}
            },
        )
        .detach();
        Self {
            store,
            moving,
            field,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            selected: 0,
            shown_pick: None,
            busy: false,
            problem: None,
        }
    }

    /// The tasks offered for what is typed, by id.
    fn targets(&self, cx: &App) -> Vec<TaskId> {
        let query = self.field.read(cx).value();
        self.store
            .read(cx)
            .move_targets(&self.moving, &query)
            .iter()
            .map(|task| task.id)
            .collect()
    }

    /// Moves the session to `task`, unless a move is on its way.
    fn pick(&mut self, task: TaskId, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.problem = None;
        let session = self.moving.session.clone();
        let moved = self
            .store
            .update(cx, |store, cx| store.move_session(session, task, cx));
        cx.spawn(async move |this, cx| {
            let moved = moved.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match moved {
                    Ok(task) => cx.emit(MoveEvent::Moved(task)),
                    Err(problem) => this.problem = Some(problem),
                }
                cx.notify();
            })
            .inspect_err(|error| log::debug!("the picker closed: {error}"))
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Moves the pick by `step` within `count` tasks.
    fn step(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        let last = self.targets(cx).len().saturating_sub(1);
        self.selected = self
            .selected
            .min(last)
            .saturating_add_signed(step)
            .min(last);
        cx.notify();
    }

    /// The card's head: what moves, and the task it is on now.
    fn head(&self, p: &Pal, cx: &App) -> Div {
        let store = self.store.read(cx);
        let on: Vec<String> = self
            .moving
            .from
            .iter()
            .map(|task| store.task_key(*task))
            .collect();
        let now = if on.is_empty() {
            "On no task now".to_owned()
        } else {
            format!("On {} now", on.join(", "))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(16.))
            .pt(px(12.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(format!("Move “{}” to a task", self.moving.title)),
            )
            .child(div().text_size(px(11.5)).text_color(p.muted).child(now))
    }

    /// The tasks offered, as rows, and the place of the picked one.
    fn list(&self, p: &Pal, cx: &Context<'_, Self>) -> (Stateful<Div>, Option<usize>) {
        let store = self.store.read(cx);
        let query = self.field.read(cx).value();
        let targets = store.move_targets(&self.moving, &query);
        let selected = self.selected.min(targets.len().saturating_sub(1));
        let mut list = div()
            .id("move-to-task-results")
            .role(Role::ListBox)
            .aria_label("Open tasks")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .py(px(6.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        if targets.is_empty() {
            let empty = if query.trim().is_empty() {
                "No other open task.".to_owned()
            } else {
                format!("No open task matches “{}”.", query.trim())
            };
            return (
                list.child(
                    div()
                        .px(px(16.))
                        .py(px(10.))
                        .text_size(px(13.))
                        .text_color(p.muted)
                        .child(empty),
                ),
                None,
            );
        }
        let today = ZonedDateTime::now().date();
        for (at, task) in targets.iter().enumerate() {
            let picked = at == selected;
            let id = task.id;
            list = list.child(
                div()
                    .id(ElementId::Name(format!("move-target-{id}").into()))
                    .role(Role::ListBoxOption)
                    .aria_selected(picked)
                    .aria_label(a11y::line(&[&task.title, &task.key, &words::name(task)]))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .mx(px(4.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(picked, |d| d.bg(p.accent.opacity(0.16)))
                    .when(!picked, |d| d.hover(|d| d.bg(p.hover)))
                    .children(task_hit(p, store, task, today))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(id, cx)))
                    .test_support(),
            );
        }
        (list, Some(selected))
    }
}

impl Render for MoveToTaskView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let (list, picked) = self.list(&p, cx);
        if picked != self.shown_pick {
            self.shown_pick = picked;
            if let Some(child) = picked {
                self.scroll.scroll_to_item(child);
            }
        }
        let field = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(10.))
            .border_b_1()
            .border_color(p.border)
            .child(glyph(IconName::Search, p.muted, 16.))
            .child(
                div()
                    .flex_1()
                    .text_size(px(15.))
                    .child(Input::new(&self.field).appearance(false)),
            )
            .when(self.busy, |d| {
                d.child(glyph(IconName::LoaderCircle, p.accent, 15.))
            });
        let problem = self.problem.clone().map(|problem| {
            div()
                .id("move-to-task-problem")
                .announced(problem.clone())
                .px(px(16.))
                .py(px(8.))
                .text_size(px(12.))
                .text_color(p.danger)
                .child(problem)
        });
        div()
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .bg(gpui_kit::black().opacity(0.12))
            // What is under the dimmed backdrop takes no click or hover while it shows.
            .occlude()
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(MoveEvent::Close)),
            )
            .child(
                div()
                    .id("move-to-task")
                    .read_as(Role::Dialog, "Move to a task")
                    .track_focus(&self.focus)
                    .key_context("MoveToTask")
                    .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                        cx.stop_propagation();
                        this.step(-1, cx);
                    }))
                    .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                        cx.stop_propagation();
                        this.step(1, cx);
                    }))
                    .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                        cx.stop_propagation();
                        cx.emit(MoveEvent::Close);
                    }))
                    .mt(px(TOP))
                    .w(px(WIDTH))
                    .max_h(px(HEIGHT))
                    .flex()
                    .flex_col()
                    .rounded(px(16.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.surface_raised)
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(self.head(&p, cx))
                    .child(field)
                    .children(problem)
                    .child(list),
            )
    }
}

impl MainWindow {
    /// Brings Move to task…'s picker up over the window.
    pub(super) fn open_move_picker(
        &mut self,
        action: &MoveToTask,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.move_picker = Some(open(
            &self.store,
            action,
            window,
            cx,
            |this, event, window, cx| {
                this.move_picker = None;
                if let MoveEvent::Moved(task) = *event {
                    this.push(Page::Task(task), window, cx);
                }
                this.keep_focus(window, cx);
                cx.notify();
            },
        ));
        cx.notify();
    }
}

/// The picker `action` asks for, in a window whose root `V` hears what it asks
/// (`answered`).
pub(crate) fn open<V: 'static>(
    store: &Entity<Store>,
    action: &MoveToTask,
    window: &mut Window,
    cx: &mut Context<'_, V>,
    answered: fn(&mut V, &MoveEvent, &mut Window, &mut Context<'_, V>),
) -> Entity<MoveToTaskView> {
    let view = cx.new(|cx| MoveToTaskView::new(store.clone(), action.moving.clone(), window, cx));
    cx.subscribe_in(&view, window, move |this, _, event, window, cx| {
        answered(this, event, window, cx);
    })
    .detach();
    view
}

/// Brings the picker up in `window` for `moving`.
fn ask(moving: &Moving, window: &mut Window, cx: &mut App) {
    window.dispatch_action(
        Box::new(MoveToTask {
            moving: moving.clone(),
        }),
        cx,
    );
}

/// The action as a button (a session's head).
pub(crate) fn button(id: impl Into<ElementId>, moving: Moving) -> Button {
    Button::new(id)
        .label(LABEL)
        .small()
        .ghost()
        .tooltip("Count this session's work on another task")
        .on_click(move |_: &ClickEvent, window, cx| ask(&moving, window, cx))
}

/// The action as a quiet link at the end of a line (a task page's session lines).
pub(crate) fn link(id: impl Into<ElementId>, moving: Moving, p: &Pal) -> impl IntoElement {
    super::controls::tab_stop(
        div()
            .id(id)
            .button_named(format!("Move “{}” to another task", moving.title))
            .hint("Count this session's work on another task")
            .border_1()
            .rounded(px(5.))
            .px(px(4.))
            .flex_none()
            .text_size(px(11.5))
            .text_color(p.muted)
            .cursor_pointer()
            .hover(|d| d.text_color(p.accent))
            .child(LABEL)
            .on_click(move |_: &ClickEvent, window, cx| ask(&moving, window, cx)),
        p,
    )
    .test_support()
}

/// The action as an item of a row's right-click menu.
pub(crate) fn menu_item(moving: Moving) -> PopupMenuItem {
    PopupMenuItem::new(LABEL).on_click(move |_: &ClickEvent, window, cx| ask(&moving, window, cx))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::ids::SessionId;
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt as _;
    use serde_json::json;

    use super::*;
    use crate::backend::Error;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::fixtures::issue;

    fn picker(
        cx: &mut TestAppContext,
        backend: &Arc<FakeBackend>,
    ) -> gpui_kit::WindowHandle<MoveToTaskView> {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let shared = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        let state = crate::fixtures::state(json!({
            "ui": {"hideProjects": true, "hideGoals": true, "taskPlans": true},
            "issues": [issue(1, "todo", json!({})), issue(2, "working", json!({})),
                       issue(3, "done", json!({}))],
        }));
        store.update(cx, |store, cx| store.show_state_for_tests(state, cx));
        let moving = Moving::new(SessionId::from("r-1"), "A session", vec![TaskId::new(1)]);
        cx.add_window(|window, cx| MoveToTaskView::new(store, moving, window, cx))
    }

    fn click(
        cx: &mut TestAppContext,
        window: gpui_kit::WindowHandle<MoveToTaskView>,
        id: &'static str,
    ) {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn a_pick_moves_the_session_and_says_so(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let window = picker(cx, &backend);
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = std::rc::Rc::clone(&events);
        window
            .update(cx, |_, _, cx| {
                cx.subscribe_self(move |_, event: &MoveEvent, _| seen.borrow_mut().push(*event))
                    .detach();
            })
            .unwrap();
        click(cx, window, "move-target-2");
        assert_eq!(
            backend.calls().last(),
            Some(&Call::MoveSession(SessionId::from("r-1"), TaskId::new(2)))
        );
        assert_eq!(*events.borrow(), [MoveEvent::Moved(TaskId::new(2))]);
    }

    #[gpui_kit::test]
    fn a_refusal_shows_the_servers_reason(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        backend.sessions.push_move(Err(Error::Refused {
            status: Some(400),
            message: Some("T-2 is done".into()),
        }));
        let window = picker(cx, &backend);
        click(cx, window, "move-target-2");
        window
            .read_with(cx, |picker, _| {
                assert_eq!(picker.problem.as_deref(), Some("Not moved: T-2 is done"));
                assert!(!picker.busy);
            })
            .unwrap();
    }

    /// The task page's head offers Move to task… for its session: the picker comes up over the
    /// main window, and once the server takes the move, the task picked opens over the page.
    #[gpui_kit::test]
    fn from_a_task_page_the_session_moves_and_its_new_task_opens(cx: &mut TestAppContext) {
        use gpui_kit::{VisualTestContext, size};

        use crate::store::Page;
        use crate::ui::window::{self, MainWindow};

        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
            window::bind_keys(cx);
        });
        let backend = Arc::new(FakeBackend::new([]));
        let session = json!({"id": "r-1", "machine": "pc", "title": "A session", "project": "",
            "status": "running", "updatedAt": "2026-11-28T08:00:00Z", "place": "PC",
            "doing": null, "account": null, "accountName": null, "ask": null, "last": null,
            "direct": true, "model": null, "tasks": [1]});
        backend.put_task(crate::fixtures::task_page(
            1,
            json!({"sessions": [session]}),
        ));
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
        cx.simulate_resize(size(px(1440.), px(900.)));
        store.update(cx, |store, cx| {
            let state = crate::fixtures::state(json!({
                "ui": {"hideProjects": true, "hideGoals": true, "taskPlans": true},
                "issues": [issue(1, "working", json!({})), issue(2, "todo", json!({}))],
            }));
            store.take_state_for_tests(state, cx);
        });
        view.update_in(cx, |main, window, cx| {
            main.open_task(TaskId::new(1), window, cx);
        });
        cx.run_until_parked();
        let click = |cx: &mut VisualTestContext, id: &'static str| {
            cx.update(|window, cx| {
                window.render_frame(cx);
                window.click(id, cx);
            });
            cx.run_until_parked();
        };
        click(cx, "task-head-session-move");
        view.read_with(cx, |main, _| {
            assert!(main.move_picker.is_some(), "the picker is up");
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("move-target-2").is_some(), "the other task");
            assert!(
                window.try_find("move-target-1").is_none(),
                "not the task it is on"
            );
        });
        click(cx, "move-target-2");
        assert!(
            backend
                .calls()
                .contains(&Call::MoveSession(SessionId::from("r-1"), TaskId::new(2)))
        );
        view.read_with(cx, |main, cx| {
            assert_eq!(main.column_page(cx), Some(Page::Task(TaskId::new(2))));
        });
        view.read_with(cx, |main, _| {
            assert!(main.move_picker.is_none(), "the picker closed");
        });
    }
}
