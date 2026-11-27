//! Integration tests: the client against a small HTTP server on 127.0.0.1 that answers as the
//! reference server does. Never a real server.

// Every module is `cfg(test)` (always true here) so clippy treats its helpers as test code,
// where the workspace lets them unwrap and panic.
#[cfg(test)]
mod agent_provider;
#[cfg(test)]
mod approvals;
#[cfg(test)]
mod browser;
#[cfg(test)]
mod composer;
#[cfg(test)]
mod endpoints;
#[cfg(test)]
mod health;
#[cfg(test)]
mod live;
#[cfg(test)]
mod pairing;
#[cfg(test)]
mod party;
#[cfg(test)]
mod releases;
#[cfg(test)]
mod renewal;
#[cfg(test)]
mod runner;
#[cfg(test)]
mod settings;
#[cfg(test)]
mod setup;
#[cfg(test)]
mod support;
#[cfg(test)]
mod uploads;
#[cfg(test)]
mod working;
