//! Why an answer or a read didn't go through, said plainly, and whether what is on screen
//! should be read again.
use crate::backend::Error;
use crate::brand::APP_NAME;

/// Why a decision or a withdrawal didn't go through, said plainly, and whether the request
/// should be read again (it changed, was decided, or is gone).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    /// What the user reads.
    pub(crate) message: String,
    /// Read the request again: what is on screen is no longer what the server has.
    pub(crate) refresh: bool,
}

/// [`Failure`] for `error`.
#[must_use]
pub(crate) fn failure(error: &Error) -> Failure {
    let (message, refresh) = match error {
        Error::Refused {
            status: Some(409), ..
        } => (
            "This request changed or was already decided, so nothing was done. Here it is as \
             it is now."
                .to_owned(),
            true,
        ),
        Error::Refused {
            status: Some(404), ..
        } => ("This request is gone.".to_owned(), true),
        // The request may have gone out and its answer been lost: say so, and read it again.
        Error::Unreachable(_) => (
            format!(
                "Couldn't reach {APP_NAME}: it may or may not have gone through. Here it is as \
                 the server has it."
            ),
            true,
        ),
        other => (
            format!(
                "Not done: {}",
                other
                    .server_message()
                    .map_or_else(|| other.to_string(), str::to_owned)
            ),
            true,
        ),
    };
    Failure { message, refresh }
}

/// Why one request couldn't be read, worded for the user (a 404 is said on its own).
#[must_use]
pub(crate) fn read_one_problem(error: &Error) -> String {
    match error {
        Error::Unreachable(_) => format!("Can't reach {APP_NAME}"),
        other => other
            .server_message()
            .map_or_else(|| "Couldn't read this request".to_owned(), str::to_owned),
    }
}

/// Why the approvals couldn't be read, worded for the user.
#[must_use]
pub(crate) fn read_problem(error: &Error) -> String {
    match error {
        Error::Unreachable(_) => format!("Can't reach {APP_NAME}"),
        Error::Refused {
            status: Some(404), ..
        } => format!("{APP_NAME}'s server is on older code: approvals come with its next update"),
        other => other
            .server_message()
            .map_or_else(|| "Couldn't read the approvals".to_owned(), str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_decision_says_why_and_whether_to_read_it_again() {
        let changed = failure(&Error::Refused {
            status: Some(409),
            message: Some("payload hash mismatch".to_owned()),
        });
        assert!(changed.refresh);
        assert!(
            changed
                .message
                .starts_with("This request changed or was already decided")
        );
        assert_eq!(
            failure(&Error::Refused {
                status: Some(404),
                message: None
            }),
            Failure {
                message: "This request is gone.".to_owned(),
                refresh: true
            }
        );
        let offline = failure(&Error::Unreachable("down".into()));
        assert!(offline.refresh, "it may have gone through: read it again");
        assert_eq!(
            offline.message,
            "Couldn't reach Emberhut: it may or may not have gone through. Here it is as the server \
             has it."
        );
        assert_eq!(
            read_one_problem(&Error::Unreachable("down".into())),
            "Can't reach Emberhut"
        );
        let other = failure(&Error::Refused {
            status: Some(400),
            message: Some("Too long".to_owned()),
        });
        assert_eq!(other.message, "Not done: Too long");
    }

    #[test]
    fn a_list_from_an_older_server_says_so() {
        assert_eq!(
            read_problem(&Error::Refused {
                status: Some(404),
                message: None
            }),
            "Emberhut's server is on older code: approvals come with its next update"
        );
    }
}
