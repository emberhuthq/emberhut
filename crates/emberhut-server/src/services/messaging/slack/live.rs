//! Slack's live events over Socket Mode: for a connected workspace whose app-level token is in
//! the secrets (`slack-<workspace>-app`), the server opens a websocket Slack pushes each event
//! down, so every message in the owner's DMs, group DMs and channels is kept as it arrives, with
//! no public URL.
//!
//! The socket's address comes from `apps.connections.open` (with the app-level token, read fresh
//! at every connect). Every envelope is acknowledged at once, before its event is looked at
//! (Slack wants the acknowledgement within 3 seconds); a message is then kept on its own task,
//! the way the Web API's messages are. Slack rotates sockets with a `disconnect` envelope: a new
//! one is opened after a short pause. A socket that drops, never opens or goes quiet is opened
//! again after a pause that doubles with each failure in a row, up to five minutes. Once messages
//! flow live, that workspace's notifications from the Mac are skipped (the live ones are the
//! whole conversation).

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::{Bytes, Message};

use super::{Live, SlackError, SlackService, text_of};

/// The longest pause between two tries to open a socket.
const PAUSE_MAX: Duration = Duration::from_secs(300);
/// How long opening a socket (TCP, TLS and the websocket handshake) may take.
const OPEN_WAIT: Duration = Duration::from_secs(20);
/// How often the server pings Slack on a quiet socket.
const PING_EVERY: Duration = Duration::from_secs(30);
/// A socket that has said nothing (not even a pong) for this long is taken as dropped.
const QUIET_LIMIT: Duration = Duration::from_secs(90);

/// The secret file that holds a workspace's app-level token, by the workspace's short name.
#[must_use]
pub(super) fn app_token_secret(short: &str) -> String {
    format!("slack-{short}-app")
}

/// A websocket's connection: plain TCP (`ws://`, as tests and a local stand-in use) or TLS
/// (`wss://`, Slack's).
#[derive(Debug)]
enum Link {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for Link {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
            Self::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Link {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
            Self::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_flush(cx),
            Self::Tls(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => Pin::new(stream).poll_shutdown(cx),
            Self::Tls(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

/// An open Socket Mode connection.
type Socket = WebSocketStream<Link>;

/// Opens the websocket at `url` (`wss://`, or `ws://`), within [`OPEN_WAIT`].
async fn open(url: &str) -> Result<Socket, SlackError> {
    let failed = |what: &str| SlackError::Unreachable(format!("the socket didn't open ({what})"));
    let opening = async {
        let request = url
            .into_client_request()
            .map_err(|_| failed("a bad address"))?;
        let uri = request.uri().clone();
        let tls = match uri.scheme_str() {
            Some("wss") => true,
            Some("ws") => false,
            _ => return Err(failed("not a websocket address")),
        };
        let host = uri
            .host()
            .ok_or_else(|| failed("no host"))?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
        let tcp = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|_| failed("connect failed"))?;
        tcp.set_nodelay(true)
            .map_err(|_| failed("connect failed"))?;
        let link = if tls {
            let config = crate::http::tls_config().map_err(|_| failed("TLS setup"))?;
            let name = rustls::pki_types::ServerName::try_from(host)
                .map_err(|_| failed("a bad host name"))?;
            let stream = TlsConnector::from(Arc::new(config))
                .connect(name, tcp)
                .await
                .map_err(|_| failed("TLS failed"))?;
            Link::Tls(Box::new(stream))
        } else {
            Link::Plain(tcp)
        };
        let (socket, _) = tokio_tungstenite::client_async(request, link)
            .await
            .map_err(|_| failed("the handshake failed"))?;
        Ok::<Socket, SlackError>(socket)
    };
    tokio::time::timeout(OPEN_WAIT, opening)
        .await
        .map_err(|_| failed("no answer in time"))?
}

/// How a socket ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    /// Slack said it is rotating the socket: a new one is opened after the first pause.
    Rotated,
    /// It dropped, closed or went quiet; `worked` when Slack had said hello on it first.
    Dropped { worked: bool },
    /// The server is stopping.
    Stopped,
}

impl SlackService {
    /// Starts the live events of each connected workspace that has an app-level token and none
    /// running yet. Called with every round of connecting, so a token saved later is taken up.
    pub(super) async fn start_live(&self) {
        let shorts: Vec<String> = {
            let memory = self.memory();
            memory
                .live
                .values()
                .map(|l| l.short.clone())
                .filter(|short| !memory.sockets.contains(short))
                .collect()
        };
        for short in shorts {
            if self
                .ctx
                .secrets
                .read(&app_token_secret(&short))
                .await
                .is_empty()
            {
                continue;
            }
            if !self.memory().sockets.insert(short.clone()) {
                continue;
            }
            let service = self.clone();
            tokio::spawn(async move { service.keep_live(&short).await });
        }
    }

    /// Stops every workspace's live events.
    pub fn stop_live(&self) {
        self.stop.cancel();
    }

    /// Keeps workspace `short`'s socket open until the server stops or its app-level token is
    /// gone, opening it again after a pause that doubles with each failure in a row.
    async fn keep_live(&self, short: &str) {
        let first = self.parts.live_pause;
        let mut pause = first;
        loop {
            let app = self.ctx.secrets.read(&app_token_secret(short)).await;
            let live = self
                .memory()
                .live
                .values()
                .find(|l| l.short == short)
                .cloned();
            let (Some(live), false) = (live, app.is_empty()) else {
                break;
            };
            match self.listen(&live, &app).await {
                Ok(Ended::Stopped) => break,
                Ok(Ended::Rotated) => pause = first,
                Ok(Ended::Dropped { worked }) => {
                    if worked {
                        pause = first;
                    }
                    tracing::debug!(team = live.team, "Slack: the live socket dropped");
                }
                Err(error) => {
                    tracing::warn!(team = live.team, %error, "Slack: no live events");
                }
            }
            tokio::select! {
                () = self.stop.cancelled() => break,
                () = tokio::time::sleep(pause) => {}
            }
            pause = (pause * 2).min(PAUSE_MAX);
        }
        self.memory().sockets.remove(short);
    }

    /// One socket of workspace `l`'s, from `apps.connections.open` with the app-level token
    /// `app` until it ends.
    async fn listen(&self, l: &Live, app: &str) -> Result<Ended, SlackError> {
        let opened = self
            .parts
            .api
            .call(app, "apps.connections.open", &[])
            .await?
            .json;
        let url = opened["url"]
            .as_str()
            .ok_or_else(|| SlackError::Unreachable("no socket address".to_owned()))?;
        let mut socket = open(url).await?;
        let mut worked = false;
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ping.reset();
        let mut heard = tokio::time::Instant::now();
        loop {
            let frame = tokio::select! {
                () = self.stop.cancelled() => {
                    drop(socket.close(None).await);
                    return Ok(Ended::Stopped);
                }
                _ = ping.tick() => {
                    if heard.elapsed() >= QUIET_LIMIT {
                        return Ok(Ended::Dropped { worked });
                    }
                    if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                        return Ok(Ended::Dropped { worked });
                    }
                    continue;
                }
                frame = socket.next() => frame,
            };
            heard = tokio::time::Instant::now();
            let text = match frame {
                Some(Ok(Message::Text(text))) => text,
                Some(Ok(Message::Close(_)) | Err(_)) | None => {
                    return Ok(Ended::Dropped { worked });
                }
                Some(Ok(_)) => continue,
            };
            let Ok(envelope) = serde_json::from_str::<Value>(text.as_str()) else {
                tracing::debug!("Slack: a socket frame that isn't JSON was skipped");
                continue;
            };
            if let Some(id) = envelope["envelope_id"].as_str() {
                let ack = json!({ "envelope_id": id }).to_string();
                if socket.send(Message::text(ack)).await.is_err() {
                    return Ok(Ended::Dropped { worked });
                }
            }
            match envelope["type"].as_str() {
                Some("hello") => worked = true,
                Some("disconnect") => {
                    drop(socket.close(None).await);
                    return Ok(Ended::Rotated);
                }
                Some("events_api") => {
                    worked = true;
                    self.event(l, &envelope["payload"]["event"]).await;
                }
                _ => {}
            }
        }
    }

    /// One event from the socket: a message is kept on its own task, so the next envelope's
    /// acknowledgement never waits on Slack's other calls.
    async fn event(&self, l: &Live, event: &Value) {
        if event["type"].as_str() != Some("message") {
            return;
        }
        let first = self.memory().flowing.insert(l.team.to_lowercase());
        if first {
            self.ctx
                .activity(format!("Slack {}: live events flowing", l.team))
                .await;
        }
        let channel = text_of(&event["channel"]);
        let (service, live, event) = (self.clone(), l.clone(), event.clone());
        tokio::spawn(async move {
            if let Err(error) = service.keep(&live, &channel, &event).await {
                tracing::warn!(%error, "Slack: a live message wasn't kept");
            }
        });
    }

    /// Whether workspace `team`'s messages come in live (its notifications are then skipped).
    pub(super) fn flowing(&self, team: &str) -> bool {
        self.memory().flowing.contains(&team.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_token_is_named_after_the_workspace() {
        assert_eq!(app_token_secret("acme"), "slack-acme-app");
    }

    #[tokio::test]
    async fn an_address_that_is_not_a_websocket_is_refused() {
        for url in ["https://slack.example/socket", "not an address"] {
            let error = match open(url).await {
                Ok(_) => None,
                Err(error) => Some(error.to_string()),
            };
            assert!(
                error.as_deref().is_some_and(|e| e.contains("didn't open")),
                "{url}: {error:?}"
            );
        }
    }
}
