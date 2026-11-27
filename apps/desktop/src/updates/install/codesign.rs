//! The code-signing checks on the new bundle: a valid, complete signature
//! (`codesign --verify --deep --strict`), made by the running app's own Developer ID team and
//! notarized (a requirement built at run time from the running app's signature, so no team id
//! is written in the repository), and Gatekeeper's own answer as an extra check.

use std::path::Path;

use super::tools::{self, ToolError};
use crate::brand::APP_ID;

/// The `codesign` tool.
const CODESIGN: &str = "/usr/bin/codesign";
/// The `spctl` tool.
const SPCTL: &str = "/usr/sbin/spctl";

/// A Developer ID team: ten capital letters and digits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Team(String);

impl Team {
    /// `text` as a team id, if it is one.
    #[must_use]
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let well_formed = text.len() == 10
            && text
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit());
        well_formed.then(|| Self(text.to_owned()))
    }

    /// The team named in `codesign -dv`'s report (`TeamIdentifier=…`); `None` for an ad-hoc
    /// signature (`TeamIdentifier=not set`) or none.
    #[must_use]
    pub(crate) fn from_report(report: &str) -> Option<Self> {
        report
            .lines()
            .find_map(|line| line.trim().strip_prefix("TeamIdentifier="))
            .and_then(Self::parse)
    }
}

/// The Developer ID team that signed the bundle at `app`; `None` when it is ad hoc signed or
/// not signed (such an app never updates itself).
///
/// # Errors
///
/// `codesign` couldn't be run.
pub(crate) fn team_of(app: &Path) -> Result<Option<Team>, ToolError> {
    match tools::run(
        CODESIGN,
        ["-dv".as_ref(), "--verbose=2".as_ref(), app.as_os_str()],
    ) {
        // codesign reports on stderr.
        Ok(said) => Ok(Team::from_report(&said.err)),
        Err(ToolError::Failed { said, .. }) => {
            log::info!("the running app isn't signed: {said}");
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// The requirement a new bundle must meet: Apple's anchor, this app's id, a Developer ID
/// chain, `team`'s leaf, and notarized.
#[must_use]
pub(crate) fn requirement(team: &Team) -> String {
    format!(
        "anchor apple generic and identifier \"{APP_ID}\" \
         and certificate 1[field.1.2.840.113635.100.6.2.6] \
         and certificate leaf[field.1.2.840.113635.100.6.1.13] \
         and certificate leaf[subject.OU] = \"{team}\" and notarized",
        team = team.0
    )
}

/// Checks the bundle at `app`: its signature whole and strict, `team`'s requirement met, and
/// Gatekeeper's assessment.
///
/// # Errors
///
/// The first check that failed.
pub(crate) fn check(app: &Path, team: &Team) -> Result<(), ToolError> {
    let app = app.as_os_str();
    tools::run(
        CODESIGN,
        [
            "--verify".as_ref(),
            "--deep".as_ref(),
            "--strict".as_ref(),
            app,
        ],
    )?;
    let requirement = format!("={}", requirement(team));
    tools::run(
        CODESIGN,
        [
            "--verify".as_ref(),
            "--test-requirement".as_ref(),
            requirement.as_ref(),
            app,
        ],
    )?;
    // An extra check only: it answers yes with Gatekeeper off, so notarization is pinned by
    // the requirement above.
    tools::run(
        SPCTL,
        [
            "--assess".as_ref(),
            "--type".as_ref(),
            "execute".as_ref(),
            app,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_team_comes_from_codesigns_report_and_ad_hoc_has_none() {
        let report = "Executable=/Applications/Emberhut.app/Contents/MacOS/emberhut-desktop\n\
                      Identifier=com.emberhut.desktop\nTeamIdentifier=AB12CD34EF\n";
        assert_eq!(Team::from_report(report), Team::parse("AB12CD34EF"));
        assert_eq!(Team::from_report("TeamIdentifier=not set\n"), None);
        assert_eq!(Team::from_report("Identifier=x\n"), None);
        for bad in ["ab12cd34ef", "AB12CD34E", "AB12CD34EF1", "AB12CD34\"F", ""] {
            assert_eq!(Team::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_requirement_pins_the_id_the_chain_the_team_and_notarization() {
        let team = Team::parse("AB12CD34EF").unwrap();
        let requirement = requirement(&team);
        assert!(
            requirement.starts_with("anchor apple generic and identifier \"com.emberhut.desktop\"")
        );
        assert!(requirement.contains("certificate 1[field.1.2.840.113635.100.6.2.6]"));
        assert!(requirement.contains("certificate leaf[field.1.2.840.113635.100.6.1.13]"));
        assert!(requirement.contains("certificate leaf[subject.OU] = \"AB12CD34EF\""));
        assert!(requirement.ends_with("and notarized"));
    }
}
