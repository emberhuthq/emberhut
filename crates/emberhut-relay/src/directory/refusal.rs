//! The directory's refusals: `{ "error": "<a sentence to show>", "code": "<code>" }` with
//! directory.md's statuses, `"signIn": true` and `WWW-Authenticate` on 401, and `Retry-After`
//! on 429 and 503.

use std::time::Duration;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use emberhut_api::directory::{DirectoryError, DirectoryErrorCode};

use super::account::AccountRefusal;
use super::json;
use super::store::WriteError;

/// How long a client waits after the directory's own store failed.
const STORE_RETRY: Duration = Duration::from_secs(5);

/// A refusal, ready to answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Refusal {
    status: StatusCode,
    code: DirectoryErrorCode,
    sentence: &'static str,
    retry_after: Option<Duration>,
}

impl Refusal {
    const fn new(status: StatusCode, code: DirectoryErrorCode, sentence: &'static str) -> Self {
        Self {
            status,
            code,
            sentence,
            retry_after: None,
        }
    }

    /// 400 `bad-request`.
    pub(super) const fn bad_request(sentence: &'static str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            DirectoryErrorCode::BadRequest,
            sentence,
        )
    }

    /// 400 `stale`.
    pub(super) const fn stale() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            DirectoryErrorCode::Stale,
            "The statement is too old, or the machine's clock is off. Make a new one.",
        )
    }

    /// 400 `replayed`.
    pub(super) const fn replayed() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            DirectoryErrorCode::Replayed,
            "This statement was already used. Make a new one.",
        )
    }

    /// 401 `sign-in`.
    pub(super) const fn sign_in() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            DirectoryErrorCode::SignIn,
            "Sign in to your account again.",
        )
    }

    /// 403 `bad-signature`.
    pub(super) const fn bad_signature() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            DirectoryErrorCode::BadSignature,
            "The machine's signature doesn't verify.",
        )
    }

    /// 403 `not-yours`.
    pub(super) const fn not_yours() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            DirectoryErrorCode::NotYours,
            "This statement is for another account.",
        )
    }

    /// 404 `not-found`.
    pub(super) const fn not_found(sentence: &'static str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            DirectoryErrorCode::NotFound,
            sentence,
        )
    }

    /// 405, with code `bad-request`: the path is served, the method isn't.
    pub(super) const fn method_not_allowed() -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            DirectoryErrorCode::BadRequest,
            "The directory doesn't take that method here.",
        )
    }

    /// 409 `account-full`.
    pub(super) const fn account_full() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            DirectoryErrorCode::AccountFull,
            "This account has all the machines it can hold. Unlink one first.",
        )
    }

    /// 409 `linked-elsewhere`.
    pub(super) const fn linked_elsewhere() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            DirectoryErrorCode::LinkedElsewhere,
            "This machine is linked to another account. Unlink it there first.",
        )
    }

    /// 429 `limit`, to try again after `retry_after`.
    pub(super) fn limit(retry_after: Duration) -> Self {
        let mut refusal = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            DirectoryErrorCode::Limit,
            "Too many requests. Try again in a moment.",
        );
        refusal.retry_after = Some(retry_after);
        refusal
    }

    /// 503 `unavailable`, to try again after `retry_after`.
    pub(super) fn unavailable(retry_after: Duration) -> Self {
        let mut refusal = Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            DirectoryErrorCode::Unavailable,
            "The account service can't be reached right now. Try again in a moment.",
        );
        refusal.retry_after = Some(retry_after);
        refusal
    }

    /// 503 `unavailable` for a failure of the directory's own: logged, never a reason to sign
    /// out.
    pub(super) fn store(error: &dyn std::error::Error) -> Self {
        tracing::error!(error = %error, cause = ?error.source(), "the directory's store failed");
        Self::unavailable(STORE_RETRY)
    }
}

impl From<AccountRefusal> for Refusal {
    fn from(refusal: AccountRefusal) -> Self {
        match refusal {
            AccountRefusal::SignIn => Self::sign_in(),
            AccountRefusal::Unavailable { retry_after } => Self::unavailable(retry_after),
        }
    }
}

impl From<WriteError> for Refusal {
    fn from(error: WriteError) -> Self {
        match error {
            WriteError::Replayed => Self::replayed(),
            WriteError::Stale => Self::stale(),
            WriteError::LinkedElsewhere => Self::linked_elsewhere(),
            WriteError::AccountFull => Self::account_full(),
            WriteError::NotLinked => Self::not_found("This machine isn't linked to an account."),
            WriteError::Store(error) => Self::store(&error),
        }
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let sign_in = self.status == StatusCode::UNAUTHORIZED;
        let body = DirectoryError {
            error: self.sentence.to_owned(),
            code: self.code,
            sign_in,
        };
        let mut response = (self.status, json(&body)).into_response();
        let headers = response.headers_mut();
        if sign_in {
            headers.insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if let Some(retry_after) = self.retry_after {
            headers.insert(header::RETRY_AFTER, retry_after_seconds(retry_after));
        }
        response
    }
}

/// `Retry-After` in whole seconds, rounded up, at least 1.
fn retry_after_seconds(wait: Duration) -> HeaderValue {
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    HeaderValue::from(seconds.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_rounds_up_to_whole_seconds() {
        assert_eq!(retry_after_seconds(Duration::ZERO), "1");
        assert_eq!(retry_after_seconds(Duration::from_millis(1_001)), "2");
        assert_eq!(retry_after_seconds(Duration::from_secs(60)), "60");
    }
}
