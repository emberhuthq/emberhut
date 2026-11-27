//! The activity log: what the assistant and the server did on their own, one line each, for
//! the app's activity list.

use jiff::Timestamp;

use crate::LogLineId;

/// One line of the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// Its id; ids grow, so they order the log.
    pub id: LogLineId,
    /// When it was written.
    pub at: Timestamp,
    /// What was done, in words for a person.
    pub text: String,
}
