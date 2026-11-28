//! Windows: a Windows service, started at boot by the service manager and restarted after a
//! failure. The binary runs as `emberhut-runner run --service` and reports to the manager through
//! [`run_as_service`].
//!
//! A Windows service runs as the account it is configured with, `LocalSystem` when this installs
//! it, whatever account installed it. The agent's logins are kept in that account's profile, so
//! either sign the agent in as that account or set the service's account (its properties, Log On)
//! to the one the agent is signed in as. Installing a service needs an administrator.
//!
//! The service has no console: the runner writes its own log ([`super::log_file`]) when it runs
//! with `--service`.

use std::path::{Path, PathBuf};

use super::{HOME_ENV, MARK, SERVICE_FLAG, ServiceCommand, ServiceError, Stop};
use crate::config::Config;

/// The service's name in the service manager's list.
const DISPLAY_NAME: &str = "Emberhut runner";

/// What the runner registers with the Windows service manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsService {
    /// The service's name ([`super::WINDOWS_SERVICE`]).
    pub name: &'static str,
    /// The name the services list shows.
    pub display_name: &'static str,
    /// Its description, which starts with [`MARK`]: a service of the same name without it isn't
    /// replaced unless forced, nor removed.
    pub description: String,
    /// The binary.
    pub program: PathBuf,
    /// Its arguments, [`SERVICE_FLAG`] among them.
    pub args: Vec<String>,
    /// The environment it adds to the service's: `NAME=value` each.
    pub environment: Vec<String>,
}

/// What the runner registers to run `command` as a Windows service with its home at `home`, its
/// `PATH` set to the installing shell's (`path`, when there is one).
#[must_use]
pub fn windows_service(
    command: &ServiceCommand,
    home: &Path,
    path: Option<&str>,
) -> WindowsService {
    let mut args = command.args.clone();
    // Without the flag the binary wouldn't report to the manager, which would then end it.
    if !args.iter().any(|a| a == SERVICE_FLAG) {
        args.push(SERVICE_FLAG.to_owned());
    }
    let mut environment = vec![format!("{HOME_ENV}={}", home.to_string_lossy())];
    if let Some(path) = path.filter(|p| !p.is_empty()) {
        environment.push(format!("PATH={path}"));
    }
    WindowsService {
        name: super::WINDOWS_SERVICE,
        display_name: DISPLAY_NAME,
        description: format!("{MARK}. Runs coding-agent sessions for an Emberhut server."),
        program: command.program.clone(),
        args,
        environment,
    }
}

/// Runs `entry` as the Windows service: reports it running to the service manager, asks it to
/// stop (through its [`Stop`]) when the manager asks, and reports it stopped when it returns. The
/// binary calls this for `run --service`; it returns once the service has stopped.
///
/// # Errors
/// The process wasn't started by the service manager, the manager refuses a report, or `entry`
/// failed. Anywhere but Windows, always: there is no Windows service manager.
pub fn run_as_service<F, E>(entry: F) -> Result<(), ServiceError>
where
    F: FnOnce(Stop) -> Result<(), E> + Send + 'static,
    E: std::fmt::Display,
{
    #[cfg(windows)]
    {
        sys::run_as_service(Box::new(move |stop| entry(stop).map_err(|e| e.to_string())))
    }
    #[cfg(not(windows))]
    {
        drop(entry);
        Err(super::unsupported())
    }
}

#[cfg(windows)]
pub(super) use sys::{install, restart, status, uninstall};

#[cfg(not(windows))]
pub(super) fn install(
    _config: &Config,
    _command: &ServiceCommand,
    _force: bool,
) -> Result<String, ServiceError> {
    Err(super::unsupported())
}

#[cfg(not(windows))]
pub(super) fn uninstall(_config: &Config) -> Result<String, ServiceError> {
    Err(super::unsupported())
}

#[cfg(not(windows))]
pub(super) fn restart() -> Result<String, ServiceError> {
    Err(super::unsupported())
}

#[cfg(not(windows))]
pub(super) fn status() -> Result<String, ServiceError> {
    Err(super::unsupported())
}

#[cfg(windows)]
mod sys {
    use std::ffi::OsString;
    use std::sync::{Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use windows_service::service::{
        Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceControl,
        ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceFailureActions,
        ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    use super::{Config, MARK, ServiceCommand, ServiceError, Stop, windows_service};
    use crate::service::{WINDOWS_SERVICE, make_home, stop_pair, usable};

    /// The service's registry key, which holds its description and environment.
    const KEY: &str = r"SYSTEM\CurrentControlSet\Services\emberhut-runner";
    /// `ERROR_SERVICE_DOES_NOT_EXIST`.
    const NO_SUCH_SERVICE: i32 = 1060;
    /// How long a stop may take before the install or restart gives up on it.
    const STOP_WAIT: Duration = Duration::from_secs(30);

    type Entry = Box<dyn FnOnce(Stop) -> Result<(), String> + Send>;

    /// The entry the service manager's thread runs, put here before the dispatcher starts.
    static ENTRY: Mutex<Option<Entry>> = Mutex::new(None);
    /// How the service ended, for the dispatcher's caller.
    static OUTCOME: Mutex<Option<Result<(), ServiceError>>> = Mutex::new(None);

    fn windows(what: &str) -> impl FnOnce(windows_service::Error) -> ServiceError + '_ {
        move |e| {
            let why = match std::error::Error::source(&e) {
                Some(source) => format!("{e}: {source}"),
                None => e.to_string(),
            };
            ServiceError::Windows {
                what: what.to_owned(),
                why,
            }
        }
    }

    fn registry<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> ServiceError + '_ {
        move |e| ServiceError::Windows {
            what: what.to_owned(),
            why: e.to_string(),
        }
    }

    fn missing(e: &windows_service::Error) -> bool {
        matches!(e, windows_service::Error::Winapi(io) if io.raw_os_error() == Some(NO_SUCH_SERVICE))
    }

    fn manager(access: ServiceManagerAccess) -> Result<ServiceManager, ServiceError> {
        ServiceManager::local_computer(None::<&str>, access).map_err(windows(
            "open the service manager (it needs an administrator)",
        ))
    }

    /// Whether the service's description carries the runner's mark.
    fn ours() -> bool {
        windows_registry::LOCAL_MACHINE
            .open(KEY)
            .and_then(|key| key.get_string("Description"))
            .is_ok_and(|d| d.contains(MARK))
    }

    /// Asks the service to stop, if it runs, and waits until it has.
    fn stop_and_wait(service: &Service) -> Result<(), ServiceError> {
        let state = |s: &Service| {
            s.query_status()
                .map(|st| st.current_state)
                .map_err(windows("ask the service's state"))
        };
        if state(service)? == ServiceState::Stopped {
            return Ok(());
        }
        // A service already stopping refuses the request; the wait below covers it.
        if service.stop().is_err() && state(service)? == ServiceState::Running {
            service.stop().map_err(windows("stop the service"))?;
        }
        let start = Instant::now();
        while state(service)? != ServiceState::Stopped {
            if start.elapsed() > STOP_WAIT {
                return Err(ServiceError::Windows {
                    what: "stop the service".to_owned(),
                    why: format!("it didn't stop within {} s", STOP_WAIT.as_secs()),
                });
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Ok(())
    }

    pub(in crate::service) fn install(
        config: &Config,
        command: &ServiceCommand,
        force: bool,
    ) -> Result<String, ServiceError> {
        usable(&config.home)?;
        make_home(&config.home)?;
        let path = std::env::var("PATH").ok();
        let spec = windows_service(command, &config.home, path.as_deref());
        let info = ServiceInfo {
            name: spec.name.into(),
            display_name: spec.display_name.into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: spec.program.clone(),
            launch_arguments: spec.args.iter().map(OsString::from).collect(),
            dependencies: Vec::new(),
            // LocalSystem; see the module's comment about the agent's logins.
            account_name: None,
            account_password: None,
        };
        let access = ServiceAccess::QUERY_STATUS
            | ServiceAccess::START
            | ServiceAccess::STOP
            | ServiceAccess::CHANGE_CONFIG;
        let manager =
            manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
        let service = match manager.open_service(WINDOWS_SERVICE, access) {
            Ok(service) => {
                if !force && !ours() {
                    return Err(ServiceError::NotOurs(format!(
                        "the Windows service {WINDOWS_SERVICE}"
                    )));
                }
                service
                    .change_config(&info)
                    .map_err(windows("change the service"))?;
                service
            }
            Err(e) if missing(&e) => manager
                .create_service(&info, access)
                .map_err(windows("create the service"))?,
            Err(e) => return Err(windows("open the service")(e)),
        };
        service
            .set_description(&spec.description)
            .map_err(windows("describe the service"))?;
        let environment: Vec<&str> = spec.environment.iter().map(String::as_str).collect();
        windows_registry::LOCAL_MACHINE
            .create(KEY)
            .and_then(|key| key.set_multi_string("Environment", &environment))
            .map_err(registry("set the service's environment"))?;
        // Restarted 5 s after a failure, as the systemd unit and the launchd agent are; a failure
        // includes stopping with an error, not only a crash.
        let restart = ServiceAction {
            action_type: ServiceActionType::Restart,
            delay: Duration::from_secs(5),
        };
        service
            .update_failure_actions(ServiceFailureActions {
                reset_period: ServiceFailureResetPeriod::After(Duration::from_hours(24)),
                reboot_msg: None,
                command: None,
                actions: Some(vec![restart.clone(), restart.clone(), restart]),
            })
            .map_err(windows("set the service's restarts"))?;
        service
            .set_failure_actions_on_non_crash_failures(true)
            .map_err(windows("set the service's restarts"))?;
        stop_and_wait(&service)?;
        service
            .start::<&str>(&[])
            .map_err(windows("start the service"))?;
        Ok(format!(
            "Windows service {WINDOWS_SERVICE} installed and started. It runs as LocalSystem: \
             sign the agent in as that account, or set the account it runs as in its properties (Log On)"
        ))
    }

    pub(in crate::service) fn uninstall(config: &Config) -> Result<String, ServiceError> {
        let manager = manager(ServiceManagerAccess::CONNECT)?;
        let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
        let what = match manager.open_service(WINDOWS_SERVICE, access) {
            Err(e) if missing(&e) => "no Windows service to remove".to_owned(),
            Err(e) => return Err(windows("open the service")(e)),
            Ok(_) if !ours() => format!(
                "the Windows service {WINDOWS_SERVICE} stays, as emberhut-runner didn't install it"
            ),
            Ok(service) => {
                stop_and_wait(&service)?;
                service.delete().map_err(windows("remove the service"))?;
                format!("Windows service {WINDOWS_SERVICE} stopped and removed")
            }
        };
        Ok(format!(
            "{what}. The runner's home ({}) stays.",
            config.home.display()
        ))
    }

    pub(in crate::service) fn restart() -> Result<String, ServiceError> {
        let manager = manager(ServiceManagerAccess::CONNECT)?;
        let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START;
        let service = manager
            .open_service(WINDOWS_SERVICE, access)
            .map_err(windows("open the service"))?;
        stop_and_wait(&service)?;
        service
            .start::<&str>(&[])
            .map_err(windows("start the service"))?;
        Ok("restarted".to_owned())
    }

    pub(in crate::service) fn status() -> Result<String, ServiceError> {
        let manager = manager(ServiceManagerAccess::CONNECT)?;
        let service = match manager.open_service(WINDOWS_SERVICE, ServiceAccess::QUERY_STATUS) {
            Ok(service) => service,
            Err(e) if missing(&e) => return Ok("not installed".to_owned()),
            Err(e) => return Err(windows("open the service")(e)),
        };
        let status = service
            .query_status()
            .map_err(windows("ask the service's state"))?;
        let state = match status.current_state {
            ServiceState::Stopped => "stopped",
            ServiceState::StartPending => "starting",
            ServiceState::StopPending => "stopping",
            ServiceState::Running => "running",
            ServiceState::ContinuePending => "resuming",
            ServiceState::PausePending => "pausing",
            ServiceState::Paused => "paused",
        };
        Ok(match status.process_id {
            Some(pid) => format!("Windows service {WINDOWS_SERVICE}: {state} (process {pid})"),
            None => format!("Windows service {WINDOWS_SERVICE}: {state}"),
        })
    }

    pub(super) fn run_as_service(entry: Entry) -> Result<(), ServiceError> {
        *ENTRY.lock().unwrap_or_else(PoisonError::into_inner) = Some(entry);
        windows_service::service_dispatcher::start(WINDOWS_SERVICE, ffi_service_main).map_err(windows(
            "start as a Windows service (only the service manager starts the runner with --service)",
        ))?;
        OUTCOME
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or(Ok(()))
    }

    /// The service's main function, on the thread the service manager starts.
    fn service_main(_arguments: Vec<OsString>) {
        let outcome = serve();
        *OUTCOME.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
    }

    fn report(state: ServiceState, accept: ServiceControlAccept, code: u32) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: if code == 0 {
                ServiceExitCode::Win32(0)
            } else {
                ServiceExitCode::ServiceSpecific(code)
            },
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        }
    }

    fn serve() -> Result<(), ServiceError> {
        let entry = ENTRY
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| ServiceError::Runner("started twice".to_owned()))?;
        let (stopper, stop) = stop_pair();
        let handle =
            service_control_handler::register(WINDOWS_SERVICE, move |control| match control {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    stopper.stop();
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            })
            .map_err(windows("register with the service manager"))?;
        handle
            .set_service_status(report(
                ServiceState::Running,
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                0,
            ))
            .map_err(windows("report the service running"))?;
        let result = entry(stop).map_err(ServiceError::Runner);
        let code = u32::from(result.is_err());
        handle
            .set_service_status(report(
                ServiceState::Stopped,
                ServiceControlAccept::empty(),
                code,
            ))
            .map_err(windows("report the service stopped"))?;
        result
    }

    // The entry point the service manager calls on its own thread: it hands the arguments to
    // `service_main`.
    windows_service::define_windows_service!(ffi_service_main, service_main);
}
