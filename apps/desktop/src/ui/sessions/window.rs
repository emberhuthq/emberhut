//! The Sessions window of its own (⇧⌘T): the Sessions screen, where a session, a helper's
//! steps, an external thread, a new session's draft or a task opens in place over the list, with Back
//! to the page under it (from the first, back to the list). It has a column of its own, as a
//! main window does, so the pages open in it the same way.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyView, App, Entity, FocusHandle, Window, WindowId, div, px};

use super::{NewSession, SessionPage, SessionsView};
use crate::shell::{WindowKind, windows};
use crate::store::{Page, Store};
use crate::ui::column::{self, Column, GoBack};
use crate::ui::move_to_task::{self, MoveEvent, MoveToTask, MoveToTaskView};
use crate::ui::palette::pal;
use crate::ui::task_page::TaskView;
use crate::ui::tasks::marks::glyph;

/// Registers the window's view, and the shortcut that opens it (once the windows are set up).
pub(crate) fn install(cx: &mut App) {
    windows::register_view(
        WindowKind::Sessions,
        Rc::new(|store, window, cx| {
            AnyView::from(cx.new(|cx| SessionsWindow::new(store, window, cx)))
        }),
        cx,
    );
    cx.on_action(|_: &super::OpenSessionsWindow, cx| {
        if !windows::open_single(WindowKind::Sessions, cx) {
            log::debug!("no Sessions window in this build");
        }
    });
}

/// The window's root.
pub(crate) struct SessionsWindow {
    store: Entity<Store>,
    window: WindowId,
    column: Entity<Column>,
    list: Entity<SessionsView>,
    session: Entity<SessionPage>,
    task: Entity<TaskView>,
    focus: FocusHandle,
    /// Move to task…'s picker, over the window while it is up.
    move_picker: Option<Entity<MoveToTaskView>>,
}

impl SessionsWindow {
    fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let column = Column::new(store.clone(), window, cx);
        cx.observe(&column, |_, _, cx| cx.notify()).detach();
        let list = cx.new(|cx| SessionsView::new(store.clone(), cx));
        let session = cx.new(|cx| SessionPage::new(store.clone(), &column, window, cx));
        let task = cx.new(|cx| TaskView::new(store.clone(), column.clone(), window, cx));
        let focus = cx.focus_handle();
        list.update(cx, |list, cx| list.focus(window, cx));
        // The window closed: its column is forgotten, so the store stops fetching for it.
        cx.on_release(|this: &mut Self, cx| column::closed(this.window, &this.store, cx))
            .detach();
        Self {
            window: window.window_handle().window_id(),
            store,
            column,
            list,
            session,
            task,
            focus,
            move_picker: None,
        }
    }

    /// The picker closed (its session moved: the task's page opens over the one shown).
    fn move_answered(
        &mut self,
        event: &MoveEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.move_picker = None;
        if let MoveEvent::Moved(task) = *event {
            self.column
                .update(cx, |column, cx| column.push(Page::Task(task), cx));
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }
}

impl Render for SessionsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let page = self.column.read(cx).current();
        let shown = match page {
            Some(Page::Session(_)) => self.session.clone().into_any_element(),
            Some(Page::Task(_)) => self.task.clone().into_any_element(),
            _ => self.list.clone().into_any_element(),
        };
        let bar = page.is_some().then(|| {
            let column = self.column.clone();
            div()
                .flex()
                .items_center()
                .h(px(40.))
                .px(px(10.))
                .border_b_1()
                .border_color(p.border)
                .child(
                    Button::new("sessions-window-back")
                        .ghost()
                        .small()
                        .icon(glyph(IconName::ChevronLeft, p.muted, 15.))
                        .label("Back")
                        .on_click(move |_, _, cx| column.update(cx, Column::back)),
                )
        });
        div()
            .id("sessions-window")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &GoBack, _, cx| {
                this.column.update(cx, Column::back);
            }))
            .on_action(cx.listener(|this, action: &MoveToTask, window, cx| {
                let picker =
                    move_to_task::open(&this.store, action, window, cx, Self::move_answered);
                this.move_picker = Some(picker);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NewSession, window, cx| {
                this.column.update(cx, Column::close);
                this.list
                    .update(cx, |list, cx| list.new_session(window, cx));
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.background)
            .text_color(p.text)
            .children(bar)
            .child(div().flex_1().min_h_0().child(shown))
            .children(self.move_picker.clone())
    }
}
