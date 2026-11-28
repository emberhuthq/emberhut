//! Which releases a copy follows: Stable (final releases only) or Release candidates (every
//! release, so a final newer than the candidates wins too). A copy that is itself a candidate
//! follows candidates until told otherwise, so people trying candidates keep getting them; any
//! other copy follows Stable. Moving from candidates to Stable never goes back a build: the copy
//! waits for a final release newer than itself.

use serde::{Deserialize, Serialize};

use super::version::Running;

/// The releases a copy follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Channel {
    /// Final releases.
    Stable,
    /// Release candidates, and final releases.
    #[serde(rename = "rc")]
    Candidates,
}

impl Channel {
    /// The channel of a copy that hasn't chosen one: candidates for a candidate, else Stable.
    #[must_use]
    pub(crate) fn default_for(running: Running) -> Self {
        if running.is_candidate() {
            Self::Candidates
        } else {
            Self::Stable
        }
    }

    /// Its name, as Settings shows it.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Candidates => "Release candidates",
        }
    }

    /// Whether a release that is (or isn't) a candidate is on this channel.
    #[must_use]
    pub(crate) fn takes(self, candidate: bool) -> bool {
        self == Self::Candidates || !candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_candidate_follows_candidates_and_anything_else_stable() {
        let rc = Running::from_parts("0.1.0-rc.1", Some("10001"));
        let stable = Running::from_parts("0.1.0", Some("10099"));
        let dev = Running::from_parts("0.1.0", None);
        assert_eq!(Channel::default_for(rc), Channel::Candidates);
        assert_eq!(Channel::default_for(stable), Channel::Stable);
        assert_eq!(Channel::default_for(dev), Channel::Stable);
    }

    #[test]
    fn stable_takes_finals_and_candidates_take_both() {
        assert!(Channel::Stable.takes(false));
        assert!(!Channel::Stable.takes(true));
        assert!(Channel::Candidates.takes(true));
        assert!(Channel::Candidates.takes(false));
    }

    #[test]
    fn it_is_kept_by_a_short_name() {
        assert_eq!(
            serde_json::to_string(&Channel::Candidates).unwrap(),
            r#""rc""#
        );
        assert_eq!(
            serde_json::to_string(&Channel::Stable).unwrap(),
            r#""stable""#
        );
        assert_eq!(
            serde_json::from_str::<Channel>(r#""rc""#).unwrap(),
            Channel::Candidates
        );
    }
}
