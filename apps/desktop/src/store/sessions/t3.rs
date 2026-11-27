//! An external thread still running, read only, as a GPUI entity its page holds: its latest steps,
//! asked for again every 10 s while the page is open (the app can't stream them).

use std::sync::Arc;
use std::time::Duration;

use emberhut_api::ids::MachineId;
use emberhut_api::threads::{ActivityQuery, ThreadActivity};
use gpui_kit::{AppContext as _, Context};

use crate::backend::Backend;

/// How often the steps are asked for again.
const EVERY: Duration = Duration::from_secs(10);

/// How many of the newest steps the page shows.
pub(crate) const SHOWN: usize = 25;

/// An external thread's page.
pub(crate) struct T3Watch {
    machine: MachineId,
    thread: String,
    activity: Option<ThreadActivity>,
    /// The first answer came (or failed).
    asked: bool,
}

impl T3Watch {
    /// Follows `thread` in the coding-thread app on `machine` while the entity is held.
    pub(crate) fn new(
        backend: Arc<dyn Backend>,
        machine: MachineId,
        thread: String,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let query = ActivityQuery {
            machine: machine.as_str().to_owned(),
            thread: thread.clone(),
        };
        cx.spawn(async move |this, cx| {
            loop {
                let (backend, query) = (Arc::clone(&backend), query.clone());
                let answer = cx
                    .background_spawn(async move { backend.t3_activity(&query) })
                    .await;
                let held = this.update(cx, |watch, cx| {
                    watch.asked = true;
                    match answer {
                        Ok(activity) => watch.activity = Some(activity),
                        Err(error) => log::info!("no steps from the coding-thread app: {error}"),
                    }
                    cx.notify();
                });
                if held.is_err() {
                    return;
                }
                cx.background_executor().timer(EVERY).await;
            }
        })
        .detach();
        Self {
            machine,
            thread,
            activity: None,
            asked: false,
        }
    }

    /// The coding-thread app it runs in.
    #[must_use]
    pub(crate) fn machine(&self) -> &MachineId {
        &self.machine
    }

    /// The thread's id.
    #[must_use]
    pub(crate) fn thread(&self) -> &str {
        &self.thread
    }

    /// Its latest steps, once they came.
    #[must_use]
    pub(crate) fn activity(&self) -> Option<&ThreadActivity> {
        self.activity.as_ref()
    }

    /// Whether an answer came (or failed): the page stops showing its spinner.
    #[must_use]
    pub(crate) fn has_asked(&self) -> bool {
        self.asked
    }
}
