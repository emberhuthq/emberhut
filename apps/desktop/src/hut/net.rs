//! What answers on a loopback port: nothing, an emberhut server (and whether it proves to be
//! this computer's hut), or something else. Behind [`HutNet`], so the tests script it.

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::time::Duration;

use emberhut_api::connect::ProbeAnswer;
use emberhut_api::health::Health;
use emberhut_identity::{MachineId, Nonce, ProbeKind};
use ureq::Agent;

/// The route kind the hut's probe names: it is reached on the loopback.
const LOOPBACK: &str = "loopback";
/// How long a probe of the hut may take: it is on this computer.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// The longest answer read: a health or a probe answer is far smaller.
const MAX_ANSWER: u64 = 64 * 1024;

/// What `GET /health` on a port found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Heard {
    /// Nothing listens there: the port is free.
    Nothing,
    /// An emberhut server answered `{ ok: true, … }`.
    Emberhut,
    /// Something listens there that isn't an emberhut server (or doesn't answer in time).
    Other,
}

/// Asks the loopback. Blocks: call it off the UI thread.
pub(crate) trait HutNet: Send + Sync {
    /// What answers `GET /health` on `127.0.0.1:<port>`.
    fn health(&self, port: u16) -> Heard;

    /// Whether the server on `port` proves to be `machine`: its answer to a fresh
    /// `GET /api/connect/probe` names that id and its signature verifies.
    fn proves(&self, port: u16, machine: &MachineId) -> bool;
}

/// The real [`HutNet`]: plain HTTP to `127.0.0.1`, through ureq.
#[derive(Debug)]
pub(crate) struct LoopbackNet {
    agent: Agent,
}

impl LoopbackNet {
    /// The client: no redirects, statuses as answers, 2 s a request.
    #[must_use]
    pub(crate) fn new() -> Self {
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .https_only(false)
            .timeout_global(Some(PROBE_TIMEOUT))
            .build()
            .new_agent();
        Self { agent }
    }

    /// The body of `GET <path>` on `port`, when it answers 200.
    fn get(&self, port: u16, path: &str) -> Result<Option<Vec<u8>>, ureq::Error> {
        let mut answer = self
            .agent
            .get(format!("http://127.0.0.1:{port}{path}"))
            .call()?;
        if answer.status() != 200 {
            return Ok(None);
        }
        let body = answer
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_vec()?;
        Ok(Some(body))
    }
}

impl HutNet for LoopbackNet {
    fn health(&self, port: u16) -> Heard {
        match self.get(port, "/health") {
            Ok(Some(body))
                if serde_json::from_slice::<Health>(&body).is_ok_and(|health| health.ok) =>
            {
                Heard::Emberhut
            }
            // No answer: free if nothing holds the port, else something that doesn't speak
            // HTTP (or not in time) has it.
            Err(_) if free(port) => Heard::Nothing,
            _ => Heard::Other,
        }
    }

    fn proves(&self, port: u16, machine: &MachineId) -> bool {
        let (Ok(nonce), Ok(kind)) = (Nonce::generate(), LOOPBACK.parse::<ProbeKind>()) else {
            return false;
        };
        let path = format!("/api/connect/probe?nonce={nonce}&kind={LOOPBACK}");
        let Ok(Some(body)) = self.get(port, &path) else {
            return false;
        };
        serde_json::from_slice::<ProbeAnswer>(&body)
            .is_ok_and(|answer| answer.verify(machine, nonce, &kind).is_ok())
    }
}

/// Whether nothing holds `127.0.0.1:<port>`: it can be bound now.
fn free(port: u16) -> bool {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).is_ok()
}

/// A loopback port nothing holds now, chosen by the system (for the real-run test).
#[cfg(test)]
pub(crate) fn any_free_port() -> std::io::Result<u16> {
    Ok(
        TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?
            .local_addr()?
            .port(),
    )
}
