//! The update keys this app trusts: the public halves of the minisign keys a release's payload
//! may be signed with, each with its key id.
//!
//! The release key's secret half signs in the release workflow (it is a secret of the `release`
//! environment, which only `v*` tags may use); its public half is here. A payload signed by any
//! other key fails closed: nothing is installed.
//!
//! Rotating: a release signed with the old key ships an app that trusts both; a later one drops
//! the old key once installed apps have moved.

/// A trusted public key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TrustedKey {
    /// Its minisign key id, as `minisign` prints it (16 hex digits), for the log.
    pub(crate) id: &'static str,
    /// The public key, base64, as the second line of a `minisign.pub` file.
    pub(crate) public: &'static str,
}

/// The keys a payload's signature is checked against, by its key id.
pub(crate) const TRUSTED: &[TrustedKey] = &[TrustedKey {
    id: "FF00313EAD564744",
    public: "RWRER1atPjEA/9GrQdW+h0cy57Fiy526c7fbp1tkAabdvdSNURP3nnyD",
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_key_is_trusted() {
        assert_eq!(TRUSTED.len(), 1);
        assert_eq!(TRUSTED[0].id, "FF00313EAD564744");
    }

    #[test]
    fn every_trusted_key_reads_as_a_minisign_key() {
        for key in TRUSTED {
            assert!(
                minisign_verify::PublicKey::from_base64(key.public).is_ok(),
                "{}",
                key.id
            );
            assert_eq!(key.id.len(), 16, "{}", key.id);
        }
    }
}
