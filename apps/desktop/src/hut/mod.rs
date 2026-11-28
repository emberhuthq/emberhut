//! The hut inside the app: this computer as the home of the person's agent, with no terminal.
//! The hut is `emberhut-server`, shipped with the app (docs/adr/0030-the-hut-inside-the-app.md).
//!
//! [`LocalHut`] finds the server binary ([`LocalHut::available`]), starts it on the loopback
//! with its files in the app's data folder and waits until it answers as this computer's hut
//! ([`LocalHut::start`]), makes a pairing code for this app with `emberhut-server pair`, the
//! shell-trust path ADR 0002 already allows ([`LocalHut::pair_code`]; the app then pairs with
//! it through `POST /api/auth/pair` as any device does), and keeps the hut lit while the app is
//! closed or from login through launchd or systemd, or else as the app's child
//! ([`LocalHut::keep_lit`], [`LocalHut::stop`], [`LocalHut::status`]).
//!
//! Each call is a GPUI [`Task`] on the background executor; the work under it blocks (it
//! runs programs and asks the loopback), as the app's network calls do. One call runs at a
//! time. The processes, the loopback and the folders are given to [`LocalHut::new`]
//! ([`Spawner`], [`HutNet`], [`HutPaths`]), so the tests run all of it against fakes.

mod error;
mod net;
mod pair;
mod paths;
mod process;
mod service;
mod state;

#[cfg(test)]
mod real_tests;
#[cfg(test)]
pub(crate) mod tests;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, UNIX_EPOCH};

use emberhut_identity::MachineId;
use gpui_kit::AppContext as _;
use gpui_kit::{App, Task};

pub(crate) use self::error::HutError;
pub(crate) use self::net::{Heard, HutNet, LoopbackNet};
pub(crate) use self::paths::{HutPaths, Manager, Place};
#[cfg(test)]
pub(crate) use self::process::Ran;
pub(crate) use self::process::{Run, Running, Spawner, SystemSpawner};
pub(crate) use self::service::KeepLit;
use self::service::{Serve, launchctl, systemctl};
use self::state::HutState;
use crate::config::ServerUrl;
use crate::private_files;

/// The port the hut listens on unless it is taken: the server's own default, which the app's
/// default server address names too.
pub(crate) const DEFAULT_PORT: u16 = 18990;

/// The variable that names the hut's first port instead of [`DEFAULT_PORT`], read only by a
/// debug or automation build: a test run on a computer whose default port is another
/// server's.
pub(crate) const PORT_ENV: &str = "EMBERHUT_HUT_PORT";

/// Whether this build reads the test runs' variables ([`PORT_ENV`], [`paths::SERVICE_ENV`]).
const READS_TEST_VARS: bool = cfg!(any(test, debug_assertions, feature = "automation"));

/// How the hut is run: its first port and how many after it may be tried, and how long it
/// gets to start and to stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HutOptions {
    /// The port tried first.
    pub(crate) port: u16,
    /// How many ports after it are tried when one is taken by another program.
    pub(crate) more_ports: u16,
    /// How long a start waits for the hut to answer.
    pub(crate) start_wait: Duration,
    /// How long a stop waits for it to go.
    pub(crate) stop_wait: Duration,
    /// How often it is asked meanwhile.
    pub(crate) poll: Duration,
}

impl Default for HutOptions {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            more_ports: 20,
            start_wait: Duration::from_secs(60),
            stop_wait: Duration::from_secs(15),
            poll: Duration::from_millis(200),
        }
    }
}

impl HutOptions {
    /// The options the app runs its hut with: the defaults, the first port as [`PORT_ENV`]
    /// says in a build that reads it.
    #[must_use]
    pub(crate) fn for_app() -> Self {
        Self::from_vars(&|name| std::env::var_os(name))
    }

    /// [`HutOptions::for_app`], the variables read through `var`.
    fn from_vars(var: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Self {
        let options = Self::default();
        if !READS_TEST_VARS {
            return options;
        }
        let Some(value) = var(PORT_ENV) else {
            return options;
        };
        match value.to_str().and_then(|port| port.parse::<u16>().ok()) {
            Some(port) if port != 0 => Self { port, ..options },
            _ => {
                log::warn!(
                    "{PORT_ENV} isn't a port ({}); the hut uses {DEFAULT_PORT}",
                    value.display()
                );
                options
            }
        }
    }
}

/// How the hut is now.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the setup starts, pairs and keeps the hut lit; stopping it and asking how it \
                  is wait for a page of their own"
    )
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HutStatus {
    /// Its address, when it answers there as this computer's hut.
    pub(crate) url: Option<ServerUrl>,
    /// Whether it stays lit without the app.
    pub(crate) keep: KeepLit,
    /// The server binary, when there is one.
    pub(crate) server: Option<PathBuf>,
}

/// Who answers on a port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Who {
    /// Nobody: the port is free.
    Nobody,
    /// This computer's hut: an emberhut server that proves to hold the hut's machine key.
    Ours,
    /// Another program, or another emberhut server.
    Stranger,
}

/// This computer's hut. Cheap to clone: the clones share one hut.
#[derive(Clone)]
pub(crate) struct LocalHut {
    inner: Arc<Inner>,
}

/// What the clones share.
struct Inner {
    paths: HutPaths,
    spawner: Arc<dyn Spawner>,
    net: Arc<dyn HutNet>,
    options: HutOptions,
    /// The hut, while it runs as this app's child.
    child: Mutex<Option<Box<dyn Running>>>,
    /// The hut's machine id, once its key is there.
    machine: Mutex<Option<MachineId>>,
    /// Held by the call running, so calls run one at a time.
    busy: Mutex<()>,
}

/// Takes `mutex` whatever an earlier holder did.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LocalHut {
    /// A hut in `paths`, running programs through `spawner` and asking the loopback through
    /// `net`, as `options` says.
    #[must_use]
    pub(crate) fn new(
        paths: HutPaths,
        spawner: Arc<dyn Spawner>,
        net: Arc<dyn HutNet>,
        options: HutOptions,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                paths,
                spawner,
                net,
                options,
                child: Mutex::new(None),
                machine: Mutex::new(None),
                busy: Mutex::new(()),
            }),
        }
    }

    /// This computer's hut, run as `options` says (`HutOptions::default()` in the app): the
    /// real processes, loopback and folders. `None` without a home folder.
    #[must_use]
    pub(crate) fn system(options: HutOptions) -> Option<Self> {
        Some(Self::new(
            HutPaths::new(Place::system()?),
            Arc::new(SystemSpawner),
            Arc::new(LoopbackNet::new()),
            options,
        ))
    }

    /// Whether this app can run a hut: its server binary is found (in the bundle, beside the
    /// app's executable, or at `EMBERHUT_SERVER_BIN`).
    #[must_use]
    pub(crate) fn available(&self) -> bool {
        self.inner.paths.server().is_some()
    }

    /// Starts the hut unless it already answers as this computer's hut (of the bundled
    /// server's build: one of another build is restarted), and gives its address once it
    /// answers.
    pub(crate) fn start(&self, cx: &App) -> Task<Result<ServerUrl, HutError>> {
        let hut = self.clone();
        cx.background_spawn(async move { hut.start_now() })
    }

    /// A one-time pairing code for this app, made by `emberhut-server pair` in the hut's own
    /// database (the hut is started first when it isn't lit).
    pub(crate) fn pair_code(&self, cx: &App) -> Task<Result<String, HutError>> {
        let hut = self.clone();
        cx.background_spawn(async move { hut.pair_code_now() })
    }

    /// Keeps the hut lit as `keep` says (a launch agent or systemd unit when it stays lit
    /// without the app, else the app's child), and lights it that way.
    pub(crate) fn keep_lit(&self, keep: KeepLit, cx: &App) -> Task<Result<(), HutError>> {
        let hut = self.clone();
        cx.background_spawn(async move { hut.keep_lit_now(keep) })
    }

    /// Stops the hut now, however it runs. One kept lit from login starts again at the next
    /// login; [`LocalHut::keep_lit`] with neither switch on stops that.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the setup starts, pairs and keeps the hut lit; stopping it and asking how it \
                      is wait for a page of their own"
        )
    )]
    pub(crate) fn stop(&self, cx: &App) -> Task<Result<(), HutError>> {
        let hut = self.clone();
        cx.background_spawn(async move { hut.stop_now() })
    }

    /// The app is quitting: a hut that runs as its child is asked to stop (`SIGTERM`, which the
    /// server shuts down cleanly on) without waiting for it, so the quit isn't held up. A hut
    /// a service runs stays lit. It doesn't wait for a call under way.
    pub(crate) fn let_go(&self) {
        let Some(child) = lock(&self.inner.child).take() else {
            return;
        };
        log::info!("the app quits: stopping the hut, process {}", child.pid());
        if let Err(error) = self.inner.spawner.terminate(child.pid()) {
            log::warn!("couldn't ask the hut to stop: {error}");
        }
    }

    /// How the hut is now.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the setup starts, pairs and keeps the hut lit; stopping it and asking how it \
                      is wait for a page of their own"
        )
    )]
    pub(crate) fn status(&self, cx: &App) -> Task<HutStatus> {
        let hut = self.clone();
        cx.background_spawn(async move { hut.status_now() })
    }

    /// The address the hut was last lit at, as `hut.json` remembers it; `None` before it was
    /// ever started here.
    #[must_use]
    pub(crate) fn remembered(&self) -> Option<ServerUrl> {
        self.read_state().port.map(ServerUrl::loopback)
    }

    /// Whether this computer's hut answers now, at the address it was last lit at.
    #[must_use]
    pub(crate) fn answers_now(&self) -> bool {
        let _busy = lock(&self.inner.busy);
        self.read_state()
            .port
            .is_some_and(|port| self.who(port) == Who::Ours)
    }

    /// [`LocalHut::start`], on this thread.
    ///
    /// # Errors
    ///
    /// No server binary, its files can't be made, every port is taken, or the hut doesn't
    /// start or answer in time.
    pub(crate) fn start_now(&self) -> Result<ServerUrl, HutError> {
        let _busy = lock(&self.inner.busy);
        self.light(false).map(ServerUrl::loopback)
    }

    /// [`LocalHut::pair_code`], on this thread.
    ///
    /// # Errors
    ///
    /// As [`LocalHut::start_now`]; [`HutError::NoCode`] when `pair` makes none.
    pub(crate) fn pair_code_now(&self) -> Result<String, HutError> {
        let _busy = lock(&self.inner.busy);
        let port = self.light(false)?;
        let server = self.server()?;
        let pair = Run::new(&server)
            .args(["pair", "--json", "--database-url"])
            .arg(self.database_url()?)
            .args(["--listen".to_owned(), format!("127.0.0.1:{port}")])
            .dir(self.inner.paths.data());
        let ran = self
            .inner
            .spawner
            .run(&pair)
            .map_err(|error| HutError::NoCode {
                said: error.to_string(),
            })?;
        if !ran.ok {
            return Err(HutError::NoCode { said: ran.said() });
        }
        pair::code_in(&ran.stdout).ok_or_else(|| HutError::NoCode {
            said: "its answer named no code".to_owned(),
        })
    }

    /// [`LocalHut::keep_lit`], on this thread.
    ///
    /// # Errors
    ///
    /// [`HutError::NoService`] when it should stay lit without the app on a system without
    /// launchd or systemd; a service manager's refusal; as [`LocalHut::start_now`].
    pub(crate) fn keep_lit_now(&self, keep: KeepLit) -> Result<(), HutError> {
        let _busy = lock(&self.inner.busy);
        if keep.by_service() && *self.inner.paths.manager() == Manager::None {
            return Err(HutError::NoService);
        }
        self.prepare()?;
        let mut state = self.read_state();
        let changed = state.keep != keep;
        let before = self.by_service(state.keep);
        let after = self.by_service(keep);
        if before && !after {
            self.remove_service();
        }
        if !before && after {
            let port = state.port.unwrap_or(self.inner.options.port);
            self.put_out_child(&state, port)?;
            state.pid = None;
        }
        state.keep = keep;
        self.write_state(&state)?;
        self.light(changed && after).map(drop)
    }

    /// [`LocalHut::stop`], on this thread.
    ///
    /// # Errors
    ///
    /// The service manager refused, or the hut didn't stop in time.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the setup starts, pairs and keeps the hut lit; stopping it and asking how it \
                      is wait for a page of their own"
        )
    )]
    pub(crate) fn stop_now(&self) -> Result<(), HutError> {
        let _busy = lock(&self.inner.busy);
        let mut state = self.read_state();
        let port = state.port.unwrap_or(self.inner.options.port);
        if self.by_service(state.keep) {
            self.stop_service()?;
            self.wait_out(port)?;
        } else {
            self.put_out_child(&state, port)?;
        }
        if state.pid.take().is_some() {
            self.write_state(&state)?;
        }
        Ok(())
    }

    /// [`LocalHut::status`], on this thread.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the setup starts, pairs and keeps the hut lit; stopping it and asking how it \
                      is wait for a page of their own"
        )
    )]
    #[must_use]
    pub(crate) fn status_now(&self) -> HutStatus {
        let _busy = lock(&self.inner.busy);
        let state = self.read_state();
        let port = state.port.unwrap_or(self.inner.options.port);
        let url = (self.who(port) == Who::Ours).then(|| ServerUrl::loopback(port));
        HutStatus {
            url,
            keep: state.keep,
            server: self.inner.paths.server(),
        }
    }

    /// Lights the hut, or finds it lit: on its remembered port (or the first), else the next
    /// one not taken by another program. A hut that answers is kept unless it runs another
    /// build of the server or `relight` asks (its service's settings changed): it is then
    /// started again. Gives the port it answers on.
    fn light(&self, relight: bool) -> Result<u16, HutError> {
        let server = self.server()?;
        self.prepare()?;
        let mut state = self.read_state();
        let build = self.build_of(&server)?;
        let (port, who) = self.choose_port(state.port)?;
        let same_build = state.server.as_deref() == Some(build.as_str());
        if who == Who::Ours && same_build && !relight {
            if state.port != Some(port) {
                state.port = Some(port);
                self.write_state(&state)?;
            }
            return Ok(port);
        }
        let by_service = self.by_service(state.keep);
        if who == Who::Ours {
            log::info!("the hut runs another build of the server, or new settings: restarting it");
            if !by_service {
                self.put_out_child(&state, port)?;
            }
        }
        state.port = Some(port);
        state.server = Some(build);
        let serve = self.serve(&server, port)?;
        if by_service {
            state.pid = None;
            self.write_state(&state)?;
            self.run_service(&serve, state.keep)?;
        } else {
            let child = self
                .inner
                .spawner
                .spawn(&serve.child())
                .map_err(HutError::Launch)?;
            log::info!("the hut started as process {} on port {port}", child.pid());
            state.pid = Some(child.pid());
            *lock(&self.inner.child) = Some(child);
            self.write_state(&state)?;
        }
        self.wait_lit(port)?;
        Ok(port)
    }

    /// The port to use, and who answers there: the remembered one (or the first), else the
    /// first after it that isn't another program's.
    fn choose_port(&self, remembered: Option<u16>) -> Result<(u16, Who), HutError> {
        let first = remembered.unwrap_or(self.inner.options.port);
        let last = first.saturating_add(self.inner.options.more_ports);
        for port in first..=last {
            match self.who(port) {
                Who::Stranger => log::info!("port {port} is another program's"),
                who => return Ok((port, who)),
            }
        }
        Err(HutError::NoPort { first, last })
    }

    /// Who answers on `port`.
    fn who(&self, port: u16) -> Who {
        match self.inner.net.health(port) {
            Heard::Nothing => Who::Nobody,
            Heard::Other => Who::Stranger,
            Heard::Emberhut => match self.machine() {
                Some(machine) if self.inner.net.proves(port, &machine) => Who::Ours,
                _ => Who::Stranger,
            },
        }
    }

    /// The hut's machine id: `emberhut-server id` once its key is there (the server makes it
    /// when it first serves the database), kept from then on.
    fn machine(&self) -> Option<MachineId> {
        if let Some(machine) = *lock(&self.inner.machine) {
            return Some(machine);
        }
        if !self.inner.paths.machine_key().is_file() {
            return None;
        }
        let server = self.inner.paths.server()?;
        let id = Run::new(&server)
            .args(["id", "--database-url"])
            .arg(self.database_url().ok()?)
            .dir(self.inner.paths.data());
        let ran = self
            .inner
            .spawner
            .run(&id)
            .inspect_err(|error| log::warn!("couldn't ask the hut's machine id: {error}"))
            .ok()?;
        let machine = ran
            .ok
            .then(|| ran.stdout.trim().parse::<MachineId>().ok())
            .flatten()?;
        *lock(&self.inner.machine) = Some(machine);
        Some(machine)
    }

    /// Waits until the hut answers on `port` as this computer's hut.
    fn wait_lit(&self, port: u16) -> Result<(), HutError> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child_exited() {
                return Err(HutError::Exited {
                    status,
                    log: self.inner.paths.log().to_path_buf(),
                });
            }
            if self.who(port) == Who::Ours {
                return Ok(());
            }
            if started.elapsed() >= self.inner.options.start_wait {
                return Err(HutError::Timeout {
                    seconds: self.inner.options.start_wait.as_secs(),
                    log: self.inner.paths.log().to_path_buf(),
                });
            }
            std::thread::sleep(self.inner.options.poll);
        }
    }

    /// Waits until nothing answers on `port`.
    fn wait_out(&self, port: u16) -> Result<(), HutError> {
        let started = Instant::now();
        while self.inner.net.health(port) != Heard::Nothing {
            if started.elapsed() >= self.inner.options.stop_wait {
                return Err(HutError::StillLit {
                    seconds: self.inner.options.stop_wait.as_secs(),
                });
            }
            std::thread::sleep(self.inner.options.poll);
        }
        Ok(())
    }

    /// How the app's child ended, when it has (it is then let go).
    fn child_exited(&self) -> Option<String> {
        let mut child = lock(&self.inner.child);
        let status = child.as_mut()?.exited()?;
        *child = None;
        Some(status)
    }

    /// Stops the hut that runs as the app's child: the one this app started, or the process
    /// `state` remembers when this computer's hut answers on `port` (a hut left by an earlier
    /// run of the app). `SIGTERM` first; the app's own child is ended at once if it doesn't
    /// stop in time.
    fn put_out_child(&self, state: &HutState, port: u16) -> Result<(), HutError> {
        let held = lock(&self.inner.child).take();
        if let Some(mut child) = held {
            if let Err(error) = self.inner.spawner.terminate(child.pid()) {
                log::warn!("couldn't ask the hut to stop: {error}");
            }
            let asked = Instant::now();
            while child.exited().is_none() {
                if asked.elapsed() >= self.inner.options.stop_wait {
                    log::warn!("the hut didn't stop when asked: ending it");
                    child.kill();
                    break;
                }
                std::thread::sleep(self.inner.options.poll);
            }
        } else if let Some(pid) = state.pid {
            if self.who(port) != Who::Ours {
                return Ok(());
            }
            self.inner
                .spawner
                .terminate(pid)
                .map_err(|error| HutError::Service {
                    what: "stop the hut",
                    said: error.to_string(),
                })?;
        } else {
            return Ok(());
        }
        self.wait_out(port)
    }

    /// Whether `keep` has a service manager run the hut on this system.
    fn by_service(&self, keep: KeepLit) -> bool {
        keep.by_service() && *self.inner.paths.manager() != Manager::None
    }

    /// Writes the hut's launch agent or unit for `serve`, kept lit as `keep` says, and
    /// (re)starts it.
    fn run_service(&self, serve: &Serve, keep: KeepLit) -> Result<(), HutError> {
        match self.inner.paths.manager() {
            Manager::Launchd { plist, uid } => {
                write_file(plist, &service::launch_agent(serve, keep))?;
                // Unloaded first, so launchd reads the new plist; it may not be loaded.
                self.quietly(&launchctl::bootout(*uid));
                self.retried(
                    &launchctl::bootstrap(*uid, plist),
                    "load the hut's launch agent",
                )?;
                self.checked(&launchctl::kickstart(*uid), "start the hut's launch agent")
            }
            Manager::Systemd { unit } => {
                write_file(unit, &service::systemd_unit(serve))?;
                self.checked(
                    &systemctl::daemon_reload(),
                    "reload the user's systemd units",
                )?;
                if keep.at_login {
                    self.checked(&systemctl::enable(), "start the hut at login")?;
                } else {
                    self.quietly(&systemctl::disable());
                }
                self.checked(&systemctl::restart(), "start the hut's systemd unit")
            }
            Manager::None => Err(HutError::NoService),
        }
    }

    /// Stops the hut's service now, leaving it installed.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the setup starts, pairs and keeps the hut lit; stopping it and asking how it \
                      is wait for a page of their own"
        )
    )]
    fn stop_service(&self) -> Result<(), HutError> {
        match self.inner.paths.manager() {
            Manager::Launchd { uid, .. } => {
                // A launch agent kept alive comes back when killed: it is unloaded.
                self.quietly(&launchctl::bootout(*uid));
                Ok(())
            }
            Manager::Systemd { .. } => self.checked(&systemctl::stop(), "stop the hut's unit"),
            Manager::None => Ok(()),
        }
    }

    /// Stops the hut's service and removes it, so it no longer starts at login.
    fn remove_service(&self) {
        match self.inner.paths.manager() {
            Manager::Launchd { plist, uid } => {
                self.quietly(&launchctl::bootout(*uid));
                remove_file(plist);
            }
            Manager::Systemd { unit } => {
                self.quietly(&systemctl::disable());
                self.quietly(&systemctl::stop());
                remove_file(unit);
                self.quietly(&systemctl::daemon_reload());
            }
            Manager::None => {}
        }
    }

    /// Runs `run`, which may fail harmlessly (unloading what isn't loaded): a failure is
    /// logged.
    fn quietly(&self, run: &Run) {
        match self.inner.spawner.run(run) {
            Ok(ran) if !ran.ok => log::info!("{:?} said: {}", run.words(), ran.said()),
            Ok(_) => {}
            Err(error) => log::warn!("couldn't run {}: {error}", run.program.display()),
        }
    }

    /// Runs `run`, which must succeed; `what` says what it does.
    fn checked(&self, run: &Run, what: &'static str) -> Result<(), HutError> {
        let ran = self
            .inner
            .spawner
            .run(run)
            .map_err(|error| HutError::Service {
                what,
                said: error.to_string(),
            })?;
        if ran.ok {
            Ok(())
        } else {
            Err(HutError::Service {
                what,
                said: ran.said(),
            })
        }
    }

    /// [`LocalHut::checked`], tried three times: right after an unload, launchd may refuse a
    /// load for a moment (as ADR 0024's installer found).
    fn retried(&self, run: &Run, what: &'static str) -> Result<(), HutError> {
        let mut tries = 0;
        loop {
            tries += 1;
            match self.checked(run, what) {
                Err(_) if tries < 3 => std::thread::sleep(self.inner.options.poll * 2),
                outcome => return outcome,
            }
        }
    }

    /// The server as the hut runs it on `port`: the database, the secrets and the agent CLI
    /// (`claude`, when it is found) its flags name, in the data folder.
    fn serve(&self, server: &Path, port: u16) -> Result<Serve, HutError> {
        let paths = &self.inner.paths;
        let secrets = paths.secrets();
        let secrets = secrets
            .to_str()
            .ok_or_else(|| HutError::Path(secrets.clone()))?;
        let mut args = vec![
            "serve".to_owned(),
            "--listen".to_owned(),
            format!("127.0.0.1:{port}"),
            "--database-url".to_owned(),
            self.database_url()?,
            "--secrets-dir".to_owned(),
            secrets.to_owned(),
        ];
        let agent = paths.agent();
        if let Some(agent) = agent.as_deref().and_then(Path::to_str) {
            args.extend(["--agent-command".to_owned(), agent.to_owned()]);
        }
        Ok(Serve {
            program: server.to_path_buf(),
            args,
            path: paths.hut_path(agent.as_deref()),
            dir: paths.data().to_path_buf(),
            log: paths.log().to_path_buf(),
        })
    }

    /// The server binary.
    fn server(&self) -> Result<PathBuf, HutError> {
        self.inner.paths.server().ok_or(HutError::NoServer)
    }

    /// The database's `sqlite://` URL.
    fn database_url(&self) -> Result<String, HutError> {
        let database = self.inner.paths.database();
        paths::database_url(&database).ok_or(HutError::Path(database))
    }

    /// The server build at `server`: its `--version`, size and modification time, so an
    /// update of the app (a new binary, even of the same version) is told apart.
    fn build_of(&self, server: &Path) -> Result<String, HutError> {
        let broken = |said: String| HutError::Broken {
            path: server.to_path_buf(),
            said,
        };
        let ran = self
            .inner
            .spawner
            .run(&Run::new(server).arg("--version"))
            .map_err(|error| broken(error.to_string()))?;
        if !ran.ok {
            return Err(broken(ran.said()));
        }
        let metadata = std::fs::metadata(server).map_err(|error| broken(error.to_string()))?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_secs());
        Ok(format!(
            "{} ({} bytes, modified {modified})",
            ran.stdout.trim(),
            metadata.len()
        ))
    }

    /// Makes the data, secrets and log folders, the user's only, and an empty database file
    /// the user's only when there is none: SQLite gives its journal files the database's
    /// mode, and the server makes the rest (the machine's key) owner-only itself.
    fn prepare(&self) -> Result<(), HutError> {
        let paths = &self.inner.paths;
        let log_dir = paths.log().parent().map(Path::to_path_buf);
        for dir in [
            Some(paths.data().to_path_buf()),
            Some(paths.secrets()),
            log_dir,
        ]
        .into_iter()
        .flatten()
        {
            private_files::folder(&dir).map_err(|source| HutError::Files { path: dir, source })?;
        }
        let database = paths.database();
        if !database.exists() {
            private_files::file(&database)
                .map_err(|source| HutError::Files {
                    path: database,
                    source,
                })
                .map(drop)?;
        }
        Ok(())
    }

    /// What the app remembers about the hut.
    fn read_state(&self) -> HutState {
        HutState::read(&self.inner.paths.state())
    }

    /// Remembers `state`.
    fn write_state(&self, state: &HutState) -> Result<(), HutError> {
        let path = self.inner.paths.state();
        state
            .write(&path)
            .map_err(|source| HutError::Files { path, source })
    }
}

impl Drop for Inner {
    /// A hut that runs as the app's child goes with the app.
    fn drop(&mut self) {
        let Some(mut child) = lock(&self.child).take() else {
            return;
        };
        if self.spawner.terminate(child.pid()).is_err() {
            child.kill();
            return;
        }
        let asked = Instant::now();
        while child.exited().is_none() {
            if asked.elapsed() >= self.options.stop_wait {
                child.kill();
                return;
            }
            std::thread::sleep(self.options.poll);
        }
    }
}

/// Writes `text` to `path` (a launch agent's plist or a unit file), making its folder.
fn write_file(path: &Path, text: &str) -> Result<(), HutError> {
    let files = |source| HutError::Files {
        path: path.to_path_buf(),
        source,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(files)?;
    }
    std::fs::write(path, text).map_err(files)
}

/// Removes `path`, logging a failure other than its being gone already.
fn remove_file(path: &Path) {
    if let Err(error) = std::fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("couldn't remove {}: {error}", path.display());
    }
}
