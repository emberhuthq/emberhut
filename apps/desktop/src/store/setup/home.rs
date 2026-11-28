//! Where the ember can live on this computer: the port the setup's first step asks to start
//! Emberhut here and to pair this app with it, and the implementations this build has.
//!
//! A home here is Emberhut running in the background on this computer, which the app starts
//! and pairs with by itself. The app wires its implementation in at launch: [`HutHome`], the
//! hut this app runs ([`crate::hut`]), when the app has its server binary; else [`DevHome`],
//! one that already answers at the app's address, whose pairing code the user types.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Task};

use crate::account::switch::is_loopback_host;
use crate::backend::Backend;
use crate::config::ServerUrl;
use crate::hut::{KeepLit, LocalHut};
use crate::store::sign_in::same_server;

/// What the step says when Emberhut can't live on this computer through this build.
pub(crate) const CANT_HERE: &str = "This build can't start a hut here yet.";

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

    /// Keeps Emberhut here lit as `keep` says (while the app is closed, from login), or why it
    /// couldn't; `None` when it isn't this app's to keep (it runs by other means).
    fn keep_lit(&self, keep: KeepLit, cx: &mut App) -> Option<Task<Result<(), String>>>;

    /// At launch: when `server` (the address the app talks to) is Emberhut here and it doesn't
    /// answer (it ran as the app's child, or it stopped), it is started again. Nothing is done
    /// for any other address, or a home that isn't this app's to start.
    fn relight(&self, _server: &ServerUrl, _cx: &mut App) -> Task<Relit> {
        Task::ready(Relit::NotHere)
    }

    /// The hut, when Emberhut here is this app's own: its Claude accounts are signed in and
    /// used through it. `None` for any other home.
    fn hut(&self) -> Option<LocalHut> {
        None
    }
}

/// What [`LocalHome::relight`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Relit {
    /// The address isn't Emberhut here (or nothing here is the app's to start).
    NotHere,
    /// It answers already.
    Lit,
    /// It was started, and answers at this address.
    Started(ServerUrl),
    /// It couldn't be started: why, in the hut's words.
    Failed(String),
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

    fn keep_lit(&self, _: KeepLit, _: &mut App) -> Option<Task<Result<(), String>>> {
        None
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

    fn keep_lit(&self, _: KeepLit, _: &mut App) -> Option<Task<Result<(), String>>> {
        None
    }
}

/// The hut: Emberhut run by this app on this computer ([`LocalHut`]), started on the loopback,
/// kept lit as asked, and paired with through a code it makes for this app, so the user types
/// nothing. What goes wrong is said in the hut's own words.
pub(crate) struct HutHome {
    hut: LocalHut,
}

impl HutHome {
    /// The home that is `hut`.
    #[must_use]
    pub(crate) fn new(hut: LocalHut) -> Self {
        Self { hut }
    }
}

/// `task`'s outcome with its error in words.
fn in_words<T: 'static>(
    task: Task<Result<T, crate::hut::HutError>>,
    cx: &App,
) -> Task<Result<T, String>> {
    cx.spawn(async move |_| task.await.map_err(|error| error.to_string()))
}

impl LocalHome for HutHome {
    fn available(&self) -> bool {
        self.hut.available()
    }

    fn start(&self, cx: &mut App) -> Task<Result<ServerUrl, String>> {
        in_words(self.hut.start(cx), cx)
    }

    fn pair_code(&self, cx: &mut App) -> Task<Result<Option<String>, String>> {
        let code = in_words(self.hut.pair_code(cx), cx);
        cx.spawn(async move |_| code.await.map(Some))
    }

    fn keep_lit(&self, keep: KeepLit, cx: &mut App) -> Option<Task<Result<(), String>>> {
        Some(in_words(self.hut.keep_lit(keep, cx), cx))
    }

    fn hut(&self) -> Option<LocalHut> {
        Some(self.hut.clone())
    }

    fn relight(&self, server: &ServerUrl, cx: &mut App) -> Task<Relit> {
        let (hut, server) = (self.hut.clone(), server.clone());
        cx.background_spawn(async move {
            let ours = hut
                .remembered()
                .is_some_and(|address| same_server(&address, &server));
            if !ours || !hut.available() {
                return Relit::NotHere;
            }
            if hut.answers_now() {
                return Relit::Lit;
            }
            log::info!("the app talks to the hut, which is out: starting it");
            match hut.start_now() {
                Ok(url) => Relit::Started(url),
                Err(error) => Relit::Failed(error.to_string()),
            }
        })
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
        assert!(
            cx.update(|cx| home.keep_lit(KeepLit::default(), cx))
                .is_none(),
            "it isn't the app's to keep lit"
        );
    }

    #[gpui_kit::test]
    fn the_hut_is_a_home_here_that_hands_over_its_own_code(cx: &mut gpui_kit::TestAppContext) {
        use crate::hut::tests::{Os, Setup};
        let computer = Setup::new(Os::Linux);
        let home = HutHome::new(computer.hut());
        assert!(home.available());
        assert_eq!(
            run(cx, |cx| home.start(cx)).map(|url| url.to_string()),
            Ok("http://127.0.0.1:18990".to_owned())
        );
        assert_eq!(
            run(cx, |cx| home.pair_code(cx)),
            Ok(Some("ABCD-EFGH".to_owned())),
            "the user types nothing"
        );
        let keep = KeepLit {
            when_closed: true,
            at_login: true,
        };
        let kept = run(cx, |cx| home.keep_lit(keep, cx).expect("the app's to keep"));
        assert_eq!(kept, Ok(()));
    }

    #[gpui_kit::test]
    fn the_hut_the_app_talks_to_is_started_again_when_it_is_out(cx: &mut gpui_kit::TestAppContext) {
        use crate::hut::tests::{Os, Setup};
        let here = |address: &str| ServerUrl::parse(address).expect("an address");
        let computer = Setup::new(Os::Linux);
        let home = HutHome::new(computer.hut());
        assert_eq!(
            run(cx, |cx| home.relight(&here("http://localhost:18990"), cx)),
            Relit::NotHere,
            "never started here: the first run's setup starts it, not a launch"
        );
        assert_eq!(computer.spawned(), 0);

        let first = computer.hut();
        first.start_now().unwrap();
        first.let_go();
        let home = HutHome::new(computer.hut());
        assert_eq!(
            run(cx, |cx| home
                .relight(&here("https://ember.example.com"), cx)),
            Relit::NotHere,
            "another address: nothing starts"
        );
        assert_eq!(computer.spawned(), 1);
        assert_eq!(
            run(cx, |cx| home.relight(&here("http://localhost:18990"), cx)),
            Relit::Started(here("http://127.0.0.1:18990"))
        );
        assert_eq!(computer.spawned(), 2);
        assert_eq!(
            run(cx, |cx| home.relight(&here("http://127.0.0.1:18990"), cx)),
            Relit::Lit,
            "it answers: nothing more starts"
        );
        assert_eq!(computer.spawned(), 2);
    }

    #[gpui_kit::test]
    fn a_hut_that_doesnt_start_again_says_why(cx: &mut gpui_kit::TestAppContext) {
        use crate::hut::tests::{Os, Setup};
        let computer = Setup::new(Os::Linux);
        let first = computer.hut();
        first.start_now().unwrap();
        first.let_go();
        computer.exits_at_start();
        let home = HutHome::new(computer.hut());
        let server = ServerUrl::loopback(18990);
        let Relit::Failed(said) = run(cx, |cx| home.relight(&server, cx)) else {
            panic!("a hut that exits doesn't start");
        };
        assert!(
            said.starts_with("The hut stopped while it was starting"),
            "{said}"
        );
    }

    #[gpui_kit::test]
    fn the_huts_errors_come_in_its_own_words(cx: &mut gpui_kit::TestAppContext) {
        use crate::hut::HutError;
        use crate::hut::tests::{Os, Setup};
        let computer = Setup::new(Os::Other);
        let home = HutHome::new(computer.hut());
        let keep = KeepLit {
            when_closed: true,
            at_login: false,
        };
        assert_eq!(
            run(cx, |cx| home.keep_lit(keep, cx).expect("the app's to keep")),
            Err(HutError::NoService.to_string())
        );
        computer.exits_at_start();
        let Err(said) = run(cx, |cx| home.start(cx)) else {
            panic!("a hut that exits doesn't start");
        };
        assert!(
            said.starts_with("The hut stopped while it was starting"),
            "{said}"
        );
        let Err(said) = run(cx, |cx| home.pair_code(cx)) else {
            panic!("nor makes a code");
        };
        assert!(
            said.starts_with("The hut stopped while it was starting"),
            "{said}"
        );
    }
}
