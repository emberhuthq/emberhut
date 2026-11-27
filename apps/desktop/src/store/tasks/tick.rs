//! A tick on the user's own to-do, as the reference client's Reminders-style circle does it:
//! the circle fills with a check (and bounces, unless the system asks for reduced motion) for a
//! beat, then the task leaves the list and Mark done is held with Undo.

use std::collections::HashSet;
use std::time::Duration;

use emberhut_api::ids::TaskId;
use gpui_kit::Context;

use super::super::{Store, update_or_log};

/// How long a ticked circle shows its check before the task leaves the list.
pub(crate) const TICK_BEAT: Duration = Duration::from_millis(450);

/// The tasks ticked whose beat isn't over yet.
#[derive(Debug, Default)]
pub(crate) struct Ticks(HashSet<TaskId>);

impl Store {
    /// The user ticked `task`: its circle shows a check for [`TICK_BEAT`], then it is marked
    /// done ([`Store::mark_done`], held with Undo). A second tick during the beat does nothing.
    pub(crate) fn tick_done(&mut self, task: TaskId, cx: &mut Context<'_, Self>) {
        if !self.tasks.ticks.0.insert(task) {
            return;
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TICK_BEAT).await;
            update_or_log(&this, cx, |store, cx| {
                store.tasks.ticks.0.remove(&task);
                store.mark_done(task, cx);
            });
        })
        .detach();
    }

    /// Whether `task`'s circle shows its check (ticked, its beat not over).
    #[must_use]
    pub(crate) fn is_ticked(&self, task: TaskId) -> bool {
        self.tasks.ticks.0.contains(&task)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_api::ids::RoomId;
    use emberhut_api::task_actions::TaskAction;
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::store::ThemeState;
    use crate::store::answers::UNDO_WINDOW;

    const ONE: TaskId = TaskId::new(1);

    #[gpui_kit::test]
    fn a_tick_shows_its_check_for_a_beat_then_the_task_is_marked_done(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::idle(shared, None, ThemeState::load(None), RoomId::MAIN));
        store.update(cx, |store, cx| {
            store.tick_done(ONE, cx);
            store.tick_done(ONE, cx);
        });
        store.read_with(cx, |store, _| {
            assert!(store.is_ticked(ONE), "the check shows");
            assert!(!store.is_gone(ONE), "still on the list");
        });
        cx.executor().advance_clock(TICK_BEAT);
        cx.run_until_parked();
        store.read_with(cx, |store, _| {
            assert!(!store.is_ticked(ONE));
            assert!(store.is_gone(ONE), "off the list, with Undo");
        });
        cx.executor().advance_clock(UNDO_WINDOW);
        cx.run_until_parked();
        let acts: Vec<Call> = backend
            .calls()
            .into_iter()
            .filter(|call| matches!(call, Call::Act(_)))
            .collect();
        assert_eq!(acts, [Call::Act(TaskAction::mark_done(ONE))], "sent once");
    }
}
