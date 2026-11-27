# 0011. Push, the sign-in status and secret links; no Sign in with Apple

- Status: Proposed
- Date: 2026-11-26

## Context

Four more things around devices come up after the first slice (ADR 0002):

- **Push.** Phones register where to push (`POST /api/devices`: an iPhone's APNs device token
  and its service, or an Android phone's FCM registration token), and each of the assistant's
  replies reaches them as an alert, unless the room is on screen (the looking header,
  `emberhut_api::headers::LOOKING`, 10 s). The state's `push` says whether the server can push at
  all.
- **The sign-in status** (`GET /api/auth/status`): the mode, whether the server is bound, the
  signed-in devices, and whether this request's session is one.
- **One-time secret links** (`GET`/`POST /s/{token}`, and the `secret_link` tool that makes
  them): a page where the person types the secrets the assistant asked for, so they land in the
  server's secrets and never in a chat.
- **Sign in with Apple** (`POST /api/auth/apple`): the app sends Apple's identity token; the
  server checks it against Apple's keys, binds the first Apple user id it sees, and opens a
  device session for that id from then on.

Signing a device out (`authRevoke` in `POST /api/settings`) is `AuthService::sign_out`,
reached through the settings dispatcher (ADR 0009) as its `authRevoke` action.

## Decision

### Push: two senders behind traits, keys as files

`PushService` (emberhut-server) keeps the addresses (`PushStore`, tables `push_devices` and
`fcm_devices`), and sends through two traits, `ApnsSender` and `FcmSender`, so tests use fakes
and never the network. Each send answers `Delivered`, `Gone` (the token will never work again:
it is forgotten) or `Failed` (logged, the token kept). A push
never fails or holds up the turn that caused it: the reply is stored and the room's live value
cleared first, then the phones are told on a task of its own, so the room's next turn doesn't
wait for APNs or FCM.

- **Who may register, and for how long.** Registering (`POST /api/devices`) needs a signed-in
  device in either auth mode: report mode's pass for a request without a session would let
  anyone who reaches the server have every reply pushed to their own phone. Each push row names
  the device that registered it (a foreign key to `devices`, deleted with it), and signing a
  device out (`authRevoke`) deletes its push rows in the same transaction that ends its
  session, so a lost or signed-out phone is sent nothing more. Keying push addresses by the
  token alone would keep pushing to a signed-out phone until Apple or Google said the token
  was gone.
- **What an alert says** is `emberhut-core`'s `Alert`: titled `Emberhut`, or `Emberhut · <room>` outside
  Main; the reply as one plain line (Markdown links become their text, marks go), cut to 240
  UTF-16 units; grouped per room (`emberhut-<id>`); opening `emberhut://chat?room=<id>`, which the
  mobile app takes as its own scheme. The Android channel id is `emberhut`
  (`emberhut_api::push::ANDROID_CHANNEL`), the channel the Android app creates.
- **Held back** while the room is on screen: `Presence`, which the looking header already fed,
  now has its consumer.
- **APNs** (`HttpApns`): HTTP/2 only, as Apple requires; a provider token (ES256 JWT naming the
  key id and the team) made every 50 minutes and dropped on `ExpiredProviderToken`; the
  sandbox or production host as the phone registered; topic, push type `alert`, priority 10.
- **FCM** (`HttpFcm`): the HTTP v1 API; an OAuth access token traded for an RS256 JWT signed
  with the service account's key, kept until five minutes before it expires and dropped on a
  401; data values as strings (`link`, `thread`, `room`).
- **Configuration.** Keys are files in the secrets directory (`--secrets-dir`,
  `EMBERHUT_SECRETS_DIR`, default `secrets`): `apns-key.p8` and `fcm-service-account.json`, read
  when they are needed (and kept for a minute), so a key put there later works without a
  restart. (A secret link can't save them: its names are lowercase letters, digits and
  dashes, with no `.`.)
  APNs also needs the key id, the team id and the app's bundle id
  (`EMBERHUT_APNS_KEY_ID`, `EMBERHUT_APNS_TEAM_ID`, `EMBERHUT_APNS_TOPIC`), configured rather than
  compiled in, since a self-hosted build has its own. No key is ever a configuration value or an
  environment variable. Without a sender's keys that kind of phone gets no pushes, and the
  state's `push` is `true` when either sender is ready.
- **Crates.** The HTTP client is the server's one client (`emberhut-server`'s `http` module, also
  used for the Mac helper and the coding-thread app): reqwest on rustls with **ring** and the
  Mozilla roots (`webpki-roots`), so a container needs no system certificates. reqwest is given
  its TLS configuration ready-made (`tls_backend_preconfigured`, with ring as the provider), so
  nothing process-wide has to be installed before a client is made, in the server or in tests;
  its own default TLS would bring `aws-lc-rs`, which needs CMake and a C toolchain on every
  build machine (ADR 0007 rejected it for the same reason). ring was in the tree already
  (rustls under the desktop's clients), and it signs both kinds of JWT and checks them in
  tests, so no JWT crate is added. `jsonwebtoken`'s pure-Rust backend would bring the `rsa`
  crate, whose RUSTSEC-2023-0071 (a timing leak in private-key operations) applies to signing,
  which FCM needs.
- **Key files are read off the async workers.** `apns-key.p8` and `fcm-service-account.json`
  are read with tokio's file system calls, and a key read is kept for a minute, so neither a
  push nor the state's `push` (asked on every `GET /api/state`) blocks a worker. A file that
  isn't there is looked for again at its next use, and a key a service refuses is read again.

### The sign-in status, with the device list for signed-in devices only

`GET /api/auth/status` answers in either mode, with or without a session: the state's `auth` (mode,
bound, devices) and `signedIn`. The route is reachable before sign-in, so a caller without a session
gets the mode and `bound` with an empty device list: device names are the owner's business. `bound`
keeps ADR 0002's meaning (a device has signed in).

### Secret links: hashed, one use, a day

`SecretLinkService` makes a link for 1 to 6 names (lowercase letters, digits and dashes, not
`api-key`) and a line saying why. The `secret_link` MCP tool (ADR 0009's registry) calls it and
answers the link's address. The link is `<public URL>/s/<token>`: 24 random bytes, base64url. The
public URL is `--public-url` (`EMBERHUT_PUBLIC_URL`), or `http://` and the listen address. Only the
token's SHA-256 is stored (`secret_links`), as with every other token (ADR 0002). Expired links are
deleted when a new one is made.

The page needs no session: the link is the key. It is plain HTML, no script, sent with
`Cache-Control: no-store`, `Referrer-Policy: no-referrer`, a Content-Security-Policy that allows
inline styles and posting to itself only, and `nosniff`; everything the assistant wrote is
escaped. Sending the form uses the link up first (one row deleted, so two sends never both
write), then writes each filled-in secret, trimmed, to its file (made for the server's user
only, in a directory made for it alone), and notes in Main which names were saved, never the
values. Reconnecting a service after its secret is saved (Slack, say) belongs to that
service's own part.

### No Sign in with Apple

There is no `POST /api/auth/apple`. ADR 0002 decided that emberhut binds no outside identity, and
ADR 0007 that an account (the emberhut.com account) finds machines and never grants access to one:
a device proves itself to the machine with a pairing code, and holds a session the machine made.
Sign in with Apple is exactly an outside identity that grants a session, so both ADRs already
replace it. It also can't work for most self-hosters: Apple's token names the app's bundle id
as its audience, which only builds signed by that one developer team carry, so a server built
from source would refuse every token but that team's. A device that could Sign in with Apple
pairs with a code like every other device.

## Left out

- **Sign in with Apple** (`POST /api/auth/apple`, and with it binding the first Apple user id
  and opening sessions for it): pairing codes and device sessions replace it (ADR 0002, which
  binds no outside identity), and so does ADR 0007's rule that an account never grants access
  to a machine; Apple's token names one developer team's bundle id as its audience, which a
  self-hosted build doesn't have.

## Consequences

- iPhones and Android phones get the assistant's replies as alerts once the keys are in the
  secrets directory, and a phone that uninstalls the app is forgotten at its first failed push.
- rustls, webpki-roots and ring become direct dependencies, and reqwest gains its `http2` and
  `rustls-no-provider` features; none brings a C toolchain beyond ring's.
- A phone registers again after it pairs again: signing out forgot its push address.
- `emberhut_api::State` gains `push`, and the contract gains `GET /api/auth/status`,
  `POST /api/devices` and the secret pages.
- The `secret_link` tool and the `authRevoke` settings action call this logic.
