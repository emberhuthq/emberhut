//! Signing this device in with a one-time pairing code, typed or in a pasted pairing link.
//!
//! A link names the machine (its id) and its routes. This app talks to one server, so it pairs
//! only when that server is one of the link's routes, and keeps the session only when the
//! answer proves it comes from the machine the link names.

use std::sync::Arc;

use emberhut_identity::PairingLink;
use gpui_kit::AppContext as _;
use gpui_kit::Context;

use super::{Store, update_or_log};
use crate::account::switch::is_loopback_host;
use crate::backend::Error;
use crate::config::ServerUrl;

impl Store {
    /// Trades `code` for this device's own session; `done` hears how it went (the server's
    /// reason when it refused).
    pub(crate) fn pair(
        &mut self,
        code: String,
        cx: &mut Context<'_, Self>,
        done: impl FnOnce(Result<(), String>, &mut Context<'_, Self>) + 'static,
    ) {
        self.pair_with(move |backend| backend.pair(&code), cx, done);
    }

    /// Trades the code of a pasted pairing `link` for this device's own session, kept only
    /// when the answer comes from the machine the link names; `done` hears how it went.
    pub(crate) fn pair_link(
        &mut self,
        link: PairingLink,
        cx: &mut Context<'_, Self>,
        done: impl FnOnce(Result<(), String>, &mut Context<'_, Self>) + 'static,
    ) {
        if let Err(why) = routes_reach(&link, self.server()) {
            done(Err(why), cx);
            return;
        }
        self.pair_with(
            move |backend| backend.pair_with_machine(link.code(), link.id()),
            cx,
            done,
        );
    }

    /// Runs `pairing` on the backend, then tells `done` how it went.
    fn pair_with(
        &mut self,
        pairing: impl FnOnce(&dyn crate::backend::Backend) -> Result<(), Error> + Send + 'static,
        cx: &mut Context<'_, Self>,
        done: impl FnOnce(Result<(), String>, &mut Context<'_, Self>) + 'static,
    ) {
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        cx.spawn(async move |this, cx| {
            let paired = cx.background_spawn(async move { pairing(&*backend) }).await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                let outcome = paired.map_err(|error| match error.server_message() {
                    Some(reason) => reason.to_owned(),
                    None if error.is_unreachable() => format!("Can't reach the server: {error}"),
                    None => error.to_string(),
                });
                if outcome.is_ok() {
                    store.signed_in();
                }
                done(outcome, cx);
                if store.is_signed_in() {
                    store.setup_signed_in(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// Whether this app's `server` is one of `link`'s routes. The code is never sent to a server
/// the link doesn't name; a link without routes reaches its machine only through the relay,
/// which this app has no transport for yet, so it is refused.
///
/// # Errors
///
/// What to do instead, in words.
fn routes_reach(link: &PairingLink, server: &ServerUrl) -> Result<(), String> {
    if link.routes().is_empty() {
        return Err(
            "This link names no address for its machine; pairing through the relay comes in a \
             later version. Type the code on a device that talks to the machine instead."
                .to_owned(),
        );
    }
    let reaches =
        |route: &String| ServerUrl::parse(route).is_ok_and(|route| same_server(&route, server));
    if link.routes().iter().any(reaches) {
        return Ok(());
    }
    let first = link.routes().first().map_or("", String::as_str);
    Err(format!(
        "This link is for the machine at {first}, and this app talks to {server}. Set this \
         app's server to one of the link's addresses, or type the code."
    ))
}

/// Whether `one` and `other` are the same server: the same address, or both on this computer
/// with the same scheme and port (`localhost` and `127.0.0.1` are one place).
#[must_use]
pub(crate) fn same_server(one: &ServerUrl, other: &ServerUrl) -> bool {
    if one == other {
        return true;
    }
    match (place(one.as_str()), place(other.as_str())) {
        (Some((scheme, authority, port)), Some((their_scheme, their_authority, their_port))) => {
            scheme == their_scheme
                && port == their_port
                && is_loopback_host(authority)
                && is_loopback_host(their_authority)
        }
        _ => false,
    }
}

/// A server address's scheme, authority and port (the scheme's own when none is written).
fn place(address: &str) -> Option<(&str, &str, u16)> {
    let (scheme, rest) = address.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let after_host = authority
        .rsplit_once(']')
        .map_or(authority, |(_, port)| port);
    let port = match after_host.rsplit_once(':') {
        Some((_, port)) => port.parse().ok()?,
        None if scheme == "https" => 443,
        None => 80,
    };
    Some((scheme, authority, port))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use emberhut_identity::SecretKey;
    use gpui_kit::{Entity, TestAppContext};

    use super::*;
    use crate::backend::fake::{Call, FakeBackend};

    fn link(routes: &[&str]) -> PairingLink {
        let id = SecretKey::from_bytes(&[7; 32]).machine_id();
        PairingLink::new(
            id,
            "ABCD-EFGH",
            routes.iter().map(|&route| route.to_owned()).collect(),
        )
        .unwrap()
    }

    fn server(address: &str) -> ServerUrl {
        ServerUrl::parse(address).unwrap()
    }

    #[test]
    fn one_server_on_this_computer_has_one_place_whatever_its_name() {
        let same = |one: &str, other: &str| same_server(&server(one), &server(other));
        assert!(same("http://localhost:18990", "http://127.0.0.1:18990"));
        assert!(same("http://127.0.0.1:18990/", "http://[::1]:18990"));
        assert!(same(
            "https://ember.example.com",
            "https://ember.example.com/"
        ));
        assert!(!same("http://localhost:18990", "http://127.0.0.1:18995"));
        assert!(!same("http://localhost:18990", "https://127.0.0.1:18990"));
        assert!(!same("http://192.0.2.5:18990", "http://127.0.0.1:18990"));
    }

    #[test]
    fn a_link_pairs_only_with_a_server_it_names() {
        let here = server("http://localhost:8791");
        assert!(
            routes_reach(&link(&[]), &here).is_err_and(|why| why.contains("names no address")),
            "a routeless link's code goes nowhere"
        );
        assert!(
            routes_reach(
                &link(&["http://192.0.2.5:8791", "http://127.0.0.1:8791"]),
                &here
            )
            .is_ok()
        );
        assert!(routes_reach(&link(&["http://[::1]:8791/"]), &here).is_ok());
        let elsewhere = routes_reach(&link(&["https://emberhut.example.com"]), &here).unwrap_err();
        assert!(
            elsewhere.contains("https://emberhut.example.com"),
            "{elsewhere}"
        );
        assert!(routes_reach(&link(&["http://127.0.0.1:9000"]), &here).is_err());
        assert!(
            routes_reach(
                &link(&["https://emberhut.example.com/"]),
                &server("https://emberhut.example.com")
            )
            .is_ok()
        );
    }

    fn store(backend: &Arc<FakeBackend>, cx: &mut TestAppContext) -> Entity<Store> {
        let shared = Arc::clone(backend) as Arc<dyn crate::backend::Backend>;
        cx.new(|_| Store::for_tests(shared))
    }

    /// Pairs from `link` on `store`, and what `done` heard.
    fn paired(
        store: &Entity<Store>,
        link: PairingLink,
        cx: &mut TestAppContext,
    ) -> Option<Result<(), String>> {
        let heard = Arc::new(std::sync::Mutex::new(None));
        let keep = Arc::clone(&heard);
        store.update(cx, |store, cx| {
            store.pair_link(link, cx, move |outcome, _| {
                *keep.lock().unwrap() = Some(outcome);
            });
        });
        cx.run_until_parked();
        heard.lock().unwrap().take()
    }

    #[gpui_kit::test]
    fn a_pasted_link_pairs_with_its_code_for_its_machine(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]).at("http://127.0.0.1:8791"));
        let store = store(&backend, cx);
        let link = link(&["http://127.0.0.1:8791"]);
        let id = *link.id();
        assert_eq!(paired(&store, link, cx), Some(Ok(())));
        assert_eq!(
            backend.calls(),
            [Call::PairWithMachine("ABCD-EFGH".into(), id)]
        );
    }

    #[gpui_kit::test]
    fn another_machines_answer_is_refused_in_words(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]).at("http://127.0.0.1:8791"));
        backend.account.push_pairing(Err(Error::WrongMachine));
        let store = store(&backend, cx);
        assert_eq!(
            paired(&store, link(&["http://127.0.0.1:8791"]), cx),
            Some(Err(
                "This server didn't prove it is the machine you meant.".to_owned()
            ))
        );
    }

    #[gpui_kit::test]
    fn the_code_never_goes_to_a_server_the_link_doesnt_name(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]).at("http://127.0.0.1:8791"));
        let store = store(&backend, cx);
        let heard = paired(&store, link(&["https://emberhut.example.com"]), cx);
        assert!(matches!(heard, Some(Err(_))), "{heard:?}");
        assert_eq!(backend.calls(), []);
    }

    #[gpui_kit::test]
    fn a_routeless_link_sends_its_code_nowhere(cx: &mut TestAppContext) {
        let backend = Arc::new(FakeBackend::new([]).at("http://127.0.0.1:8791"));
        let store = store(&backend, cx);
        let heard = paired(&store, link(&[]), cx);
        assert!(matches!(heard, Some(Err(_))), "{heard:?}");
        assert_eq!(backend.calls(), []);
    }
}
