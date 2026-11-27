//! The relay handshake's statement: a machine proves its key to one relay, for one greeting.

use crate::machine_name::MachineName;
use crate::nonce::Nonce;
use crate::relay_domain::RelayDomain;
use crate::statement::{Encoder, Statement, prefix, sealed};
use crate::time::UnixMillis;

/// `emberhut relay auth v1`: relay domain, nonce, machine name, time.
///
/// The machine signs the relay's nonce and domain, so a captured handshake can't be replayed to
/// this relay or to another one, and the name it is shown under.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelayAuthStatement {
    relay_domain: RelayDomain,
    nonce: Nonce,
    name: MachineName,
    time: UnixMillis,
}

impl RelayAuthStatement {
    /// The statement for the relay at `relay_domain`, which greeted with `nonce`, from the
    /// machine called `name`, signed at `time`.
    #[must_use]
    pub const fn new(
        relay_domain: RelayDomain,
        nonce: Nonce,
        name: MachineName,
        time: UnixMillis,
    ) -> Self {
        Self {
            relay_domain,
            nonce,
            name,
            time,
        }
    }

    /// The relay's domain.
    #[must_use]
    pub const fn relay_domain(&self) -> &RelayDomain {
        &self.relay_domain
    }

    /// The relay's nonce.
    #[must_use]
    pub const fn nonce(&self) -> &Nonce {
        &self.nonce
    }

    /// The machine's name.
    #[must_use]
    pub const fn name(&self) -> &MachineName {
        &self.name
    }

    /// When the machine signed it.
    #[must_use]
    pub const fn time(&self) -> UnixMillis {
        self.time
    }
}

impl sealed::Sealed for RelayAuthStatement {}

impl Statement for RelayAuthStatement {
    const PREFIX: &'static str = prefix::RELAY_AUTH;

    fn to_bytes(&self) -> Vec<u8> {
        Encoder::new(Self::PREFIX)
            .text(self.relay_domain.as_str())
            .bytes(self.nonce.as_bytes())
            .text(self.name.as_str())
            .time(self.time)
            .finish()
    }
}
