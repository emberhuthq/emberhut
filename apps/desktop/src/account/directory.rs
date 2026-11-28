//! The account directory's requests (`docs/directory.md`): the account's machines, a machine
//! linked, a machine unlinked, and its refusals read by their `code`, never by their sentence.

use std::time::Duration;

use emberhut_api::connect::SignedLink;
use emberhut_api::directory::{
    DirectoryError, DirectoryErrorCode, LinkedMachine, MACHINES, MachineList, machine_path,
};
use emberhut_identity::MachineId;
use serde::de::DeserializeOwned;

use super::http::{Request, Response};

/// `GET /v1/machines` at the directory whose origin is `base`.
#[must_use]
pub(crate) fn list_request(base: &str) -> Request {
    Request::get(format!("{base}{MACHINES}"))
}

/// `POST /v1/machines` with the machine's signed link, as the server gave it.
///
/// # Errors
///
/// The link couldn't be written as JSON (its types always can).
pub(crate) fn link_request(base: &str, link: &SignedLink) -> Result<Request, serde_json::Error> {
    Ok(Request::json(
        format!("{base}{MACHINES}"),
        serde_json::to_vec(link)?,
    ))
}

/// `DELETE /v1/machines/{id}`.
#[must_use]
pub(crate) fn unlink_request(base: &str, id: &MachineId) -> Request {
    Request::delete(format!("{base}{}", machine_path(id)))
}

/// Why the directory didn't do what was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// 401 `sign-in`: the token is missing or failed a check.
    SignIn,
    /// 429 `limit` or 503 `unavailable`: try again later (never a reason to sign out).
    Later(Option<Duration>),
    /// Any other refusal, by its code, with the words to show.
    Refused {
        /// What went wrong, for the app to act on.
        code: DirectoryErrorCode,
        /// The words for it.
        words: String,
    },
    /// An answer that isn't the directory's: another status, or a body that doesn't read.
    Unreadable(u16),
}

/// The machines in a 200 answer to [`list_request`].
///
/// # Errors
///
/// The [`Refusal`] in any other answer.
pub(crate) fn read_list(response: &Response) -> Result<Vec<LinkedMachine>, Refusal> {
    read::<MachineList>(response, 200).map(|list| list.machines)
}

/// The machine in a 200 answer to [`link_request`].
///
/// # Errors
///
/// The [`Refusal`] in any other answer.
pub(crate) fn read_linked(response: &Response) -> Result<LinkedMachine, Refusal> {
    read(response, 200)
}

/// A 204 answer to [`unlink_request`].
///
/// # Errors
///
/// The [`Refusal`] in any other answer.
pub(crate) fn read_unlinked(response: &Response) -> Result<(), Refusal> {
    if response.status == 204 || response.status == 200 {
        Ok(())
    } else {
        Err(refusal(response))
    }
}

/// The `T` in an answer of status `ok`, or the refusal in any other.
fn read<T: DeserializeOwned>(response: &Response, ok: u16) -> Result<T, Refusal> {
    if response.status != ok {
        return Err(refusal(response));
    }
    serde_json::from_slice(&response.body).map_err(|_| Refusal::Unreadable(response.status))
}

/// The refusal in a non-success answer. 401, 429 and 503 are read by their status when the
/// body isn't the directory's (a proxy's own page), so an outage never signs the account out
/// and a lost token always asks for sign-in.
fn refusal(response: &Response) -> Refusal {
    let body = serde_json::from_slice::<DirectoryError>(&response.body).ok();
    match (response.status, body) {
        (401, _) => Refusal::SignIn,
        (429 | 503, _) => Refusal::Later(response.retry_after),
        (_, Some(error)) => Refusal::Refused {
            words: words(&error.code).map_or(error.error, str::to_owned),
            code: error.code,
        },
        (status, None) => Refusal::Unreadable(status),
    }
}

/// The app's own words for the refusals it explains; the directory's sentence for the rest.
#[must_use]
fn words(code: &DirectoryErrorCode) -> Option<&'static str> {
    Some(match code {
        DirectoryErrorCode::AccountFull => {
            "This account has all the huts it can hold. Unlink one first."
        }
        DirectoryErrorCode::LinkedElsewhere => {
            "This hut is linked to another account. Unlink it there first."
        }
        DirectoryErrorCode::Stale => {
            "The link is too old: this computer's clock is more than 5 minutes off, or the \
             hut was linked or unlinked after it was signed. Check the clock and try again."
        }
        DirectoryErrorCode::Replayed => "That link was already used. Try again.",
        DirectoryErrorCode::NotFound => "That hut isn't in this account any more.",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn answer(status: u16, body: &serde_json::Value) -> Response {
        Response {
            status,
            retry_after: None,
            body: body.to_string().into_bytes(),
        }
    }

    fn error(status: u16, code: &str) -> Response {
        answer(
            status,
            &json!({"error": "The directory's own sentence.", "code": code}),
        )
    }

    #[test]
    fn the_paths_are_the_directorys() {
        let id = emberhut_identity::SecretKey::from_bytes(&[3; 32]).machine_id();
        assert_eq!(
            list_request("https://relay.example").url,
            "https://relay.example/v1/machines"
        );
        assert_eq!(
            unlink_request("https://relay.example", &id).url,
            format!("https://relay.example/v1/machines/{id}")
        );
    }

    #[test]
    fn the_codes_that_ask_something_of_the_person_read_in_the_apps_words() {
        let Err(Refusal::Refused { code, words }) = read_linked(&error(409, "account-full")) else {
            panic!("a refusal")
        };
        assert_eq!(code, DirectoryErrorCode::AccountFull);
        assert_eq!(
            words,
            "This account has all the huts it can hold. Unlink one first."
        );
        let Err(Refusal::Refused { code, words }) = read_linked(&error(409, "linked-elsewhere"))
        else {
            panic!("a refusal")
        };
        assert_eq!(code, DirectoryErrorCode::LinkedElsewhere);
        assert_eq!(
            words,
            "This hut is linked to another account. Unlink it there first."
        );
        let Err(Refusal::Refused { words, .. }) = read_linked(&error(403, "bad-signature")) else {
            panic!("a refusal")
        };
        assert_eq!(words, "The directory's own sentence.");
    }

    #[test]
    fn sign_in_and_later_are_told_apart() {
        assert_eq!(read_list(&error(401, "sign-in")), Err(Refusal::SignIn));
        assert_eq!(read_list(&answer(401, &json!("x"))), Err(Refusal::SignIn));
        let mut busy = error(503, "unavailable");
        busy.retry_after = Some(Duration::from_secs(30));
        assert_eq!(
            read_list(&busy),
            Err(Refusal::Later(Some(Duration::from_secs(30))))
        );
        assert_eq!(read_list(&error(429, "limit")), Err(Refusal::Later(None)));
        assert_eq!(
            read_list(&answer(502, &json!("bad gateway"))),
            Err(Refusal::Unreadable(502))
        );
    }

    #[test]
    fn a_list_and_an_unlink_read() {
        let key = emberhut_identity::SecretKey::from_bytes(&[4; 32]);
        let list = json!({"machines": [{
            "id": key.machine_id().to_string(),
            "publicKey": key.public_key().to_string(),
            "name": "Main computer",
            "relayUrl": "wss://relay.example/v1/sealed/x",
            "connected": false,
            "lastSeen": null,
            "linkedAt": 1_795_000_000_000_u64,
        }]});
        let machines = read_list(&answer(200, &list)).unwrap();
        assert_eq!(machines[0].name.as_str(), "Main computer");
        assert_eq!(machines[0].last_seen, None);
        assert_eq!(read_unlinked(&answer(204, &json!(null))), Ok(()));
        assert!(matches!(
            read_unlinked(&error(404, "not-found")),
            Err(Refusal::Refused {
                code: DirectoryErrorCode::NotFound,
                ..
            })
        ));
    }
}
