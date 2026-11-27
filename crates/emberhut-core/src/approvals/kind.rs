//! What an approval is for.

use crate::wire::wire_enum;

wire_enum! {
    /// The kind of action an approval is for. The first six are configurable per agent; the
    /// last five are the fixed core ([`ActionKind::is_fixed_core`]), always the owner's.
    pub enum ActionKind {
        /// Publishing: a social post, a comment, a public page.
        Post = "post",
        /// A message to people outside the agent's own conversations.
        Send = "send",
        /// A merge.
        Merge = "merge",
        /// A deploy.
        Deploy = "deploy",
        /// An account, a record or a resource made in another system.
        Create = "create",
        /// Anything else outward.
        Other = "other",
        /// Money moves or is committed: a payment, a purchase, a subscription, a top-up.
        Payment = "payment",
        /// A password, a code, a one-time code, a key.
        Credential = "credential",
        /// Reading or handing over a secret.
        Secret = "secret",
        /// Sign-in, recovery, two-factor, closing an account, permissions.
        AccountSecurity = "account_security",
        /// Anything said or sent in the owner's name.
        AsOwner = "as_owner",
    }
}

impl ActionKind {
    /// Whether this kind is the fixed core's: the owner's, whatever a policy says.
    #[must_use]
    pub const fn is_fixed_core(self) -> bool {
        matches!(
            self,
            Self::Payment | Self::Credential | Self::Secret | Self::AccountSecurity | Self::AsOwner
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_words_are_stable() {
        assert_eq!(
            ActionKind::WORDS,
            [
                "post",
                "send",
                "merge",
                "deploy",
                "create",
                "other",
                "payment",
                "credential",
                "secret",
                "account_security",
                "as_owner"
            ]
        );
        for kind in ActionKind::ALL {
            assert_eq!(ActionKind::parse(kind.as_str()), Some(*kind));
        }
    }

    #[test]
    fn the_fixed_core() {
        let fixed: Vec<_> = ActionKind::ALL
            .iter()
            .filter(|k| k.is_fixed_core())
            .map(|k| k.as_str())
            .collect();
        assert_eq!(
            fixed,
            [
                "payment",
                "credential",
                "secret",
                "account_security",
                "as_owner"
            ]
        );
    }
}
