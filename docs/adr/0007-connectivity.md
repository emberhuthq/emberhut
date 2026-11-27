# 0007. Connectivity: direct routes first, our relay last, and account sign-in

- Status: Accepted; amended by [ADR 0008](0008-end-to-end-encryption.md)
- Built so far: `emberhut-identity` and the contract (part 1); the client's transport trait (3); of
  part 2, the machine's key and id, the pairing answer's `machine`, the pairing link and its QR
  code, the probe, `emberhut-server id` and the `report` guard; the account directory with its
  Clerk checks (6); of part 7, the link statement over loopback and `emberhut-server link` and
  `unlink`; account sign-in and QR pairing in the desktop (8) and the phone (the account and
  pairing-link parts of 11), each off unless the build turns it on. Linking a machine ships before the relay: with no
  relay, a link only lists the machine in the account (section 9).
- Still to build: the rest of part 2 (the advertisement, the names a listener answers to, the
  device key and its proof, the uncounted wrong code, `Listener`, CORS and
  `EMBERHUT_WEB_ORIGINS`); the client's routes and hosts (4); the relay's gateway (5); the uplink
  (the rest of 7); Connections (9); the TypeScript routes (10); setup on the providers' consoles
  for release (12); sealed sessions (13, 14); the browser's sealed fetch (15); and the rest of the
  contract after ADR 0008 (16: the relay ticket and device key statements).
- Date: 2026-11-25
- Amended by: [ADR 0008](0008-end-to-end-encryption.md): the relay carries only sealed
  (end-to-end encrypted) sessions between devices and machines, from its first release, and
  never ends HTTP for device traffic; it serves one host name, with no per-machine subdomain,
  wildcard certificate or Public Suffix List entry. The passages that change say so.
- Amends: [ADR 0002](0002-rust-server.md): the pairing answer carries the machine's public key;
  the server keeps one secret of its own, its identity key; `report` mode can't be combined
  with a relay or public URL; a wrong pairing code doesn't count against the brake while no
  code is outstanding; direct listeners check `Host`.
- Amends: [ADR 0003](0003-monorepo-layout.md): two crates join the workspace, `emberhut-identity`
  and `emberhut-relay`, and one package, `packages/connect`.
- Amends: [ADR 0001](0001-workspace-and-quality-bar.md): `emberhut-api` is no longer contract types
  only. It still does no I/O, but it depends on `emberhut-identity`, so the signing and checking of
  the contract's statements, and their crypto dependencies, live behind its types.
- Specified in:
  - [api.md, "Connect"](../api.md#connect): statements, encodings, the probe, the route list,
    the pairing link;
  - [connect.md](../connect.md): the client's route rules, version 1;
  - [relay.md](../relay.md): the relay's wire protocol, version 1;
  - [directory.md](../directory.md): the account directory's API, version 1;
  - [sealed.md](../sealed.md): the sealed sessions of ADR 0008, version 1.

  This ADR holds the decisions and their reasons; those pages hold the formats and numbers.

## Context

A client reaches a server at one base URL (`ClientConfig::new`), over whatever network the
person set up: the same computer, the local network or a VPN. A phone away from home can't
reach a computer behind NAT unless the person runs a VPN or publishes the server themselves.
And a device knows only one address per server, so it can't fall back or move to a better one.

Words used below:

- A **machine** is a computer that runs `emberhut-server`, with its own database and device
  sessions (ADR 0002). It is not a runner machine of `GET /api/machines`. In code, the things
  a client connects to are **hosts** (`hosts.rs`, the desktop's Connections page), so the two
  never share a name.
- A **route** is one way to reach a machine: a base URL, or an SSH port forward.
- The **relay** is a server that machines dial out to. It carries requests to them when nothing
  else works. emberhut.com runs one; anyone can run their own.
- The **account** is an optional emberhut.com account. It finds your machines; it never lets
  anyone into one.

What matters:

- Most of the time a short route exists (the same computer, the local network, a VPN). It is
  faster and more private than any relay, and it costs nothing to run.
- Machines sit behind NAT and firewalls that often allow only outbound HTTPS. A relay that
  machines dial out to works on all of them.
- Browsers isolate by origin: script that runs in a browser for a machine must not share an
  origin with other machines or with code it shouldn't trust (ADR 0008, section 6).
- Self-hosting stays complete: no emberhut.com service is needed for anything.
- Running costs stay flat as the number of machines grows.

## Decision

### 1. The machine's identity

- **The key.** On first start `emberhut-server` makes an Ed25519 key pair from the OS's randomness.
  It keeps it in `machine.key` beside the database (mode `0600`, written to a temporary file
  and renamed; `EMBERHUT_MACHINE_KEY` sets the path). It is the one secret the server keeps that
  is not a hash. Nothing is configured, so ADR 0002's "no secrets to configure" still holds.
  Backups copy it with the database. A machine whose key is lost is a new machine: its devices
  pair again, and it is linked to the account again. There is no rotation short of that
  (section 6 says what a copied key allows).
- **The machine id** is a hash of the public key: 160 bits, written as 32 base32 characters
  (api.md has the exact form). It names the machine in the path of a device's connection to the
  relay (ADR 0008; it was first chosen as a DNS label, and stays fit for one). That makes it:
  - **DNS-safe**: one label, under 63 characters, never `xn--` (no hyphen), and unchanged by
    DNS's case folding;
  - **stable** for the key's life;
  - **not enumerable**: 160 bits;
  - **self-certifying**: whoever shows the public key proves the id. The relay keeps no
    registry to tell machines apart, and a device that knows an id can check that a key
    belongs to it.

  The id is a name, not a secret. It travels in pairing links, route lists and the account's
  list, and the relay sees it on every relayed connection (inside TLS, so the network on the way
  doesn't). Knowing it lets someone reach the machine's probe and pairing routes, and open sealed
  handshakes and pairing sessions through the relay, and nothing else (section 6).
- **Statements.** Everything signed is a length-prefixed, domain-separated statement with one
  fixed byte encoding (api.md). No field can slide into another, and each prefix keeps a
  signature made for one purpose from being valid for another.
- **The pairing keys.** The pairing answer also carries the machine's id and public key, and the
  device pins the key. The device sends its own Ed25519 key, with proof that it holds it (ADR
  0008), which the machine stores on the device's row. Neither key grants access on its own (the
  opaque tokens do). They are the keys that sign the static keys of ADR 0008's sealed sessions,
  so the relay route needs them. Both fields are optional, and readers ignore fields they don't
  know.
- **Pairing by link.** `emberhut-server pair`, and "Pair a device" on a signed-in device, show a
  universal link, `https://emberhut.com/pair#id=…&code=…&route=…`, and its QR code.
  - The code, the id and the routes are all in the fragment, which browsers never send to a
    server, so not even emberhut.com sees the code. Without the app, the page explains how to get
    it, and no script on it reads the fragment.
  - The universal link is bound to the signed app through its associated domain. No custom
    URL scheme is registered for pairing: any app can claim a custom scheme, and which one
    receives the link would be undefined. A pairing code is the credential that lets a new
    device in.
  - The QR code is scanned with Emberhut's own scanner, so the system never routes it.
  - The device pairs over any of the routes, and checks that the key in the answer has the
    link's id. Whoever answers on the way can't put its own key in place.

  Typing an address and a code stays the fallback. Then the device trusts the first key it
  sees (trust on first use), which is accepted, and Connections shows the id so it can be
  compared. `emberhut-server pair` runs as a process of its own, so it prints the routes that the
  configuration gives (section 2), not the running server's live state.
- **`crates/emberhut-identity`** holds the key types, the id, the statement encoding, and signing
  and checking. It does no I/O. emberhut-client, emberhut-server and emberhut-relay all use it, and
  `packages/connect` implements the same encoding against the same vectors (section 2).

**Legacy hosts.** A server that predates this ADR answers pairing without `machine`, and the
probe with 404 or 405. To the client it is a **legacy host**: it keeps exactly the one URL it
was configured or paired with, used through `Direct` (section 4) as today. There is no probe,
race, advertisement or relay, and its plain-HTTP behaviour stays as it is today. Connections
says "Direct address only: this server doesn't support other routes".

Only a first, typed pairing can make a legacy host. A device that pairs from a link (which
names an id), or that already holds a key for the host, refuses an answer without `machine` as
"This server didn't prove it is the machine you meant". Stripping the field can't downgrade it.
The vectors include both cases.

### 2. Routes, and how a client picks one

[connect.md](../connect.md) has the client's rules, with every number and the per-platform
details. It is versioned with the shared test vectors, which both implementations run in CI,
so the Rust and TypeScript clients behave the same. What is decided here, and why:

**The order: loopback, local network, VPN, public URL, SSH, relay.** Shorter paths come first:
they are faster, they keep traffic off other people's servers, and a relayed request costs
whoever runs the relay. SSH (desktop only) comes after a public URL, since starting `ssh` is
costly. The relay is the route of last resort.

**The machine advertises; the account knows only the relay.** The machine lists every route its
configuration gives (`GET /api/connect`), with whether each is up right now.

- An address counts as `vpn` only when it is on a VPN interface. The range alone never decides:
  RFC 6598 shared addresses are also what carrier-grade NAT hands out.
- With the default loopback `EMBERHUT_LISTEN`, no local or VPN address is advertised, so nothing
  reaches the network unless the person opens it.
- The relay route is listed whether the uplink is up or not, and an advertisement never removes
  a route a device knows (routes age out after 30 days unseen). A machine whose relay link is
  briefly down therefore never makes a device forget the relay.
- The account never sees a machine's local or VPN addresses.
- In a container with mapped ports the server can't see the address a device used, so its
  address routes fail the probe's binding. Such machines use host networking or name routes.

**A direct listener answers only to its own names.** On a TCP connection, a request whose
`Host` is not a name the machine answers to gets 421. These names are loopback, the host name
and `<host name>.local`, its listening addresses, the configured routes, and `EMBERHUT_HOSTS` for
other aliases. A web page that uses DNS rebinding to aim a browser at the machine's loopback or
local address therefore gets nowhere, not even to the probe or the pairing route. Names in use
today keep working: host names, `.local` names and addresses need no setting, and the rest takes
one line.

**What "answers" means: a probe bound to the route.** The probe needs no session and carries no
token, since a route must prove the machine before it sees one. The device sends a fresh nonce
and the route kind. The machine signs them together with what it was reached at:

- how: directly or relayed;
- where: the local socket address that accepted a direct connection;
- as what: the host asked for.

A route answers only when the machine's pinned key signed a binding that matches that very
address or name. A server that holds a remembered address and forwards the probe elsewhere (to
the relay, to a public name) gets back an answer bound to the other route, which fails. The
binding doesn't help when the machine really is reachable at that address from where the
attacker is: a global IPv6 address, or a port the router forwards. That case is why plain-HTTP
grants are bound to a network (below).

For the relay route, the sealed handshake does this work instead of the probe (ADR 0008): it
proves the machine's key, and its prologue binds the machine id and the relay domain.

A 401 with `signIn` on the first real request concerns the session, which is the same on every
route, so it doesn't count against the route.

**Racing.** A client probes every known route at once, and takes the best-ranked one that
answered within a 500 ms head start, or else the first to answer. A network the device has just
left fails slowly, by timeout. Waiting the whole timeout for it before using the relay would
show a stall, and 500 ms is several times a local or VPN round trip.

- **Moving back**: while on a worse route, it checks the better ones every 60 s, and on network
  changes and coming to the foreground. At that interval, someone arriving home moves off the
  relay within a minute, and the cost is one small request per route.
- It moves only to a strictly better rank, so routes of equal rank don't flap.
- A route that passes probes but fails real requests is held down for a while.
- Failed races are retried after random waits, so clients that lost the same relay don't probe
  in step.
- **After a network change, no request carries a token** until the new race has finished. That
  way a poll queued before the change can't go to the same address on the new network.

**Tokens over plain HTTP, on direct routes.** (Relay routes carry tokens only inside sealed
sessions, ADR 0008.) They go only to:

- loopback;
- a VPN address while the device's own path to it goes through that VPN;
- a local route the device holds a **grant** for, on the network it is on now.

A grant comes from pairing over the route or from the person allowing it. It is bound to a
fingerprint of the network (its gateway, and its Wi-Fi network where the platform tells),
not to an address.

- Sending the refresh token only over TLS was weighed and rejected: a home that has only a
  local route could then never renew.
- What stays accepted is an attacker on the granted network itself, who can read plain HTTP
  there (section 6).

**Phones without location permission make no grant.** iOS and Android give the Wi-Fi network's
name only with location permission, and hide the gateway's hardware address. Without the
permission a phone's fingerprint would be just "Wi-Fi", which every Wi-Fi network shares. So
such a phone skips plain-HTTP local routes. It asks for the permission only when the person
wants the local network, never at start-up. The fingerprint and the VPN check come from one
native module on the phone (`apps/mobile/modules/emberhut-network`), since JavaScript can't read
them. The web app uses only `https` direct routes, as a page on an HTTPS origin can't fetch plain
`http`, and reaches the relay through its sealed fetch (ADR 0008).

**What the person sees.** A quiet label by the machine's name ("This Mac", "Local network",
"Tailscale", "Emberhut relay"…) that changes when the route does, with no other notice. Connections
shows each route's state and the machine's id. When nothing answers, it says which route failed
how. A relay that carries their traffic is never hidden from them.

### 3. The relay: `emberhut-relay`

`crates/emberhut-relay` is one binary, `emberhut-relay` (a library with a thin `main.rs`, like the
server), built on tokio, hyper, rustls, tokio-tungstenite and yamux. [relay.md](../relay.md)
specifies its protocol, version 1. What is decided here, and why:

**Machines dial out over one WebSocket carrying yamux.** A machine opens one `wss://`
connection to the relay and keeps it. Each device connection is one yamux stream on it, opened
by the relay, carrying a sealed session whose bytes the relay copies without reading them (ADR
0008). The machine serves the requests inside with the same router as on TCP.

- One stream per device connection means the relay keeps no connection pool, a reset ends one
  device's connection and no more, and each gets its own flow-control window, so backpressure is
  per device.
- Outbound HTTPS on port 443 gets through almost every NAT and firewall.

What else was weighed:

- **yamux** (chosen) is a small, stable multiplexer with flow control per stream. It runs over
  any ordered byte stream, a WebSocket included, and leaves the machine an ordinary HTTP/1.1
  server on each stream. An idle session costs little: a few counters and no header-compression
  state.
- **HTTP/2 over the WebSocket** is the runner-up. hyper speaks it, GOAWAY and stream limits
  would be its drain and limit tools, and axum serves WebSockets over it (RFC 8441). But the
  relay would be the h2 *client* on a connection the machine opened, which is unusual for both
  libraries. Each idle machine would also carry h2's flow-control and HPACK state. And
  requests would reach the machine as HTTP/2 rather than the plain HTTP/1.1 per stream it
  serves on TCP. The `Listener` seam (section 4) keeps it possible.
- **QUIC or WebTransport** need UDP, which many networks that allow HTTPS block. That path is
  Iroh's, later.
- **A framing of our own** would have to be written and fuzzed, for nothing yamux lacks.

**The key is the credential.** The machine signs the relay's nonce and domain with its key,
and the relay derives the id from the key. No token or shared secret is involved. Taking an id
off the allow list, or unlinking it in the directory, revokes it. A captured handshake can't be
replayed to this relay or to another one. There is one connection per id: a newer one replaces
the older, so a machine whose network dropped without closing the socket gets back at once.

**One host name** (amended by ADR 0008). The relay serves one name, the relay domain: machines
dial `/v1/uplink`, devices open `/v1/sealed/<machine id>`, and the directory answers at `/v1`.
This ADR first gave each machine a subdomain, with a wildcard certificate and a Public Suffix
List entry, so that a machine's pages ran in browsers on an origin and a site of their own.
Browsers isolate by origin, not by path: cookies, storage, service workers and permissions on one
shared origin would have been shared by every machine. Through the relay a browser no longer
loads a machine's origin at all: it reaches machines only from the web app, through sealed
sessions (ADR 0008, section 6). So one name is enough, with one ordinary certificate, and no
list entry. The relay domain stays a registrable domain of its own, not a name under `emberhut.com`,
so that no emberhut.com cookie and no same-site page reaches the relay box (ADR 0008, section 7).

**Reconnections are spread at random.** Every delay a machine waits is drawn at random from a
window, starting with the first retry (relay.md has the windows). A relay crash or restart
therefore brings 100,000 machines back spread over a minute, about 1,700 handshakes a second,
which the capacity targets below include. Retrying at once, or doubling from a small first
delay, would bring them all back within the same second.

**Topology: one active box, and restarts without an outage.**

- **One active box per relay domain**, with a cold standby. The relay domain's record points
  at a floating address, which moves to the standby by hand or through the provider's API. The
  directory's SQLite file is copied to the standby every 5 minutes (`VACUUM INTO`, then a
  copy), so a failover loses at most 5 minutes of links.
- **A planned restart hands over in place.** The new process takes over the old one's listening
  socket, so no queued connection is reset. The old process tells every machine, and every
  device connection, to move within 10 s, at a random moment, and the new one holds device
  connections for machines that haven't arrived yet. Requests are delayed by at most those 10 s,
  and none fails. relay.md has the steps.
- **Past one box (specified now, built when needed: open question 7)**, a thin front sits on the
  floating address with boxes behind it. Every connection names the one relay domain, so it
  ends TLS (nothing a device sends is readable there either, ADR 0008), reads each upgrade
  request's path, and splices the connection to the box that holds that machine, with the
  directory as the source of truth for which box that is. The alternative was a DNS record per
  machine, managed by the directory. It brings back a name per machine (zone quotas, API rate
  limits, a wildcard certificate), and makes every move and failover wait on DNS caches. The
  front adds a hop and must itself be redundant (two fronts on the floating address), but a
  move takes effect at once.

**Limits** (amended by ADR 0008). relay.md has the table. The relay sees device connections
and bytes, not requests, so it limits those. The principles:

- **Per machine**: device connections open, and new connections a second. That way one
  machine's traffic can't starve the others on the box.
- **Paired devices, strangers and pairing have separate budgets.** A connection counts as a
  paired device's once the machine marks it so on the uplink, after the sealed handshake has
  authenticated the device's keys. Before that, a device that shows a **ticket** (a statement the
  machine signed and gave it in an earlier session) counts on a ticketed budget, with limits per
  ticket. Connections without a ticket, and pairing sessions, get a small budget of their own,
  with a share per client address, so a flood of handshakes from many addresses can't take paired
  devices' connections. An unmarked connection must be marked within 15 s, and carries at most
  16 KiB each way until then.
- **Per account**: machines connected.
- **A client address** is an IPv4 address or an IPv6 /64, everywhere a limit is counted per
  address, since one host usually holds a whole /64. One IPv6 subscriber often holds a /56 or a
  /48, so each per-address share also has a /48 tier, at most a quarter of the machine's
  budget.
- **Per client address, only failed handshakes** (the uplink's, and device connections the
  machine ended unmarked), and the share of the budget without a ticket, with a large burst, so
  machines and devices behind one NAT address are never refused for reconnecting together.
- **Buffers have ceilings** per stream, per machine and per box. A slow client makes its own
  stream wait, never the relay buffer more.

**The relay's own answers are marked** (amended by ADR 0008). The relay closes a device
connection with a close code and a reason word of its own (the machine is offline, over a
limit, too slow, the box is full), which a browser can read where it can't read a refused
upgrade's status. Only its plain HTTP refusals before an upgrade carry `Emberhut-Relay-Error`.
Clients act on the code, never on the text alone, and only the machine's own `close`, inside the
session, speaks for the machine.

**TLS, and what the relay sees** (amended by ADR 0008). The relay ends TLS for its one name with
a certificate from an ACME CA such as Let's Encrypt, by any challenge. The binary reads the
certificate and key from files and reloads them when they change. A standard ACME client renews
them.

The relay sees no request in the clear: every byte between a device and a machine is inside a
sealed session that ends on both of them. It sees which machine, when, how many bytes, the
client's address, and whether the machine marked a connection as a paired device's or a pairing
(ADR 0008 has the list). It can still refuse or drop traffic, one more reason direct routes come
first.

**Capacity targets.** An idle machine costs a socket, a TLS session, a WebSocket, a yamux
session and a task. The budget is 100 KiB per idle machine, user and kernel memory together.
It holds only with tuned buffers:

- tokio-tungstenite's read buffer at 8 KiB rather than its default of about 128 KiB, and its
  write buffer flushed per message, with a 64 KiB cap;
- rustls's buffer limits set;
- the kernel's socket buffers left to grow from their small defaults only while busy.

A load-test binary in the crate (simulated machines, run by hand, not in CI) checks these
targets:

- **The first box**: 1,000 idle machines on 2 vCPUs and 4 GB, in under 200 MB, about a
  twentieth of its memory.
- **One large box**: 100,000 idle machines on 8 vCPUs and 16 GB, in under 10 GB and under one
  core.
- **A restart**: every machine back within 60 s, at that count, after a crash, and no request
  failed during a planned handover.
- **Latency**: under 5 ms added inside the relay at the 99th percentile, at 1,000 device
  messages a second.

How it measures:

- Memory is the process's resident memory plus the kernel's socket memory (`/proc/net/sockstat`).
- One source address reaches one `address:443` from at most about 28,000 to 64,000 ephemeral
  ports. So 100,000 machines take several load generators, or several source addresses on
  each.
- The test turns off the failed-handshake limit and raises `nofile` above 200,000 on both ends.
- One case sends clients from many addresses of a single IPv6 /64, and checks that they share
  one client address's limits and shares, and its count of failed handshakes.

Throughput is bounded by the box's network, and one region adds the distance to it: that is
the price of the route of last resort.

**Observability without content.**

- **Metrics**, in Prometheus text format at `EMBERHUT_RELAY_METRICS` (a private address, off by
  default). They count machines connected, connections and closes by code, handshake failures
  by reason, device connections open by kind, bytes each way, time to the machine's mark,
  buffered bytes, window stalls and limit hits. Every one is an aggregate: none has a machine
  label.
- **Logs** (`tracing`, as in ADR 0002) have one line when a machine connects and one when it
  leaves, with an 8-character prefix of its id, the close code, the duration and the bytes. A
  line per device connection is written only at `debug`, with its kind, close code, bytes and
  duration. No ticket, client address or byte of a session is ever logged.
- Client addresses are held in memory for the per-address limits, and nowhere else.

### 4. Transports behind traits

**In emberhut-client.** Every request today goes through `Http`, which builds URLs from one base
URL. That becomes a trait:

```rust
/// One way to send a request to a machine and read its answer.
pub trait Transport: Send + Sync + fmt::Debug {
    /// Sends `request` (a path and query on the machine) and returns the answer's head,
    /// whose body is read as it arrives.
    fn send(&self, request: Request, timeouts: Timeouts) -> Result<Answer, TransportError>;
}
```

`Request` and `Answer` are emberhut-client's own types over the `http` crate's parts, so `ureq`
stays out of the public API. There are three implementations:

- **`Direct`**: ureq to one base URL. It covers loopback, lan, vpn and public. Legacy hosts use
  only this one. The relay route is `Sealed<RelayDial>` (ADR 0008), not a `Direct`.
- **`SshForward`**: it runs the `ssh` forward of connect.md, then is a `Direct` to its port.
- **`Routes`**: itself a `Transport` in front of the others. It runs the race, the moves, the
  hold-downs and the plain-HTTP rule of connect.md.

`Client` takes a `Transport`, and `ClientConfig::new(url)` still works by making a `Direct`, so
no method changes its signature. `Client::base_url()` is then the base URL of the route in use
(the configured URL before the first race finishes). It is for display and logs only; nothing
builds requests from it. `Sealed<D: Dial>` (ADR 0008, which explains why its parameter is a
dialler of byte streams rather than a `Transport`) and, later, `Iroh` are more implementations,
and callers don't change.

**In emberhut-server.** Serving moves to axum's `serve::Listener` trait (axum 0.8). The TCP listener
is one implementation. The **uplink** is another: its `accept` returns the next yamux stream
from the relay, one per device connection, as an `AsyncRead + AsyncWrite`. The machine serves
it only through `Sealed<L: Listener>` (ADR 0008), which runs the sealed handshake on each
accepted stream and yields the streams inside the session, with a peer that names the device.
The server runs `axum::serve` on the TCP listener and on `Sealed<Uplink>`, with the same router
and the same shutdown token (ADR 0002). Later, an Iroh endpoint's streams are one more
listener. Handlers don't change.

**`report` mode stays private.** With a relay or a public URL configured,
`EMBERHUT_AUTH_MODE=report` stops the server at start-up: "report mode lets anyone in; it can't be
used with a relay or a public URL". Those routes put the machine on the internet.

**End-to-end encryption** is decided in [ADR 0008](0008-end-to-end-encryption.md), which
replaces the sketch this section first held. Every relayed byte is inside a Noise session
(`Noise_XX_25519_ChaChaPoly_SHA256`) that ends on the device and on the machine, with each side's
static key signed by the pairing key the other side pinned, from the relay's first release. There
is no readable mode, a client that can't seal has no relay route, and direct routes are
unchanged.

**Iroh, later.** An Iroh node id is an Ed25519 public key, so the machine's key can serve as its
node id. Iroh routes (direct, with hole punching, and Iroh's relays otherwise) then need no new
identity: one more `Transport` and one more listener.

### 5. The emberhut.com account, with Clerk

**What it is for, exactly.**

- It lists your machines (name, id, relay URL, connected or last seen) on every device you sign
  in on.
- It lets a machine use the emberhut.com relay: the relay's authoriser asks the directory whether
  the machine's id is linked to an account in good standing.
- Nothing else. It never grants access to a machine. A device still pairs with a code and holds
  a device session that the machine made and checks on every request (ADR 0002). The account
  finds the machine; the machine decides who gets in.

A self-hoster needs no account: direct routes and pairing codes work without one, and a
self-hosted relay authorises machines by its allow list. Builds from source ship with the
account off. Official builds turn it on with build-time settings (the Clerk publishable key,
the issuer, the OAuth client id (the native applications' and, for the web build, its own) and
the directory's URL). The publishable key is public by
design, and no Clerk secret is in any app or machine.

**The directory** is part of `emberhut-relay`. `emberhut-relay serve` runs the gateway and the
directory together, and `--role` splits them later (`gateway`, `directory`, `front`). It has its
own SQLite file, which holds accounts (the Clerk user id only) and machines (id, public key,
name, account, when linked, last seen, and the box holding it), and nothing more. Its API is at
`https://<relay domain>/v1`, specified in [directory.md](../directory.md). Every call
carries a Clerk token, except the machine's own unlink:

- `GET /machines` lists the account's machines.
- `POST /machines` links a machine, with a statement the machine signed naming the account's
  user id, the relay domain and the time. The directory checks:
  - that the user id is the token's `sub`, so a statement made for one account can't be used by
    another;
  - that the time is within 5 minutes;
  - that the id isn't linked to another account. If it is, the answer is 409 "This machine is
    linked to another account. Unlink it there first."
- `DELETE /machines/{id}` unlinks it, with the account's token.
- `POST /machines/{id}/unlink`, with no account token, takes a statement signed by the machine
  itself, naming the id, the relay domain and the time. A machine can always leave an account
  it can no longer sign in to (`emberhut-server unlink`).
- **Replays.** The directory keeps every link and unlink signature it has accepted until its
  5 minutes have passed, and refuses it a second time. It also refuses a statement older than
  the machine's latest link or unlink. An unlink captured just before a relink can't undo the
  relink.

**The directory and the gateways.**

- A gateway asks the directory synchronously about an id it doesn't know, batching the ids
  that arrive within a second.
- It keeps known answers for 5 minutes when the directory can't be reached, so a directory
  restart drops no machine. An id it can't get an answer for is told to retry soon (relay.md's
  4502), not that it was refused.
- Unlinks are pushed: each gateway holds a stream from the directory (a channel when they share
  a process), and closes an unlinked machine's uplink at once. A check of all connected ids
  every 30 s is the backstop.

**How a machine is linked.**

- `emberhut-server link` signs in with the device grant (below), signs the statement with the
  machine's key, and posts it.
- The desktop's "Add this Mac to your account" asks the server on the same computer to sign the
  statement, with its device session, for the account it is signed in to
  (`POST /api/connect/link-statement`). The server serves that route only to connections its
  own TCP listener accepted from a loopback peer, with no proxy's forwarding header, never
  relayed or from another address, so a remote device can't make the machine sign for an
  account of its choosing. The desktop then posts the statement with its own account token.
  The server never sees an account token.
  - What the server can't tell apart is a forward on the same computer that adds no header (a
    layer-4 port forward, or a proxy set to add none): it looks local, and is served. That is
    accepted. Whoever reaches the machine through one still needs a device session, which
    already reads and acts on everything the machine holds; the account grants no access to
    the machine; and a linked machine can't be moved to another account without being
    unlinked there first (409). A token in a file only the server's user can read would close
    the gap, at the cost of the desktop reading it; it isn't needed for now.

**Checking Clerk tokens.** Only the directory checks them; emberhut-server needs no Clerk settings
or keys. The check sits behind a small `AccountVerifier` trait, so another provider can replace
Clerk in one module.

- **Keys.** They come from Clerk's JWKS URL
  (`https://<Clerk frontend API>/.well-known/jwks.json`) and are cached by `kid`:
  - The set is fetched again every hour. On an unknown `kid`, it is fetched at most once a
    minute, so tokens with made-up `kid`s can't make the directory flood Clerk.
  - The old set is kept until a fetch succeeds, so a key rotation (a new key published before
    it signs) never fails a valid token.
  - A token whose `kid` is unknown while a fetch is held back by that limit, or while Clerk
    can't be reached, gets 503 with `Retry-After`, never 401. A client doesn't sign out of the
    account because of an outage or a burst of junk.
- **Algorithms**: RS256 only, set in configuration and never taken from the token (no `none`,
  no HS256).
- **Common claims**: `iss` must be the configured issuer. `exp` and `nbf` are checked with 30 s
  of leeway for clock skew, and `sub` must be present.
- **Session tokens** (the phone) must carry the JWT template's `aud` (`emberhut-directory`). An
  `azp`, when present, must be among the allowed origins. Native apps may send no `Origin`,
  from which Clerk fills `azp`, so the check rests on `aud`.
- **OAuth access tokens** (desktop, CLI, and the web app of ADR 0008) must be access-token-form
  JWTs issued to exactly one of two client ids: the native application's (desktop and CLI) or
  the web application's, matched by `client_id` (or `azp` where Clerk puts it there). Any other
  client id is refused (amended by ADR 0008, which gives the web build a client id of its own).
  - An ID token, from the same keys with either client id as its `aud`, is refused. The verifier
    requires the access-token form: `typ: at+jwt` (RFC 9068) if Clerk sets it, or otherwise a
    claim that only access tokens carry. Part 12 checks which, and the vectors test both kinds.
  - Opaque tokens are refused.
- **Lifetimes.** Clerk's session tokens live 60 s, and both OAuth applications' access tokens
  (the native one's and the web app's, ADR 0008) are set to 10 minutes each. Ending a session in
  the account page stops new tokens at once. Tokens already issued live out those few minutes,
  which is accepted.
- **Crate.** `jsonwebtoken` checks the tokens, with its `rust_crypto` backend. That backend
  brings the RustCrypto `rsa` crate, whose RUSTSEC-2023-0071 advisory (a timing leak in
  private-key operations) `cargo deny` reports. The directory only verifies, with public keys,
  so `deny.toml` ignores it with that reason. `aws_lc_rs` was rejected: it needs a C toolchain
  and CMake on every build machine. No Clerk SDK is needed.
  - Amended when part 6 was built: no ignore is needed. jsonwebtoken 12 brings rsa 0.10, in which
    RUSTSEC-2023-0071 is fixed, so `cargo deny` reports nothing.

**On each client.**

- **The phone** uses `@clerk/expo`, with its token cache in the Keychain or Keystore through
  `expo-secure-store`, and passkeys through `@clerk/expo-passkeys`. It calls the directory with
  `getToken({ template: "emberhut-directory" })`.
- **The desktop** treats Clerk as an OAuth 2.0 and OpenID Connect provider, through a public
  OAuth application (no client secret), with Clerk's consent screen required for it. The OAuth
  application has exactly one redirect URI registered: `https://emberhut.com/oauth/callback`, a
  universal link. No custom-scheme redirect is registered. Any app can start its own sign-in
  with our public client id and a custom scheme it has claimed, and receive the code in its
  own session; that works on every macOS version for as long as such a redirect is registered.
  An HTTPS callback on emberhut.com can be received only by the app that emberhut.com's associated
  domain names.
  - **On macOS 14.4 and later**: an `ASWebAuthenticationSession` at the authorization endpoint
    (from Clerk's OIDC discovery), with PKCE (`S256`, a 32-byte verifier), a random `state`, the
    scopes `openid` and `offline_access`, and that HTTPS callback. The app checks `state`, then
    exchanges the code and the verifier for tokens.
  - **On macOS 13 to 14.3**, which can't take an HTTPS callback in the session: the device
    grant, as for the CLI below. The app shows the code in a sheet and opens the verification
    page in the browser. There is no redirect to catch at all. The loopback redirect was the
    alternative, but it would have to be registered on the OAuth application, and any local
    app could then use it on every version (RFC 8252, section 8.6). The device grant's own risk
    is a person entering a code someone else sent them. The consent screen, which names Emberhut
    and the code's request, is the defence there.
  - **The tokens** go in the login Keychain, as one generic-password item of their own (service
    `com.emberhut.desktop.account`), never in `session.json`. Signing out of the account then
    touches no device session, and signing out of a machine touches no account.

  The session is called through `objc2-authentication-services`. Its calls are unsafe, so they
  sit in one small module with an `#[expect(unsafe_code, reason = …)]`, as ADR 0005 does for
  its one system call. Linux and Windows builds will use the device grant too.

  **Associated domains need a provisioning profile.** The HTTPS callback and the pairing
  universal link both rest on the Associated Domains entitlement for emberhut.com. On macOS that
  entitlement is restricted: a Developer ID app that claims it must embed a provisioning profile
  granting it (`Contents/embedded.provisionprofile`), or it won't launch.
  - The release build gets it. Part 8 adds the profile and the entitlement to
    `cargo xtask bundle-macos`, and ADR 0005's publish step signs with them as it signs
    everything else. That is the one change to ADR 0005's signing: the profile is renewed before
    it expires, and publish refuses to run with an expired one.
  - Development builds, ad-hoc signed builds and builds from source carry no profile and claim
    no associated domain. They sign in to the account with the device grant, and pair by
    scanning the QR code in the app or pasting the link or code.
  - The phone's release builds carry the same capability in their App ID, set up with the
    store builds (part 12).
- **The CLI and headless machines** (`emberhut-server link`, later an `emberhut` CLI) use the OAuth
  device authorization grant (RFC 8628). It prints the verification URL and the user code,
  polls the token endpoint at the `interval` given, slows down on `slow_down`, and gives up at
  `expires_in`. It uses the token for the link call and doesn't keep it. Clerk turns this grant
  on per OAuth application, so setup (part 12) checks it first. Without it, a machine is
  linked from a signed-in desktop on that machine, and desktops on macOS before 14.4 can't
  sign in to the account until they update.

**Clerk's plan** is Pro: the free plan has no passkeys and no MFA. Section 8 has its price.

### 6. Threat model

Relayed routes are sealed from the relay's first release (ADR 0008), so this table has one
column for them: what was first written as "after end-to-end encryption" is the only case. Direct
routes keep section 2's rules. ADR 0008's section 8 adds the parties this table doesn't list (a
malicious directory, stolen static keys, the web app's origin).

| Who | Can | Can't |
| --- | --- | --- |
| The relay's operator (emberhut.com for its relay, whoever runs one, or whoever has broken into the box) | see which machine, when, how long, how many bytes and the client's address, and whether a connection is a paired device's or a pairing; link one device's connections to a machine by its ticket; drop, delay or cut traffic | read or change tokens, requests, answers or pairing codes; act as the machine or as a device; swap the machine's key at pairing (the link's id, or the PAKE); reach a machine that isn't connected to it; see traffic on direct routes; pass a probe for a direct route (the answer is bound to being relayed) |
| The network (Wi-Fi, an ISP, a hostile local network) | on a granted plain `lan` route, on the network it was granted on, read everything, tokens included; see metadata on TLS, VPN and relayed routes; block routes, which the client then falls back from | on another network, get a token by holding a remembered address: the grant doesn't apply there (the network fingerprint), and a forward to another route fails the probe's binding. The binding alone covers a phone without a fingerprint, since such a phone makes no grant. Read TLS routes, VPN routes while the VPN is up, or relayed routes; see the machine id of a relayed connection (it is inside TLS) |
| Someone with a stolen device (unlocked, or its files) | use its session and its keys on each machine it paired with, on every route, until another device signs it out (at once, ADR 0002); make pairing codes, and so pair more devices; list and unlink the account's machines with its account token | reach a machine it never paired with; move a machine to another account (link statements are made only for direct loopback connections, and a linked id can't be linked elsewhere); use the relay route after sign-out: the machine forgets the device's key and closes its sealed sessions |
| Malware running as the person on the desktop | read `session.json` (every device session) and the device key beside it, which any process of the user can; ask for the account's Keychain item, which brings up a system prompt naming the item, since its access list names Emberhut's signature. Sealed sessions don't change this: they don't protect against the device's own user | catch the account sign-in's code (the only redirect is an HTTPS one bound to Emberhut), or a pairing link (it's a universal link, and the QR code is scanned inside Emberhut) |
| An app on the phone | — | receive a pairing code or an account code: neither travels through a custom scheme |
| Someone with a copy of `machine.key` (from a backup) | connect to the relay as the machine (the real one gets 4409 and comes back, so the two keep displacing each other); complete sealed handshakes as the machine, since the key signs its static key, and so collect the tokens and requests of devices that reach it through the relay while it holds the uplink; answer probes; unlink the machine with its own signature and link it to an account of their own | read direct routes, or sessions it didn't take part in (their keys are ephemeral); read the database without its copy too. Recovery from a known copy is a new key: devices pair again and the machine is linked again, the accepted cost of having no rotation |
| A compromised machine | read everything its own devices send it (it is the end point); serve hostile pages on its own direct routes; flood the relay, up to the limits | take another machine's id (that needs the other key); act as its devices towards other machines (it never holds their private keys); run script in the web app's origin (its HTML runs on another site, sandboxed, ADR 0008); set cookies for other machines or for emberhut.com (the relay serves no machine pages, and is on a domain of its own); see the account's other machines (machines hold no account token) |
| Clerk, or anyone with the account's sign-in or our Clerk secret | sign in to the directory as the person: list their machines, unlink them (no relay for them), link a machine of their own (it then shows in the list) | get into any machine: there is no device session, and pairing still needs a code from the machine or a paired device; get a device to pair with their machine in place of the person's, since pairing from the list runs the PAKE with the person's code; reach the web app's keys or traffic, since no Clerk script runs on its origin, which gets account tokens by OAuth with PKCE (ADR 0008); touch a self-hoster without the account |
| Anyone who learns a machine's id (pairing links, route lists, the account's list; the relay sees it) | call the probe and the pairing route on any direct route it can reach; open sealed handshakes and pairing sessions through the relay from anywhere; with many client addresses (many IPv4 addresses or IPv6 /64s), use up the machine's budget for connections without a ticket, so that new pairings, and devices without a live ticket (back after more than a day, or a new install), wait or fail there, which takes 8 IPv4 addresses or 4 IPv6 /48s, since one address gets 2 of its 16 connections and one /48 a quarter; while a code is outstanding, hold pairing up with wrong codes (ADR 0002's brake, whose cost is accepted there) | touch connections with a ticket, or the ones the machine has marked as a paired device's; with one client address (an IPv4 address or an IPv6 /64), hold more than 2 of the connections without a ticket; learn a code from a sealed pairing attempt (each is one online guess, counted on the brake); brake pairing while no code is outstanding (wrong codes then aren't counted, since none can be right); anything else without a paired device's keys and a session |
| A web page in the person's browser | nothing on the machine's routes | reach loopback or local routes through DNS rebinding (direct listeners check `Host`); reach a machine through the relay (only a sealed session gets in, and its keys are in the web app's origin or a native client) |

After a device is stolen: sign it out from another device, along with any device paired since
the theft, and end the account session from the account page.

### 7. What a self-hoster runs

- **The server alone.** `emberhut-server`, reached on loopback, the local network, a VPN, or a
  public HTTPS URL. The settings are:
  - `EMBERHUT_LISTEN`, a non-loopback address to serve the local network or a VPN;
  - `EMBERHUT_PUBLIC_URL`, the public name, if there is one;
  - `EMBERHUT_ROUTES`, more routes to advertise;
  - `EMBERHUT_HOSTS`, other names the server answers to;
  - `EMBERHUT_WEB_ORIGINS`, the origins of a self-hosted copy of the web app, which its direct
    listeners answer with CORS beside `https://app.emberhut.com` (api.md, ADR 0008);
  - `EMBERHUT_ADVERTISE`.

  No account and no relay are involved.
- **Behind a reverse proxy** (for the public URL), the proxy must:
  - pass the original `Host` (with nginx, `proxy_set_header Host $host;`, not its default,
    which sends the upstream's own address);
  - not buffer `text/event-stream` (`proxy_buffering off` for the live stream).

  `EMBERHUT_PUBLIC_URL` must name the public host. The server then answers to that name, and the
  probe binds to it. A proxy on nginx's default sends `Host: 127.0.0.1:<port>`. That is a
  loopback name, so the request is served, but the probe binds to `127.0.0.1:<port>` and the
  `public` route doesn't answer. Connections then shows the host the probe answered as next to
  the route, which points at the proxy's setting.
- **With their own relay**, they also need:
  - `emberhut-relay` on a host with a public address and port 443 open;
  - one name whose record points at that host, under any domain of theirs (the relay serves no
    pages, ADR 0008);
  - a certificate for that name, from any ACME client (certbot, lego, acme.sh) with any
    challenge.

  The relay's settings:
  - `EMBERHUT_RELAY_DOMAIN`;
  - `EMBERHUT_RELAY_LISTEN` (default `[::]:443`);
  - `EMBERHUT_RELAY_CERT` and `EMBERHUT_RELAY_KEY`, the certificate and key files;
  - `EMBERHUT_RELAY_MACHINES`, a file of allowed machine ids, one per line, read again when it
    changes;
  - `EMBERHUT_RELAY_METRICS`, if wanted.

  On each machine, `EMBERHUT_RELAY=wss://<relay domain>/v1/uplink`, and `emberhut-server id` prints the
  id to put on the list. `deploy/relay/` has a container image and a compose file.
- **Their own account service** is possible, with their own Clerk application
  (`EMBERHUT_RELAY_CLERK_ISSUER`, `EMBERHUT_RELAY_CLERK_JWKS_URL`, `EMBERHUT_RELAY_CLERK_AUDIENCE`, and
  the same values in source builds of the apps), but it is never needed. The HTTPS callback
  and the pairing universal link belong to the official builds, which emberhut.com's associated
  domain names. Self-built apps sign in with the device grant and pair by scanning or pasting,
  unless their builder sets up an associated domain of their own.

### 8. Costs

This ADR and its first parts create no paid resource. Development uses Clerk's free
development instance, which has every Pro feature, and a relay on a developer's computer. The
estimates below are monthly, for when the emberhut.com relay and account go live. Clerk's prices
come from its public pricing page (25 Nov 2026). VPS prices are typical list prices of small
providers, not a quote for a specific plan, and are confirmed before anything is bought.
Totals use €1 ≈ $1.10.

**Traffic per relayed machine**, worked from the desktop's poll:

- An open app asks for the state every 3 s, 1,200 requests an hour.
- Mostly the answer is `304`, about 0.5 KB with headers and TLS, so about 0.6 MB an hour.
- A change sends the whole state, about 150 KB gzipped. At one change every 2 minutes, that is
  about 4.5 MB an hour.
- The live stream and messages are small beside that. Sealed sessions (ADR 0008) add about a
  fifth to the 304s and a fraction of a percent to everything else.
- That makes about 5 MB per open hour. With two devices open 4 hours a day each, it is about
  1.2 GB a month.

**The assumptions:**

- **First users**: up to 1,000 machines and 1,000 monthly retained users.
- **Later**: up to 50,000 machines and 50,000 users.
- At any moment a third of the machines have no direct route and use the relay.
- Every user counts as retained, which is the worst case on Clerk's bill.

That gives, through the relay:

- first users: 1,000 × ⅓ × 1.2 GB ≈ 0.4 TB a month;
- 10,000 machines: ≈ 4 TB;
- 50,000 machines: ≈ 20 TB, which is about what one box's plan includes;
- about 1,100 requests a second at 50,000 machines, if a fifth of the relayed machines have a
  client open at the peak.

| Item | First users | Later (up to 50,000 machines) | Notes |
| --- | --- | --- | --- |
| Relay VPS | €5–10 (one box, 2 vCPUs, 4 GB) | €30–80 (one active box with 8 vCPUs and 16 GB, and a cold standby) | typical plans include about 20 TB of traffic |
| Traffic over the included amount | €0 | €0–10 | about €1–2 per TB at typical EU providers; other regions include far less |
| VPS backups | €1–2 | €6–16 | snapshots, about 20% of the VPS price |
| Clerk Pro | $25 ($20 billed yearly) | $25 ($20) | flat up to 50,000 monthly retained users, then $0.02 each: 100,000 users ≈ $1,025 |
| Relay domain | ≈ $1 ($10–15 a year) | ≈ $1 | a second registrable domain, with no Public Suffix List entry (ADR 0008) |
| DNS | $0 | $0 | one record, on any free DNS plan |
| Certificates | $0 | $0 | a Let's Encrypt certificate for one name |
| Monitoring | $0 | $0 | the relay's own metrics, read on the box |
| **Total** | **≈ $33–39** (≈ $28–34 with Clerk billed yearly) | **≈ $66–143** | |

For comparison, a hosted tunnel per machine at a list price of $4.50 a month per tunnel past
1,000 free comes to about $40,500 a month at 10,000 machines. Past 50,000 users, Clerk becomes
the largest item. The `AccountVerifier` trait keeps a move to a provider that is cheaper at that
scale down to one module.

### 9. The work that follows

Each part has its own files, and **a file is written by one part at a time**. When two parts need
the same file, the table says which comes first, and the second starts on it only after the
first has merged. That includes files that ADR 0005's parts own. Dependencies are handled as in
ADR 0005: the part that first uses a dependency adds it to its crate's manifest and to the root
`[workspace.dependencies]` together. Those lines, `Cargo.lock` and `deny.toml`'s license list
are shared and only appended to, and they are merged in merge order.

This ADR's own change already writes the first versions of `docs/connect.md`, `docs/relay.md`,
`docs/directory.md` and api.md's "Connect" section, and ADR 0008's change writes
`docs/sealed.md` and amends the others. Later changes to them belong to the parts named below.
Part 1 writes `docs/api.md`'s "Connect" section and `docs/architecture.md` first, and part 3
writes `crates/emberhut-client/src/` first; ADR 0005's parts 1 and 4, which would also write those
files, rebase over them. The "Connect" section is appended at the end of `api.md` and touches no
line of ADR 0005's.

| Part | What | Files it writes | After |
| --- | --- | --- | --- |
| 1. Identity and contract | `emberhut-identity` (keys, ids, statements); the `connect`, `relay` and `directory` types and the two pairing fields; the vectors and their Rust tests; the new crates, package and `deploy/relay/` in architecture.md's tree, crate list and dependency graph | `crates/emberhut-identity/`; `crates/emberhut-api/src/{connect,relay,directory,auth,lib}.rs`; `crates/emberhut-api/tests/fixtures/connect/`, `crates/emberhut-api/tests/contract/{connect,main}.rs`; `docs/api.md`'s "Connect" section and `docs/architecture.md` (before ADR 0005's parts 1 and 4, which rebase over them) | — |
| 2. The server: identity, routes and pairing | `machine.key`; `GET /api/connect` and the probe; the advertisement; the names a listener answers to; the machine in the pairing answer, the device key; the uncounted wrong code; `emberhut-server id`, and `pair` printing the link; the `report` guard; serving through `Listener`; CORS on direct listeners for the web app's origin and `EMBERHUT_WEB_ORIGINS` (ADR 0008, api.md), with a test that preflights every header a client sends, the presence header included | `crates/emberhut-server/src/{identity,lib,app,config,main}.rs`, `routes/{connect,auth,mod}.rs`, `services/{advertise,pairing,mod}.rs`, `openapi.json`; `crates/emberhut-core/src/pairing/{wrong_codes,mod}.rs`; `crates/emberhut-db/src/migration/{mod.rs,<new migration>}`, `crates/emberhut-db/src/{sessions,pairing}.rs` and their entity | 16 |
| 3. The client's transport trait | `Transport`, `Direct`, and `Client` over them, with no change in behaviour | `crates/emberhut-client/src/{transport/,http,client,config,lib}.rs` (before ADR 0005's part 1, which rebases over it) | 1 |
| 4. The client's routes and hosts | the probe check, the race, moving back, hold-downs, grants and fingerprints (behind a trait the desktop implements), `SshForward`, legacy hosts and the refused downgrade, acting on the relay's close codes; the relay route, through part 13's `Sealed<RelayDial>`; several hosts per store; connect.md kept in step. The store's format goes to version 2: it reads version 1, and writes version 2 only once the app says it is healthy (ADR 0005's rule for a new copy) | `crates/emberhut-client/src/{routes/,hosts,session,file_store,renewal}.rs`, `crates/emberhut-client/tests/client/`; after part 3, `lib.rs` and `client.rs`; `docs/connect.md` | 2, 3, 16; its relay route, 13 |
| 5. The relay's gateway | the crate and `serve`; TLS and reloading; the uplink's handshake; yamux and framing; the device pipe of ADR 0008 (routing by the machine id in the path, an opaque byte copy, no HTTP parsing of device traffic), marks, tickets and close codes; the limits and buffers; the takeover; metrics; the allow list; the load test; relay.md kept in step | `crates/emberhut-relay/` (all but `src/directory/`), `deploy/relay/`, `docs/relay.md` | 16 |
| 6. The directory | the account API of directory.md, link and unlink statements and their replay rules, Clerk checks behind `AccountVerifier`, gateway lookups and pushed unlinks, `--role`; the `rsa` advisory's ignore (none was needed: rsa 0.10 fixes it, see section 5) | `crates/emberhut-relay/src/directory/`; after part 5, `crates/emberhut-relay/src/{lib,cli}.rs`; `deny.toml`'s advisory list; `docs/directory.md` | 5 |
| 7. The machine's uplink and linking | dialling, the handshake, framing, the `Listener`, backoff, drain and close codes; serving the uplink only through part 13's `Sealed<L>`, and marking device connections; `emberhut-server link` and `unlink` with the device grant; `POST /api/connect/link-statement` over loopback only | `crates/emberhut-server/src/{uplink/,link.rs,oauth.rs,routes/link.rs}`; after part 2, `crates/emberhut-server/src/{lib,app,config,main}.rs`, `routes/mod.rs`, `openapi.json` | linking: 2, 6; the uplink: 2, 5, 6, 13 |
| 8. Desktop sign-in | `ASWebAuthenticationSession` with the HTTPS callback; the device grant for macOS 13 to 14.3; the Keychain item; "Add this Mac" (which calls part 7's link-statement route); the pairing universal link | `apps/desktop/src/account/`; after ADR 0005's part 2, its registration in `apps/desktop/src/{main,app}.rs` and `apps/desktop/Cargo.toml`; after ADR 0005's part 4, the associated domain, its entitlement and the embedded provisioning profile in `xtask/src/bundle_macos/` | 6, 7 |
| 9. Desktop Connections | a new Connections page (hosts, routes, labels, grants, ids, the account's machines); the macOS fingerprint; one configured server becomes hosts. The page of runner machines in `ui/machines/` stays as it is | `apps/desktop/src/ui/connections/`, `apps/desktop/src/store/connections.rs`, `apps/desktop/src/network.rs`, `apps/desktop/src/backend/client.rs`; after ADR 0005's part 2, `apps/desktop/src/{config.rs,store/mod.rs}`; after ADR 0005's part 3, `apps/desktop/src/ui/{mod,settings}.rs` (a Connections entry) | 4, 8 |
| 10. The TypeScript routes | `packages/connect`: the rules of part 4, the statement encoding, Ed25519 through `@noble/curves`, the platform interface for fingerprints and the VPN check, acting on the relay's close codes, the vectors; after part 4, connect.md's changes for TypeScript. It also creates the `js` CI job that ADR 0003 plans, if no part has yet, with this package's tests in it | `packages/connect/`; after ADR 0005's part 6, `.github/workflows/ci.yml` (the `js` job); after part 4, `docs/connect.md` | 16 |
| 11. The phone | `@clerk/expo`; the `emberhut-network` Expo module (fingerprint, VPN check), its entitlement and permission strings; Connections and the route label; the in-app QR scanner and the pairing universal link | `apps/mobile/src/{account,connect}/`, `apps/mobile/modules/emberhut-network/`; after ADR 0005's part 8, `apps/mobile/package.json` and `apps/mobile/app.config.ts` | 6, 10, and the mobile app moving into the repository; its relay route, 14 |
| 12. Setup (no code) | the relay domain (a registrable domain of its own, served at one name, ADR 0008); the Clerk application on its development instance; the OAuth application with the consent screen, the one HTTPS redirect, the device grant, JWT access tokens and their 10-minute lifetime; later, with part 15, the web app's OAuth application, whether Clerk's token endpoint answers CORS, and its refresh-token rotation and lifetimes; which claim marks an access token; the JWT template; `apple-app-site-association` on emberhut.com for the callback and the pairing link, and the page at `emberhut.com/pair`; the Developer ID provisioning profile with the Associated Domains capability, and the same capability on the phone's App ID; the VPS and Clerk Pro only when the relay goes live | the providers' consoles; the emberhut.com site | — |
| 13. Sealed sessions in Rust | `emberhut-sealed`: the handshake, hellos, records, rekeying and close codes, and the CPace pairing exchange, the Noise handshake on `snow` 0.10 (minimal features, with a resolver wrapper that refuses an all-zero key agreement) and CPace written on `curve25519-dalek` 5 and `sha2` 0.11, against snow-generated, cacophony's and the CPace draft's vectors; the `deny.toml` skip for snow's curve25519-dalek 4 and sha2 0.10; `emberhut-identity`'s static-key statement; in emberhut-client, `Sealed<D: Dial>` and `RelayDial`, with the I/O thread that drives TLS, the WebSocket, the records and yamux; in emberhut-server, `Sealed<L: Listener>`: the handshake per accepted stream, the device found by its key, pairing sessions, keys forgotten at sign-out and their sessions closed, tickets in `accept` and their retirement, early data held until `accept`, tokens bound to the session's device; the device key's proof at direct pairing; the sealed pairing routes; sealed.md's vectors and their Rust tests; sealed.md kept in step | `crates/emberhut-sealed/`; `deny.toml`'s `[bans]` skip; after part 1, `crates/emberhut-identity/src/`, `crates/emberhut-api/src/{sealed,lib}.rs`, `crates/emberhut-api/tests/fixtures/connect/sealed/`, `crates/emberhut-api/tests/contract/{sealed,main}.rs`, `docs/architecture.md` (the new crate), `docs/sealed.md`; after part 2, `crates/emberhut-server/src/{sealed/,routes/pair_sealed.rs,lib.rs,app.rs}`, `routes/{mod,auth}.rs`, `services/pairing.rs`, `openapi.json`, `crates/emberhut-core/src/pairing/`, `crates/emberhut-db/src/{sessions,pairing}.rs` and a migration (the device key's unique index); after part 3, `crates/emberhut-client/src/transport/{sealed/,mod.rs}` | 2, 3, 16 |
| 14. Sealed sessions in TypeScript | in `packages/connect`: the Noise state machine on `@noble/curves`, `@noble/ciphers` and `@noble/hashes`, with an asynchronous static-key interface; records; CPace on ristretto255; the yamux client and the HTTP/1.1 codec, with gzip decoding; the relay's WebSocket, tickets and close codes; a sealed fetch per machine; the vectors; an end-to-end test against a machine built from the same commit | after part 10, `packages/connect/`; after part 13, `docs/sealed.md` | 10, 13 |
| 15. The browser's sealed fetch (later: the first release has no browser client) | the web app on its own origin (`app.emberhut.com`), with its headers and published bundle hashes; account sign-in by OAuth with PKCE, as an OAuth application of its own with its own client id, and the refresh-token check of ADR 0008 (rotation with reuse detection, 30 days idle and 90 at most, or no `offline_access`); its data layer on part 14's sealed fetch; one tab owning each session (Web Locks, `BroadcastChannel`); media through `blob:` URLs, Media Source Extensions and streamed downloads; keys as non-extractable WebCrypto keys in IndexedDB, persistent storage; a machine's HTML on the viewer's site | `apps/web/src/connect/`; the web app's hosting configuration; the viewer page; the web app's OAuth application at Clerk (setup) | 14, and the web app itself |
| 16. The contract after ADR 0008 | part 1's contract brought to ADR 0008: the route list's `sealed` list of versions; the relay route as `wss://<relay domain>/v1/sealed/<id>`; probe bindings without a relayed arm; one relay host name with `/v1/uplink`, the device close codes, `mark`, `retire` and 4410's reason word; the `emberhut relay ticket v1` and `emberhut device key v1` statements (and the identity-key hand-over statement's format) in `emberhut-identity`; the vectors, their generator and api.md's examples to match | `crates/emberhut-identity/src/`; `crates/emberhut-api/src/{connect/,relay,auth,lib}.rs`; `crates/emberhut-api/tests/fixtures/connect/`, `crates/emberhut-api/tests/contract/connect.rs`; `crates/emberhut-server/openapi.json` | 1 |

The order: part 1; then part 16; then parts 2, 3, 5 and 10 in parallel; then 4, 6 and 13; then 7
and 14; then 8 and 11; then 9. Part 4's relay route merges after part 13, and part 11's after
part 14. The relay route ships only with sealed sessions: it doesn't ship before part 13 has
merged, and no TypeScript client gets a relay route before part 14. Machine linking may ship
before them: with no relay, a link only lists the machine in the account. Part 15
waits for the web app. Part 12 runs alongside.
Part 8's pairing from a link checks the answer's machine but keeps no key; part 4 pins that key
with the host, so a later answer without it, or with another, is refused there.
Iroh gets an ADR of its own; end-to-end encryption has ADR 0008.

### Alternatives

- **A hosted tunnel per machine**, such as Cloudflare Tunnel. There is no relay to run, but
  each tunnel has a price, each machine needs its own DNS record (and zones have quotas), every
  machine runs a third-party binary, and self-hosters are tied to the provider.
- **A relay on a serverless edge**, such as Workers with Durable Objects. There is no box to
  run, but it is billed per message, written for that platform's runtime, and self-hosters
  can't run it without an account with the same provider.
- **Iroh now.** It has end-to-end encryption and hole punching from the start, but browsers
  can use it only through its relays, and the phone needs a native module. It comes later,
  behind the same traits.
- **One origin, with a path per machine**: ruled out in section 3 for machines' pages. With ADR
  0008 no machine page is served through the relay, and the relay uses one name.
- **Tailscale only.** It works very well where it is installed, but it asks every person and
  every device to join a VPN. It stays a first-class route.
- **Accounts of our own** (OpenID Connect and WebAuthn in our own service). This is the cheapest
  at scale, and self-hostable, but passkeys, MFA, recovery and abuse protection are a lot of
  security code to own. The `AccountVerifier` trait keeps a later move to one module.

## Consequences

- Most traffic never touches a relay. The emberhut.com relay is a flat-priced box, and a
  self-hoster runs the same binary or none.
- The operator of a relay sees only the metadata of what goes through it: sessions are sealed
  end to end (ADR 0008). The route label still shows when the relay is in use.
- Each machine has a key file to back up with its database. Every copy is as sensitive as the
  key: losing it, or learning that it was copied, means a new key, pairing every device again
  and linking the machine again.
- Older servers keep working, as legacy hosts on their one address.
- A phone uses the local network unencrypted only with location permission, which it asks for
  only when the person wants that.
- Desktops on macOS before 14.4 sign in to the account with a code, not a redirect.
- The route rules exist twice, in Rust and in TypeScript, held together by shared vectors that
  both run in CI.
- The contract grows by `GET /api/connect`, its probe, the link statement and two pairing
  fields. The relay's protocol has a reference of its own, versioned apart from this ADR.
- The relay domain is one name with an ordinary certificate (ADR 0008); it needs no Public
  Suffix List entry, so it can be changed later at the cost of every machine's `EMBERHUT_RELAY` and
  every device's relay route.
- New dependencies, each added by the part that first uses it:
  - signing and hashing: `ed25519-dalek` and `data-encoding` (base32);
  - the relay's plumbing: `hyper` and `hyper-util`, `tokio-tungstenite` with a small adapter
    from WebSocket messages to `AsyncRead`/`AsyncWrite` for yamux, `yamux`, `tokio-rustls`
    with the rustls crypto provider the workspace already uses, `notify` to reload
    certificates and the allow list, and a safe `SCM_RIGHTS` wrapper (`rustix` or `nix`) for
    the takeover;
  - accounts: `jsonwebtoken`, `security-framework` and `objc2-authentication-services` (the
    desktop), and `@noble/curves` (TypeScript, since the phone's JavaScript engine has no
    Ed25519 in WebCrypto);
  - sealed sessions: `snow` and the others ADR 0008 lists; snow's curve25519-dalek 4 and
    sha2 0.10 sit beside the workspace's versions, accepted in `deny.toml` until snow moves.

## Open questions

Each one has the recommendation this ADR assumes.

1. **The relay domain.** Resolved by ADR 0008: a registrable domain of its own, served at one
   name, with no Public Suffix List entry, since no machine page runs on it.
2. **Plain HTTP on the local network.** Recommended: section 2's
   rule. Tokens go only to a local route granted for the current network, with the probe bound
   to the address. A phone needs location permission for a grant, and asks for it only when the
   person wants the local network. The phone needs the `emberhut-network` native module either way,
   for the VPN check. So the alternatives are weighed on what they cost beyond it:
   - **HTTPS on the local network now**, with a self-signed certificate on each machine pinned
     through the probe. It removes plain HTTP, the grants and the location permission. But
     every machine then serves TLS on its local port, and the phone's local requests go through
     the native module's own HTTP client (`URLSession` or OkHttp with the pin), not `fetch`,
     which is a second request path to keep equal with the first. Sealing direct routes
     (ADR 0008's open question 1) would remove the need for it.
   - **No local route.** It is the simplest, but a home without a VPN then goes through the
     relay.
3. **Pairing through the account, without a code.** Recommended: no. Codes stay the only way
   in, so a compromised account or Clerk can't get into any machine. The alternative is that a
   machine accepts a new device from its own linked account, which is easier but makes Clerk a
   way in.
4. **When end-to-end encryption lands.** Resolved by ADR 0008: with the relay's first release.
   The relay never carries readable traffic, for anyone.
5. **Limits per account on the emberhut.com relay.** Recommended: 5 machines per account, and
   50 GB relayed a month, past which the machine is slowed rather than cut off. Direct routes
   are never limited.
6. **Where the first relay box runs.** Recommended: one region near most of the first people,
   and a second region once people far from it see slow first bytes.
7. **When to build the front of section 3.** Recommended: once the active box passes half of
   its capacity targets, or a second region is needed, whichever comes first. Until then, one
   box and a cold standby.
