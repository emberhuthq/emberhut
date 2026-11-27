# How a client picks a route, version 1

These are the rules every emberhut client follows to reach a machine: emberhut-client in Rust (the
desktop, the CLI) and `packages/connect` in TypeScript (the phone, the web app). The decisions
and their reasons are in [ADR 0007](adr/0007-connectivity.md), section 2, and, for relay routes,
[ADR 0008](adr/0008-end-to-end-encryption.md).

Related references:

- the formats (the route list, the probe, statements, encodings): [api.md's "Connect"
  section](api.md#connect);
- the relay's protocol and its own answers: [relay.md](relay.md);
- the sealed sessions that relay routes carry: [sealed.md](sealed.md).

The shared test vectors in `crates/emberhut-api/tests/fixtures/connect/` are the executable form of
this page, and both implementations run them in CI. A change to a number here is a change to
the vectors, in the same pull request.

**What the vectors cover.** Part 1 of ADR 0007's work list owns them; later parts add cases
with the rules they change.

- statements, machine ids and the encodings (base64url, hosts and ports, socket addresses, the
  pairing link's fragment);
- probe answers and their bindings;
- route lists and address classification;
- the names a direct listener answers to;
- race timelines with their expected choice;
- grants, fingerprints and network changes;
- legacy hosts and the refused downgrade;
- Clerk tokens: an access token accepted, an ID token refused;
- renewal over a plain local route without a grant for the current network: it isn't sent at
  all, and the route is skipped;
- sealed sessions: sealed.md lists their vectors, in `sealed/`.

## Route kinds

| Rank | Kind | Address | Comes from |
| --- | --- | --- | --- |
| 0 | `loopback` | `http://127.0.0.1:<port>` | the desktop probing its own computer's port; kept when the key matches |
| 1 | `lan` | `http://<local address>:<port>` | the machine's advertisement |
| 2 | `vpn` | an address on a VPN interface, or a VPN's HTTPS name | the advertisement, or added by hand |
| 3 | `public` | `https://<a name the person runs>` | `EMBERHUT_PUBLIC_URL`, or added by hand |
| 4 | `ssh` | `ssh://<host>`, a local forward (desktop only) | added by hand |
| 5 | `relay` | `wss://<relay domain>/v1/sealed/<machine id>`, carrying a sealed session (sealed.md) | the advertisement, the pairing link, or the account's list |

## What the machine advertises

`GET /api/connect` lists every route the configuration gives, with `up`. The machine builds
the list like this:

- When `EMBERHUT_LISTEN` is a loopback address (the default), it lists no `lan` or `vpn` address.
- When it listens on an unspecified address (`0.0.0.0` or `[::]`), it lists the addresses of
  its interfaces, leaving out:
  - loopback and link-local addresses;
  - bridges and virtual interfaces (container and VM networks), known by interface type where
    the OS reports it and by name otherwise;
  - temporary (privacy) IPv6 addresses. Stable IPv6 addresses are listed.
- An address on a VPN interface (WireGuard, Tailscale, or any tunnel the OS reports as one) is
  `vpn`, labelled with the VPN's name when known. Every other address is `lan`. The address
  range alone never decides.
- It adds `EMBERHUT_ROUTES` (URLs, each with its kind), `EMBERHUT_PUBLIC_URL`, and, when `EMBERHUT_RELAY`
  is set, the relay route, `up` or not. With `EMBERHUT_ADVERTISE=off` it lists only these.
- A client skips a `relay` route from the list unless it speaks one of the sealed versions in
  the list's `sealed` (api.md). A client that speaks none has no relay route.

`emberhut-server pair` prints the routes its configuration gives, in its pairing link: for now
its listen address, when other devices can reach it, and `EMBERHUT_PUBLIC_URL` (the interfaces'
addresses come with the full advertisement).

## Learning and keeping routes

- A client learns routes at pairing (the link, or the typed address), from `GET /api/connect`
  after every connection and every 10 minutes while connected, and from the account's list
  (the relay URL only).
- A relay route from a pairing link or the account's list comes with no `sealed` list. The
  client tries it at the highest sealed version it speaks; a machine that doesn't speak it
  answers with the versions it does, and the client tries again once with the highest both
  speak (sealed.md, "Versioning"). That is how pairing through the relay starts.
- Routes are kept per machine, beside its session. Each one has:
  - its kind, URL and source (advertised, link, added, account);
  - its plain-HTTP grants;
  - when it was last advertised and last answered;
  - its place in the person's order;
  - its hold-down, if one applies.
- An advertisement adds and updates routes, and never removes one. A route that is in no
  advertisement and hasn't answered for 30 days is dropped, the relay route included. Routes
  added by hand stay until the person removes them.

## Legacy hosts

A host whose first, typed pairing answered without `machine` is a legacy host. It has exactly
one route, the URL it was configured or paired with, used directly. There is no probe, race,
advertisement or relay, and none of the plain-HTTP rules below apply to it. A host paired from
a link, or one already holding a key, never becomes legacy (api.md).

## The race

A race probes every cached route of the machine at once, each with a 2.5 s timeout. A direct
route answers when its probe passes (api.md). A `relay` route answers when its sealed session
opens: the handshake passes and the machine's `accept` arrives (sealed.md). The handshake proves
the machine's key and binds the machine id and the relay domain, so no probe is sent through the
relay, and the session is kept for the requests that follow.

- **When**: when the client starts; when the network changes; when the app comes to the
  foreground; when the route in use fails. A route fails on a transport error, a sealed session
  that was cut, or the relay closing the connection with `offline`. The relay's `limit`, `full`,
  `idle` and `moving` are about one connection, and don't fail the route (relay.md).
- **The head start**: 500 ms after the start, the client takes the best-ranked route that has
  answered. If none has, it takes the first that answers.
- **A late better answer**: a better-ranked route that answers after the head start, but within
  2.5 s, is moved to at once.
- **No answer**: if none answers within 2.5 s, the machine is unreachable. The client races
  again after a random wait between 0 and `min(60 s, 2 s × 2^n)`, where n counts the failed
  races, while the app is open.
- **Among equals** (the same rank): the person's order, then the first to answer in this race,
  then the most recently used.
- **After a network change**, no request carries a token, on any route, until the new race has
  finished. Requests wait for it (at most 2.5 s). The rule keeps a token from reaching an
  address that hasn't proved the machine.

## Moving back

While the route in use is ranked below the best one known, the client probes the better-ranked
ones every 60 s, and on network changes and coming to the foreground. When one answers:

- new requests go to it, and requests in flight finish where they are;
- the live stream reopens on it.

A client moves only to a strictly better rank.

## Hold-down

A route that passes the probe but then fails two real requests within 5 minutes is held down:
it is left out of races for 10 minutes, then 20, doubling up to an hour. The hold-down resets
once the route has carried requests for 10 minutes.

## SSH (desktop only)

The forward starts only when no better-ranked route answered within the head start, as:

```sh
ssh -N -o BatchMode=yes -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 \
  -L 127.0.0.1:<free port>:127.0.0.1:<machine port> <host>
```

The probe goes to the forward's local port, and answers when it was reached directly on the
machine's loopback at `<machine port>`, for the host `127.0.0.1:<free port>` (api.md).
It uses the person's own SSH configuration, with no terminal, and stays up while in use. While
the relay is in use, SSH is tried again at most every 5 minutes, not on every race.

## Tokens over plain HTTP

A device sends its tokens (access and refresh alike) over plain `http` only to:

- `loopback`;
- a `vpn` address, while the device's own route to it goes through a VPN interface that is up
  and whose range holds it. Otherwise the route is treated like `public` and never gets plain
  HTTP;
- a `lan` route with a grant for the network the device is on now.

`public` and VPN-name routes must be `https`. `relay` routes carry tokens only inside sealed
sessions, which the relay can't read (sealed.md). Probes carry no token.

**Grants.** A grant is made when the device pairs over the route, or when the person allows it
("Use this local network without encryption"). It is bound to the current network's
fingerprint, not to an address:

- a new address advertised on the same network inherits it;
- on another network the route is skipped, shown as "Local network (another network): not
  encrypted".

**Fingerprints, per platform.**

| Platform | Fingerprint | What it costs |
| --- | --- | --- |
| macOS (desktop) | the interface type, the default gateway's hardware address (from the ARP table), and the Wi-Fi network's name and access point when Location Services is allowed | nothing without the Wi-Fi name; Location Services for it |
| iOS | the Wi-Fi network's name and access point (`NEHotspotNetwork`) | the "Access Wi-Fi Information" entitlement and location permission. The gateway's hardware address isn't available to apps |
| Android | the Wi-Fi network's name and access point, and the default gateway's address | location permission (and location turned on) for the Wi-Fi name and access point. The gateway's hardware address is hidden |

**The VPN check** reads the path to an address and its interfaces:

- on iOS and macOS, Network.framework (`NWPathMonitor`, and the current path of an
  `NWConnection`);
- on Android, `ConnectivityManager`'s link properties and `TRANSPORT_VPN`.

On the phone, the fingerprint and the VPN check come from the Expo module
`apps/mobile/modules/emberhut-network`, behind an interface `packages/connect` defines.

**A phone without location permission** has a fingerprint of only "Wi-Fi". It makes no grant
then, and its `lan` routes are skipped. Connections offers "Allow Emberhut to recognise this Wi-Fi
network", and the app asks for location permission only when the person taps it.

**The web app** uses only `https` direct routes, since a page on an HTTPS origin can't fetch
plain `http`, and relay routes through its sealed fetch (ADR 0008, section 6).

## What the person sees

- **The label** by the machine's name says the route in use: "This Mac", "Local network",
  "Tailscale" (or the VPN's name, or "VPN"), "Public address", "SSH" or "Emberhut relay".
- **Connections** shows, for each route:
  - its state (in use, answers, no answer, held down, skipped because unencrypted);
  - when it was last checked;
  - for a name route that answered with the wrong host, the host it answered as ("answered as
    127.0.0.1:18990"), so a reverse proxy that doesn't pass `Host` is visible.

  It also shows the machine's id, the grant for the current network, and lets the person
  reorder, add and remove routes.
- **When nothing answers**: "Can't reach <machine>", one line per route, and Try again.
- **On iOS**, `lan` routes are probed only once one is known, since the first local address
  reached brings up the system's local network prompt.

## Pairing by QR

- `emberhut-server pair` prints a one-time code, its pairing link
  (`https://emberhut.com/pair#id=…&code=…&route=…`, api.md's "The pairing link") and the link as a
  QR code drawn in the terminal. "Pair a device" on a signed-in device gets the same link from
  `POST /api/auth/pair/new` (`link`) and draws its QR code.
- `emberhut-server id` prints the machine's id, for comparing with what Connections shows after a
  typed pairing.
- The machine's key is `machine.key` beside the database (`EMBERHUT_MACHINE_KEY` sets the path).
  The server makes it the first time it serves the database (mode `0600` on Unix), and the
  database remembers its id. A new key makes a new machine, which every device pairs with
  again, so the server refuses to start, saying why, when the key is missing or is another
  machine's while the database remembers one (a mistyped path, a backup restored without its
  key), when the file isn't a key, or when other users can read it (Unix). Starting once with
  `--new-machine-key` makes a new machine on purpose. Commands (`pair`, `id`, `link`, `unlink`)
  only read the key.

## Linking a machine

A machine is linked to an emberhut.com account so it appears in the account's list and can use the
emberhut.com relay (directory.md). No account is needed for anything else.

- **From the machine's command line**: `emberhut-server link` prints a verification address and a
  code, which the person enters in a browser on any device (the OAuth device grant). It then
  signs a link statement with the machine's key and posts it to the directory, and keeps no
  account token. `emberhut-server unlink` signs an unlink statement and posts it with no token, so a
  machine can always leave an account.
- **From a desktop on the same computer**: the desktop asks the server to sign the statement
  (`POST /api/connect/link-statement`, served to loopback only) and posts it with its own account
  token.
- **Settings.** The account's issuer, the native OAuth client id, the relay's domain and the
  directory's address are `EMBERHUT_ACCOUNT_ISSUER`, `EMBERHUT_ACCOUNT_CLIENT_ID`, `EMBERHUT_RELAY_DOMAIN` and
  `EMBERHUT_DIRECTORY_URL` (by default `https://<relay domain>`). Official builds set them when they
  are built; flags and the same variables at run time override them. A build from source has
  none, and `link` says the account is off and how to turn it on. `EMBERHUT_MACHINE_NAME` names the
  machine in the account's list (by default its host name).
