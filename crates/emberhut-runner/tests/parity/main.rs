//! Parity between two runners: one scenario of the server's own calls, played against both,
//! and the shapes of every answer and every event they report compared (keys and JSON types,
//! not values: the agent's words differ from run to run).
//!
//! Ignored unless asked for: it needs two runners already running, each reporting to its own
//! stand-in server that this test serves, and it spends a little of a real agent's use.
//!
//! ```text
//! EMBERHUT_PARITY_A=http://127.0.0.1:8827  EMBERHUT_PARITY_A_TOKEN=…  EMBERHUT_PARITY_A_HEARD=127.0.0.1:18997
//! EMBERHUT_PARITY_B=http://127.0.0.1:8828  EMBERHUT_PARITY_B_TOKEN=…  EMBERHUT_PARITY_B_HEARD=127.0.0.1:18998
//! EMBERHUT_PARITY_WORK=<a git repository under the user's home>  EMBERHUT_PARITY_REPORT=<file for the report>
//! cargo test -p emberhut-runner --test parity -- --ignored --nocapture
//! ```

// Every module is `cfg(test)` (always true here) so clippy treats its helpers as test code,
// where the workspace lets them unwrap and panic.
#[cfg(test)]
mod scenario;
