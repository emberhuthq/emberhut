# The emberhut relay protocol, version 1

This is the wire protocol between a machine (`emberhut-server`), the relay (`emberhut-relay`) and the
devices that reach a machine through it. The decisions and their reasons are in
[ADR 0007](adr/0007-connectivity.md), as amended by [ADR 0008](adr/0008-end-to-end-encryption.md);
this page is the reference the implementations follow. The relay carries only sealed sessions
between devices and machines: it copies their bytes and never reads them. This page changes with
the protocol: a change that breaks either side is version 2, at a new path (`/v2`), and a relay
serves every version it has served unless a later ADR retires one.

Related references:

- the signed statements, the machine id, the encodings, the probe and the route list:
  [api.md's "Connect" section](api.md#connect);
- the sealed sessions the relay carries: [sealed.md](sealed.md);
- the account directory's API: [directory.md](directory.md).

The types for the messages below are in `emberhut_api::relay`.

## Names

The relay serves one host name, the **relay domain** (for example `relay.example.org`), over
HTTPS on port 443, with ALPN `http/1.1` only.

| Path | What it is |
| --- | --- |
| `/v1/uplink` | where machines dial (a WebSocket) |
| `/v1/sealed/<machine id>` | a device's connection to that machine (a WebSocket) |
| `/v1/machines`, and the paths under it | the account directory (directory.md) |
| anything else | 404 `unknown-path` |

One DNS record and one certificate for that name cover all of them. The certificate can come
from any ACME client with any challenge. No page is served for people: a browser that opens a
relay URL gets a 404.

## The machine's connection

### Opening

The machine opens a WebSocket to `wss://<relay domain>/v1/uplink`, over HTTP/1.1.

- `EMBERHUT_RELAY` is that URL. The relay domain is its host, in api.md's normal form, and the
  machine refuses a URL whose path isn't `/v1/uplink`.
- It checks TLS against the web PKI roots, or against the CA a self-hosted relay is configured
  with.
- Text messages carry JSON control messages for the connection's whole life. Binary messages
  carry yamux.
- In control messages, durations are whole seconds, sizes are bytes, and times are JSON numbers
  of milliseconds since the Unix epoch. Unknown fields are ignored, and so are unknown message
  types, so a later addition doesn't break older peers.

### The handshake

1. Relay to machine: `{"type": "hello", "version": 1, "nonce": "<32 bytes>", "time": <ms>}`.
2. Machine to relay: `{"type": "auth", "publicKey": "<32 bytes>", "name": "<text>",
   "time": <ms>, "signature": "<64 bytes>"}`. Binary values are base64url without padding.
   `name` is the machine's name (api.md's rule): the machine trims and cuts it before it
   signs, and the relay refuses one that isn't so (4400). The signature covers the
   `emberhut relay auth v1` statement over the relay domain, the nonce, `name` and `time`.
3. The relay derives the id from the key and checks the signature.
4. It checks that `time` is within 5 minutes of its own clock. That catches a wrong clock
   before the machine signs link statements with it; the nonce alone provides freshness.
5. It asks its authoriser (the allow list, or the directory) whether the id may connect. It
   checks an unknown id at once, batching the ids that arrive within a second.
6. Relay to machine: `{"type": "welcome", "url": "wss://<relay domain>/v1/sealed/<id>",
   "limits": {"streams": 256, "window": 262144}, "ping": 20}`. `url` is the machine's relay
   route, and `streams` the device connections it may have open at once. yamux starts with the
   next binary message. If any check fails, the relay closes instead, with a code from the table
   below.

**Deadlines.** Each step has 10 s:

- from TLS to the WebSocket upgrade request;
- from `hello` to `auth`, on the relay;
- from the upgrade to `hello`, and from `auth` to `welcome`, on the machine.

A missed deadline closes with 4410 (or drops the TCP connection before the upgrade), and the
relay counts it as a failed handshake. A deadline is missed by slow networks and busy hosts, not
by broken peers, so the machine retries it on the normal reconnection window, not after an
hour.

There is one connection per id within a relay process: a newer one replaces the older, which
is closed with 4409.

### Close codes

A close reason is ASCII `key=value` pairs separated by `;`, at most 123 bytes (WebSocket's
limit): `reason=<word>` always, plus the keys the table names. A code means the same on the
uplink and on device connections (below), and no code is used for two things.

| Code | Meaning | Reason keys | The machine then |
| --- | --- | --- | --- |
| 4400 | bad handshake or unknown version | `reason` | tries again after an hour, and logs an error |
| 4401 | the signature doesn't verify | `reason` | tries again after an hour, and logs an error |
| 4403 | not allowed, or revoked | `reason` | tries again every 15 minutes, and at once after `emberhut-server link` |
| 4408 | its clock is more than 5 minutes off | `time=<the relay's ms>` | logs the difference, and tries again after 60 s |
| 4409 | replaced by a newer connection with the same id | `reason` | warns that another process holds the key, and tries again after 5 minutes |
| 4410 | a handshake deadline was missed | `reason=timeout` | reconnects as after a drop (between 0 and 60 s, then 0 and 120 s), and logs a warning |
| 4429 | too many connections for this id or account | `retry=<s>` | waits that many seconds |
| 4502 | the authoriser can't be reached | `reason` | tries again after a random 5 to 30 s |
| 1009 | a message over 64 KiB | — | reconnects as after a drop, and logs an error |

### Control messages after `welcome`

- `{"type": "drain", "within": <s>}`, relay to machine: open a new connection at a random
  moment within `within` seconds (see Handover), then send yamux GoAway on this one and let its
  streams finish.
- `{"type": "mark", "stream": <yamux stream id>, "kind": "device" | "pairing" | "declined"}`,
  machine to relay: the machine has decided the device connection on that stream (sealed.md,
  message 3). `device` and `pairing` name the session it opened; `declined` says it turned the
  connection away for a reason that isn't the client's fault (it is busy, or doesn't speak the
  device's version), so the relay counts no failed handshake and retires no ticket when it ends.
  The machine sends it with its `accept` or `close`, and again with `device` when a pairing
  session pairs. The relay ignores a mark for a stream it doesn't hold, and a mark after a
  `device` one.
- `{"type": "retire", "tickets": ["<16 bytes>", …]}`, machine to relay: these tickets are no
  longer live (the machine has issued their rows newer ones, or signed the rows out). The relay
  treats a retired ticket as no ticket until it expires. A connection that showed a ticket
  before its `retire` arrived keeps its count until it ends, which is harmless: a ticket buys
  only relay budget.

### yamux

- yamux version 0, over the binary messages.
- The relay is the yamux client (odd stream ids), and the only side that opens streams: one
  for each device connection. The machine resets any stream opened towards it.
- The relay writes nothing of its own on a device connection's stream: only the device's bytes,
  as they came.
- The machine and the relay cut the yamux byte stream into binary messages of at most 16 KiB.
  The cuts fall anywhere: the reader concatenates.
- A message over 64 KiB closes the connection with 1009.
- The windows are fixed: 256 KiB per stream, and at most 4 MiB per machine across its streams.
  The yamux crate's receive-window auto-tuning is capped at those values.
- yamux's keep-alive is off.

### Keep-alives

The machine sends a WebSocket ping every 20 s. Either side closes after 60 s without any
frame.

### Reconnecting

Every delay is drawn at random from a window:

- after the machine's own network changes: once, at once;
- after the socket dies with no local cause, or a close without a code from the table:
  between 0 and 60 s, then between 0 and 120 s while it keeps failing;
- after a code from the table: as the table says.

## Device connections

A device reaches a machine by opening a WebSocket to `wss://<relay domain>/v1/sealed/<machine
id>`, over HTTP/1.1, and running a sealed session on it (sealed.md).

- **Before the upgrade**, the relay refuses with a plain HTTP answer: a path that isn't a
  well-formed id (32 characters from `a-z2-7`) gets 404, a request that isn't a WebSocket
  upgrade gets 426, a request head over 8 KiB gets 431, and a client address over the
  failed-handshake limit gets 429. These answers carry `Emberhut-Relay-Error` (below). The relay
  refuses `permessage-deflate`: sealed bytes don't compress. It ignores `Origin`.
- **Otherwise it accepts the upgrade**, whether or not the machine is connected, so that every
  later refusal is a close code, which a browser can read (it can't read a refused upgrade's
  status).
- **The ticket.** If the device holds a ticket for this machine (sealed.md), its first message
  is a text message, with binary values in base64url without padding:

  ```json
  {"type": "ticket", "id": "<16 bytes>", "expires": <ms>, "signature": "<64 bytes>"}
  ```

  The relay checks the `emberhut relay ticket v1` statement over the id in the path, its own relay
  domain, the ticket id and `expires`, against the key the machine authenticated its uplink
  with. A ticket that is expired, retired or doesn't verify is ignored, and the connection
  counts as one without a ticket. A connection whose first message is binary has no ticket. The
  relay waits at most 10 s for the first message.
- **Then the relay opens one yamux stream** on the machine's uplink and copies bytes both ways,
  without reading them: the device's binary messages onto the stream, and the stream's bytes into
  binary messages of at most 16 KiB. Message boundaries carry no meaning. A device's message over
  64 KiB closes the connection with 1009, and a text message after the first closes it with 4400.
- **The mark.** The machine marks the stream once it has decided it (Control messages, above).
  Until then the connection is **unmarked**: it must be marked within 15 s of the relay opening
  the stream, and carries at most 16 KiB each way, or it is closed. A connection that ends
  unmarked counts as a failed handshake for the client's address, and retires the ticket it
  showed, if any: a ticket taken from a signed-out device stops counting after one try. A cut by
  the network before the mark retires the device's own ticket the same way; its next connection
  goes without one until a session gives it a new ticket.
- **Keep-alives.** The relay sends a WebSocket ping every 20 s, and closes after 60 s without any
  frame from the device. Browsers answer pings without the page's help. A connection that carries
  no byte of the session either way for 5 minutes is closed with `idle`.
- **Ends.** When the machine ends the stream, the relay closes the WebSocket with 1000; when it
  resets it, with `reset`. When the device closes, the relay ends the stream.
- **`drain`.** During a planned handover the relay sends the device a text message,
  `{"type": "drain", "within": <s>}`: open a new connection at a random moment within `within`
  seconds, send new requests on it, and close this one once its requests have finished.

### The relay's own answers

The relay's close codes and reasons are about the route, never about the session, and are not
authenticated: the machine's own reasons travel inside the session (sealed.md's close codes). A
close reason is ASCII `key=value` pairs separated by `;`, at most 123 bytes: `reason=<word>`
always, plus the keys the table names. Clients act on the code and its keys; the word is for
logs.

| Code | `reason` | When | The device then |
| --- | --- | --- | --- |
| 1000 | — | the machine ended the stream | it has the session's own `close`; if not, the session was cut |
| 1009 | — | a message over 64 KiB | logs an error |
| 4001 | `moving` | a planned handover's window ended | reconnects at once, on the same route |
| 4400 | `bad-request` | the device broke this page's protocol: a text message after the first, or a malformed one | logs an error |
| 4410 | `timeout` | a deadline was missed: no first message within 10 s, no mark within 15 s, or a `pairing` connection past its 150 s | the route fails in this race |
| 4411 | `idle` | 5 minutes without a byte of the session | reconnects when it next needs the machine |
| 4413 | `too-large` | more than 16 KiB either way before the mark | the route fails in this race |
| 4429 | `limit` | a limit below; `retry=<s>` | waits `retry` seconds, then tries the same route |
| 4499 | `reset` | the machine reset the stream (it refused the handshake, or the session failed) | the route fails in this race |
| 4503 | `offline` | no uplink for that id (after a handover's hold), or the uplink went away | the route fails; a race starts (connect.md) |
| 4507 | `full` | the box's buffers stayed full for 5 s | waits 5 s, then tries the same route |

Before an upgrade, the relay's plain HTTP refusals carry `Emberhut-Relay-Error: <word>` and the
contract's error body (`{"error": "..."}`): 404 `unknown-path`, 426 `upgrade-required`, 431
`too-large`, 429 `limit` with `Retry-After`. A device treats any refused upgrade as the route
failing in this race.

## Limits

The relay sees device connections and bytes, not requests, so it limits those. What it can tell
apart:

- **Marked `device`**: the machine has authenticated a paired device's keys on the connection.
- **With a ticket**: unmarked yet, but the device showed a valid ticket, which the machine gave
  a paired device in an earlier session. Each ticket has limits of its own.
- **Without a ticket**: unmarked, with no valid ticket (a new device, a device away for more than
  a day, or anyone who knows the id), and connections marked `pairing`.

**A client address** is, for every limit, share and penalty on this page, an IPv4 address, or
the /64 prefix of an IPv6 address. One host usually holds a whole /64 (often more), so counting
single IPv6 addresses would give it unlimited shares. A home line or a server usually holds a
/56 or a /48, so the per-address shares below have a second tier per IPv6 /48 as well: a /48
gets about a quarter of each per-machine budget. Failed handshakes, which have no per-machine
budget, are also counted per /48, with the figure in their row.

Connections with a ticket are limited per ticket, not per address, since one household's
devices behind one address each have their own. Connections without a ticket are limited per
address: one address can use only a small share of a machine's budget for them, so it takes many
addresses to use it up, and even then connections with a ticket and marked ones keep their own
budgets. Carrier-grade NAT puts many homes and phones behind one address, so nothing else is
counted per client address alone.

| Limit | Value | Over it |
| --- | --- | --- |
| Device connections open per machine | 256 (the welcome's `streams`) | 4429 `limit` |
| Unmarked connections open per machine, with a ticket | 32 | 4429 `limit` |
| Connections open per ticket | 8 | 4429 `limit` |
| New connections per ticket | 1 a second, bursts of 10 | 4429 `limit` |
| New connections per machine, with a ticket | 20 a second, bursts of 100 | 4429 `limit` |
| Unmarked and `pairing` connections open per machine, without a ticket | 16, of which at most 4 `pairing` | 4429 `limit` |
| The same, per machine and client address | 2 | 4429 `limit` |
| The same, per machine and IPv6 /48 | 4 (a quarter of the machine's) | 4429 `limit` |
| New connections per machine, without a ticket | 5 a second, bursts of 20 | 4429 `limit` |
| The same, per machine and client address | 1 a second, bursts of 3 | 4429 `limit` |
| The same, per machine and IPv6 /48 | 2 a second, bursts of 5 (about a quarter of the machine's) | 4429 `limit` |
| Time to the first message, from the upgrade | 10 s | 4410 `timeout` |
| Time to the mark, from the stream's opening | 15 s | 4410 `timeout` |
| Bytes each way before the mark | 16 KiB (a device sends under 9 KiB: the handshake and at most 8 KiB of early data, sealed.md) | 4413 `too-large` |
| A `pairing` connection's life, from its mark until a `device` mark | 150 s (the machine ends it 120 s after its `accept`) | 4410 `timeout` |
| An idle connection | 5 minutes without a byte of the session | 4411 `idle` |
| A device's WebSocket message | 64 KiB | 1009 |
| Window per stream, each way | 256 KiB | the sender waits |
| Buffered bytes per machine | 4 MiB | its connections wait |
| Buffered bytes per box | a set share of memory (default a quarter) | connections wait, then 4507 `full` after 5 s |
| Request head of an upgrade | 8 KiB | 431 `too-large` |
| Uplink connections per machine id | 6 a minute | 4429 |
| Machines connected per account | the account's cap | 4429 |
| Bytes relayed per account | the account's monthly cap (ADR 0007's open question 5) | the machine's connections are slowed, not closed |
| Failed handshakes per client address (on the uplink: bad signatures, missed deadlines, refused handshakes; on device connections: connections that ended unmarked, and `timeout` and `too-large` closes; never a connection marked `declined`) | 100 a minute, bursts of 1,000; per IPv6 /48 as well, 1,000 a minute, bursts of 10,000 | 4429 on the uplink; 429 before the upgrade for devices |

## Handover

A planned restart moves every machine to the new process without failing a request:

1. The new process starts with `--takeover`. It receives the old process's listening socket
   over a Unix socket (`SCM_RIGHTS`, through a crate's safe wrapper), or both inherit it from
   systemd's socket activation. Connections already queued are not reset, because the socket
   is the same one. Tickets are signed statements, so nothing else needs to pass between them.
2. The old process stops accepting, and sends every machine and every device connection
   `drain { within: 10 }`.
3. Each machine opens a new uplink within those 10 s, and the new process welcomes it. The
   machine then sends yamux GoAway on the old uplink, whose streams stay until their devices
   leave.
4. Each device opens a new connection, with a new sealed session, at a random moment within the
   window, moves new requests to it, and closes the old one once its requests have finished.
   Until the window ends, the new process holds a device connection for a machine that hasn't
   arrived yet, rather than closing it with `offline`.
5. After 3 minutes the old process closes what is left with 4001 `moving`, and exits.

## The front (for more than one box)

`emberhut-relay --role front` sits on the relay domain's address, with boxes behind it.

- Every connection names the same host, so the front can't route by SNI. It ends TLS, reads the
  request head, and splices the connection, head included, to a box. It sees no
  device-to-machine content, as a box sees none: device bytes are sealed.
- The hop from front to box is TLS of its own, with certificates from a private CA that the
  fronts and boxes share.
- `/v1/uplink` goes to the least-loaded box. On `welcome` the box records `id → box` in the
  directory, which pushes the change to every front.
- `/v1/sealed/<id>` goes to the box that holds the id. The directory's paths go to the directory.
- The front passes the client's address to the box in a PROXY protocol (version 2) header, since
  the box's limits count client addresses. A box accepts the header only from the fronts'
  addresses (a setting), and refuses a connection from anywhere else that carries one.
