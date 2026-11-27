//! The answers on their way to the server, so that quitting can wait for them: a send counts
//! from [`InFlight::start`] until the [`Sending`] it hands out is dropped, on whatever thread
//! the send runs.

use std::convert::Infallible;

use smol::channel::{self, Receiver, RecvError, Sender};

/// The sends under way.
#[derive(Debug, Default)]
pub(crate) struct InFlight {
    /// One end per send, closed once the send's [`Sending`] is dropped (nothing is ever sent
    /// on it).
    ends: Vec<Receiver<Infallible>>,
}

/// A send under way: dropping it says the send is over.
#[derive(Debug)]
#[must_use = "the send counts as over once this is dropped"]
pub(crate) struct Sending {
    _open: Sender<Infallible>,
}

impl InFlight {
    /// A send starts; it is over when the returned [`Sending`] is dropped.
    pub(crate) fn start(&mut self) -> Sending {
        self.ends.retain(|end| !end.is_closed());
        let (open, end) = channel::bounded(1);
        self.ends.push(end);
        Sending { _open: open }
    }

    /// Resolves once every send started so far is over (not those started later).
    pub(crate) fn over(&self) -> impl Future<Output = ()> + use<> {
        let ends = self.ends.clone();
        async move {
            for end in ends {
                // Nothing can be sent on it, so `recv` returns only once it closes.
                let Err(RecvError) = end.recv().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use smol::future::poll_once;

    use super::*;

    #[test]
    fn nothing_under_way_is_over_at_once() {
        let in_flight = InFlight::default();
        smol::block_on(async {
            assert_eq!(poll_once(in_flight.over()).await, Some(()));
        });
    }

    #[test]
    fn over_waits_for_every_send_started() {
        let mut in_flight = InFlight::default();
        let first = in_flight.start();
        let second = in_flight.start();
        smol::block_on(async {
            let mut over = pin!(in_flight.over());
            assert_eq!(poll_once(&mut over).await, None, "both are under way");
            drop(second);
            assert_eq!(poll_once(&mut over).await, None, "the first is under way");
            drop(first);
            assert_eq!(poll_once(&mut over).await, Some(()));
        });
    }

    #[test]
    fn a_send_ending_on_another_thread_ends_the_wait() {
        let mut in_flight = InFlight::default();
        let sending = in_flight.start();
        let over = in_flight.over();
        let thread = std::thread::spawn(move || drop(sending));
        smol::block_on(over);
        assert!(thread.join().is_ok());
    }

    #[test]
    fn a_send_started_later_isnt_waited_for() {
        let mut in_flight = InFlight::default();
        let over = in_flight.over();
        let later = in_flight.start();
        smol::block_on(async {
            assert_eq!(poll_once(over).await, Some(()));
        });
        drop(later);
    }

    #[test]
    fn sends_that_ended_are_let_go() {
        let mut in_flight = InFlight::default();
        drop(in_flight.start());
        drop(in_flight.start());
        let sending = in_flight.start();
        assert_eq!(in_flight.ends.len(), 1, "only the one under way is kept");
        drop(sending);
    }
}
