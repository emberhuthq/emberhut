//! Where the ember can live on this computer: the port the setup's first step asks to start
//! Emberhut here and to pair this app with it, and the implementations this build has.
//!
//! A home here is Emberhut running in the background on this computer, which the app starts
//! and pairs with by itself. The app wires its implementation in at launch; until one can
//! start Emberhut, [`DevHome`] uses one that already answers at the app's address and lets the
//! user type its pairing code.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Task};

use crate::account::switch::is_loopback_host;
use crate::backend::Backend;
use crate::config::ServerUrl;

/// What the step says when Emberhut can't live on this computer through this build.
pub(crate) const CANT_HERE: &str = "This build can't start Emberhut here yet.";

/// How long the first step waits for Emberhut on this computer to answer.
const LOOK_WITHIN: Duration = Duration::from_secs(3);

/// Emberhut on this computer, as the setup's first step uses it.
pub(crate) trait LocalHome {
    /// Whether this build can give the ember a home on this computer at all.
    fn available(&self) -> bool;

    /// Starts Emberhut here (or finds it running), in the background: the address it answers
    /// at, or why it couldn't, in words for the user.
    fn start(&self, cx: &mut App) -> Task<Result<ServerUrl, String>>;

    /// A one-time pairing code for this app from Emberhut here, read where only this user can
    /// read it; `None` when the user has to type one.
    fn pair_code(&self, cx: &mut App) -> Task<Result<Option<String>, String>>;
}

/// A build with no home on this computer: the step offers only another computer.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NoLocalHome;

impl LocalHome for NoLocalHome {
    fn available(&self) -> bool {
        false
    }

    fn start(&self, _: &mut App) -> Task<Result<ServerUrl, String>> {
        Task::ready(Err(CANT_HERE.to_owned()))
    }

    fn pair_code(&self, _: &mut App) -> Task<Result<Option<String>, String>> {
        Task::ready(Err(CANT_HERE.to_owned()))
    }
}

/// Emberhut already running on this computer, at the address the app talks to: it is used as
/// it is, and the user types the pairing code it gave. Nothing is started.
pub(crate) struct DevHome {
    backend: Arc<dyn Backend>,
}

impl DevHome {
    /// The home at `backend`'s address.
    pub(crate) fn new(backend: Arc<dyn Backend>) -> Self {
        Self { backend }
    }
}

impl LocalHome for DevHome {
    fn available(&self) -> bool {
        on_this_computer(self.backend.server())
    }

    fn start(&self, cx: &mut App) -> Task<Result<ServerUrl, String>> {
        let backend = Arc::clone(&self.backend);
        cx.background_spawn(async move {
            if backend.answers_within(LOOK_WITHIN) {
                Ok(backend.server().clone())
            } else {
                Err(CANT_HERE.to_owned())
            }
        })
    }

    fn pair_code(&self, _: &mut App) -> Task<Result<Option<String>, String>> {
        Task::ready(Ok(None))
    }
}

/// Whether `server` is on this computer (a loopback address).
#[must_use]
pub(crate) fn on_this_computer(server: &ServerUrl) -> bool {
    let address = server.as_str();
    let authority = address.split_once("://").map_or(address, |(_, rest)| rest);
    is_loopback_host(authority)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::FakeBackend;

    #[test]
    fn only_an_address_on_this_computer_is_a_home_here() {
        let home = |address: &str| {
            DevHome::new(Arc::new(FakeBackend::new([]).at(address)) as Arc<dyn Backend>).available()
        };
        assert!(home("http://localhost:18990"));
        assert!(home("http://127.0.0.1:18990"));
        assert!(!home("https://ember.example.com"));
    }

    /// What the task `make` gives answers, once the app has run it.
    fn run<T: 'static>(
        cx: &mut gpui_kit::TestAppContext,
        make: impl FnOnce(&mut App) -> Task<T>,
    ) -> T {
        let out = std::rc::Rc::new(std::cell::RefCell::new(None));
        let keep = std::rc::Rc::clone(&out);
        cx.update(|cx| {
            let task = make(cx);
            cx.spawn(async move |_| *keep.borrow_mut() = Some(task.await))
                .detach();
        });
        cx.run_until_parked();
        out.borrow_mut().take().expect("the task finished")
    }

    #[gpui_kit::test]
    fn a_home_here_is_used_only_when_it_answers(cx: &mut gpui_kit::TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]).at("http://localhost:18990"));
        let home = DevHome::new(Arc::clone(&backend) as Arc<dyn Backend>);
        assert_eq!(run(cx, |cx| home.start(cx)), Err(CANT_HERE.to_owned()));
        backend.setup.set_answers(true);
        assert_eq!(
            run(cx, |cx| home.start(cx)).map(|url| url.to_string()),
            Ok("http://localhost:18990".to_owned())
        );
        assert_eq!(
            run(cx, |cx| home.pair_code(cx)),
            Ok(None),
            "the user types it"
        );
    }
}
