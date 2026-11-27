//! The code flow's pieces for the web authentication session: PKCE with `S256` over a 32-byte
//! verifier (RFC 7636), a random `state`, the authorization URL, and the callback read back,
//! its `state` checked before its code is used.

use data_encoding::BASE64URL_NOPAD;
use sha2::{Digest as _, Sha256};

use super::oidc::{CALLBACK, SCOPES};

/// One sign-in's secrets: the PKCE verifier and the `state`.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CodeFlow {
    verifier: String,
    state: String,
}

impl std::fmt::Debug for CodeFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodeFlow(<redacted>)")
    }
}

/// Why a callback doesn't give a code.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CallbackError {
    /// It isn't the callback, or its `state` isn't this sign-in's.
    #[error("the sign-in came back for another request")]
    NotOurs,
    /// The provider sent an error (the person cancelled, say).
    #[error("the account's provider refused the sign-in ({0})")]
    Refused(String),
    /// It has no code.
    #[error("the sign-in came back without a code")]
    NoCode,
}

impl CodeFlow {
    /// A new sign-in's verifier and state, from the system's randomness.
    ///
    /// # Errors
    ///
    /// The system's random source failed.
    pub(crate) fn new() -> Result<Self, getrandom::Error> {
        let mut verifier = [0_u8; 32];
        let mut state = [0_u8; 16];
        getrandom::getrandom(&mut verifier)?;
        getrandom::getrandom(&mut state)?;
        Ok(Self::from_bytes(&verifier, &state))
    }

    /// The flow for these random bytes.
    fn from_bytes(verifier: &[u8; 32], state: &[u8; 16]) -> Self {
        Self {
            verifier: BASE64URL_NOPAD.encode(verifier),
            state: BASE64URL_NOPAD.encode(state),
        }
    }

    /// The verifier, for the code's exchange.
    #[must_use]
    pub(crate) fn verifier(&self) -> &str {
        &self.verifier
    }

    /// The challenge: the verifier's SHA-256, base64url without padding.
    #[must_use]
    fn challenge(&self) -> String {
        BASE64URL_NOPAD.encode(&Sha256::digest(self.verifier.as_bytes()))
    }

    /// The page the session opens: `endpoint` with this sign-in's request.
    #[must_use]
    pub(crate) fn authorize_url(&self, endpoint: &str, client_id: &str) -> String {
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", CALLBACK)
            .append_pair("scope", SCOPES)
            .append_pair("state", &self.state)
            .append_pair("code_challenge", &self.challenge())
            .append_pair("code_challenge_method", "S256")
            .finish();
        let joint = if endpoint.contains('?') { '&' } else { '?' };
        format!("{endpoint}{joint}{query}")
    }

    /// The code in `callback`, the URL the session came back with.
    ///
    /// # Errors
    ///
    /// [`CallbackError`] when it isn't this sign-in's callback, or carries no code.
    pub(crate) fn code_from(&self, callback: &str) -> Result<String, CallbackError> {
        let query = callback
            .strip_prefix(CALLBACK)
            .and_then(|rest| rest.strip_prefix('?'))
            .ok_or(CallbackError::NotOurs)?;
        let query = query.split('#').next().unwrap_or(query);
        let (mut state, mut code, mut error) = (None, None, None);
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match &*key {
                "state" => state = Some(value.into_owned()),
                "code" => code = Some(value.into_owned()),
                "error" => error = Some(value.into_owned()),
                _ => {}
            }
        }
        if state.as_deref() != Some(self.state.as_str()) {
            return Err(CallbackError::NotOurs);
        }
        if let Some(error) = error {
            return Err(CallbackError::Refused(error));
        }
        code.filter(|code| !code.is_empty())
            .ok_or(CallbackError::NoCode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_is_s256_of_a_32_byte_verifier() {
        // RFC 7636, appendix B.
        let verifier = [
            116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212,
            37, 77, 105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
        ];
        let flow = CodeFlow::from_bytes(&verifier, &[0; 16]);
        assert_eq!(
            flow.verifier(),
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        );
        assert_eq!(
            flow.challenge(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let fresh = CodeFlow::new().unwrap();
        assert_eq!(fresh.verifier().len(), 43);
        assert_ne!(fresh, CodeFlow::new().unwrap());
    }

    #[test]
    fn the_authorization_url_asks_for_the_code_with_the_challenge() {
        let flow = CodeFlow::from_bytes(&[1; 32], &[2; 16]);
        let url = flow.authorize_url("https://accounts.example/oauth/authorize", "client");
        let (base, query) = url.split_once('?').unwrap();
        assert_eq!(base, "https://accounts.example/oauth/authorize");
        let pairs: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let get = |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("response_type"), Some("code"));
        assert_eq!(
            get("redirect_uri"),
            Some("https://emberhut.com/oauth/callback")
        );
        assert_eq!(get("scope"), Some("openid offline_access"));
        assert_eq!(get("code_challenge_method"), Some("S256"));
        assert_eq!(get("code_challenge"), Some(flow.challenge().as_str()));
        assert_eq!(get("state"), Some(flow.state.as_str()));
    }

    #[test]
    fn the_callback_gives_its_code_only_with_this_sign_ins_state() {
        let flow = CodeFlow::from_bytes(&[1; 32], &[2; 16]);
        let state = flow.state.clone();
        assert_eq!(
            flow.code_from(&format!("{CALLBACK}?code=abc&state={state}")),
            Ok("abc".to_owned())
        );
        assert_eq!(
            flow.code_from(&format!("{CALLBACK}?code=abc&state=other")),
            Err(CallbackError::NotOurs)
        );
        assert_eq!(
            flow.code_from(&format!("https://evil.example/cb?code=abc&state={state}")),
            Err(CallbackError::NotOurs)
        );
        assert_eq!(
            flow.code_from(&format!("{CALLBACK}?error=access_denied&state={state}")),
            Err(CallbackError::Refused("access_denied".into()))
        );
        assert_eq!(
            flow.code_from(&format!("{CALLBACK}?state={state}")),
            Err(CallbackError::NoCode)
        );
    }

    #[test]
    fn the_secrets_never_show_in_debug() {
        assert_eq!(
            format!("{:?}", CodeFlow::from_bytes(&[1; 32], &[2; 16])),
            "CodeFlow(<redacted>)"
        );
    }
}
