//! Agents and approvals: the agents, the approval requests, and the owner's answers on them.

use emberhut_api::agents::AgentsPage;
use emberhut_api::approvals::{
    ApprovalReply, ApprovalsPage, ApprovalsQuery, DecisionRequest, WithdrawRequest,
};
use emberhut_api::ids::ApprovalId;
use emberhut_api::paths;

use crate::exchange;
use crate::{Client, Error};

impl Client {
    /// Every agent, the default agent first, with how many of its requests wait on an answer
    /// (`GET /api/agents`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 from a server without agents; [`Error::SignInRequired`], the
    /// transport's and decoding errors.
    pub fn agents(&self) -> Result<AgentsPage, Error> {
        exchange::json(self.get(paths::AGENTS, "")?)
    }

    /// The approval requests `query` names, newest first (`GET /api/approvals`).
    ///
    /// # Errors
    ///
    /// As [`Client::agents`].
    pub fn approvals(&self, query: &ApprovalsQuery) -> Result<ApprovalsPage, Error> {
        exchange::json(self.get(paths::APPROVALS, &query.to_query())?)
    }

    /// Request `id` as it is now, with its whole payload (`GET /api/approvals/{id}`).
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 for a request the server doesn't have; as [`Client::agents`].
    pub fn approval(&self, id: ApprovalId) -> Result<ApprovalReply, Error> {
        exchange::json(self.get(&paths::approval(id), "")?)
    }

    /// The owner's answer on request `id`, on the payload hash they were shown
    /// (`POST /api/approvals/{id}/decision`); the request as it is after.
    ///
    /// # Errors
    ///
    /// [`Error::Status`] 404 for a request the server doesn't have, 409 for one that is no
    /// longer pending or whose payload hash isn't the one given, each with the server's words;
    /// as [`Client::agents`].
    pub fn decide(
        &self,
        id: ApprovalId,
        request: &DecisionRequest,
    ) -> Result<ApprovalReply, Error> {
        exchange::json(self.post(&paths::approval_decision(id), request)?)
    }

    /// Withdraws request `id` (`POST /api/approvals/{id}/withdraw`); the request as it is after.
    ///
    /// # Errors
    ///
    /// As [`Client::decide`].
    pub fn withdraw(
        &self,
        id: ApprovalId,
        request: &WithdrawRequest,
    ) -> Result<ApprovalReply, Error> {
        exchange::json(self.post(&paths::approval_withdraw(id), request)?)
    }
}
