//! A machine's name, as the route list, the relay and the directory show it, and as the link
//! and relay statements sign it.

use std::fmt;
use std::str::FromStr;

/// A machine's name: 1 to 60 Unicode scalar values, with no white space at either end and no
/// control characters (Unicode's `Cc`) or bidirectional controls (U+061C, U+200E, U+200F,
/// U+202A to U+202E, U+2066 to U+2069) anywhere, so a name can't hide or reorder text.
///
/// White space is Unicode's `White_Space` property (U+0009 to U+000D, U+0020, U+0085, U+00A0,
/// U+1680, U+2000 to U+200A, U+2028, U+2029, U+202F, U+205F, U+3000). U+FEFF is not white
/// space. The sender makes a name with [`MachineName::cut`], which trims and cuts, before it
/// signs; a receiver reads one with `FromStr`, which refuses anything else, so a signed name
/// has one form.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MachineName(String);

/// Why text isn't a machine name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum MachineNameError {
    /// Nothing is left once trimmed.
    #[error("the machine's name is empty")]
    Empty,
    /// More than 60 characters (Unicode scalar values).
    #[error("the machine's name is longer than 60 characters")]
    TooLong,
    /// White space at either end.
    #[error("the machine's name has white space at an end")]
    NotTrimmed,
    /// A control character or a bidirectional control.
    #[error("the machine's name has a control character")]
    Control,
}

/// Whether `c` may not be in a name: a control character (`Cc`) or a bidirectional control.
fn is_refused(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

impl MachineName {
    /// The longest name, in Unicode scalar values.
    pub const MAX_CHARS: usize = 60;

    /// The name a sender makes from `text`: trimmed of white space, cut after 60 characters,
    /// and trimmed again at the end.
    ///
    /// # Errors
    ///
    /// [`MachineNameError::Empty`] when nothing is left, and [`MachineNameError::Control`] for
    /// a control character left once trimmed.
    pub fn cut(text: &str) -> Result<Self, MachineNameError> {
        let trimmed = text.trim();
        let end = trimmed
            .char_indices()
            .nth(Self::MAX_CHARS)
            .map_or(trimmed.len(), |(at, _)| at);
        let name = trimmed[..end].trim_end();
        if name.is_empty() {
            return Err(MachineNameError::Empty);
        }
        if name.chars().any(is_refused) {
            return Err(MachineNameError::Control);
        }
        Ok(Self(name.to_owned()))
    }

    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for MachineName {
    type Err = MachineNameError;

    /// Reads a name as a receiver: already trimmed and at most 60 characters, never cut.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.is_empty() {
            return Err(MachineNameError::Empty);
        }
        if text.trim() != text {
            return Err(MachineNameError::NotTrimmed);
        }
        if text.chars().count() > Self::MAX_CHARS {
            return Err(MachineNameError::TooLong);
        }
        if text.chars().any(is_refused) {
            return Err(MachineNameError::Control);
        }
        Ok(Self(text.to_owned()))
    }
}

impl fmt::Display for MachineName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for MachineName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MachineName({:?})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn the_sender_trims_and_cuts() {
        assert_eq!(
            MachineName::cut("  Main computer \n").unwrap().as_str(),
            "Main computer"
        );
        let long = "é".repeat(61);
        assert_eq!(MachineName::cut(&long).unwrap().as_str(), "é".repeat(60));
        let emoji = "🖥".repeat(61);
        assert_eq!(
            MachineName::cut(&emoji).unwrap().as_str().chars().count(),
            60
        );
        let gap = format!("{} x", "a".repeat(59));
        assert_eq!(
            MachineName::cut(&gap).unwrap().as_str(),
            "a".repeat(59),
            "trimmed after the cut"
        );
        assert_eq!(
            MachineName::cut(" \u{3000}\u{85} "),
            Err(MachineNameError::Empty)
        );
        assert_eq!(
            MachineName::cut("\u{feff}Main\u{feff}").unwrap().as_str(),
            "\u{feff}Main\u{feff}",
            "U+FEFF is not white space"
        );
    }

    #[test]
    fn the_receiver_refuses_rather_than_cuts() {
        assert_eq!(
            "Main computer".parse::<MachineName>().unwrap().to_string(),
            "Main computer"
        );
        assert_eq!(
            "🖥"
                .repeat(60)
                .parse::<MachineName>()
                .map(|n| n.as_str().len()),
            Ok(240)
        );
        assert_eq!(
            "é".repeat(61).parse::<MachineName>(),
            Err(MachineNameError::TooLong)
        );
        assert_eq!(
            " Main".parse::<MachineName>(),
            Err(MachineNameError::NotTrimmed)
        );
        assert_eq!(
            "Main\u{85}".parse::<MachineName>(),
            Err(MachineNameError::NotTrimmed)
        );
        assert_eq!("".parse::<MachineName>(), Err(MachineNameError::Empty));
    }

    #[test]
    fn control_characters_are_refused() {
        for name in [
            "Main\tbox",
            "Main\u{7}",
            "\u{202e}Main",
            "a\u{2066}b",
            "a\u{61c}b",
            "a\u{200e}b",
        ] {
            assert_eq!(
                name.parse::<MachineName>(),
                Err(MachineNameError::Control),
                "{name:?}"
            );
            assert_eq!(
                MachineName::cut(name),
                Err(MachineNameError::Control),
                "{name:?}"
            );
        }
        assert_eq!(
            MachineName::cut("\tMain\n").unwrap().as_str(),
            "Main",
            "trimmed at the ends"
        );
    }

    proptest! {
        #[test]
        fn what_the_sender_cuts_the_receiver_reads(text in "\\PC{0,80}") {
            if let Ok(name) = MachineName::cut(&text) {
                prop_assert_eq!(name.as_str().parse::<MachineName>(), Ok(name));
            }
        }
    }
}
