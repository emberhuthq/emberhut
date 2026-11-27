//! Emberhut Hands: the helper on the owner's Mac that gives the assistant hands there (ADR 0024).
//!
//! It is a small HTTP service the server calls (`POST /snapshot`, `/press`, `/screen/act`…, with
//! a bearer token), and the only program that holds the Mac's Accessibility and Screen
//! Recording grants for the assistant. Two ways of working:
//!
//! - **One app, in the background**: its window read as elements with refs, pressed, set and
//!   its menus picked through Accessibility, keys sent to that app only, a picture of that
//!   window only. The owner's mouse never moves and the app in front stays in front. The helper
//!   holds one app at a time ([`hands::session`]).
//! - **The whole screen**, with the owner's OK: a bar on every Space says the assistant is using
//!   the screen, with Stop; the owner's own touch of the mouse or keyboard also ends it
//!   ([`hands::screen`]).
//!
//! It also records the owner's calls when asked ([`record`]) and keeps the browser extension's
//! folder up to date ([`extension`]).
//!
//! The rules (never a password, never Chrome, the apps it never uses) are enforced here as well
//! as on the server ([`rules`]). Everything that touches the system goes through the
//! [`desktop::Desktop`] trait, so the rules and the HTTP layer are tested on any platform; the
//! real implementation is in `mac`, built on macOS only.

pub mod config;
pub mod desktop;
pub mod extension;
pub mod fail;
pub mod hands;
pub mod http;
pub mod keys;
pub mod network;
pub mod record;
pub mod rules;
pub mod text;
pub mod token;

#[cfg(target_os = "macos")]
pub mod mac;

/// `error` with its causes, one after the other, for the log.
#[must_use]
pub fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

#[cfg(test)]
mod testing;
