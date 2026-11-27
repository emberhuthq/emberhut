//! The words that make a step the owner's every time: money, credentials and account
//! security. The ask guard ([`super::ask_guard`]) reads them to keep such an ask for the owner,
//! and the approvals' fixed core (`crate::approvals::fixed_core`) reads them to raise a request
//! to the matching fixed-core kind. One list, so the two never disagree on what counts.

use std::sync::LazyLock;

use regex::Regex;

static MONEY: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(pay|pays|paying|payment|payments|purchase|purchases|buy|buying|checkout|check out|credit card|debit card|billing details|add (a|the|his|her|their|your) card)\b").ok()
});
static CREDENTIAL: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(password|passwords|passcode|passkey|otp|one-time code|verification code|security code|login code|sign-in code|recovery (code|codes|key))\b").ok()
});
static ACCOUNT_SECURITY: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(2fa|two-factor|mfa|recovery (email|phone)|authenticator|revoke|(delete|close|deactivate) (my |your |his |her |their |the |this )?account)\b").ok()
});

/// What a text is about, of the things that are always the owner's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HardTopic {
    /// Money moves or is committed: paying, buying, a card.
    Money,
    /// A password, a passkey or a code.
    Credential,
    /// Two-factor, recovery, revoking access, closing an account.
    AccountSecurity,
}

impl HardTopic {
    /// Every topic, in the order [`topic`] tries them.
    pub const ALL: [Self; 3] = [Self::Money, Self::Credential, Self::AccountSecurity];

    fn pattern(self) -> &'static LazyLock<Option<Regex>> {
        match self {
            Self::Money => &MONEY,
            Self::Credential => &CREDENTIAL,
            Self::AccountSecurity => &ACCOUNT_SECURITY,
        }
    }

    /// Whether `text` is about this topic.
    #[must_use]
    pub fn in_text(self, text: &str) -> bool {
        self.pattern().as_ref().is_some_and(|re| re.is_match(text))
    }
}

/// The first topic `text` is about, trying money, then credentials, then account security.
///
/// ```
/// use emberhut_core::guard::hard_words::{HardTopic, topic};
///
/// assert_eq!(topic("Pay the invoice"), Some(HardTopic::Money));
/// assert_eq!(topic("Here is the one-time code"), Some(HardTopic::Credential));
/// assert_eq!(topic("Turn on two-factor"), Some(HardTopic::AccountSecurity));
/// assert_eq!(topic("Publish the release notes"), None);
/// ```
#[must_use]
pub fn topic(text: &str) -> Option<HardTopic> {
    HardTopic::ALL.into_iter().find(|t| t.in_text(text))
}

/// Whether `text` is about any of the topics.
#[must_use]
pub fn is_hard(text: &str) -> bool {
    topic(text).is_some()
}

/// Every pattern here compiles.
#[must_use]
pub fn patterns_compile() -> bool {
    HardTopic::ALL.iter().all(|t| t.pattern().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_compile_here() {
        assert!(patterns_compile());
    }

    #[test]
    fn each_topic_reads_its_words() {
        for text in [
            "buy the domain",
            "checkout now",
            "add your card",
            "Payments",
        ] {
            assert_eq!(topic(text), Some(HardTopic::Money), "{text}");
        }
        for text in [
            "the password is",
            "send the OTP",
            "recovery codes",
            "a passkey",
        ] {
            assert_eq!(topic(text), Some(HardTopic::Credential), "{text}");
        }
        for text in [
            "set up 2FA",
            "the recovery email",
            "close my account",
            "revoke it",
        ] {
            assert_eq!(topic(text), Some(HardTopic::AccountSecurity), "{text}");
        }
    }

    #[test]
    fn whole_words_only() {
        for text in [
            "repayment plan",
            "buyer persona",
            "a passwordless flow",
            "unrevoked",
        ] {
            assert_eq!(topic(text), None, "{text}");
        }
    }

    #[test]
    fn money_comes_first() {
        assert_eq!(
            topic("pay with the password manager"),
            Some(HardTopic::Money)
        );
    }
}
