# The emberhut account directory, version 1

The directory lists an account's machines and tells the relay which machines may connect. The
decisions and their reasons are in [ADR 0007](adr/0007-connectivity.md), section 5. It runs
inside `emberhut-relay`, at `https://<relay domain>/v1`, on the relay's one host name (relay.md).
A change that breaks a client is version 2, at `/v2`, and the directory serves every version it
has served unless a later ADR retires one.

The statements and encodings (base64url without padding, times as JSON numbers of
milliseconds) are in [api.md's "Connect" section](api.md#connect). The types are in
`emberhut_api::directory`, and the test vectors beside the others, in
`crates/emberhut-api/tests/fixtures/connect/`.

## Requests

- **Authentication.** Every route except the machine's own unlink takes a Clerk token as
  `Authorization: Bearer <token>`: a session token with the `emberhut-directory` template from the
  phone, or an OAuth access token from the desktop and the CLI, or from the web app, which is
  an OAuth application of its own (ADR 0008, section 6). ADR 0007 lists the checks; an access
  token issued to either application passes them.
- **Which token is which.** A token is an OAuth access token when its header's `typ` is
  `at+jwt` (RFC 9068), or else when it carries a claim that only access tokens carry
  (`client_id` unless `EMBERHUT_RELAY_CLERK_ACCESS_TOKEN_CLAIM` names another). Its client id is
  `client_id`, or `azp` when there is no `client_id`, and must be the native or the web
  application's. Every other token is a session token: its `aud` must name the template's
  audience, and its `azp`, which Clerk sets only for a browser's origin, one of
  `EMBERHUT_RELAY_CLERK_ALLOWED_ORIGINS`. An ID token has a client id as its `aud`, so it fails
  there. `exp` and `nbf` have 30 s of leeway against the directory's clock.
- **Keys.** A token whose key is cached is checked at once. When the hourly refresh is due, it
  runs in the background, so no request waits on Clerk.
- **Lifetimes.** Session tokens from the template live 60 s. ADR 0007 sets the OAuth access
  tokens to 10 minutes; Clerk issues them for a day unless its application is set otherwise, and
  until then a token issued before its session ended keeps working for that day. The directory
  checks `exp` as the token states it.
- **Bodies** are JSON (`Content-Type: application/json`, which is required), at most 16 KiB,
  and readers ignore fields they don't know.
- **CORS.** Native apps need none. Answers carry `Access-Control-Allow-Origin` for
  `https://app.emberhut.com` only, the web app's origin (ADR 0008, section 6), with
  `Authorization` and `Content-Type` allowed and no credentials (no cookies are used). An answer
  to another origin carries no `Access-Control-Allow-Origin`. `Retry-After` is exposed, so the
  web app can read it.

## Errors

Every refusal is `{ "error": "<a sentence to show>", "code": "<code>" }`. Clients act on
`code`, never on the sentence. A 401 also carries `WWW-Authenticate: Bearer`. A path the
directory doesn't serve is 404 `not-found`, and a method it doesn't take on a path it serves is
405 with `bad-request`.

| Status | `code` | When |
| --- | --- | --- |
| 400 | `bad-request` | a body that isn't JSON, or lacks a field; a statement for another relay, or an unlink whose key isn't the path's machine |
| 400 | `stale` | a statement's time is more than 5 minutes from the directory's clock, or older than the machine's latest link or unlink |
| 400 | `replayed` | a statement already accepted |
| 401 | `sign-in` | no token, or one that fails a check; the body also has `"signIn": true` |
| 403 | `bad-signature` | the statement doesn't verify against `publicKey` |
| 403 | `not-yours` | the statement names another account than the token's `sub` |
| 404 | `not-found` | no such machine in this account (or, for the machine's own unlink, not linked) |
| 409 | `account-full` | the account already has as many machines as it may link: "This account has all the machines it can hold. Unlink one first." |
| 409 | `linked-elsewhere` | the machine is linked to another account: "This machine is linked to another account. Unlink it there first." |
| 429 | `limit` | too many requests; `Retry-After` says when to try again |
| 503 | `unavailable` | Clerk's keys can't be had right now (a token's `kid` is unknown while Clerk can't be reached, or within a minute of the last fetch of the keys), or the directory's own database failed; `Retry-After` says when to try again. Never a reason to sign out |

The checks run in this order. With an account token: the token; the request limit; the body;
the statement's relay, account and signature; its time against the clock; whether it was
accepted before; whether it is older than the machine's latest; then the account's machines.
For the machine's own unlink: the path's id; the body; the statement's relay, key and
signature; the request limit, counted only once the signature verifies, so that someone who
knows only a machine's id can't hold its unlink back; then the time and replay checks, and
whether it is linked.

**Request limits.** Counted in memory, per account on the routes that take a token (30 at once,
then one every 2 s), and per machine on its own unlink (5 at once, then one every 12 s).

## GET /v1/machines

The account's machines. 200:

```json
{
  "machines": [
    {
      "id": "<machine id>",
      "publicKey": "<base64url>",
      "name": "Main computer",
      "relayUrl": "wss://<relay domain>/v1/sealed/<machine id>",
      "connected": true,
      "lastSeen": 1795561073000,
      "linkedAt": 1795000000000
    }
  ]
}
```

`connected` says whether the machine's uplink is up now. `lastSeen` is when it last was (now,
while it is connected), or `null` if never. Machines are listed oldest link first.

## POST /v1/machines

Links a machine to the token's account. The body is the machine's `SignedLink`, as
`POST /api/connect/link-statement` gives it or `emberhut-server link` makes it:

```json
{
  "publicKey": "<base64url>",
  "name": "Main computer",
  "account": "<user id>",
  "relayDomain": "<relay domain>",
  "time": 1795561073000,
  "signature": "<base64url>"
}
```

The signature covers `emberhut link v1` over `account`, `relayDomain`, `name` and `time`. The
directory derives the id from `publicKey`, and checks:

- that `account` is the token's `sub`;
- that `relayDomain` is its own (400 `bad-request` otherwise);
- that `time` is fresh and the statement not seen before;
- that the id isn't linked to another account;
- that the account has fewer machines than its cap (ADR 0007's open question 5). The sixth is
  refused with 409 `account-full`.

`name` is the machine's name (api.md's rule, signed); one not trimmed or over 60 characters
is 400 `bad-request`. The relay's 4429 for machines connected per account is the second check,
for a cap lowered after machines were linked.

200 with the machine, in the form `GET /v1/machines` lists it. Linking a machine already in
this account again only updates its name.

## DELETE /v1/machines/{id}

Unlinks a machine from the token's account. 204. The gateway holding the machine closes its
uplink with 4403.

## POST /v1/machines/{id}/unlink

The machine leaves its account by itself, with no account token (`emberhut-server unlink`):

```json
{
  "publicKey": "<base64url>",
  "relayDomain": "<relay domain>",
  "time": 1795561073000,
  "signature": "<base64url>"
}
```

The signature covers `emberhut unlink v1` over the id, `relayDomain` and `time`. The id in the path
must be the one `publicKey` gives, and `relayDomain` the directory's own (400 `bad-request`
otherwise). 204, and the uplink is closed with 4403.

## Between gateways and the directory

Gateways look ids up and receive unlinks over an internal interface. It listens only on a
private address, or is an in-process channel when both run in one process. It is not part of
this API and can change with the relay's own version.

## Running the directory

`emberhut-relay serve --role directory` serves the directory alone (`--role all`, the default, and
`--role gateway` need the relay's gateway, which comes later). For now it serves plain HTTP on
its listen address, meant for loopback behind a reverse proxy that ends TLS for the relay
domain and passes `/v1/machines` to it. Every setting is a flag with a variable:

| Variable | Default | What |
| --- | --- | --- |
| `EMBERHUT_RELAY_ROLE` | `all` | what the process runs: `directory` for now (`all` and `gateway` need the gateway) |
| `EMBERHUT_RELAY_DOMAIN` | (required) | the relay domain statements must name |
| `EMBERHUT_RELAY_LISTEN` | `127.0.0.1:18980` | the address to serve on (see below) |
| `EMBERHUT_RELAY_DATABASE` | `directory.db` | the directory's SQLite file |
| `EMBERHUT_RELAY_ACCOUNT_CAP` | `5` | machines an account may link |
| `EMBERHUT_RELAY_CLERK_ISSUER` | (required) | Clerk's issuer, exactly as `iss` carries it |
| `EMBERHUT_RELAY_CLERK_JWKS_URL` | (required) | Clerk's keys, `https://<Clerk frontend API>/.well-known/jwks.json` |
| `EMBERHUT_RELAY_CLERK_AUDIENCE` | `emberhut-directory` | the session template's audience |
| `EMBERHUT_RELAY_CLERK_NATIVE_CLIENT_ID` | none | the native OAuth application's client id |
| `EMBERHUT_RELAY_CLERK_WEB_CLIENT_ID` | none | the web app's OAuth application's client id |
| `EMBERHUT_RELAY_CLERK_ALLOWED_ORIGINS` | none | origins a session token's `azp` may name, comma-separated |
| `EMBERHUT_RELAY_CLERK_ACCESS_TOKEN_CLAIM` | `client_id` | the claim that marks an access token without `typ: at+jwt` |
| `EMBERHUT_RELAY_LOG` | `info` | which log lines to write |

`EMBERHUT_RELAY_LISTEN` defaults to a loopback address rather than ADR 0007's `[::]:443` (section
7) because the directory serves plain HTTP until the gateway ends TLS itself; the default moves
to `[::]:443` with it. The directory refuses to start with settings that would let one kind of
token pass as another: an audience equal to either client id, or an access-token claim that ID
or session tokens carry too (`azp`, `aud`, `sub`, `iss`, `sid`, `exp`, `iat`, `nbf`, `jti` and
the like).

No setting is secret. The database holds accounts (the Clerk user id, kept only while the
account has a machine), machines, the signatures of statements accepted in the last 5 minutes,
and each machine's latest statement time.
