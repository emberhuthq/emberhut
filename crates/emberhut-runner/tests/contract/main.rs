//! The runner's side of the runner protocol, checked against the server's own types.

// Every module is `cfg(test)` (always true here) so clippy treats its helpers as test code,
// where the workspace lets them unwrap and panic.
#[cfg(test)]
mod runner;
