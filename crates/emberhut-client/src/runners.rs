//! The runners the server knows: a one-time code that pairs a new one, with the lines that
//! install it, the list, and forgetting a paired one ([`emberhut_api::runner::pairing`]).

use emberhut_api::error::OkReply;
use emberhut_api::paths;
use emberhut_api::runner::pairing::{NewRunnerCode, RunnerCode, RunnersPage, is_runner_name};

use crate::{Client, Error, exchange};

impl Client {
    /// A one-time code a new runner pairs with (`POST /api/runners/codes`), with the server's
    /// address the runner is to use and the lines that install and pair it. `server` is the
    /// server's address as this app reaches it, when another machine can reach it too; left
    /// out, the server picks one, and [`RunnerCode::reachable`] says whether another machine
    /// can reach it.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 400 for a `server` that isn't an `http://` or `https://` address;
    /// [`Error::SignInRequired`] without a signed-in device; the transport's and decoding
    /// errors.
    pub fn runner_pair_code(&self, server: Option<&str>) -> Result<RunnerCode, Error> {
        let asked = NewRunnerCode {
            server: server.map(|server| server.trim_end_matches('/').to_owned()),
        };
        exchange::json(self.post(paths::RUNNER_CODES, &asked)?)
    }

    /// Every runner the server knows (`GET /api/runners`): the configured ones, then the paired
    /// ones.
    ///
    /// # Errors
    ///
    /// [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn runners(&self) -> Result<RunnersPage, Error> {
        exchange::json(self.get(paths::RUNNERS, "")?)
    }

    /// Forgets the paired runner `name` (`DELETE /api/runners/{name}`): its token stops working
    /// at once.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidPath`] for a name no runner can have (never sent); [`Error::Status`] 404
    /// for a name no paired runner has, 409 for a runner the server's configuration names;
    /// [`Error::SignInRequired`], the transport's and decoding errors.
    pub fn forget_runner(&self, name: &str) -> Result<(), Error> {
        let path = paths::runner(name);
        if !is_runner_name(name) {
            return Err(Error::InvalidPath(path));
        }
        let _: OkReply = exchange::json(self.delete(&path)?)?;
        Ok(())
    }
}
