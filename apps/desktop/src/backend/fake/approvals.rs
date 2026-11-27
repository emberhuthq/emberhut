//! The fake's approval requests: a small server of its own. It serves the requests it was
//! given, filtered as asked, and decides or withdraws them as the real one does: a decision
//! goes through only on a pending request and its own payload hash, else it is refused with a
//! 409. Its list can be made to refuse every read, as a server without it does, and its
//! answers to decisions can be lost on the way back, after the decision was made.

use std::sync::{Mutex, MutexGuard, PoisonError};

use emberhut_api::approvals::{
    Approval, ApprovalReply, ApprovalState, ApprovalsPage, ApprovalsQuery, Decision,
    DecisionRequest,
};
use emberhut_api::ids::ApprovalId;

use crate::backend::Error;

/// The requests served.
#[derive(Debug, Default)]
pub(crate) struct ApprovalReplies {
    /// The requests on the fake server, newest first.
    requests: Mutex<Vec<Approval>>,
    /// The status every read of the list is refused with, if it is.
    refusing: Mutex<Option<u16>>,
    /// Whether a decision's answer is lost on its way back (the decision is made).
    losing: Mutex<bool>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The server's refusal of a decision on a request that changed or was decided.
fn conflict() -> Error {
    Error::Refused {
        status: Some(409),
        message: Some("This request changed or was already decided.".to_owned()),
    }
}

/// The server's answer for a request it doesn't have.
fn not_found() -> Error {
    Error::Refused {
        status: Some(404),
        message: Some("No such request.".to_owned()),
    }
}

impl ApprovalReplies {
    /// Serves `approval` (replacing the one of its id): the newest first.
    pub(crate) fn put(&self, approval: Approval) {
        let mut requests = lock(&self.requests);
        requests.retain(|served| served.id != approval.id);
        requests.insert(0, approval);
    }

    /// Refuses every read of the list with `status` from now on (none: serves it again).
    pub(crate) fn refuse_reads(&self, status: Option<u16>) {
        *lock(&self.refusing) = status;
    }

    /// Loses every decision's answer on its way back from now on: the decision is made, and the
    /// app hears that the server couldn't be reached.
    pub(crate) fn lose_answers(&self) {
        *lock(&self.losing) = true;
    }

    /// Request `id` as it is on the fake server.
    pub(crate) fn served(&self, id: ApprovalId) -> Option<Approval> {
        lock(&self.requests)
            .iter()
            .find(|served| served.id == id)
            .cloned()
    }

    /// The list as the fake server has it for `query`, as a read of it answers.
    pub(crate) fn page(&self, query: &ApprovalsQuery) -> Result<ApprovalsPage, Error> {
        if let Some(status) = *lock(&self.refusing) {
            return Err(Error::Refused {
                status: Some(status),
                message: None,
            });
        }
        let approvals = lock(&self.requests)
            .iter()
            .filter(|served| {
                query
                    .state
                    .as_ref()
                    .is_none_or(|state| served.state == *state)
            })
            .filter(|served| {
                query
                    .agent
                    .as_ref()
                    .is_none_or(|agent| served.agent == *agent)
            })
            .cloned()
            .collect();
        Ok(ApprovalsPage { approvals })
    }

    pub(super) fn one(&self, id: ApprovalId) -> Result<ApprovalReply, Error> {
        self.served(id)
            .map(|approval| ApprovalReply { approval })
            .ok_or_else(not_found)
    }

    pub(super) fn decide(
        &self,
        id: ApprovalId,
        decision: &DecisionRequest,
    ) -> Result<ApprovalReply, Error> {
        let state = match decision.decision {
            Decision::Approve => ApprovalState::Approved,
            Decision::Reject => ApprovalState::Rejected,
        };
        let answer = self.change(id, |served| {
            (served.payload_hash == decision.payload_hash).then_some(state)
        });
        if *lock(&self.losing) {
            return Err(Error::Unreachable("the answer was lost on its way".into()));
        }
        answer
    }

    pub(super) fn withdraw(&self, id: ApprovalId) -> Result<ApprovalReply, Error> {
        self.change(id, |_| Some(ApprovalState::Withdrawn))
    }

    /// Moves pending request `id` to the state `to` gives (none: refused with a 409, as is a
    /// request no longer pending).
    fn change(
        &self,
        id: ApprovalId,
        to: impl FnOnce(&Approval) -> Option<ApprovalState>,
    ) -> Result<ApprovalReply, Error> {
        let mut requests = lock(&self.requests);
        let served = requests
            .iter_mut()
            .find(|served| served.id == id)
            .ok_or_else(not_found)?;
        if served.state != ApprovalState::Pending {
            return Err(conflict());
        }
        served.state = to(served).ok_or_else(conflict)?;
        served.waits_on_owner = false;
        Ok(ApprovalReply {
            approval: served.clone(),
        })
    }
}
