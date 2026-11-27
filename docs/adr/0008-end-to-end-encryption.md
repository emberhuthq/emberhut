# 0008. End-to-end encryption: sealed sessions between devices and machines

- Status: Proposed
- Date: 2026-11-25
- Amends: [ADR 0007](0007-connectivity.md): the relay carries only sealed sessions, from its first
  release, and never ends HTTP for device traffic; it serves one host name, with no per-machine
  subdomain, wildcard certificate or Public Suffix List entry; its limits count connections and
  bytes, not requests; `Sealed<D: Dial>` replaces the sketched `Sealed<T: Transport>`; a device
  key sent at pairing comes with proof that the device holds it; the directory accepts OAuth
  access tokens from two client ids, the native application's and the web application's; the
  machine's direct listeners answer the web app with CORS; the relay route and machine
  linking depend on the work below, and the gateway is built as a pipe; its open questions 1
  and 4 are resolved.
- Amends: [ADR 0003](0003-monorepo-layout.md): one more crate joins the workspace, `emberhut-sealed`.
- Specified in:
  - [sealed.md](../sealed.md): the sealed session protocol, version 1: handshake messages,
    payloads, records, the pairing exchange, error codes and vectors;
  - [relay.md](../relay.md): the relay's device connections, marks, tickets, close codes and
    limits;
  - [api.md, "Connect"](../api.md#connect): the new statements, the `sealed` field, the proof of
    a device key and the sealed pairing routes;
  - [connect.md](../connect.md): when a relay route answers, and which version a client tries.

  This ADR holds the decisions and their reasons; those pages hold the formats and numbers.

## Context

ADR 0007 gives a device several routes to a machine, with a relay as the route of last resort.
As written there, the relay ends TLS and HTTP for every relayed request. It reads every path and
header, so every access token; every body, so refresh tokens, pairing codes, messages and state.
It is written to keep none of it, but that is a policy of the code it runs, not a property of the
protocol. Whoever runs a relay can change the code: emberhut.com for its relay, a self-hoster for
theirs, and anyone who has broken into the box. A relay that can read tokens can also change
requests and act as the device.

ADR 0007 sketched end-to-end encryption for later (its section 4) and planned to open the
emberhut.com relay to everyone only once it landed. A relay protocol that ends HTTP and a later one
that doesn't would both have to be kept: clients that predate encryption would need the readable
mode for as long as they exist, and a device can't tell from the outside which mode a relay is
running. So the decision is taken before any relay ships.

Words used below, beside ADR 0007's:

- A **sealed session** is one Noise session between a device and a machine. It starts on the
  device and ends on the machine, and nothing between them can read or change what it carries.
- The **identity keys** are ADR 0007's Ed25519 keys: the machine's (`machine.key`, pinned by
  each device at pairing) and the device's (stored on the device's row at pairing).
- A **static key** is the X25519 key each side uses in the Noise handshake, signed by its
  identity key.
- A **device connection** is one WebSocket from a device to the relay, carried to the machine
  as one yamux stream on the machine's uplink.

What matters:

- Nobody who runs or controls a relay can read or change tokens or content, including the
  operator of a self-hosted relay and an attacker on a compromised relay box.
- Pairing through a relay can't be intercepted, whether the device has a pairing link or only
  a typed code.
- The relay stays cheap per idle machine (ADR 0007's 100 KiB) and simple to run.
- Native clients (the desktop, the phone, the CLI) carry their own code, so for them the
  guarantee rests on the keys alone. A browser runs whatever script its origin serves, so for a
  browser the guarantee can be no stronger than trust in whoever serves the web app's script,
  emberhut.com included (section 6). The first release has no browser client.
- One protocol in Rust and TypeScript, held together by shared vectors.
- Direct routes keep working exactly as ADR 0007 specifies.

## Decision

### 1. Every relayed byte is sealed, from the first release

- **The relay never carries readable traffic.** Every byte the relay carries between a device
  and a machine is inside a sealed session that ends on the device and on the machine. The relay
  copies those bytes; it parses no HTTP of device traffic, and it holds no key that opens them.
- **There is no readable mode.** No setting, flag or build makes the relay end HTTP for device
  traffic, and the machine serves no uplink stream without a sealed session. A readable mode
  would be one setting away on any relay, a device couldn't tell it was on, and it would be the
  target of every downgrade.
- **A client that can't seal has no relay route.** That covers legacy hosts (which never had
  one), any build of a client without sealed sessions, and a browser page other than the web app
  of section 6. A machine's route list names the sealed versions it speaks (api.md's `sealed`),
  and a client skips relay routes whose listed versions it doesn't speak. A relay route learned
  without a list (from a pairing link, or the account's list) is tried at the client's highest
  version, and the machine's preamble refusal names the versions it speaks (sealed.md).
- **What end to end means for a browser.** The session ends in the browser's script. Someone
  using the web app trusts whoever serves that script (emberhut.com, for its copy), as well as the
  machine. Section 6 says how far that trust goes and what bounds it.
- **Direct routes are unchanged.** Loopback, `lan`, `vpn`, `public` and `ssh` routes follow ADR
  0007 exactly, including the plain-HTTP rule for tokens on loopback, VPN addresses and granted
  local routes, the network fingerprints and the probe. Sealing them as well is open question 1.
- **Tokens still travel inside.** Every request in a device session carries the device's bearer
  token, as on a direct route (section 4).

The emberhut.com relay can therefore be open to everyone from its first day: ADR 0007's open
question 4 is resolved.

### 2. The handshake

**The protocol name: `Noise_XX_25519_ChaChaPoly_SHA256`**, from the Noise Protocol Framework
(revision 34).

```text
XX:
  -> e
  <- e, ee, s, es
  -> s, se
```

**Why XX and not IK.** IK lets the device send in its first message, encrypted to the machine's
static key, and was the pattern ADR 0007 sketched. XX is chosen:

- **IK needs the machine's static key before the first byte.** A device pairing from a link
  knows only the machine's id, a device pairing with a typed code knows nothing yet, and static
  keys rotate (below). IK would therefore need XX anyway, as the fallback of Noise Pipes, so both
  implementations would carry two patterns and a fallback path between them.
- **IK's early data can't carry requests.** Its first message can be replayed: whoever holds it
  (the relay) can send it again, and the machine would run the request again. So no request goes
  in it, and the first request waits for the machine's answer in both patterns. With XX it goes
  out with message 3, without waiting: the first request leaves after one round trip either way.
- **XX hides the device even from an active relay.** The device sends its static key and its
  statement only in message 3, after it has authenticated the machine. In IK they are encrypted
  only to the machine's static key, with no forward secrecy for the device's identity.
- **The cost** is a third message (the device's static key and hello travel in it rather than
  in the first, so the bytes barely change), and the machine shows its static key and statement
  to anyone who opens a handshake. Neither is a secret: the statement is signed with the
  machine's identity key, whose public half is what the id is derived from.

**The payloads carry the signed statements.** Message 1 carries no payload. Message 2's payload
is the machine's **hello**: its identity key, and the expiry and signature of its static-key
statement. Message 3's payload is the device's hello, in the same form. Each side takes the
peer's static key from the handshake itself and checks the statement over it:

- **The machine's key.** The device requires that the identity key in the machine's hello is the
  key it pinned at pairing, or, before pairing, that it hashes to the machine id the device is
  connecting to (ADR 0007's id, which is self-certifying).
- **The device's key.** The machine looks the identity key up among its active device rows. The
  row's key was stored at pairing, with proof that the device holds it (section 5), so a key the
  machine finds is a paired device's.
- **The statement** is `statement("emberhut sealed static v1", [role, static key, expires])` in ADR
  0007's encoding (api.md), with the role `machine` or `device`. The prefix keeps it from being
  valid for any other purpose, and the role keeps a device's statement from passing as a
  machine's. sealed.md has the bytes.
- **Signatures are checked by api.md's one rule for statements**, as every statement of ADR 0007
  is: `s < L`; `A` and `R` canonical, in the prime-order subgroup and not of small order; the
  cofactorless equation. So a signature has exactly one valid form, and the two implementations
  can't disagree on an edge case.
- **A key agreement whose result is all zeros fails the handshake.** A peer that sends a
  low-order X25519 point would otherwise force a known shared secret. snow's own 25519 doesn't
  check this, so `emberhut-sealed` hands snow a resolver that wraps snow's 25519 and fails such a
  result; the TypeScript side checks it directly.

**The cipher suite.**

- **25519** is the one key agreement available everywhere this runs: snow (on curve25519-dalek)
  in Rust, `@noble/curves`, and WebCrypto in current browsers, which can keep the private key
  non-extractable (section 6).
- **ChaChaPoly** is constant-time and fast in software. Phones and JavaScript engines often have
  no AES instructions to reach, and AES-GCM in plain JavaScript is either slow or not
  constant-time. WebCrypto's AES is asynchronous per record, which would put a promise on every
  record of every stream.
- **SHA256** is the hash the rest of the system already uses (machine ids, pairing-code hashes),
  native in WebCrypto and on every platform. The hash runs only during the handshake, so BLAKE2s
  (the sketch's choice) would save almost nothing, and SHA-256 leaves one hash in the system.

**The prologue** binds the session to what the device meant to reach:
`statement("emberhut sealed prologue v1", [machine id, route kind, route host])`.

- The version is in the prefix, so a session can't be completed under another version than both
  sides think they speak.
- The machine id: a session the relay routes to another machine fails, even before the keys are
  checked.
- The route: version 1 has only the `relay` kind, with the relay domain as host. The device takes
  it from its route URL, the machine from `EMBERHUT_RELAY`. A server that forwards a device's bytes
  through another relay name fails, as a forwarded probe fails ADR 0007's binding.

Both sides mix the prologue in before message 1. A mismatch surfaces on the device at message 2,
whose encrypted static key then doesn't decrypt.

**Key rotation.**

- **Static keys live in memory only.** Each side makes a static key when it starts, signs its
  statement with its identity key, and makes a new one at least every 24 hours. Neither side
  writes a static key to disk, so no backup and no file copy holds one. Sessions already open
  keep their own keys, which don't depend on the static key after the handshake.
- **Statements expire.** A statement says when it expires, 48 hours after it was made. A
  verifier accepts it until 24 hours past that, for clock skew, and refuses one that expires more
  than 72 hours ahead of its clock, which no honest signer makes. A static key taken from memory
  is then useful for at most three days, and a device's only together with its tokens.
- **Identity keys don't rotate,** as ADR 0007 decides for the machine. A device's identity key
  is replaced by pairing again. The statement a later hand-over will use is fixed now (open
  question 4), so that no client has to change its encoding then.

Keeping a separate X25519 key, rather than converting the Ed25519 identity key to X25519, keeps
one key per purpose, and lets the browser keep both as non-extractable WebCrypto keys, which can't
be converted.

**Implementations.** The project never writes its own cryptography where a mature, widely used
implementation exists.

- **Rust: `snow` 0.10**, in `emberhut-sealed`, with default features off and only its pure-Rust
  resolver's `use-curve25519`, `use-chacha20poly1305`, `use-sha2` and `use-getrandom`: no
  AES-GCM, no BLAKE2, no ring, and not its `std` feature, which in 0.10 also turns on ring and
  BLAKE2. snow is the Rust implementation of Noise in production use, tested against the
  community's cacophony vectors, and its code is reviewed by people who use it: its one advisory,
  RUSTSEC-2024-0011, was found and fixed in 0.9.5. `emberhut-sealed` uses its `HandshakeState` and
  `TransportState` for this one protocol name, with its own resolver wrapper for the zero check
  above.
- **The cost is a second set of primitives in the build.** snow 0.10 is built on
  curve25519-dalek 4 and sha2 0.10 (and chacha20poly1305 0.10), while the workspace is on
  curve25519-dalek 5 and sha2 0.11 (through `ed25519-dalek` 3). No snow release is on the newer
  ones yet: its main branch has moved only sha2 to 0.11, unreleased. Two versions of a crate cost
  compile time and binary size, not security, so the duplicates are accepted explicitly: a
  `[bans]` skip in `deny.toml` for `curve25519-dalek@4` and `sha2@0.10`, with that reason and a
  note to drop the skip when snow moves to curve25519-dalek 5.
- **TypeScript: in `packages/connect`, on `@noble/curves` (X25519), `@noble/ciphers`
  (ChaCha20-Poly1305) and `@noble/hashes` (SHA-256, HMAC)**, the dependency-free libraries the
  project already uses for Ed25519. Here nothing mature fits, so the state machine is Noise's own
  `CipherState`, `SymmetricState` and `HandshakeState` (its section 5) for one pattern, about 300
  lines. libp2p's Noise package exports only its libp2p connection encrypter (with libp2p's
  payloads) and wrappers of primitives; `noise-handshake` runs on libsodium bindings with
  synchronous key agreement, which rules out non-extractable WebCrypto keys and adds a native
  module on the phone. The static key's operations go through an asynchronous interface, so a
  browser can hold it in WebCrypto.
- **The two check each other.** snow generates sealed.md's Noise vectors, and the TypeScript
  state machine must reproduce them byte for byte; cacophony's vectors for this protocol name
  run in both as well.

### 3. Sessions and framing

**One sealed session per device connection, with streams inside it.** A device opens one
connection to the relay per machine (a WebSocket), and that connection carries one sealed session.
Inside, yamux carries one stream per request, each an HTTP/1.1 exchange with `Connection: close`,
as the uplink carried them in ADR 0007. The machine serves each inner stream with the same router
as a TCP connection.

The other way was a session per request: each request a yamux stream of its own between the
relay and the machine, with its own handshake. It loses on each count:

- **Handshake cost.** The desktop polls every 3 s, 1,200 requests an hour per open app. A
  session per request is 1,200 handshakes an hour, each with signatures, and each request would
  wait a round trip for its handshake (early data being replayable, section 2). Through a distant
  relay that doubles the time of every request. One session per connection pays one handshake
  per connection: four X25519 operations and one Ed25519 check on each side, well under a
  millisecond.
- **The relay's memory.** Sealed sessions cost an idle machine nothing at the relay: the relay
  holds no Noise state either way, and device connections exist only while a device is
  connected. An open device connection costs the relay a socket, a TLS session, a WebSocket and
  one yamux stream on the uplink, with the same tuned buffers as an idle machine's (relay.md).
  A session per request would have the relay demultiplex the device's side too, a stream per
  request on both legs.
- **Head-of-line blocking.** A device's requests share one ordered byte stream either way: the
  client's TCP connection to the relay, and the uplink's TCP connection to the machine, which all
  of the machine's devices share. A session per request doesn't remove that. Within the session,
  inner streams have windows of their own, the machine's reader always drains the session without
  waiting on any inner stream, and records hold at most 16 KiB, so a large body delays a small
  request by at most one record.
- **Metadata.** With one session per connection the relay can't count requests: it sees records
  of a connection, not streams.

**Framing.** sealed.md has the bytes. In short:

- The device starts with a five-byte preamble that names the version; the machine answers with
  its own, or with the versions it speaks and an end.
- Every handshake message and every record is a frame: a two-byte big-endian length, then the
  bytes. Handshake frames hold at most 4,096 bytes.
- A record is one Noise transport message. Its plaintext is a type byte, a two-byte body length,
  the body, and zero padding, at most 16,384 bytes in all: well inside Noise's limit of 65,535,
  so the length always fits two bytes, and each side needs only a 16 KiB buffer to decrypt. Any
  record may carry padding from version 1 on; version 1 senders send none (open question 2).
- The types are `accept`, `data`, `ping`, `pong`, `rekey`, `close` and `padding`. Types below
  `0x80` must be understood: an unknown one closes the session. Types from `0x80` up may be
  ignored, so later additions within version 1 don't break older peers.
- The machine's first record is `accept`, or a `close` when it refuses the session (a refused
  statement, too many sessions, an unknown key with no pairing code outstanding). `accept` says
  whether the session is a paired device's or a pairing session, and carries the device's relay
  ticket (section 7).

**Early data.** The device may send records right after message 3, before it has read `accept`,
so that the first request leaves after one round trip. It sends at most 8 KiB of them, and the
machine handles none of them before it has sent `accept`: if it closes instead, or the session
is a pairing session, it drops them unread. A request the machine refused with `busy`,
`statement` or `not-paired` was therefore never run, and is safe to send again. The bound keeps
the device's bytes before the machine's mark (handshake and early data together, under 9 KiB)
inside the relay's 16 KiB for unmarked connections, which the relay can count without reading
any frame.

**Rekeying.** A side rekeys its sending direction (Noise's `Rekey()`) after 65,536 records, and at
least every hour in which it has sent any, by sending a `rekey` record and then rekeying; the
receiver rekeys its receiving direction as it reads that record. The old key is gone after a
rekey, so a key read from a running process doesn't open what was sent before the last rekey.
ChaCha20-Poly1305 needs no rekeying for its own sake: its limits per key are far past anything a
session sends.

**Nonces.** Noise counts nonces implicitly, 64 bits per direction, and never sends the last one.
A device session lasts at most 24 hours (below), which keeps that far out of reach; an
implementation still closes the session rather than reach it.

**Replay, reordering and truncation.**

- Each record's nonce is its position, so the relay can't replay, reorder or drop a record: the
  next one then fails to decrypt, and the session ends.
- A whole session can't be replayed: message 3 depends on the machine's fresh ephemeral key, so
  a replayed message 1 gets a new message 2 that only the device's keys can answer.
- A session that ends without a `close` record was cut, by the relay or the network. Inner
  streams that were open fail as on a reset connection, and the device treats the route as it
  treats a transport error.

**Closing and errors.**

- A `close` record carries a code and a short reason, and is authenticated like every record. So
  the machine can say, in a way the relay can't forge, that the device was signed out, that it
  is shutting down, or that the device's statement was refused. sealed.md has the codes.
- A failure that leaves no keys (a frame that doesn't decrypt, a wrong length) gets no record:
  the side that sees it ends the byte stream at once. The relay reports the end with its own,
  unauthenticated close code (section 7).
- The relay's close codes and the machine's `close` codes are separate spaces. A device trusts
  only the machine's for anything about its session (signed out, refused), and treats the relay's
  as reasons about the route.

**Timers.** Each one starts at a defined moment (sealed.md and relay.md have the table): the
machine wants message 3 within 10 s of its stream opening; the relay wants the machine's mark
within 15 s of opening that stream; the device wants message 2 within 10 s of its WebSocket
opening, and `accept` within 5 s of sending message 3. A pairing session must start pairing
within 10 s of its `accept` and ends 120 s after it. A device session ends 24 hours after its
`accept`; the device opens its next one after 23 hours and moves new requests to it. While an
inner stream is open, the device sends a `ping` record after 30 s without receiving anything, and
treats 15 s more of silence as a dead connection.

**What the relay still learns.**

- which machine (the id is in the connection's path), from which client address, when, and for
  how long;
- how many bytes each way, in records whose lengths it can read, and their timing, so the rough
  size of requests and answers;
- whether the machine marked the connection as a paired device's or a pairing (section 7);
- a device's ticket id, so it can link one device's connections to that machine while the
  ticket lives (at most 24 hours). It can't tell which device it is.

It learns no device-to-machine content: no path, header, token, body or code. Answers that carry
tokens (pairing and renewal) are under api.md's 16 KiB compression threshold, so no secret is
compressed beside text someone else chose, and lengths reveal nothing about it.

### 4. `Sealed<D: Dial>` and `Sealed<L: Listener>`

**`emberhut-sealed`** is a new crate that does no I/O: the handshake (on snow), the hellos, the
records, CPace and the pairing exchange, and their errors, as state machines over bytes.
emberhut-client and emberhut-server drive it; the relay never depends on it. The static-key statement is
in `emberhut-identity`, with the other statements.

**The client: `Sealed<D: Dial>` is a `Transport`.** ADR 0007 sketched `Sealed<T: Transport>`.
ADR 0007's `Transport` (now built in emberhut-client) is one request in, one answer out:
`send(&self, Request<'_>, Timeouts) -> Result<Answer, TransportError>`, with the answer's body a
blocking `Read`. A sealed session is the wrong shape to sit behind that as a parameter:

- it is long-lived, with many requests over one handshake, where a `Transport` has no notion of
  a connection that outlives a request;
- it carries streams (an SSE answer that lasts all day, an `Upgrade`), as a byte stream under
  HTTP, not as one answer;
- and to wrap a `Transport` the relay route would have to be HTTP the relay answers, which is the
  HTTP parsing this ADR takes out of the relay.

So the parameter is a dialler of byte streams, and `Sealed<D>` is itself the `Transport`:

```rust
/// Opens an ordered, reliable byte stream towards a machine, below HTTP.
pub trait Dial: Send + Sync + fmt::Debug {
    /// The stream it opens: non-blocking, readable until `WouldBlock`, and registrable with
    /// mio by its socket.
    type Stream: io::Read + io::Write + mio::event::Source + Send + 'static;

    /// Opens a stream to the machine, or fails by `deadline`.
    fn dial(&self, deadline: Instant) -> Result<Self::Stream, TransportError>;

    /// The route the prologue binds: its kind and host.
    fn route(&self) -> RouteBinding;
}
```

**The threading model: one I/O thread per session, and no runtime.** emberhut-client stays blocking,
with no async runtime (ADR 0001):

- **The I/O thread** owns the stream, in non-blocking mode, registered with a `mio::Poll`
  (epoll, kqueue or IOCP) through `Registry::register`, which is safe code, as the workspace's
  `unsafe_code = "deny"` requires (`polling`'s `Poller::add` is an `unsafe fn`). It waits with a
  timeout for the next timer (ping, rekey, lifetime, deadlines). Whenever it wakes, it reads
  until `WouldBlock`, since TLS and the WebSocket buffer bytes the socket no longer signals,
  decrypts records, and writes what is queued.
- **yamux is driven in that thread, by polling.** The yamux crate is futures-based. The I/O
  thread gives its `Connection` an in-memory byte pipe (filled with decrypted `data` and drained
  into new records) and polls the connection and its streams with a `std::task::Waker` that
  calls a `mio::Waker` registered on the same `Poll`, which other threads can wake too. That is
  the whole of an executor this needs: one thread polling one connection, which nothing else
  shares.
- **Callers stay on their own threads.** `send` puts "open a stream" on a channel to the I/O
  thread and wakes it through the `mio::Waker`; it writes the request head and body to the
  stream through a bounded channel (reading a streaming `Body` itself, so nothing borrowed
  crosses threads, and so a slow machine pushes back on the caller); and it reads the answer
  from the stream's own channel. HTTP/1.1 is written and parsed with `ureq-proto`, the sans-I/O
  HTTP/1.1 that ureq itself runs on. `send` returns once the head has arrived, within
  `Timeouts`; the `Answer`'s body is a `Read` over the stream's channel, which blocks the
  caller's thread and no other.
- **The race** dials and handshakes on the I/O thread of a new session; a session whose handshake
  completes after the race chose another route is kept idle until the relay closes it.

The first `Dial` is `RelayDial`: TCP (a `mio::net::TcpStream`), then TLS through rustls with the
same roots as `Direct` (`webpki-roots`, or a self-hosted relay's configured CA), then the
WebSocket to `/v1/sealed/<machine id>` through `tungstenite` over the non-blocking TLS stream,
the device's ticket, and the relay's close codes and `drain` messages as typed events. A local
TCP connection could be another `Dial` later.

`Direct` no longer covers the relay: ADR 0007's "the relay too, which to a client is just an
HTTPS URL" is replaced by `Sealed<RelayDial>`.

**The server: `Sealed<L: Listener>` is an axum `serve::Listener`.**

- It wraps any listener whose connections are byte streams. Its `accept` returns the next inner
  stream of any sealed session, with a peer that names the session: a device's row, or a pairing
  session, with the handshake hash.
- It runs each accepted connection's handshake in a task of its own, with the deadlines, so a
  slow or hostile handshake never holds up `accept` for others.
- The machine serves the uplink only as `Sealed<Uplink>`. The uplink's raw streams are never
  passed to axum, and a test checks that plain HTTP written on an uplink stream gets no answer.
- Handlers don't change. They see the peer in the connection info, as they see a TCP peer.

**The device, authenticated at the transport.** After message 3 the machine checks the device's
statement and looks its identity key up among its active device rows. A key is on at most one
active row: a pairing that proves a key already on a row replaces that row, as an install id does
(ADR 0002), and a key without proof replaces nothing (section 5).

**Forgetting a device's key.** Signing a device out (ADR 0002), its row lapsing after 14 days
unseen, or the row being replaced, clears the key from the row. The machine keeps a registry of
open sessions by device row, and closes that device's sessions at once with `signed-out`.
Signing out stays immediate through the relay, and the device learns it from a record the relay
can't forge.

**A device whose key the machine doesn't know.** A device that holds tokens for the machine but
gets `accept` with kind `pairing` (or a `close` with `not-paired`) has been forgotten there,
perhaps signed out while it was away. It sends no token in that session, closes it, and treats
the relay route as unavailable. If a direct route answers, its own session there says whether it
is still signed in; otherwise Connections says "Pair this device again to use the relay".

**A stream that fails the handshake is refused.** The machine ends it without a record when
nothing decrypts, or with a `close` record when the keys exist but the session is refused. It
never reaches the router. The relay sees the stream end before the machine marked it, and counts
a failed handshake against the client's address, unless the machine declined it for a reason
that isn't the client's fault, such as being busy or a version it doesn't speak (section 7).

**Bearer tokens still go inside, as defence in depth.** The session authenticates the device's
keys; the token carries ADR 0002's session: its hour of life, its renewal and reuse check, and
sign-out. Every request in a device session still needs the device's access token, and the
machine also requires that the token's row is the session's row; any other token gets 401 with
`signIn`. So a token taken from a device can't be used through the relay without that device's
keys, and the keys can't be used without a live token. Without tokens inside, the device's keys
alone would be the credential, and relayed requests would follow expiry and renewal rules of
their own, apart from ADR 0002's on every other route.

### 5. First contact through the relay

A new device makes its identity key before it pairs, and its first static key with it. It has no
row on the machine yet, so its first sealed session is a pairing session.

**What the machine accepts from an unknown device.** A device whose identity key is on no active
row gets a **pairing session** (`accept` with kind `pairing`), and only while a code is
outstanding:

- With no code outstanding, the machine closes the handshake with `not-paired` instead. That
  tells a stranger whether pairing is under way, a small thing, and saves the machine every
  stranger's pairing work the rest of the time. A new device opens its pairing session only once
  the person has typed the code (or opened the link), so `not-paired` answers that code: the
  device shows "That code isn't right. Check it, or make a new one."
- Only the two sealed pairing routes are served (api.md). Every other request, the probe
  included, gets 401 with `signIn`.
- `start` must come within 10 s of `accept`, and there is one attempt per session. The session
  ends 120 s after `accept`, or at once after a failed attempt.
- At most 4 pairing sessions at once per machine; a fifth handshake is closed with `busy`.
- At the relay, a pairing connection counts against the budget for connections without a ticket,
  and the machine marks it `pairing` (section 7).
- Each attempt counts against ADR 0002's wrong-code brake from the moment it starts, and is
  forgiven when it succeeds.

**The code is never sent through the relay.** In a pairing session, pairing runs as a PAKE with
the code as its password, whether or not the device has a link:

- **With a link** (or its QR code), the device knows the machine id before the first byte, and
  requires at message 2 that the machine's key hashes to it. The relay can't put its own key in
  place.
- **With a typed code**, or a machine picked from the account's list (which the directory, and
  so whoever controls it, supplies), the device has no trusted id. The PAKE is then what
  authenticates the machine: only a party that knows the code completes it.
- One path for both keeps the two implementations to one pairing protocol through the relay,
  and keeps the code out of every relayed byte, even towards an authenticated machine.

**The PAKE: CPace, as draft-irtf-cfrg-cpace-21 specifies it**, with its ristretto255 group and
SHA-512, in the initiator-responder mode. sealed.md spells out the generator string, the map to
the group (RFC 9496's element derivation from 64 uniform bytes, not RFC 9380's
`hash_to_ristretto255`), the scalars, the checks and the `ISK`, byte for byte.

- **Why CPace and not SPAKE2.** CPace is the balanced PAKE the CFRG selected. It needs no
  fixed points of unknown discrete logarithm (SPAKE2's M and N), works in a prime-order group with
  no cofactor to clear, and takes a channel identifier and a session id, which bind it to the
  sealed session (below). The draft publishes test vectors for the ristretto255 suite (its
  appendix B.3), which both implementations run.
- **Written in `emberhut-sealed`, on curve25519-dalek 5's ristretto255 and sha2 0.11.** The protocol
  is a generator, two scalar multiplications with their checks, and a hash: about a hundred
  lines over the group's own operations, which are curve25519-dalek's. No mature CPace
  implementation exists in Rust: the crates are young, or follow older drafts. So this is the
  one protocol written here, held to draft 21's vectors. RustCrypto's `spake2` is well
  maintained, but it is built to be compatible with an earlier implementation (python-spake2)
  rather than with RFC 9382's ciphersuites, and has no TypeScript counterpart or independent
  vectors to check one against.
- **TypeScript: the same on `@noble/curves`' ristretto255** (its map from 64 uniform bytes) and
  `@noble/hashes`' SHA-512, against the draft's vectors and ours.

**The exchange** (sealed.md has the bytes):

1. The device computes the password: the SHA-256 of the normalised code, which is the form the
   machine stores (ADR 0002), so the machine keeps no code in the clear for this.
2. It runs four CPace initiators with that password, one per **slot**, and sends their shares
   (`POST /api/auth/pair/sealed/start`).
3. The machine gives each slot one of its outstanding codes, newest first. For each slot without
   a code it runs the same responder with a fresh random password, so every slot costs the same
   work and looks the same. For each slot it answers the device's share, with a confirmation tag
   under that slot's key.
4. The device finishes each slot, and checks each tag. A slot whose tag verifies is the code's
   slot: the machine has proved that it knows the code. If none does, the code is wrong, and the
   device sends nothing more.
5. The device sends its own tag for that slot, with its install id, name and model
   (`POST /api/auth/pair/sealed/finish`). The machine checks it, uses the code up, and makes the
   device's row with the session's identity key, in one transaction. It answers as pairing does
   (ADR 0002's `AuthReply`, with `machine`, and a relay ticket), and the session becomes that
   device's: the pairing limits, the machine's 120 s and the relay's, stop there.

The reasons for its shape:

- **Bound to the session.** CPace's session id is the Noise handshake hash, and its channel
  identifier names the machine id. A relay that runs two handshakes, one with each side, has two
  different hashes, so neither side's PAKE completes unless it guesses the code, one guess per
  slot, counted on the machine's brake.
- **Slots, because several codes can be outstanding.** The device can't say which code it has
  without revealing something an attacker could test offline. Four slots cover the codes a person
  makes in one sitting. A guess sent in each slot has the same odds as one typed guess against a
  machine with four codes outstanding, so each attempt still counts once on the brake. A fifth
  outstanding code can't be used through the relay until the older ones are used or expire, and
  the random fillers keep how many codes are outstanding from showing.
- **Four guesses against the device.** The same slots work the other way: a party posing as the
  machine (the relay, with a key of its own, when the device has no link) can put a different
  guess in each slot, so each of the device's attempts gives it four guesses at the device's
  code, out of about 850 billion codes. They aren't braked, since the device has no brake to
  count them on; a wrong guess ends the attempt, and the person sees "That code isn't right".
- **The device key is the session's.** The row's identity key comes from the authenticated
  handshake, never from a body field.
- **The machine confirms first.** A party that doesn't know the code can't make a tag that
  verifies, and the device's own tag goes out only after the machine's verified. CPace keeps a
  tag from being tested offline against other codes.
- **The stored hash is the password.** A copy of the database holds the hashes of outstanding
  codes, and through the relay a hash is as good as its code: whoever has it can complete CPace
  with no cracking. ADR 0002 already accepts that a database copy reveals unused codes within
  their ten minutes; for relay pairing it needs no GPU either. The ten minutes and single use
  still bound it.

**Pairing on direct routes** keeps `POST /api/auth/pair` with the code, as ADR 0002 and 0007
specify, with one added rule: a `publicKey` in the request must come with `keySignature`, the
device key's signature over `statement("emberhut device key v1", [machine id, code hash])`. The
machine checks it against this request's code hash before it looks the code up. A bad one gets
400, counts on the wrong-code brake like a wrong code, and uses nothing up; the key is stored
only when the signature verifies.
Without it, a person who holds a code could put another device's public key on a row of their
own, and through the "one active row per key" rule push that device's row out. This changes ADR
0007's field: it was taken as sent. No released client sends the field yet, so the rule changes
nothing in use. The machine id keeps the proof from serving at another machine, and the code
hash ties it to this pairing.

### 6. Browsers

**What a browser can be promised.** Encryption in a page is only as trustworthy as the script
that does it. A browser user trusts whoever serves the web app's script: emberhut.com for its copy,
or a self-hoster for theirs. Whoever controls that script (its operator, or anyone who breaks
into its hosting or its build) can read the traffic of every machine that browser reaches
through the app, while the person uses it. Nothing below removes that; it keeps the trust to that
one origin, and makes changes to its script visible. The first release has no browser client
(part 15 in ADR 0007's table is later).

**The web app has an origin of its own, `https://app.emberhut.com`, which serves only its bundles.**

- No machine page, no account page, no site page and no third-party script runs there: no
  Clerk script loads on it. The site, `/pair` and the desktop's `/oauth/callback` stay on
  `https://emberhut.com`.
- Script served through a relay or by a machine never runs there: the relay serves no pages, and
  a machine's HTML is shown in a frame on another site (below).
- **Published bundle hashes are the chosen check.** Each release of the web app is built
  reproducibly from the repository, and the hashes of its bundles are published with the
  release. Anyone can compare them with what `app.emberhut.com` serves, so a changed bundle can be
  seen, though not stopped, by people who look. Browser-enforced pinning (signed or isolated web
  bundles) is later work; it isn't available across browsers for an ordinary origin today. A
  self-hoster can serve the same bundles from an origin of their own, and then trusts only
  themselves.

**The app gets account tokens by OAuth with PKCE, as the desktop does.** Clerk's script is large,
third-party, and loads more of itself from Clerk's servers; wherever it runs in the app's origin,
or in a frame that hands tokens over, it runs beside the device keys or next to them. The desktop
already treats Clerk as an OAuth 2.0 and OpenID Connect provider (ADR 0007, section 5), and the
web app does the same:

- The web app is a public OAuth application of its own at Clerk (no client secret), with
  Clerk's consent screen and exactly one redirect URI, `https://app.emberhut.com/oauth/callback`.
  A second application, rather than a second redirect on the desktop's, keeps ADR 0007's rule:
  each application has one HTTPS redirect, which only the origin or app it names can receive.
- Sign-in is a top-level redirect to Clerk's authorization endpoint (from its OIDC discovery),
  with PKCE (`S256`, a 32-byte verifier), a random `state` and the scopes `openid` and
  `offline_access`. Back on `app.emberhut.com`, the app checks `state` and exchanges the code and
  the verifier at the token endpoint with `fetch`. Setup checks that Clerk's token endpoint
  answers CORS for that origin. If it doesn't, the directory offers a code-exchange endpoint
  that forwards the code and the verifier, and later `refresh_token` grants, to Clerk's token
  endpoint and returns its answer. It forwards only to that endpoint, only for the web client
  id and its one redirect URI, and answers CORS only for `https://app.emberhut.com`.
  That is acceptable for a public client: there is no secret to keep, PKCE still binds the code
  to the app that started the sign-in, and the directory is the one service those tokens are
  for.
- The tokens are JWT access tokens with the desktop's 10-minute lifetime, and a refresh token,
  kept in IndexedDB beside the device's tokens. The web build has its own client id. The
  directory accepts access-token-form JWTs from exactly two client ids, the native
  application's and the web application's, matched by `client_id` (or `azp`), and refuses ID
  tokens from both (ADR 0007, section 5, as amended).
- **The refresh token.** Setup (parts 12 and 15) confirms that Clerk rotates OAuth refresh
  tokens on every use with reuse detection (a refresh token used twice ends the grant), and sets
  their lifetime to 30 days idle and 90 days at most. If Clerk can't do both, the web app asks
  without `offline_access` and signs in again by redirect whenever its account session ends.
  Script injected into the app's origin could take the refresh token, and with it lasting
  account access: list the machines, unlink them, and link a machine of its own. It still has
  no way into a machine (ADR 0007's account grants none), and the access tokens stay at 10
  minutes.
- No Clerk script and no token frame exist on any page the app loads. What Clerk controls is the
  tokens it issues, and those reach only the directory.
- The directory allows CORS from `https://app.emberhut.com` only (directory.md).

**How the browser carries HTTP inside Noise.**

- `packages/connect` opens a WebSocket to the relay (`wss://<relay domain>/v1/sealed/<machine
  id>`), runs the handshake, and runs yamux and HTTP/1.1 inside, as the native clients do. The
  relay ignores `Origin`: through it a page can do nothing it couldn't do with any WebSocket,
  since the machine admits only sealed sessions.
- **A fetch layer.** The app's data layer (`packages/api-client`) calls a `sealedFetch` with the
  shape of `fetch`, which `packages/connect` provides per machine. Compressed answers are
  decoded with `DecompressionStream` where the platform has it, and `fflate` otherwise.
- **One tab owns each session.** The tab that holds a Web Lock named for the machine
  (`navigator.locks`) opens the session; other tabs send their requests to it, and get their
  answers, over a `BroadcastChannel` of the same origin. When that tab closes, the lock passes to
  another, which opens a new session. A person with several tabs open then costs the machine one
  session, within its four per device.
- **No service worker.** A service worker is stopped when it has been idle for a short while, so
  it can't hold a session, and would pay a handshake each time it wakes. It would also be one
  more context with access to the keys and tokens.
- **Media.** Pictures and files up to 64 MiB are fetched through the fetch layer into `blob:`
  URLs. Audio and video stream through Media Source Extensions where the format allows, fed from
  the inner stream. Anything larger is saved to disk as it arrives (through the File System
  Access API where the browser has it), never held whole in memory.

**Where keys live.**

- **The device's identity key** is a WebCrypto Ed25519 key made with `extractable: false` and
  kept in IndexedDB, which stores the key object without its private bytes. Script on the origin
  can sign with it while the page is open, but can't copy it out. **A browser without WebCrypto
  Ed25519 gets no relay route**: an identity key stored as bytes could be copied out by any
  script that ever runs on the origin, and used from anywhere for as long as the row lives.
- **The static key** is a non-extractable WebCrypto X25519 key, made per page load and never
  stored. Where WebCrypto has no X25519, it is an `@noble/curves` key in memory, which lives no
  longer than the page and dies with its statement.
- Tokens are kept in IndexedDB beside the key.
- **Persistence.** The app asks for persistent storage (`navigator.storage.persist()`). A browser
  may still evict the origin's storage (under storage pressure, or after a period without use,
  as some browsers do for script-written storage); the device is then a new device and pairs
  again. Connections shows whether storage is persistent.

**A machine's HTML runs on a site of its own.** A machine's HTML (a report, ADR 0006) is shown
in an iframe whose document is a static viewer page on a separate registrable domain used for
nothing else, with `sandbox="allow-scripts"` and none of `allow-same-origin`,
`allow-top-navigation`, `allow-popups`, `allow-forms` or `allow-modals`. The app posts the HTML
to the viewer, which renders it under a Content Security Policy of its own, with no network
access. Another site means another process where browsers isolate sites, which bounds
Spectre-style reads of the app's memory; the sandbox gives it an opaque origin, with no storage
and no access to the app. The app accepts from the frame only messages from that frame's window,
of the expected shape (a height, a link to open, which the app shows before it opens anything),
and never sends it a key or token.

**The app's headers.**

- Content Security Policy: `default-src 'none'; script-src 'self'; style-src 'self';
  img-src 'self' blob:; media-src 'self' blob:; font-src 'self'; connect-src 'self' https: wss:;
  frame-src https://<the viewer's site>; worker-src 'self'; object-src 'none'; base-uri 'none';
  form-action 'none'; frame-ancestors 'none'; require-trusted-types-for 'script';
  trusted-types emberhut`. `connect-src` allows `https:` and `wss:` to any host: the app's own direct
  routes are `https` URLs of the person's machines (a public name, a VPN's HTTPS name), a person
  may add any self-hosted relay, and Clerk's token endpoint and the directory are `https` too.
  None of them can be listed ahead of time. What crosses is sealed, or goes to an `https` route
  the probe has bound to the machine (ADR 0007). So `connect-src` here is no barrier against
  data leaving the page: script that runs on the origin can send anything to any `https` or
  `wss` host. The barriers are that no such script runs (the rest of this policy and Trusted
  Types) and that keys can't be copied out. `http:` and `ws:` stay excluded, as mixed content
  is anyway.
- Trusted Types (where the browser has them), so no string reaches an HTML sink without the app's
  one policy.
- `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Resource-Policy: same-origin`,
  `Origin-Agent-Cluster: ?1` and `Cross-Origin-Embedder-Policy: require-corp`, so the app is
  cross-origin isolated, in a process of its own where the browser can give it one. COEP stays
  because the only cross-origin subresource the app embeds is the viewer's page, which we serve
  with `Cross-Origin-Resource-Policy: cross-origin` and a COEP of its own. Everything else it
  loads is its own bundles, and its requests to routes, relays, Clerk and the directory are CORS
  `fetch` and WebSockets, which COEP doesn't block.

**A browser without that app gets no relay route.** Opening a relay URL in a browser shows no
machine page: the relay serves none. The web app's direct routes stay as ADR 0007 says: only
`https` ones, since a page on an HTTPS origin can't fetch plain `http`. The machine's direct
listeners answer them with CORS for `https://app.emberhut.com` and a self-hoster's own origins
(api.md, "CORS on direct listeners"), which part 2 builds.

The native clients keep their identity keys where they keep their tokens: the desktop beside its
device sessions, the phone in the Keychain or Keystore through `expo-secure-store`. Static keys
are in memory, as on every side.

### 7. What this changes in the relay

**The relay becomes a pipe for device traffic.** For each device connection it routes by the
machine id in the path, copies bytes both ways without reading them, and enforces limits on what
it can see. It parses HTTP only for its own requests: the two WebSocket upgrades and the
directory's API. That removes from ADR 0007's relay:

- ending HTTP/1.1 and HTTP/2 for clients, ALPN `h2`, and translating RFC 8441 WebSockets;
- routing by `Host`, and the 421 for connections reused across names;
- writing `Forwarded`, removing `Emberhut-Auth` and `Emberhut-Relay-Error` from machine answers, and
  dropping `Set-Cookie` with `Domain`;
- the per-request timeout, head-size limit and request counts;
- the token hashes of the verified and pending budgets.

**One host name.** The per-machine subdomain, its wildcard certificate and the Public Suffix
List entry existed so that a machine's pages ran in browsers on an origin, and a site, of their
own (ADR 0007 section 3). A browser no longer loads a machine's origin through the relay at all
(section 6), so none of them is needed:

- The relay serves one name, the **relay domain**: machines dial `/v1/uplink`, devices open
  `/v1/sealed/<machine id>`, and the directory answers at `/v1` (relay.md). The term "relay
  domain" stays, and keeps its place in the signed statements, but it now names one host.
- **One certificate for one name**, from any ACME client with any challenge (HTTP-01 or
  TLS-ALPN-01). DNS-01, a DNS provider's API credentials and a wildcard record are no longer
  needed, for emberhut.com or for a self-hoster.
- **No Public Suffix List entry.** There is no machine site left to separate, and the slowest
  setup step (weeks of review, then months until browsers ship the list) goes.
- **The machine id leaves DNS and SNI.** It is now in the path, inside TLS, so only the relay
  sees it on a relayed connection, not the network on the way. The id is still lowercase base32
  with no hyphen, so it remains safe as a label for anything that uses it as one.
- **Still a registrable domain of its own** for the emberhut.com relay, not a name under emberhut.com.
  The relay box is the most exposed host the project runs, and a host under emberhut.com would be
  same-site with the web app and the account pages. A compromised box could then serve pages
  that `SameSite` cookies treat as same-site, so the account pages' cookies would go with
  requests those pages make (cross-site request forgery that `SameSite` no longer stops); it
  could set cookies for emberhut.com that the account pages then read (cookie tossing); and it would
  receive any cookie scoped to the parent domain. A domain of its own costs the same ≈ $1 a month
  as before. ADR 0007's open question 1 is resolved this way, without the Public Suffix List
  entry. A self-hoster's relay can use any name, under their own domain included, since it serves
  no pages.

**The front, past one box**, can no longer route by SNI, since every connection names the same
host. It ends TLS and routes the upgrade request by its path (`/v1/uplink` to the least-loaded
box, `/v1/sealed/<id>` to the box holding the id, the directory's paths to the directory), then
splices the connection to the box. The hop from front to box is TLS of its own (a private CA the
fronts and boxes share), and carries the client's address in a PROXY protocol header, which a box
accepts only from the fronts' addresses. The front sees no device-to-machine content, as a box
sees none. It does the TLS work of every connection, which is counted when it is built (ADR
0007's open question 7).

**What the relay still sees, and so where its checks move.**

- **The uplink's authentication** is unchanged: the machine's signature over the relay's nonce,
  the id derived from its key, the allow list or the directory's account check, one connection
  per id, and the close codes.
- **Marks from the machine.** When a handshake completes, the machine tells the relay on the
  uplink which kind the device connection is: `device`, `pairing`, or `declined` for one it
  turns away for a reason that isn't the client's fault (busy, or a version it doesn't speak). It
  is a new control message; peers ignore unknown control messages, so the uplink stays at
  version 1. A connection the machine hasn't marked within 15 s, or that carries more than
  16 KiB either way before its mark, is closed. A connection that ends unmarked counts as a
  failed handshake for its client's address. A machine that marks falsely only spends its own
  budget.
- **Relay tickets.** A paired device's handshake looks the same to the relay as anyone's until
  the machine marks it. So that a flood of handshakes can't keep paired devices from connecting,
  the machine gives each device session a **ticket** in its `accept` record (and in the answer
  of a pairing that succeeds): a statement the machine signs, naming the machine, the relay
  domain, a random ticket id and an expiry 24 hours out (api.md). The device shows it to the
  relay as the first message of its next connection. The relay checks it against the key the
  machine authenticated with, and counts the connection on a ticketed budget, with limits per
  ticket. A ticket gives no access to the machine, only relay budget. It takes over ADR 0007's
  verified budget, without the relay seeing any token.
- **Tickets are retired.** Each device row has one live ticket: when the machine issues a new one
  it tells the relay to retire the row's previous one (a `retire` control message). A connection
  that showed a ticket and then ends unmarked retires that ticket too, so a ticket taken from a
  device that was signed out stops counting after one try. The relay keeps retired ids until
  they expire.
- **Limits count connections and bytes**: device connections open and opened per machine, per
  ticket, and per client address (an IPv4 address or an IPv6 /64, with a /48 tier, as before),
  connections without a ticket and pairing connections on a small budget of their own, failed
  handshakes per client address, bytes per machine, and the same buffer ceilings. relay.md has
  the table.
- **`Emberhut-Relay-Error` moves to close codes.** A browser can't read the status or headers of a
  refused WebSocket upgrade, so the relay accepts a well-formed upgrade and then closes with a
  code and a reason: `offline`, `limit`, `full`, `timeout`, `idle`, `reset`, `moving` and the rest
  (relay.md). Each code means one thing on both the uplink and device connections.
  `Emberhut-Relay-Error` stays only on the relay's plain HTTP refusals before an upgrade.
- **Planned restarts** send device connections a `drain` text message, as machines get one: the
  device opens a new session within the window and moves its requests to it.
- **Observability** keeps aggregates only: connections by kind and close code, bytes each way,
  time to the mark, handshake failures. There are no per-request metrics or log lines, since
  there are no requests to see.

### 8. Threat model

For relayed routes, ADR 0007's "after" column is now the only one; its section 6 is amended to
say so. Direct routes keep ADR 0007's rows. This table covers relayed routes, and adds the
parties ADR 0007 didn't list.

| Who | Can | Can't |
| --- | --- | --- |
| A malicious relay: its operator (emberhut.com, or whoever runs one), or whoever has broken into the box | see the metadata of section 3 (which machine, the client's address, when, how long, bytes and record sizes, device or pairing, a ticket id); drop, delay or cut sessions; send forged `drain` messages and relay close codes, which make devices reconnect or try other routes; open handshakes and pairing attempts of its own, within the machine's limits and brake; as a stand-in machine for a device pairing without a link, try four guesses at that device's code per attempt | read or change tokens, requests, answers or pairing codes; replay, reorder or drop records unnoticed; act as the machine to a device (that needs the machine's identity key) or as a device to the machine (its identity and static keys); swap keys at pairing (the link's id, or the PAKE bound to the handshake hash); learn which device connects (XX sends the device's keys only to an authenticated machine); learn a code from a pairing attempt (one online guess per slot, braked on the machine); switch a session to a readable mode (there is none); forge a machine's `close`, so make a device sign out |
| A malicious directory or account provider (Clerk, or anyone with the account's sign-in) | as ADR 0007: list, unlink, link a machine of its own; also list a machine under a false id or relay URL; in a browser, hand the web app account tokens of its choosing | get a device to pair with a machine of its choosing: pairing from the account's list runs the PAKE, so it completes only with the machine whose code the person typed; get into any machine; reach the web app's keys, device tokens or traffic: no Clerk script runs on the app's origin, which gets account tokens by OAuth with PKCE |
| Someone with a device's identity key and its tokens (its files, or the unlocked device) | use its relay route as the device until it is signed out, like its direct routes | keep it after sign-out: the machine forgets the key and closes the device's sessions at once; use the key alone (requests need a live token) or the tokens alone (the relay route needs the keys) |
| Someone with only a static key, read from a process's memory | complete new handshakes as that side until its statement expires, at most about three days (as the machine, only where it also carries the device's connection) | read any session, past or present: session keys also come from ephemeral keys; act as a device without its tokens |
| Someone who reads a session's keys from a running process | read and change that session from then on, until it ends (at most 24 hours) | read what the session carried before its last rekey, or any other session |
| Someone with a copy of `machine.key` | as ADR 0007: take the uplink (the real machine and the copy keep displacing each other with 4409), complete handshakes as the machine, and so collect the tokens and requests of devices that reach it through the relay while it holds the uplink | decrypt sessions it didn't take part in, past or present: their keys come from ephemeral keys (forward secrecy); recovery is a new key, as ADR 0007 says |
| Someone with a copy of the machine's database | pair a device of their own through the relay with an outstanding code's hash, within its ten minutes, with no cracking (section 5) | use a used or expired code; use any token (they are stored hashed, ADR 0002) |
| A compromised machine | read everything its own devices send it (it is the end point) | act as its devices towards other machines: it never holds their identity or static private keys, and a device's statement is useless without its static private key; run script in the web app's origin (its HTML runs on the viewer's site, sandboxed) |
| The web app's origin: whoever controls the script `app.emberhut.com` serves (its operator, or anyone who compromises its hosting or build) | read the traffic of the machines a browser reaches through that app, and sign with the device keys held there, while the person uses it | reach native clients (desktop, phone), which carry their own code; take the keys out of a browser that holds them as non-extractable keys, and use them once the page is closed; change the bundles without the change showing against the published hashes; reach machines through browsers that don't load the app |
| A flaw in an implementation: snow, our TypeScript Noise, or our CPace | whatever the flaw allows, up to reading or changing sessions, or pairing without the code | go unnoticed by the vectors: snow and the TypeScript state machine must produce the same bytes, and both CPace implementations must reproduce the draft's vectors, so a flaw that changes any output fails CI. A flaw that leaves the bytes right (a timing leak, a missing check on input no vector covers) is what review and the edge-case vectors (low-order points, non-canonical encodings, malformed frames) are for. snow is used as released, with its advisories followed by `cargo deny`; the in-house code is the small part that has no mature implementation |
| The network between a device and the relay, or the relay and a machine | see that a device talks to a relay, and the sizes and timing of the TLS records | see the machine id (it is in the path, inside TLS); read or change anything inside |

### 9. Costs

- **The relay domain** costs what ADR 0007 planned, about $1 a month, without the Public Suffix
  List entry. The wildcard certificate becomes an ordinary one, still free.
- **The web app's viewer site**, when the browser client ships, is one more registrable domain,
  about $1 a month, with a static page on the emberhut.com site's hosting.
- **Traffic.** A record adds 21 bytes (its length, type byte, body length and tag) to at most
  16 KiB, and the inner yamux 12 bytes a frame: about 0.2% on large bodies. A 304 on the poll is
  about 0.5 KB with headers; the yamux frames and records around it add about 100 bytes. A
  handshake, with the preambles and the `accept` record, is about 550 bytes per device connection,
  beside the TLS and WebSocket set-up. ADR 0007's traffic estimates hold within about a fifth for
  the poll, and within a few percent for everything else.
- **The relay's CPU** goes down: no HTTP parsing for devices, no HTTP/2, no header rewriting.
- **The machine** does one handshake per device connection, under a millisecond.
- **The build** carries a second curve25519-dalek and sha2 for snow: some compile time and a few
  hundred KiB of binary, until snow moves to the newer versions.

### 10. The work that follows

The parts are in ADR 0007's section 9 table, which this ADR amends: part 13 (sealed sessions in
Rust), part 14 (sealed sessions in TypeScript), part 15 (the browser's sealed fetch, later) and
part 16, which brings part 1's contract (built against ADR 0007 alone) to this ADR: the `sealed`
list, the relay route's URL, probe bindings, the relay's one host name and close codes, and the
ticket and device-key statements. Parts 2, 4, 5, 10 and 13 come after part 16, and part 13
after parts 2 and 3 too. The relay route in part 4, the uplink and linking (part 7)
and the phone's relay route (part 11) depend on it, through part 14 for TypeScript. The gateway
(part 5) is built as the pipe of section 7 from its start, and needs nothing of `emberhut-sealed`.
No relay route ships before sealed sessions do. Machine linking may (ADR 0007, section 9):
with no relay, a link only lists the machine in the account.

## Alternatives

- **A readable relay first, sealed sessions later** (ADR 0007's plan). It gets the relay out
  sooner, but tokens and content are readable by the relay until then, clients that predate
  sealing would need the readable mode indefinitely, and a device couldn't tell which mode a
  relay runs. Rejected for the reasons of section 1.
- **A WireGuard-style tunnel per device** (userspace WireGuard, such as boringtun). Its
  handshake is itself Noise (IKpsk2), but it carries IP packets, so both ends would need a TCP/IP
  stack in user space, the browser one in WebAssembly. It is built for UDP and roaming, which a
  WebSocket through a relay doesn't use, and it has no story for pairing with a short code. The
  part that matters, a Noise handshake between pinned keys, is used directly.
- **MLS-style group keys** (RFC 9420). MLS solves agreement on keys for groups of many members
  who join and leave asynchronously, with a delivery service and group state. A device and a
  machine are two parties, both online, for the length of a connection; MLS's credentials, group
  state machine and epochs would be carried for nothing a two-party session needs.
- **TLS with pinned self-signed certificates, through the relay** (TLS inside the WebSocket,
  certificates or RFC 7250 raw public keys pinned to the identity keys). rustls would serve the
  Rust side well. But a browser can't run TLS over a WebSocket: it would need a TLS stack in
  JavaScript or WebAssembly, and none is maintained and vetted for that. The phone would need one
  too, or a native module. TLS has no standard way to bind a PAKE for code pairing, and its
  certificate path, client authentication and session tickets are much more than a two-party
  session with pinned keys needs. Noise is a small state machine in each language, and binds the
  statements and the prologue directly.
- **Iroh now.** Iroh's connections are QUIC with end-to-end TLS keyed by Ed25519 node ids, and
  the machine's key could be its node id. But QUIC needs UDP, which many networks that allow
  HTTPS block; Iroh then falls back to its own relays, which replace ours rather than seal them.
  Browsers reach it only through those relays, from WebAssembly, and the phone needs a native
  Rust module. It remains a later route (ADR 0007): its connections are already end-to-end, so
  an Iroh route needs no sealed session on top.
- **Noise written in-house in Rust,** on the workspace's curve25519-dalek 5 and sha2 0.11, to
  avoid a second version of each. Weighed in section 2: it would trade compile time and binary
  size for hand-written cryptography beside a mature implementation.
- **A CPace crate** (`pakery-cpace`, `pake-cpace`, `cpace`). Weighed in section 5.
- **Noise IK, or IK with XX as a fallback (Noise Pipes).** Weighed in section 2.
- **A session per request.** Weighed in section 3.
- **HTTP/2 inside the session** instead of yamux and HTTP/1.1. hyper could serve it on the
  machine, but ureq on the desktop speaks only HTTP/1.1, and no JavaScript HTTP/2 client runs
  over a custom byte stream. yamux is already in the uplink.
- **The X25519 form of the Ed25519 identity keys** instead of separate static keys. One key fewer,
  but one key would serve two purposes, and non-extractable WebCrypto keys can't be converted.
- **Clerk bundled into the web app at a pinned version.** Weighed in section 6: it would put a
  large third-party library beside the device keys.
- **A service worker carrying the sessions.** Weighed in section 6.
- **SPAKE2 through RustCrypto's `spake2`.** Weighed in section 5.

## Consequences

- The relay can't read or change what it carries. People on any relay, the emberhut.com one
  included, trust it only with metadata, and the emberhut.com relay can be open to everyone from its
  first day.
- Browser users also trust whoever serves the web app's script. Native clients don't.
- The relay is simpler to build and run: a pipe for devices, one name, one ordinary certificate.
  Self-hosters need no DNS provider plugin and no wildcard record.
- The relay's limits are coarser: they count connections and bytes, and rest on the machine's
  marks and tickets to tell paired devices from strangers.
- A client must implement sealed sessions to use a relay at all. Tools such as `curl` can't
  reach a machine through a relay; a local sealed proxy in a CLI is a later option.
- Noise runs on snow in Rust and on our own state machine in TypeScript; CPace is our own on
  both sides, since no mature implementation exists. The TypeScript side also carries a yamux
  client and an HTTP/1.1 codec. snow-generated vectors, cacophony's, CPace's draft vectors, ours,
  and an end-to-end test against a machine built from the same commit hold them together.
- New dependencies, each added by the part that first uses it: `snow` 0.10 (minimal features,
  section 2) and `hmac` 0.13 in `emberhut-sealed`, beside `curve25519-dalek` 5 and `sha2` 0.11 for
  CPace, already in the tree; `mio`, `tungstenite`, `yamux` and `ureq-proto` in emberhut-client,
  with rustls and `webpki-roots` as `Direct` has them; `@noble/ciphers`, `@noble/hashes` and
  `fflate` in TypeScript, beside `@noble/curves`. snow brings curve25519-dalek 4 and sha2 0.10
  beside the workspace's versions, accepted in `deny.toml` until snow moves.
- Pairing through the relay uses at most the four newest outstanding codes, and needs one to be
  outstanding before a stranger's handshake goes past `accept`.
- A database copy's code hashes work for relay pairing as they are.
- A device whose row holds no identity key (it paired before keys existed) gets no relay route
  until it pairs again, directly or through the relay.
- Direct pairing that sends a device key must prove it (`keySignature`).
- The machine id no longer shows in DNS or SNI for relayed traffic.
- The front, once built, ends TLS, and re-encrypts to the boxes.
- Direct local routes still send tokens over plain HTTP under ADR 0007's grants, and phones still
  need location permission for them, until direct routes are sealed too (open question 1).

## Open questions

Each one has the recommendation this ADR assumes.

1. **Sealed sessions on direct routes.** Sealing `lan` and `vpn` routes would end plain HTTP
   there, and with it the grants, the network fingerprints and the phone's location permission.
   Recommended: next, in an ADR of its own once relayed sessions have shipped. The machine would
   accept sealed sessions on its direct listener (by the preamble), and the prologue would bind
   the route's kind and address as the probe does. Native clients gain the most. The web app
   gains little: a page on an HTTPS origin may not open `ws:` to a local address (mixed
   content), and a `wss:` local route needs a certificate the browser trusts.
2. **When to pad.** Record lengths show the rough size of requests and answers. Padding inside
   records is allowed from version 1 on, and version 1 sends none. Recommended: pad answers to
   size classes in a later release if traffic analysis turns out to matter; every peer already
   accepts it.
3. **The CPace draft becoming an RFC.** Recommended: stay on draft-irtf-cfrg-cpace-21, whose
   vectors pin the bytes. An RFC that changes them would be a new pairing version, offered beside
   this one.
4. **Rotating identity keys without pairing again.** Recommended: not now; a machine's new key
   means pairing every device again (ADR 0007), and a device's new key means pairing that device
   again. The statement a hand-over will use is fixed now, so its encoding never changes:
   `statement("emberhut key handover v1", [role, old public key, new public key, time])`, signed by
   the old key (api.md). The flow that uses it is a later decision.
5. **Browsers without WebCrypto X25519.** Recommended: allow them, with the static key in memory
   (section 6). Browsers without WebCrypto Ed25519 get no relay route.
6. **Sessions across browser tabs.** Recommended: one tab owns each session, with Web Locks and
   a `BroadcastChannel` (section 6), rather than a higher session cap for browser rows.
