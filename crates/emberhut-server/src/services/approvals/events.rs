//! The live stream's `approvals` event: a request changed, and how many wait on an answer now.
//! A client fetches what it shows; a subscriber that falls behind skips to the latest event.

use std::sync::Arc;

use emberhut_api::approvals::ApprovalsEvent;
use tokio::sync::watch;

/// Every subscriber's channel of approvals events.
#[derive(Clone, Debug)]
pub struct ApprovalEvents {
    sender: Arc<watch::Sender<Option<ApprovalsEvent>>>,
}

impl Default for ApprovalEvents {
    fn default() -> Self {
        Self {
            sender: Arc::new(watch::Sender::new(None)),
        }
    }
}

impl ApprovalEvents {
    /// Tells every subscriber of `event`.
    pub fn send(&self, event: ApprovalsEvent) {
        self.sender.send_replace(Some(event));
    }

    /// A receiver of the events sent from now on (the last one sent counts as seen).
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Option<ApprovalsEvent>> {
        self.sender.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use emberhut_api::approvals::ApprovalState;
    use emberhut_api::ids::ApprovalId;

    use super::*;

    #[tokio::test]
    async fn a_subscriber_hears_only_what_comes_after_it() {
        let events = ApprovalEvents::default();
        let event = |id| ApprovalsEvent {
            id: ApprovalId::new(id),
            state: ApprovalState::Pending,
            pending: 1,
        };
        events.send(event(1));
        let mut receiver = events.subscribe();
        assert!(!receiver.has_changed().unwrap());
        events.send(event(2));
        receiver.changed().await.unwrap();
        assert_eq!(receiver.borrow_and_update().clone(), Some(event(2)));
    }
}
