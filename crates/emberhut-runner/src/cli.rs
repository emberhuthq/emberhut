//! The runner's command line. `run` is the daemon (what the service starts); the rest set it up
//! on this machine: pairing with a server and installing the service in one step (`install`),
//! the service alone, the accounts, and a look at whether it answers.

use std::io::Write as _;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};

use crate::accounts::{Account, Accounts, Logins, Provider};
use crate::config::Config;
use crate::engine::claude::ClaudeCode;
use crate::pair::{self, PairAsk};
use crate::runner::{Options, Runner};
use crate::service::{self, ServiceCommand};

/// The command line.
#[derive(Debug, Parser)]
#[command(name = "emberhut-runner", version = crate::version(), about = "Runs coding-agent sessions on this machine for an Emberhut server.")]
pub struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// One command.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// The daemon (what the service starts).
    Run {
        /// Started by the Windows service manager: report to it.
        #[arg(long)]
        service: bool,
    },
    /// Pair with a server (a code from the app's Machines page, "Add a hut"), then install and
    /// start the service. Already paired: installs the service again.
    Install(InstallArgs),
    /// Pair with a server, then install the service unless `--no-service`.
    Pair(PairArgs),
    /// The service alone.
    Service {
        /// install, uninstall, restart or status.
        #[arg(value_parser = ["install", "uninstall", "restart", "status"])]
        action: String,
        /// Replace a service file the runner didn't write.
        #[arg(long)]
        force: bool,
    },
    /// Whether the runner here answers, and with what.
    Status,
    /// Exits 0 when paired (for installers).
    Paired,
    /// The accounts, their use and logins.
    #[command(visible_alias = "account")]
    Accounts {
        /// list (the default), add, login, check or clear.
        #[command(subcommand)]
        action: Option<AccountsCommand>,
    },
    /// The version.
    Version,
}

/// `install`'s options.
#[derive(Debug, Args)]
pub struct InstallArgs {
    /// The server's address.
    #[arg(long, env = "EMBERHUT_SERVER")]
    pub server: Option<String>,
    /// The one-time code from the app.
    #[arg(long = "pair", env = "EMBERHUT_PAIR_CODE")]
    pub code: Option<String>,
    /// The name to ask for (this machine's name when not given).
    #[arg(long)]
    pub name: Option<String>,
    /// Where the machine is, in words.
    #[arg(long)]
    pub place: Option<String>,
    /// The address the server calls this runner at.
    #[arg(long)]
    pub advertise: Option<String>,
    /// Replace a service file the runner didn't write.
    #[arg(long)]
    pub force: bool,
}

/// `pair`'s options.
#[derive(Debug, Args)]
pub struct PairArgs {
    /// The server's address.
    #[arg(long, env = "EMBERHUT_SERVER")]
    pub server: String,
    /// The one-time code from the app.
    #[arg(long, env = "EMBERHUT_PAIR_CODE")]
    pub code: String,
    /// The name to ask for.
    #[arg(long)]
    pub name: Option<String>,
    /// Where the machine is, in words.
    #[arg(long)]
    pub place: Option<String>,
    /// The address the server calls this runner at.
    #[arg(long)]
    pub advertise: Option<String>,
    /// Only pair; leave the service as it is.
    #[arg(long)]
    pub no_service: bool,
}

/// `accounts`' subcommands.
#[derive(Debug, Subcommand)]
pub enum AccountsCommand {
    /// The accounts, their use and logins.
    List {
        /// As JSON: id, name, email, plan, use and state of each.
        #[arg(long)]
        json: bool,
    },
    /// Adds an account. With no id, a new Claude subscription folder (`claude-1`, `claude-2`, …)
    /// in the runner's home, signed in at once with Claude Code's own browser sign-in.
    Add {
        /// Its id (letters, digits, - and _); the next free `claude-N` when not given.
        id: Option<String>,
        /// Its name in words (`Claude N` when not given).
        name: Option<String>,
        /// claude (a subscription login), api or bedrock.
        #[arg(long, default_value = "claude")]
        provider: String,
        /// The agent's configuration folder for it (`<runner home>/accounts/<id>` when not given).
        #[arg(long)]
        config_dir: Option<String>,
        /// Tried first when lower.
        #[arg(long)]
        order: Option<i64>,
        /// Only list it; sign in later with `accounts login <id>`.
        #[arg(long)]
        no_login: bool,
    },
    /// Signs an account in (Claude Code's own sign-in, in its folder).
    Login {
        /// The account.
        id: String,
    },
    /// Takes an account off the list (its folder and login stay where they are).
    Remove {
        /// The account.
        id: String,
    },
    /// Looks at every account's use now.
    Check,
    /// Forgets that an account was out.
    Clear {
        /// The account.
        id: String,
    },
}

/// What went wrong with a command, in words for a person.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct CliError(pub String);

impl CliError {
    fn of(e: impl std::fmt::Display) -> Self {
        Self(e.to_string())
    }
}

fn out(s: &str) {
    let mut o = std::io::stdout().lock();
    // Nothing to do when the terminal is gone.
    o.write_all(s.as_bytes())
        .and_then(|()| o.write_all(b"\n"))
        .unwrap_or_default();
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

fn hostname() -> String {
    crate::machine::hostname()
}

/// The settings for this machine.
///
/// # Errors
/// They are wrong.
pub fn config() -> Result<Config, CliError> {
    Config::load(&env, &hostname()).map_err(CliError::of)
}

async fn accounts_of(c: &Config) -> Result<Accounts, CliError> {
    let user = c.user.clone();
    let logins = Logins::new(user, c.secrets_dir(), cfg!(target_os = "macos"));
    let a = Accounts::open(
        c,
        logins,
        Arc::new(ClaudeCode::new(c.agent_command.clone())),
    )
    .await
    .map_err(CliError::of)?;
    a.logins.refresh(&a.list()).await;
    Ok(a)
}

fn show_accounts(a: &Accounts) {
    let pct = |x: Option<f64>| x.map_or_else(|| "   ?".to_owned(), |x| format!("{x:>3.0}%"));
    for x in a.summary() {
        let state = if !x.enabled {
            "off".to_owned()
        } else if !x.logged_in {
            format!("not logged in: emberhut-runner accounts login {}", x.id)
        } else if let Some(e) = &x.error {
            format!("error: {e}")
        } else if let Some(u) = &x.limited_until {
            format!(
                "out until {} ({})",
                u.chars().take(16).collect::<String>(),
                x.limited_why.clone().unwrap_or_default()
            )
        } else {
            "ok".to_owned()
        };
        out(&format!(
            "{:<12} {:<28} {:<26} 5h {}  week {}  {:<8} {state}",
            x.id,
            x.name,
            x.email.clone().unwrap_or_default(),
            pct(x.used5h),
            pct(x.used7d),
            x.provider
        ));
    }
}

/// Signs an account in with Claude Code's own browser sign-in (`claude auth login --claudeai`)
/// in its folder; the runner never sees the login.
async fn login(c: &Config, a: &Accounts, acc: &Account) -> Result<(), CliError> {
    if acc.provider() != Provider::Claude {
        let file = acc.key_file().unwrap_or("(its keyFile)");
        return Err(CliError(format!(
            "{} signs in with a key: put it in {} (only you may read it)",
            acc.name,
            c.secrets_dir().join(file).display()
        )));
    }
    let dir = a.logins.prepare_dir(acc, c.skills_dir.as_deref());
    out(&format!(
        "Signing in \"{}\" (folder {}). A browser page opens: sign in with the right account.",
        acc.name,
        dir.display()
    ));
    let env = a.logins.env_for(acc, c.skills_dir.as_deref());
    let mut cmd = tokio::process::Command::new(&c.agent_command);
    cmd.args(["auth", "login", "--claudeai"]);
    for k in &env.remove {
        cmd.env_remove(k);
    }
    cmd.envs(env.set.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let status = cmd
        .status()
        .await
        .map_err(|e| CliError(format!("cannot start {}: {e}", c.agent_command.display())))?;
    if !status.success() {
        return Err(CliError(format!(
            "the sign-in didn't finish; run `emberhut-runner accounts login {}` again",
            acc.id
        )));
    }
    a.logins.forget(&acc.id);
    a.logins.refresh(std::slice::from_ref(acc)).await;
    let u = a.probe(acc).await;
    match u.as_ref().and_then(|u| u.email.clone()) {
        Some(e) if u.as_ref().is_some_and(|u| u.error.is_none()) => {
            out(&format!("Signed in as {e}."));
        }
        _ => out(&format!(
            "Not signed in yet ({}).",
            u.and_then(|u| u.error)
                .unwrap_or_else(|| "unknown".to_owned())
        )),
    }
    Ok(())
}

/// The next free `claude-N` id.
fn next_claude_id(list: &[Account]) -> (String, u32) {
    // One more than the list is long is always free.
    let n = (1..=u32::try_from(list.len()).unwrap_or(u32::MAX - 1) + 1)
        .find(|n| !list.iter().any(|a| a.id == format!("claude-{n}")))
        .unwrap_or(1);
    (format!("claude-{n}"), n)
}

async fn accounts_command(c: &Config, action: Option<AccountsCommand>) -> Result<(), CliError> {
    let a = accounts_of(c).await?;
    match action.unwrap_or(AccountsCommand::List { json: false }) {
        AccountsCommand::List { json: false } => show_accounts(&a),
        AccountsCommand::List { json: true } => {
            out(&serde_json::to_string_pretty(&a.summary()).map_err(CliError::of)?);
        }
        AccountsCommand::Check => {
            a.probe_all().await;
            show_accounts(&a);
        }
        AccountsCommand::Clear { id } => {
            a.clear_limit(&id).await;
            show_accounts(&a);
        }
        AccountsCommand::Login { id } => {
            let acc = a.get(Some(&id)).ok_or_else(|| {
                let ids: Vec<String> = a.list().into_iter().map(|x| x.id).collect();
                CliError(format!("no account {id}; accounts: {}", ids.join(", ")))
            })?;
            login(c, &a, &acc).await?;
        }
        AccountsCommand::Remove { id } => {
            let mut list = a.try_list().map_err(CliError::of)?;
            let Some(at) = list.iter().position(|x| x.id == id) else {
                return Err(CliError(format!("no account {id}")));
            };
            let gone = list.remove(at);
            crate::accounts::list::write_accounts(&c.accounts_file, &list).map_err(CliError::of)?;
            let folder = gone.config_dir.as_deref().map_or_else(String::new, |d| {
                format!(" Its folder {d} and its login stay there.")
            });
            out(&format!("Removed {}.{folder}", gone.id));
        }
        AccountsCommand::Add {
            id,
            name,
            provider,
            config_dir,
            order,
            no_login,
        } => {
            let mut list = a.try_list().map_err(CliError::of)?;
            let (id, name) = if let Some(id) = id {
                let name = name.unwrap_or_else(|| id.clone());
                (id, name)
            } else {
                let (id, n) = next_claude_id(&list);
                (id, name.unwrap_or_else(|| format!("Claude {n}")))
            };
            if !crate::accounts::list::is_account_id(&id) {
                return Err(CliError(
                    "an account id is letters, digits, - and _".to_owned(),
                ));
            }
            if list.iter().any(|x| x.id == id) {
                return Err(CliError(format!("{id} is already listed")));
            }
            let provider = match provider.as_str() {
                "claude" => Provider::Claude,
                "api" => Provider::Api,
                "bedrock" => Provider::Bedrock,
                _ => return Err(CliError("provider: claude, api or bedrock".to_owned())),
            };
            let config_dir = (provider == Provider::Claude).then(|| {
                config_dir
                    .unwrap_or_else(|| c.home.join("accounts").join(&id).display().to_string())
            });
            let acc = Account {
                id: id.clone(),
                name,
                provider: Some(provider),
                config_dir,
                order,
                ..Account::default()
            };
            list.push(acc.clone());
            crate::accounts::list::write_accounts(&c.accounts_file, &list).map_err(CliError::of)?;
            if provider != Provider::Claude {
                out(&format!(
                    "Added {id}. Put its key in {} (see the README).",
                    c.secrets_dir().display()
                ));
            } else if no_login {
                out(&format!(
                    "Added {id}. Sign it in with: emberhut-runner accounts login {id}"
                ));
            } else {
                out(&format!("Added {id}."));
                login(c, &a, &acc).await?;
            }
        }
    }
    Ok(())
}

async fn status_command(c: &Config) -> Result<(), CliError> {
    let ip = c
        .listen
        .first()
        .copied()
        .unwrap_or(std::net::IpAddr::from([127, 0, 0, 1]));
    let ip = if ip.is_unspecified() {
        std::net::IpAddr::from([127, 0, 0, 1])
    } else {
        ip
    };
    let base = format!("http://{}", std::net::SocketAddr::new(ip, c.port));
    let http = crate::client::client().map_err(CliError::of)?;
    let health = http
        .get(format!("{base}/health"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await;
    match health {
        Ok(r) if r.status().is_success() => out(&format!(
            "The runner at {base} answers: {}",
            r.text().await.unwrap_or_default()
        )),
        _ => {
            out(&format!("The runner at {base} doesn't answer."));
            out(&service::status().unwrap_or_else(|e| e.to_string()));
            return Err(CliError("not running".to_owned()));
        }
    }
    if c.token().is_some() {
        out(&format!(
            "Paired with {} as {}.",
            c.server.as_deref().unwrap_or("(no server set)"),
            c.name
        ));
    } else {
        out("Not paired yet: emberhut-runner install --server URL --pair CODE");
    }
    Ok(())
}

async fn pair_and_install(
    c: &Config,
    ask: Option<PairAsk>,
    install: bool,
    force: bool,
) -> Result<(), CliError> {
    if let Some(ask) = ask {
        let http = crate::client::client().map_err(CliError::of)?;
        let p = pair::pair(c, &ask, &http).await.map_err(CliError::of)?;
        out(&format!(
            "Paired with {} as {}; the server calls this runner at {}.",
            p.server, p.name, p.url
        ));
    }
    if install {
        // The settings pairing just wrote.
        let c = config()?;
        let command = ServiceCommand::current(&env).map_err(CliError::of)?;
        out(&service::install(&c, &command, force).map_err(CliError::of)?);
    }
    Ok(())
}

/// Runs the daemon until it is asked to stop (a signal, or the service manager).
async fn run(c: Config, stop: service::Stop) -> Result<(), CliError> {
    let r = Runner::start(c, Options::default())
        .await
        .map_err(CliError::of)?;
    tokio::select! {
        () = stop.wait() => tracing::info!("asked to stop; running turns carry on after the next start"),
        () = signal() => tracing::info!("stopping; running turns carry on after the next start"),
    }
    r.close().await;
    Ok(())
}

async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) {
            (Ok(mut term), Ok(mut int)) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = int.recv() => {}
                }
            }
            _ => std::future::pending::<()>().await,
        }
    }
    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, CliError> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(CliError::of)
}

/// Runs one command.
///
/// # Errors
/// What went wrong, in words.
pub fn main(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Version => {
            out(crate::version());
            Ok(())
        }
        Command::Run { service: true } => service::run_as_service(|stop| {
            let c = config()?;
            runtime()?.block_on(run(c, stop))
        })
        .map_err(CliError::of),
        Command::Run { service: false } => {
            let c = config()?;
            let (_stopper, stop) = service::stop_pair();
            runtime()?.block_on(run(c, stop))
        }
        Command::Install(a) => {
            let c = config()?;
            let ask = match (a.server, a.code) {
                (Some(server), Some(code)) => Some(PairAsk { server, code, name: a.name, place: a.place, advertise: a.advertise }),
                (None, None) if c.token().is_some() && c.server.is_some() => None,
                _ => return Err(CliError("install needs --server URL and --pair CODE (the code from the app's Machines page, \"Add a hut\")".to_owned())),
            };
            runtime()?.block_on(pair_and_install(&c, ask, true, a.force))
        }
        Command::Pair(a) => {
            let c = config()?;
            let ask = PairAsk {
                server: a.server,
                code: a.code,
                name: a.name,
                place: a.place,
                advertise: a.advertise,
            };
            runtime()?.block_on(pair_and_install(&c, Some(ask), !a.no_service, false))
        }
        Command::Service { action, force } => {
            let c = config()?;
            let said = match action.as_str() {
                "install" => service::install(
                    &c,
                    &ServiceCommand::current(&env).map_err(CliError::of)?,
                    force,
                ),
                "uninstall" => service::uninstall(&c),
                "restart" => service::restart(),
                _ => service::status(),
            };
            out(&said.map_err(CliError::of)?);
            Ok(())
        }
        Command::Status => {
            let c = config()?;
            runtime()?.block_on(status_command(&c))
        }
        Command::Paired => {
            let c = config()?;
            if c.token().is_some() && c.server.is_some() {
                out(&format!(
                    "paired with {} as {}",
                    c.server.as_deref().unwrap_or_default(),
                    c.name
                ));
                Ok(())
            } else {
                out("not paired");
                Err(CliError(String::new()))
            }
        }
        Command::Accounts { action } => {
            let c = config()?;
            runtime()?.block_on(accounts_command(&c, action))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_subscription_gets_the_next_free_number() {
        let acc = |id: &str| Account {
            id: id.to_owned(),
            name: id.to_owned(),
            ..Account::default()
        };
        assert_eq!(next_claude_id(&[]).0, "claude-1");
        assert_eq!(
            next_claude_id(&[acc("claude-1"), acc("default"), acc("claude-3")]).0,
            "claude-2"
        );
        assert_eq!(
            next_claude_id(&[acc("claude-1"), acc("claude-2")]).0,
            "claude-3"
        );
    }

    #[test]
    fn the_command_line_takes_what_the_installers_and_the_app_run() {
        for args in [
            &[
                "emberhut-runner",
                "install",
                "--server",
                "https://hut.example.com",
                "--pair",
                "ABCD-1234",
            ][..],
            &["emberhut-runner", "account", "add"],
            &["emberhut-runner", "accounts", "list", "--json"],
            &["emberhut-runner", "account", "remove", "claude-2"],
            &["emberhut-runner", "service", "restart"],
            &["emberhut-runner", "run", "--service"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok(), "{args:?}");
        }
    }
}
