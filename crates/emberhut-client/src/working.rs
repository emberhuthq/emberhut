//! Where the assistant works and what it uses: the environments page and its rows' actions,
//! the Claude accounts' use and the account the assistant runs on, and the background jobs.

use std::time::Duration;

use emberhut_api::agent_accounts::AgentAccounts;
use emberhut_api::environments::{EnvActionRequest, EnvPage};
use emberhut_api::error::OkReply;
use emberhut_api::jobs::{self, JobAction, JobActionRequest, JobsPage};
use emberhut_api::paths;
use emberhut_api::usage::{UsageCheckReply, UseAccountReply, UseAccountRequest};

use crate::exchange;
use crate::{Client, Error};

/// How long an environment's action may take: the server looks at the place again (a
/// runner's health, a browser that starts) before it answers.
const ENV_ACTION_TIMEOUT: Duration = Duration::from_secs(40);

/// How long stopping a job may take: the server looks at every job again first.
const JOB_ACTION_TIMEOUT: Duration = Duration::from_secs(30);

impl Client {
    /// Every place the assistant works and connection it reads through
    /// (`GET /api/environments`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 from a server without the page; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn environments(&self) -> Result<EnvPage, Error> {
        exchange::json(self.get(paths::ENVIRONMENTS, "")?)
    }

    /// Runs one row's action (`POST /api/environments/check`): look again, a re-pair or
    /// sign-in link, start, unlink. The page as it is after, with what the action made.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason (an action the row doesn't have, a
    /// start that failed); as [`Client::environments`].
    pub fn environment_action(&self, request: &EnvActionRequest) -> Result<EnvPage, Error> {
        exchange::json(self.post_within(paths::ENVIRONMENTS_CHECK, request, ENV_ACTION_TIMEOUT)?)
    }

    /// Asks the server to check every Claude account's use now (`POST /api/usage/check`);
    /// whether a check started (the server checks at most once in two minutes). The state's
    /// next polls bring the numbers.
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], [`Error::Status`], the transport's and decoding errors.
    pub fn check_usage(&self) -> Result<bool, Error> {
        let reply: UsageCheckReply =
            exchange::json(self.post(paths::USAGE_CHECK, &serde_json::json!({}))?)?;
        Ok(reply.asked)
    }

    /// Pins the assistant to an account, or sets it to Auto (`POST /api/accounts/use`); the
    /// account as it is after, and every account's use.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason (no such account); as
    /// [`Client::check_usage`].
    pub fn use_account(&self, request: &UseAccountRequest) -> Result<UseAccountReply, Error> {
        exchange::json(self.post(paths::ACCOUNTS_USE, request)?)
    }

    /// The agent CLI accounts the server runs on and whether each can take work
    /// (`GET /api/agent/accounts`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 from a server without the list; [`Error::SignInRequired`] and
    /// the transport's and decoding errors.
    pub fn agent_accounts(&self) -> Result<AgentAccounts, Error> {
        exchange::json(self.get(paths::AGENT_CLI_ACCOUNTS, "")?)
    }

    /// Asks to hear when `list` is ready (`PUT /api/waitlist/{list}`, `cloud-home`): kept on
    /// the server, nothing paid.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 for a list the server doesn't know, 404 from a server without
    /// waitlists; [`Error::SignInRequired`] and the transport's errors.
    pub fn join_waitlist(&self, list: &str) -> Result<(), Error> {
        let answer = self.put_within(
            &paths::WAITLIST.replace("{list}", list),
            &serde_json::json!({}),
            Duration::from_secs(20),
        )?;
        if !answer.status().is_success() {
            return Err(exchange::refusal(answer));
        }
        Ok(())
    }

    /// The background jobs on the machines (`GET /api/jobs`); `fresh` looks again rather
    /// than taking the server's few-seconds-old list.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 from a server without the list; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn jobs(&self, fresh: bool) -> Result<JobsPage, Error> {
        let query = if fresh { jobs::FRESH } else { "" };
        exchange::json(self.get(paths::JOBS, query)?)
    }

    /// Stops a job, or takes a failed one off the list (`POST /api/jobs/stop|clear`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 with the server's reason (it ended already, it can't be stopped
    /// from here); as [`Client::jobs`].
    pub fn act_on_job(&self, action: JobAction, request: &JobActionRequest) -> Result<(), Error> {
        let _: OkReply = exchange::json(self.post_within(
            &paths::job_action(action),
            request,
            JOB_ACTION_TIMEOUT,
        )?)?;
        Ok(())
    }
}
