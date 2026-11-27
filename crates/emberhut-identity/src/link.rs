//! Linking a machine to an account, and a machine leaving it.

use crate::account::AccountId;
use crate::machine_id::MachineId;
use crate::machine_name::MachineName;
use crate::relay_domain::RelayDomain;
use crate::statement::{Encoder, Statement, prefix, sealed};
use crate::time::UnixMillis;

/// `emberhut link v1`: account user id, relay domain, machine name, time.
///
/// The machine signs it to be linked to that account at that relay's directory, under that
/// name. Naming the account keeps a statement made for one account from being used by another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LinkStatement {
    account: AccountId,
    relay_domain: RelayDomain,
    name: MachineName,
    time: UnixMillis,
}

impl LinkStatement {
    /// The statement linking the signer, called `name`, to `account` at `relay_domain`, made
    /// at `time`.
    #[must_use]
    pub const fn new(
        account: AccountId,
        relay_domain: RelayDomain,
        name: MachineName,
        time: UnixMillis,
    ) -> Self {
        Self {
            account,
            relay_domain,
            name,
            time,
        }
    }

    /// The account's user id.
    #[must_use]
    pub const fn account(&self) -> &AccountId {
        &self.account
    }

    /// The relay's domain.
    #[must_use]
    pub const fn relay_domain(&self) -> &RelayDomain {
        &self.relay_domain
    }

    /// The machine's name.
    #[must_use]
    pub const fn name(&self) -> &MachineName {
        &self.name
    }

    /// When it was made.
    #[must_use]
    pub const fn time(&self) -> UnixMillis {
        self.time
    }
}

impl sealed::Sealed for LinkStatement {}

impl Statement for LinkStatement {
    const PREFIX: &'static str = prefix::LINK;

    fn to_bytes(&self) -> Vec<u8> {
        Encoder::new(Self::PREFIX)
            .text(self.account.as_str())
            .text(self.relay_domain.as_str())
            .text(self.name.as_str())
            .time(self.time)
            .finish()
    }
}

/// `emberhut unlink v1`: machine id, relay domain, time.
///
/// The machine signs it to leave its account at that relay's directory, with no account token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnlinkStatement {
    machine: MachineId,
    relay_domain: RelayDomain,
    time: UnixMillis,
}

impl UnlinkStatement {
    /// The statement unlinking `machine` at `relay_domain`, made at `time`.
    #[must_use]
    pub const fn new(machine: MachineId, relay_domain: RelayDomain, time: UnixMillis) -> Self {
        Self {
            machine,
            relay_domain,
            time,
        }
    }

    /// The machine's id.
    #[must_use]
    pub const fn machine(&self) -> &MachineId {
        &self.machine
    }

    /// The relay's domain.
    #[must_use]
    pub const fn relay_domain(&self) -> &RelayDomain {
        &self.relay_domain
    }

    /// When it was made.
    #[must_use]
    pub const fn time(&self) -> UnixMillis {
        self.time
    }
}

impl sealed::Sealed for UnlinkStatement {}

impl Statement for UnlinkStatement {
    const PREFIX: &'static str = prefix::UNLINK;

    fn to_bytes(&self) -> Vec<u8> {
        Encoder::new(Self::PREFIX)
            .text(&self.machine.to_string())
            .text(self.relay_domain.as_str())
            .time(self.time)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretKey;

    #[test]
    fn link_and_unlink_signatures_are_not_interchangeable() {
        let key = SecretKey::from_bytes(&[3; 32]);
        let domain: RelayDomain = "relay.example".parse().unwrap();
        let time = UnixMillis::new(1_795_561_073_000).unwrap();
        let name: MachineName = "Main computer".parse().unwrap();
        let account = AccountId::new(key.machine_id().to_string()).unwrap();
        let link = LinkStatement::new(account, domain.clone(), name, time);
        let unlink = UnlinkStatement::new(key.machine_id(), domain, time);
        let signature = key.sign(&link);
        assert!(key.public_key().verify(&unlink, &signature).is_err());
        assert_eq!(key.public_key().verify(&link, &signature), Ok(()));
        assert_eq!(link.name().as_str(), "Main computer");
    }
}
