//! What the server's agent thinks with (`/api/agent/provider`): the provider saved, checking one
//! and listing its models, saving it, and removing it.
//!
//! A request carries a key one way, to the server; no answer carries it back, and the request's
//! `Debug` leaves it out.

use std::time::Duration;

use emberhut_api::agent_provider::{AgentProvider, ProviderRequest, ProviderTest};
use emberhut_api::paths;

use crate::exchange;
use crate::{Client, Error};

/// How long a check may take: the server gives the provider 15 s to answer, then words what
/// came back.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

impl Client {
    /// The provider saved, without its key (`GET /api/agent/provider`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 from a server without the route; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn agent_provider(&self) -> Result<AgentProvider, Error> {
        exchange::json(self.get(paths::AGENT_PROVIDER, "")?)
    }

    /// Checks a provider with what it needs and lists its models, saving nothing
    /// (`POST /api/agent/provider/test`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] with the server's sentence: 400 for what the provider doesn't take,
    /// 422 for a key it refused, 502 when it didn't answer; as [`Client::agent_provider`].
    pub fn test_agent_provider(&self, request: &ProviderRequest) -> Result<ProviderTest, Error> {
        exchange::json(self.post_within(paths::AGENT_PROVIDER_TEST, request, CHECK_TIMEOUT)?)
    }

    /// Checks a provider and saves it in place of whatever was saved
    /// (`PUT /api/agent/provider`); what is saved now.
    ///
    /// # Errors
    ///
    /// As [`Client::test_agent_provider`] (nothing is saved then).
    pub fn save_agent_provider(&self, request: &ProviderRequest) -> Result<AgentProvider, Error> {
        exchange::json(self.put_within(paths::AGENT_PROVIDER, request, CHECK_TIMEOUT)?)
    }

    /// Removes the saved provider and its key (`DELETE /api/agent/provider`): the agent runs on
    /// its CLI's own login again.
    ///
    /// # Errors
    ///
    /// As [`Client::agent_provider`].
    pub fn forget_agent_provider(&self) -> Result<AgentProvider, Error> {
        exchange::json(self.delete(paths::AGENT_PROVIDER)?)
    }
}
