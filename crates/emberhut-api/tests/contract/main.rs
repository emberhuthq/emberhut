//! Contract tests: the types against JSON shaped exactly as the wire carries it
//! (`tests/fixtures`, made-up data).

// Every module is `cfg(test)` (always true here) so clippy treats its helpers as test code,
// where the workspace lets them unwrap and panic.
#[cfg(test)]
mod agent_provider;
#[cfg(test)]
mod auth;
#[cfg(test)]
mod browser;
#[cfg(test)]
mod builds;
#[cfg(test)]
mod chat;
#[cfg(test)]
mod choices;
#[cfg(test)]
mod composer;
#[cfg(test)]
mod connect;
mod days;
#[cfg(test)]
mod errors;
#[cfg(test)]
mod health;
#[cfg(test)]
mod live;
#[cfg(test)]
mod mac;
#[cfg(test)]
mod machines;
#[cfg(test)]
mod mcp;
#[cfg(test)]
#[cfg(feature = "openapi")]
mod openapi;
#[cfg(test)]
mod party;
#[cfg(test)]
mod places;
#[cfg(test)]
mod push;
#[cfg(test)]
mod releases;
#[cfg(test)]
mod requests;
#[cfg(test)]
mod runner;
#[cfg(test)]
mod runner_pairing;
#[cfg(test)]
mod settings;
#[cfg(test)]
mod setup;
#[cfg(test)]
mod state;
#[cfg(test)]
mod support;
#[cfg(test)]
mod tasks;
#[cfg(test)]
mod theme;
#[cfg(test)]
mod threads;
#[cfg(test)]
mod uploads;
#[cfg(test)]
#[cfg(feature = "validate")]
mod validate;
#[cfg(test)]
mod week;
