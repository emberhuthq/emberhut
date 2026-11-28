//! Renaming a task from its page: the title typed, trimmed, goes to the server unless it is the
//! title the task has; the server checks it (1 to 200 characters) and answers the task as it is
//! now, which the state takes at once before the state and the pages shown are asked for again.
//! A refusal reads as the server's reason.

use std::sync::Arc;

use emberhut_api::ids::TaskId;
use emberhut_api::tasks::Issue;
use gpui_kit::{AppContext as _, Context, Task};

use super::reason;
use crate::store::{Store, update_or_log};

/// The title to send when `typed` is in the field over a task titled `current`: trimmed, none
/// when it is the title the task has (nothing to change).
#[must_use]
pub(crate) fn new_title(current: &str, typed: &str) -> Option<String> {
    let typed = typed.trim();
    (typed != current.trim()).then(|| typed.to_owned())
}

/// What a rename that didn't happen says: the server's reason.
#[must_use]
pub(crate) fn failed(reason: &str) -> String {
    format!("Not renamed: {reason}")
}

impl Store {
    /// Renames `task` to `title`: once the server answers, the state's copy takes the new
    /// title and the state and the task pages shown are asked for again. The task as it is
    /// now, or what the failure says.
    pub(crate) fn rename_task(
        &mut self,
        task: TaskId,
        title: String,
        cx: &mut Context<'_, Self>,
    ) -> Task<Result<Issue, String>> {
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        let sending = self.in_flight.start();
        let request = cx.background_spawn(async move {
            let renamed = backend.rename_task(task, &title);
            drop(sending);
            renamed
        });
        cx.spawn(async move |this, cx| {
            let renamed = request.await;
            if let Err(error) = &renamed {
                log::info!("a task wasn't renamed: {error}");
            }
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                if let (Ok(renamed), Some(state)) = (&renamed, store.state.as_mut())
                    && let Some(listed) = state.issues.iter_mut().find(|t| t.id == renamed.id)
                {
                    listed.title.clone_from(&renamed.title);
                }
                store.refetch_open_task(cx);
                cx.notify();
            });
            renamed.map_err(|error| failed(&reason(&error)))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::TestAppContext;
    use serde_json::json;

    use super::*;
    use crate::backend::Error;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::fixtures::{issue, state};

    #[test]
    fn the_title_typed_goes_trimmed_unless_it_is_the_tasks() {
        assert_eq!(
            new_title("Bake", "  Bake rye "),
            Some("Bake rye".to_owned())
        );
        assert_eq!(new_title("Bake", " Bake "), None, "nothing changed");
        assert_eq!(
            new_title("Bake", "   "),
            Some(String::new()),
            "the server says why not"
        );
        assert_eq!(failed("too long"), "Not renamed: too long");
    }

    #[gpui_kit::test]
    fn a_rename_takes_in_the_state_or_says_why_not(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        store.update(cx, |store, cx| {
            store.take_state_for_tests(state(json!({"issues": [issue(4, "todo", json!({}))]})), cx);
        });
        let renamed = store.update(cx, |store, cx| {
            store.rename_task(TaskId::new(4), "Bake rye".to_owned(), cx)
        });
        cx.run_until_parked();
        let renamed = cx.foreground_executor().block_test(renamed).unwrap();
        assert_eq!(renamed.title, "Bake rye");
        store.read_with(cx, |store, _| {
            assert_eq!(store.task_of(TaskId::new(4)).unwrap().title, "Bake rye");
        });
        assert!(
            backend
                .calls()
                .contains(&Call::RenameTask(TaskId::new(4), "Bake rye".to_owned()))
        );
        backend.push_rename(Err(Error::Refused {
            status: Some(400),
            message: Some("a title is 1 to 200 characters".to_owned()),
        }));
        let refused = store.update(cx, |store, cx| {
            store.rename_task(TaskId::new(4), String::new(), cx)
        });
        cx.run_until_parked();
        assert_eq!(
            cx.foreground_executor().block_test(refused).unwrap_err(),
            "Not renamed: a title is 1 to 200 characters"
        );
    }
}
