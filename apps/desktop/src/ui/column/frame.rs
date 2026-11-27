//! How the column is drawn beside the chat and driven from the keyboard: its bar (Back, the
//! title a page without a head of its own wants, Close) over the page, and ⌘[ Back on the main
//! window. Esc closes it as it closes the report (the window's `CloseReport`), and its width is
//! one rule for every page ([`crate::ui::task_page::column_width`]).

use gpui_kit::component::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, FontWeight, KeyBinding, Pixels, SharedString, Window, actions, div, px,
};

use super::super::controls::icon_button;
use super::super::edge;
use super::super::palette::pal;
use super::super::window::{CHAT, COLUMN, MainWindow};
use super::Column;
use crate::store::{Page, Store};

actions!(
    column,
    [
        /// Back to the page under the one in the column (from the first, to the chat alone).
        GoBack,
        /// Closes the column beside the chat, whatever it shows.
        CloseColumn,
    ]
);

/// The height of the column's bar.
const BAR_HEIGHT: f32 = 36.;

/// Binds the column's key: ⌘[ Back (Ctrl elsewhere). Esc closes it as it closes the report
/// (the window's `CloseReport`).
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-[", GoBack, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-[", GoBack, None),
    ]);
}

/// The page's name, for Close's tip.
#[must_use]
pub(crate) fn name(page: Page) -> &'static str {
    match page {
        Page::Room(_) => "Chat",
        Page::Needs => "Needs you",
        Page::Tasks => "Tasks",
        Page::Machines => "Machines",
        Page::Settings => "Settings",
        Page::Task(_) => "Task",
        Page::Project(_) => "Project",
        Page::Goal(_) => "Goal",
        Page::Person(_) => "Person",
        Page::Report => "Weekly report",
        Page::Sessions => "Sessions",
        Page::Session(_) => "Session",
        Page::Environment(_) => "Environment",
        Page::Jobs => "Background jobs",
        Page::Job(_) => "Background job",
        Page::Scheduled => "Scheduled",
        Page::SettingsPane(settings) => settings.title(),
        Page::Approvals(_) => "Approvals",
        Page::Approval(_) => "Approval request",
    }
}

/// The title in the column's bar: only for a page without a head of its own (a task's page
/// shows its key there); the others name themselves in their head.
#[must_use]
pub(crate) fn title(page: Page, store: &Store) -> Option<SharedString> {
    match page {
        Page::Task(task) => Some(store.task_key(task).into()),
        Page::Approval(id) => store
            .approval(id)
            .map(|approval| crate::store::approvals::words::kind_name(&approval.kind).into()),
        Page::Room(_)
        | Page::Needs
        | Page::Tasks
        | Page::Machines
        | Page::Settings
        | Page::Project(_)
        | Page::Goal(_)
        | Page::Person(_)
        | Page::Report
        | Page::Sessions
        | Page::Session(_)
        | Page::Environment(_)
        | Page::Jobs
        | Page::Job(_)
        | Page::Scheduled
        | Page::SettingsPane(_)
        | Page::Approvals(_) => None,
    }
}

/// The chat with the column beside it, `width` wide, showing `page` as `body` under the bar, its
/// left edge dragging it wider or narrower ([`edge`]); with no width (no room for the column
/// beside the chat) the column alone, in the chat's place, until Back or Close brings the chat
/// back.
pub(crate) fn beside(
    this: &MainWindow,
    page: Page,
    chat: AnyElement,
    body: AnyElement,
    width: Option<Pixels>,
    cx: &mut Context<'_, MainWindow>,
) -> Div {
    let p = pal(cx);
    // In the chat's place, Back from the first page is the way back to the chat.
    let can_go_back = this.column.read(cx).can_go_back() || width.is_none();
    let title = title(page, this.store.read(cx));
    let bar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(BAR_HEIGHT))
        .px(px(8.))
        .when(can_go_back, |d| {
            d.child(
                icon_button("column-back", IconName::ChevronLeft, "Back", &GoBack, &p)
                    .on_click(cx.listener(|this, _, window, cx| this.go_back(window, cx))),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.5))
                .text_color(p.muted)
                .font_weight(FontWeight::MEDIUM)
                .children(title),
        )
        .child(
            icon_button(
                "column-close",
                IconName::Close,
                format!("Close {}", name(page)),
                &CloseColumn,
                &p,
            )
            .on_click(cx.listener(|this, _, window, cx| this.close_column(window, cx))),
        );
    let column = div()
        .flex()
        .flex_col()
        .h_full()
        .bg(p.surface)
        .child(bar)
        .child(div().flex_1().min_h_0().child(body));
    let row = div().size_full().flex();
    match width {
        Some(width) => row
            .child(
                div()
                    .debug_selector(|| CHAT.to_owned())
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(chat),
            )
            .child(
                div()
                    .debug_selector(|| COLUMN.to_owned())
                    .flex()
                    .flex_none()
                    .w(width)
                    .h_full()
                    .child(edge::edge(cx))
                    .child(column.flex_1().min_w_0()),
            ),
        None => row.child(
            column
                .debug_selector(|| COLUMN.to_owned())
                .flex_1()
                .min_w_0(),
        ),
    }
}

impl MainWindow {
    /// Back in this window's column (only while it shows: its keys don't act on a hidden
    /// column).
    pub(crate) fn go_back(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.column_shows(cx) {
            return;
        }
        self.column.update(cx, Column::back);
        self.keep_focus(window, cx);
    }

    /// Closes this window's column.
    pub(crate) fn close_column(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.column.update(cx, Column::close);
        self.keep_focus(window, cx);
    }

    /// The column's actions on the window's root.
    pub(in crate::ui) fn with_column_actions(div: Div, cx: &mut Context<'_, Self>) -> Div {
        div.on_action(cx.listener(|this, _: &GoBack, window, cx| this.go_back(window, cx)))
            .on_action(cx.listener(|this, _: &CloseColumn, window, cx| {
                if this.column_shows(cx) {
                    this.close_column(window, cx);
                }
            }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::ids::TaskId;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Entity, TestAppContext, VisualTestContext};

    use super::*;
    use crate::backend::Backend;
    use crate::backend::fake::FakeBackend;
    use crate::store::OpenReport;
    use crate::store::settings::SettingsPane;
    use crate::ui::palette::Pal;
    use crate::ui::report::ReportSize;
    use crate::ui::window::{CloseReport, ToggleSidebar};

    /// A main window past its gate, so the column shows beside the chat.
    fn open_window(cx: &mut TestAppContext) -> (Entity<MainWindow>, VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        store.update(cx, |store, cx| {
            store.take_state_for_tests(crate::fixtures::state(serde_json::json!({})), cx);
        });
        let window =
            cx.add_window(|window, cx| MainWindow::new(store, ReportSize::default(), window, cx));
        let main = window.root(cx).unwrap();
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.update(Window::render_frame);
        (main, cx)
    }

    fn shown(main: &Entity<MainWindow>, cx: &VisualTestContext) -> Option<Page> {
        main.read_with(cx, |main, cx| main.column.read(cx).current())
    }

    #[gpui_kit::test]
    fn back_esc_and_the_sidebar_work_from_the_keyboard(cx: &mut TestAppContext) {
        let (main, mut cx) = open_window(cx);
        let column = main.read_with(&cx, |main, _| main.column.clone());
        column.update(&mut cx, |column, cx| {
            column.open(Page::Needs, cx);
            column.push(Page::Task(TaskId::new(4)), cx);
        });
        cx.update(Window::render_frame);
        cx.dispatch_action(GoBack);
        assert_eq!(shown(&main, &cx), Some(Page::Needs), "Back");
        let store = main.read_with(&cx, |main, _| main.store.clone());
        store.update(&mut cx, |store, cx| {
            store.show_report(
                Some(OpenReport {
                    name: "Week".into(),
                    path: "/files/week.html".into(),
                }),
                cx,
            );
        });
        cx.update(Window::render_frame);
        cx.dispatch_action(CloseReport);
        assert_eq!(shown(&main, &cx), Some(Page::Needs), "the report first");
        assert!(store.read_with(&cx, |store, _| store.open_report().is_none()));
        cx.update(Window::render_frame);
        cx.dispatch_action(CloseReport);
        assert_eq!(shown(&main, &cx), None, "then the column");

        cx.dispatch_action(ToggleSidebar);
        cx.update(Window::render_frame);
        cx.dispatch_action(ToggleSidebar);
        cx.update(Window::render_frame);
    }

    #[gpui_kit::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "a second main window opened bare, for its typed handle; no unseen run here"
    )]
    fn each_window_has_its_own_column_over_the_one_room(cx: &mut TestAppContext) {
        let (first, mut cx) = open_window(cx);
        let store = first.read_with(&cx, |main, _| main.store.clone());
        let second = cx.update(|_, cx| {
            cx.open_window(gpui_kit::WindowOptions::default(), |window, cx| {
                cx.new(|cx| MainWindow::new(store.clone(), ReportSize::default(), window, cx))
            })
        });
        let second = second.unwrap().root(&mut cx).unwrap();
        first.update_in(&mut cx, |main, window, cx| {
            main.open(Page::Tasks, window, cx);
        });
        second.update_in(&mut cx, |main, window, cx| {
            main.open(Page::Room(emberhut_api::ids::RoomId::new(2)), window, cx);
        });
        assert_eq!(shown(&first, &cx), Some(Page::Tasks));
        assert_eq!(
            shown(&second, &cx),
            None,
            "Tasks opened in the other window"
        );
        assert_eq!(
            store.read_with(&cx, |store, _| store.room()),
            emberhut_api::ids::RoomId::new(2),
            "the room is every window's"
        );
        assert!(store.read_with(&cx, |store, _| store.shows_page(Page::Tasks)));
    }

    #[gpui_kit::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "a second main window opened bare, for its typed handle; no unseen run here"
    )]
    fn settings_pages_go_on_each_windows_own_column_and_back_returns_to_settings(
        cx: &mut TestAppContext,
    ) {
        use crate::ui::routes::open_settings;

        let (first, mut cx) = open_window(cx);
        let store = first.read_with(&cx, |main, _| main.store.clone());
        let second = cx.update(|_, cx| {
            cx.open_window(gpui_kit::WindowOptions::default(), |window, cx| {
                cx.new(|cx| MainWindow::new(store.clone(), ReportSize::default(), window, cx))
            })
        });
        let second = second.unwrap();
        let second_main = second.root(&mut cx).unwrap();
        cx.update(|window, cx| open_settings(Some(SettingsPane::AboutYou), window, cx));
        let mut second_cx = VisualTestContext::from_window(second.into(), &cx);
        second_cx.update(|window, cx| open_settings(None, window, cx));
        let about = Page::SettingsPane(SettingsPane::AboutYou);
        assert_eq!(shown(&first, &cx), Some(about));
        assert_eq!(
            shown(&second_main, &cx),
            Some(Page::Settings),
            "the other window keeps its own page of Settings"
        );
        assert!(store.read_with(&cx, |store, _| store.shows_page(about)));
        cx.update(Window::render_frame);
        cx.dispatch_action(GoBack);
        assert_eq!(shown(&first, &cx), Some(Page::Settings), "Back to Settings");
        assert!(store.read_with(&cx, |store, _| !store.shows_page(about)));
    }
}
