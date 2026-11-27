# Sealed sessions, version 1

A sealed session is one Noise session between a device and a machine, carrying the device's
requests to the machine through a relay that can't read or change them. The decisions and their
reasons are in [ADR 0008](adr/0008-end-to-end-encryption.md); this page is the reference the
implementations follow: `emberhut-sealed` in Rust, and `packages/connect` in TypeScript. It changes
with the protocol: a change that breaks either side is version 2 (see Versioning).

Related references:

- the statement encoding, base64url, the machine id and the host normal form:
  [api.md's "Connect" section](api.md#connect);
- how a device reaches the relay, the machine's uplink, marks, tickets and the relay's close
  codes: [relay.md](relay.md);
- when a relay route answers in a race, and which version a client offers:
  [connect.md](connect.md).

## Notation

- `u8`, `u16`, `u32` and `u64` are unsigned integers of 1, 2, 4 and 8 bytes, big-endian unless a
  line says otherwise.
- `a || b` is `a` followed by `b`.
- `statement(prefix, fields)` is api.md's encoding: the ASCII prefix, a `0x00` byte, then each
  field as a `u32` length and its bytes. A time field is a `u64` of milliseconds since the Unix
  epoch, a key its raw bytes, a text field its UTF-8 bytes.
- Hex bytes are written `65 6d`.
- **Identity keys** are Ed25519 (RFC 8032): 32-byte public keys, 64-byte signatures. The
  machine's is in `machine.key` (ADR 0007); a device's is on its row.
- **Static and ephemeral keys** are X25519 (RFC 7748), 32 bytes.

### Signatures

Every Ed25519 signature on this page is verified by api.md's one rule for statements (its
"Statements" section), and nothing here changes it: `s < L`; `A` and `R` canonical, in the
prime-order subgroup and not of small order; the cofactorless equation. Anything else fails as a
bad signature. Its vectors are api.md's `signatures.json`.

### Key agreement

Every X25519 operation on this page whose result is 32 zero bytes fails: in the handshake, with
the device's error `handshake` (or the machine ending the stream). A peer that sends a low-order
point would otherwise force a known shared secret. snow's 25519 doesn't check this, so the Rust
side gives snow a resolver that wraps snow's 25519 and returns an error for such a result.

## Where a session runs

Version 1 runs only on `relay` routes.

1. The device opens `wss://<relay domain>/v1/sealed/<machine id>` (relay.md, "Device
   connections"). If it holds a ticket for this machine, it sends it as the first message, a
   text message.
2. From then on, binary WebSocket messages carry the session's byte stream, both ways. Message
   boundaries mean nothing: a reader concatenates them.
3. The relay opens one yamux stream on the machine's uplink for the connection, and copies the
   bytes unchanged. The machine reads the same byte stream from that stream.

Everything below is that byte stream.

## The protocol

- **Noise protocol name**: `Noise_XX_25519_ChaChaPoly_SHA256`, as the Noise Protocol Framework,
  revision 34, specifies it. In particular:
  - ChaChaPoly's 96-bit nonce is 4 zero bytes followed by the 64-bit counter, little-endian
    (Noise, section 12.3);
  - `HASH` is SHA-256, and `HKDF` is Noise's, on HMAC-SHA-256 (section 4.3);
  - transport messages are encrypted with empty associated data.
- **The device is the initiator**, the machine the responder.
- **The pattern**:

  ```text
  XX:
    -> e
    <- e, ee, s, es
    -> s, se
  ```

## Static keys and their statement

Each side makes a static key from the OS's randomness when it starts, and a new one at least
every 24 hours. Static keys are never written to disk. Each has a statement, signed by the
side's identity key:

```text
statement("emberhut sealed static v1", [role, static public key, expires])
```

- `role` is the ASCII text `machine` or `device`.
- `static public key` is the 32-byte X25519 public key the side uses in the handshake.
- `expires` is a time: 48 hours after the statement was made.

For a machine whose static key is 32 bytes of `11` and whose statement expires at
`1795561073000`, the statement is these 85 bytes:

```text
65 6d 62 65 72 68 75 74 20 73 65 61 6c 65 64 20 73 74 61 74 69 63   "emberhut sealed static
            20 76 31 00                                              v1" 00
00 00 00 07 6d 61 63 68 69 6e 65                                     len 7, "machine"
00 00 00 20 11 11 11 11 11 11 11 11 11 11 11 11 11 11 11 11          len 32, the key …
            11 11 11 11 11 11 11 11 11 11 11 11 11 11 11 11          … continued
00 00 00 08 00 00 01 a2 0f c7 a9 68                                  len 8, expires
```

A verifier accepts a statement when:

1. the signature verifies under the identity key in the same hello (see Signatures);
2. the role is the peer's: `machine` in message 2, `device` in message 3;
3. `expires` plus 24 hours is later than the verifier's clock;
4. `expires` is at most 72 hours later than the verifier's clock.

## The prologue

Both sides call Noise's `Initialize` with this prologue:

```text
statement("emberhut sealed prologue v1", [machine id, route kind, route host])
```

- `machine id` is the 32-character id (api.md).
- `route kind` is `relay`, the only kind in version 1.
- `route host` is the relay domain in api.md's host normal form, with `wss` treated as `https`:
  lowercase, IDNA A-labels, and the port dropped when it is 443.
- The device takes the id and the host from its route URL. The machine takes its own id, and
  the host of `EMBERHUT_RELAY`.

For the id `aaaabbbbccccddddeeeeffffgggghhhh` and the relay domain `relay.example.org`, the
prologue is these 94 bytes:

```text
65 6d 62 65 72 68 75 74 20 73 65 61 6c 65 64 20 70 72 6f 6c 6f 67 75 65
            20 76 31 00
00 00 00 20 61 61 61 61 62 62 62 62 63 63 63 63 64 64 64 64 65 65 65 65
            66 66 66 66 67 67 67 67 68 68 68 68
00 00 00 05 72 65 6c 61 79
00 00 00 11 72 65 6c 61 79 2e 65 78 61 6d 70 6c 65 2e 6f 72 67
```

## The byte stream

### Frames

After the preambles, everything is a **frame**: a `u16` length, then that many bytes.

| Frame | Length |
| --- | --- |
| a handshake message | 32 to 4,096 |
| a record | 19 to 16,400 |

A reader that gets a length outside its frame's range, or a stream that ends inside a frame,
stops at once (see Failures).

### 1. Preambles

The device sends its preamble, then message 1 at once, without waiting:

```text
65 6d 62 68 01        "embh", version 1
```

The machine answers with its own preamble before message 2:

- `65 6d 62 68 01` when it speaks version 1;
- otherwise `65 6d 62 68 00`, then a `u8` count, then that many `u8` versions it speaks, in
  ascending order. It marks the connection `declined` on the uplink (relay.md), and ends the
  stream. It ignores anything the device sent after its preamble.

A device that gets a refusal reconnects once, with the highest version both speak, unless that
is below its floor for this machine (see Versioning). Otherwise it fails with `version`. Any other
first four bytes, on either side, fail with `protocol`.

### 2. Message 1: device to machine

`-> e`. The payload is empty, so the frame is:

```text
00 20 || e (32 bytes)
```

The machine refuses a message 1 of any other length by ending the stream.

### 3. Message 2: machine to device

`<- e, ee, s, es`, with the machine's hello as payload:

```text
00 ca || e (32) || encrypted s (48) || encrypted hello (106 + 16)
```

**A hello**, the same layout in messages 2 and 3:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 32 | the sender's identity key (Ed25519 public key) |
| 32 | 8 | the static statement's `expires` (`u64` milliseconds) |
| 40 | 64 | the signature of the static statement |
| 104 | 2 | `X`, the length of the extensions (`u16`) |
| 106 | X | the extensions |

The statement's static key is not repeated in the hello: the verifier takes it from the
handshake (Noise's remote static key `rs`).

**Extensions** are a sequence of entries: a `u16` type, a `u16` length, and that many bytes.
Entries come in ascending order of type, each type at most once, and fill exactly `X` bytes.
A reader ignores types it doesn't know, and fails with `protocol` on a malformed sequence.
Version 1 defines no hello extension, so its hellos are 106 bytes, message 2 is 202 bytes
(`00 ca`), and message 3 is 170 bytes (`00 aa`).

**The device checks message 2**, in this order, and stops at the first failure:

1. It decrypts, and no key agreement gave zeros (`handshake`). A wrong prologue (another machine
   id or relay domain), a changed message 1, or a party that isn't the machine fails here.
2. The machine id derived from the hello's identity key is the id the device is connecting to,
   and, once the device has paired, the key is the one it pinned (`machine-key`).
3. The statement passes, with the role `machine` (`statement`).

### 4. Message 3: device to machine

`-> s, se`, with the device's hello as payload:

```text
00 aa || encrypted s (48) || encrypted hello (106 + 16)
```

**Early data.** The device may send records right after message 3, before it has read `accept`:
at most 8,192 bytes of frames in all. It sends nothing more until `accept` arrives. The machine
handles no record before it has sent `accept`; if it sends `close` instead, it drops them unread,
and if the `accept` is of a pairing session, it drops them unread too: in a pairing session, only
records the device sends after it has read `accept` count.
A request refused with `busy`, `statement` or `not-paired` was never run, and the device may send
it again.

**The machine checks message 3**:

1. It decrypts, and no key agreement gave zeros. Otherwise the machine ends the stream with no
   record.
2. The statement passes, with the role `device`. Otherwise it sends `close` with `statement` and
   ends the stream.
3. It looks the identity key up among its active device rows, and decides:

   | The key | And | The machine sends | Its mark (relay.md) |
   | --- | --- | --- | --- |
   | is on a row | the row has fewer than 4 sessions, and the machine fewer than 256 | `accept`, kind device | `device` |
   | is on a row | a limit is reached | `close` with `busy` | `declined` |
   | is on no row | a pairing code is outstanding, and fewer than 4 pairing sessions are open | `accept`, kind pairing | `pairing` |
   | is on no row | a code is outstanding, and 4 pairing sessions are open | `close` with `busy` | `declined` |
   | is on no row | no code is outstanding | `close` with `not-paired` | none |

   The machine sends the mark on the uplink before it writes that `accept` or `close` to the
   stream, so the relay has it before the machine's bytes need it.

### 5. After the handshake

- `h`, the **handshake hash**, is Noise's `h` after message 3: 32 bytes. The pairing exchange
  binds to it.
- Noise's `Split()` gives two cipher states: the first encrypts device to machine, the second
  machine to device.

## Records

A record is one Noise transport message, framed. Its plaintext is:

```text
type (u8) || body length (u16) || body || padding
```

- The plaintext is 3 to 16,384 bytes; the frame length is the ciphertext's, the plaintext's
  plus 16.
- **Padding** is zero bytes after the body, of any length that keeps the plaintext within
  16,384. Any record may carry it. A reader refuses a padding byte that isn't zero (`protocol`).
  Version 1 senders send no padding.

| Type | Name | Sent by | Body |
| --- | --- | --- | --- |
| `0x01` | `accept` | the machine, first and once | `u8` kind (`0x01` device, `0x02` pairing), `u16` X, X bytes of extensions |
| `0x02` | `data` | both | 1 to 16,381 bytes of the inner stream |
| `0x03` | `ping` | both | 8 bytes, any value |
| `0x04` | `pong` | both | the 8 bytes of the `ping` it answers |
| `0x05` | `rekey` | both | empty |
| `0x06` | `close` | both | `u16` code, then a reason: 0 to 123 bytes of UTF-8 |
| `0x80` | `padding` | both | empty; the record is only padding, and is ignored |

- Types `0x00` and `0x07` to `0x7f` must be understood: a reader that gets one it doesn't know
  fails with `protocol`. Types `0x81` to `0xff` may be ignored when unknown.
- **`accept`** is the machine's first record, unless that is a `close`. A device that gets any
  other record first, or a second `accept`, fails with `protocol`. Its extensions use the
  hellos' encoding. Version 1 defines one:

  | Type | Name | Value | When |
  | --- | --- | --- | --- |
  | `0x0001` | relay ticket | ticket id (16 bytes), `expires` (`u64`), signature (64 bytes) | in a device session through the relay |

  An `accept` of a device session with a ticket is 98 bytes of plaintext:
  `01 00 5f 01 00 5c 00 01 00 58 || ticket id || expires || signature`.
- **`data`** bodies, concatenated in order, are the inner stream (see Inside a session).
- **`ping`** may be sent by either side, and is answered with `pong` at once. While it has an
  inner stream open, the device sends a `ping` after 30 s without receiving a record, and treats
  15 s more without one as a dead connection. The machine sends none.
- **`rekey`**: the sender calls Noise's `Rekey()` on its sending cipher state right after
  sending the record. The receiver calls `Rekey()` on its receiving cipher state right after
  decrypting it. Each side rekeys after every 65,536 records it has sent since the handshake or
  its last rekey, and at least once in each hour in which it has sent a record.
- **`close`** ends the session: the sender sends nothing after it, and ends its writing side of
  the byte stream (the machine with a yamux FIN, which the relay reports as 1000; the device by
  closing its WebSocket). The receiver stops reading. A session whose byte stream ends without a
  `close` was **cut**: inner streams still open fail as on a reset connection.
- **Nonces** are Noise's counters. A side never sends with the counter at `2^64 - 1`; it sends
  `close` with `lifetime` before that, which the session's lifetime keeps out of reach.

### Relay tickets

The ticket's statement is:

```text
statement("emberhut relay ticket v1", [machine id, relay domain, ticket id, expires])
```

signed by the machine's identity key. `expires` is 24 hours after the ticket was made, and the
ticket id is 16 random bytes.

- The machine issues one with each device session's `accept`, and in the answer of a sealed
  pairing that succeeds.
- **One live ticket per device row.** When it issues a ticket, the machine sends the relay a
  `retire` for the row's previous one (relay.md). Signing a row out, its lapsing, or its
  replacement retires its live ticket too.
- **Races are harmless.** A connection that showed a ticket before its `retire` reached the relay
  keeps its ticketed count until it ends; a ticket only buys relay budget. And a connection the
  network cuts before the machine's mark retires the ticket it showed, the device's own: its
  next connection goes without a ticket until a session gives it a new one.
- The device keeps the newest ticket per machine and relay, and shows it on its next connection.

### Close codes

The `close` codes are the machine's and the device's own. They are authenticated, unlike the
relay's close codes (relay.md), which are about the route.

| Code | Name | Sent by | Meaning | The device then |
| --- | --- | --- | --- | --- |
| 0 | `normal` | either | the side is done with the session | — |
| 1 | `going-away` | the machine | it is shutting down or restarting | reconnects after a random 0 to 5 s |
| 2 | `signed-out` | the machine | the device was signed out, and its key forgotten | signs out of the machine, as on a 401 with `signIn` |
| 3 | `lifetime` | the machine | the session reached its lifetime | reconnects at once |
| 4 | `statement` | the machine | the device's static statement didn't pass | if its clock disagrees with the statement's time window, shows "This device's clock is wrong"; otherwise logs an error |
| 5 | `protocol` | either | a record broke this page's rules | logs an error; the route doesn't answer in this race |
| 6 | `pairing-over` | the machine | the pairing session's attempt is done, no `start` came within 10 s, or its 120 s have passed | — |
| 7 | `busy` | the machine | too many sessions: for this device, pairing sessions, or in all | reconnects after a random 1 to 5 s, at most 3 times a minute |
| 8 | `not-paired` | the machine | the device's key is on no row, and no pairing code is outstanding | a new device, which opens its pairing session only once the code is typed, shows "That code isn't right. Check it, or make a new one."; a device that holds tokens acts as for an unknown key (see Sessions on the machine) |

A code the reader doesn't know is read as `protocol`. The reason is for logs, never shown.

### Failures

- **Nothing to encrypt with**: a frame that doesn't decrypt, a key agreement that gave zeros, a
  length out of range, a stream that ends inside a frame, or a handshake message that fails. The
  side that sees it ends the byte stream at once, with no record: the other side may not be who
  it seems, or the bytes were changed. The machine resets its yamux stream, which the relay
  reports to the device as `reset` (relay.md); the device closes its WebSocket.
- **A session that stands, but a rule broken**: a record of an unknown must-understand type, an
  `accept` out of place, a malformed body, padding that isn't zero, early data over 8 KiB. The
  side sends `close` with `protocol` and ends the stream.

**The device's errors**, for its own logs and Connections (never sent):

| Error | When |
| --- | --- |
| `version` | no version both sides speak, or only one below the device's floor for this machine |
| `handshake` | message 2 doesn't decrypt, or a key agreement gave zeros |
| `machine-key` | the machine's identity key isn't the pinned one, or doesn't give the route's id |
| `statement` | the machine's statement doesn't pass; a failed time check is shown as a wrong clock |
| `timeout` | a deadline below passed |
| `refused` | the stream ended before `accept` or `close`: the machine refused the handshake, or the relay closed |
| `protocol` | a broken rule, on either side |

In a race, every one of them, and a `close` in place of `accept`, means the route doesn't answer.

### Timers

Each timer starts at the moment the table names.

| Who | Starts when | Ends when | Limit | Then |
| --- | --- | --- | --- | --- |
| the relay | it accepts the device's upgrade | the device's first message arrives | 10 s | the relay's `timeout` |
| the machine | its yamux stream opens | message 1 is read | 5 s | it ends the stream |
| the machine | its yamux stream opens | message 3 is read | 10 s | it ends the stream |
| the relay | it opens the yamux stream | the machine's mark arrives | 15 s | the relay's `timeout` |
| the device | its WebSocket opens | message 2 is read | 10 s | `timeout` |
| the device | it sends message 3 | `accept` or `close` is read | 5 s | `timeout` |
| the machine | it sends `accept` for a pairing session | `start` is read | 10 s | `close` with `pairing-over` |
| the machine | it sends `accept` for a pairing session | the session ends (unless `finish` succeeds first) | 120 s | `close` with `pairing-over` |
| the relay | its `pairing` mark | the connection ends (unless a `device` mark comes first) | 150 s | the relay's `timeout` |
| the machine | it sends `accept` for a device session, or a `finish` succeeds | the session ends | 24 hours | `close` with `lifetime` |
| the device | the same moment | it opens the next session, and moves new requests to it | 23 hours | — |
| the device | an inner stream is open and a record arrived | the next record arrives | 30 s | it sends `ping` |
| the device | it sends that `ping` | any record arrives | 15 s | the connection is dead |

A race waits 2.5 s for a route (connect.md); the handshake may go on after that, and its session
is kept if it completes.

## The state machine

```text
Device:
  Dialling ──WebSocket open──▶ Sent(preamble, message 1)
  Sent ──preamble 00 (refusal)──▶ Closed(version), or Dialling at a version both speak (once)
  Sent ──preamble 01, message 2──▶ Checking ──checks pass──▶ Sent(message 3, early data)
  Sent(message 3) ──accept, kind device──▶ Open(device)
  Sent(message 3) ──accept, kind pairing──▶ Open(pairing)
                    (a device that holds tokens for the machine sends none, closes,
                     and marks the relay route unavailable)
  Sent(message 3) ──close (busy)──▶ Closed ──random 1 to 5 s──▶ Dialling
  Sent(message 3) ──close (statement, not-paired)──▶ Closed(error)
  Open(pairing) ──finish answered 200──▶ Open(device)
  Open(device) ──23 hours──▶ Dialling a successor, then Closing once its streams finish
  Open ──close sent or read, or stream ends──▶ Closed ──as the close code says──▶ Dialling
  any state ──failure or deadline──▶ Closed(error)

Machine (per stream):
  Accepted ──preamble 01, message 1──▶ Sent(preamble, message 2)
  Accepted ──another version──▶ Closed(refusal sent, marked declined)
  Sent ──message 3 decrypts──▶ Checking
  Checking ──statement fails──▶ Closed(close: statement)
  Checking ──decided (table above)──▶ Open(device) | Open(pairing) | Closed(close: busy, not-paired)
           (the mark goes to the relay with accept or close)
  Open(pairing) ──finish succeeds──▶ Open(device), marked device
  Open(pairing) ──no start in 10 s, failed attempt, 120 s──▶ Closed(close: pairing-over)
  Open ──close, sign-out, lifetime, or stream ends──▶ Closed
  any state ──failure or deadline──▶ Closed (stream reset, no record)
```

## Sessions on the machine

- **Device sessions**: at most 4 per device row at once, and at most 256 sessions in all.
- **Pairing sessions**: at most 4 at once, machine-wide, and none while no code is outstanding.
- **Signing out** (ADR 0002), a row lapsing after 14 days unseen, or a row replaced by pairing
  again clears the row's identity key, and closes that row's sessions with `signed-out` at once.
- A key is on at most one active row: a pairing that proves a key already on a row replaces that
  row.
- **A device that holds tokens for the machine but gets `accept` with kind pairing, or `close`
  with `not-paired`,** has been forgotten there. It sends no token in that session, closes it with
  `normal`, and treats the relay route as unavailable until it pairs again. If a direct route
  answers, its session there says whether it is still signed in.

## Inside a session

- **yamux** (version 0, the HashiCorp specification) runs over the inner stream. The device is
  the yamux client (odd stream ids) and the only side that opens streams; the machine resets any
  stream opened towards it.
- Windows are 256 KiB per inner stream, fixed. At most 64 inner streams are open at once; the
  machine resets a 65th. yamux's keep-alive is off: `ping` records do that work.
- **Each inner stream carries one HTTP/1.1 request and its answer**, as a TCP connection does:
  api.md's contract, unchanged. Every request carries `Connection: close`, and the machine closes
  the stream after its answer, so no stream carries a second request. The device sends
  `Host: <machine id>`, and the machine doesn't check `Host` on sealed sessions. An `Upgrade`
  works as on TCP, and an SSE answer lasts as long as its stream.
- **Tokens.** In a device session every request carries the device's access token, as on a
  direct route, and a token whose row isn't the session's gets 401 with `signIn`. In a pairing
  session, every route but the two below gets 401 with `signIn`.
- **The probe** (`GET /api/connect/probe`) answers in device sessions, with `via: relayed`. In a
  pairing session it gets 401 like any other route.
- **Writers** cut the inner stream into `data` records of at most 16,381 bytes. yamux already
  takes inner streams' frames in turn, so a large body delays others by at most one record.
- **Readers** always read the session's byte stream, whatever inner stream is waiting; each inner
  stream's window bounds what is buffered for it.

## Pairing in a sealed session

Through the relay, a device pairs in a pairing session with CPace, and never sends the code.
Direct routes keep `POST /api/auth/pair` (api.md).

### CPace

The protocol is CPace as **draft-irtf-cfrg-cpace-21** specifies it, with the group
ristretto255 (RFC 9496) and the hash SHA-512, in the initiator-responder setting, with no
optional session-id output. The device is the initiator (the draft's A), the machine the
responder (B). This section repeats every byte, so it can be built from here alone; the draft's
appendix B.3 vectors check it.

**Encodings.**

- `prepend_len(x)` is the LEB128 encoding of `len(x)`, then `x`. For every input on this page the
  length is below 128, so it is one byte.
- `lv_cat(a, b, …)` is `prepend_len(a) || prepend_len(b) || …`.

**Inputs.**

| Input | Value |
| --- | --- |
| `DSI` | the 17 ASCII bytes `CPaceRistretto255` |
| `PRS` (the password) | `SHA-256(code)`, 32 bytes, where `code` is the typed code normalised as ADR 0002 does: its letters and digits only, uppercase, so 8 ASCII bytes (`ABCDEFGH` for `abcd-efgh`). It is the hash the machine stores |
| `CI` (the channel identifier) | `statement("emberhut sealed pair v1", [machine id])`, 60 bytes |
| `sid` (the session id) | `h`, the session's handshake hash, 32 bytes |
| `ADa`, `ADb` | the slot's number, one byte: `00` to `03` |

**The generator.**

1. `len_zpad = max(0, 128 - 1 - len(prepend_len(PRS)) - len(prepend_len(DSI)))`, where 128 is
   SHA-512's input block size: `128 - 1 - 33 - 18 = 76`.
2. `gen_str = lv_cat(DSI, PRS, zero_bytes(len_zpad), CI, sid)`, 222 bytes:

   | Offset | Bytes |
   | --- | --- |
   | 0 | `11`, then `DSI` (17 bytes) |
   | 18 | `20`, then `PRS` (32 bytes) |
   | 51 | `4c`, then 76 zero bytes |
   | 128 | `3c`, then `CI` (60 bytes) |
   | 189 | `20`, then `sid` (32 bytes) |

3. `gen_str_hash = SHA-512(gen_str)`, 64 bytes.
4. `g` is RFC 9496's element derivation from 64 uniform bytes (its section 4.3.4, the one-way map
   applied to each half, the two points added) on `gen_str_hash`: `RistrettoPoint::
   from_uniform_bytes` in curve25519-dalek. It is **not** RFC 9380's `hash_to_ristretto255`,
   which first runs `expand_message_xmd`. A `g` that is the identity fails (it doesn't happen
   for a correct map).

**Scalars.** `sample_scalar` takes 32 random bytes, clears the top 4 bits of the last one
(`b[31] &= 0x0f`), and reads them as a little-endian integer, below `2^252` and so below the
group order. A zero scalar is drawn again.

**Shares and the key.**

- The device: `ya = sample_scalar()`, `Ya = encode(ya · g)` (32 bytes, RFC 9496's encoding).
- The machine: `yb = sample_scalar()`, `Yb = encode(yb · g)`.
- `scalar_mult_vfy(y, X)`: decode `X` by RFC 9496's decoding, which refuses non-canonical
  encodings; if it fails, the result is the identity. Otherwise the result is `encode(y · X)`.
- The machine computes `K = scalar_mult_vfy(yb, Ya)`, the device `K = scalar_mult_vfy(ya, Yb)`.
  A `K` that is the identity's encoding (32 zero bytes) aborts that slot.

**The intermediate session key**, 64 bytes:

```text
ISK = SHA-512( lv_cat(DSI || "_ISK", sid, K) || lv_cat(Ya, ADa) || lv_cat(Yb, ADb) )
```

that is, 158 bytes hashed: `15` and the 21 bytes `CPaceRistretto255_ISK`, `20` and `sid`, `20`
and `K`, `20` and `Ya`, `01` and the slot, `20` and `Yb`, `01` and the slot.

**Confirmation tags** are HMAC-SHA-256, keyed by the slot's `ISK`:

```text
machine's tag = HMAC-SHA-256(ISK, statement("emberhut sealed confirm v1", ["machine", slot, h]))
device's tag  = HMAC-SHA-256(ISK, statement("emberhut sealed confirm v1", ["device", slot, h]))
```

where `slot` is its one byte. Tags are compared in constant time.

### POST /api/auth/pair/sealed/start

Only in a pairing session, within 10 s of its `accept`, and once. The device runs a CPace
initiator per slot, each with its own scalar, and sends the four shares (`Ya`, base64url):

```json
{ "shares": ["<base64url>", "<base64url>", "<base64url>", "<base64url>"] }
```

The machine:

1. counts the attempt against the wrong-code brake from this moment (ADR 0002). While the brake
   is on, it answers 429 as pairing does;
2. gives slots 0 upwards to its outstanding codes, newest first, at most four;
3. for each slot, runs the CPace responder on that slot's share: with the code's stored hash as
   `PRS` for a code's slot, and with 32 fresh random bytes as `PRS` for a slot without one, so
   every slot does the same work and gives an answer of the same form. It makes each slot's tag
   from that slot's `ISK`;
4. answers 200:

```json
{
  "ok": true,
  "status": 200,
  "shares": ["<base64url>", "<base64url>", "<base64url>", "<base64url>"],
  "confirms": ["<base64url>", "<base64url>", "<base64url>", "<base64url>"]
}
```

A share that doesn't decode, or gives a `K` that is the identity, gets 400 (`AuthReply`,
"Pairing failed. Try again."), stays counted, and the session ends with `pairing-over`. A second
`start` in one session gets 400 and ends the session the same way.

**The device** finishes each slot's initiator with the machine's share, and checks the slot's
tag. The first slot whose tag verifies is the code's slot. If none does, the code is wrong: the
device shows "That code isn't right. Check it, or make a new one." and ends the session, sending
nothing more. (A party posing as the machine gets four guesses at the device's code this way,
one per slot; ADR 0008 weighs that.)

### POST /api/auth/pair/sealed/finish

```json
{ "slot": 0, "confirm": "<base64url>", "install": "zr-…", "name": "Emberhut on a laptop", "model": "linux" }
```

`install`, `name` and `model` are read as in `PairRequest` (api.md). The machine checks the
device's tag for that slot. Then, in one transaction, it uses the code up and makes the device's
row, with the session's identity key as its key (never a body field), and forgives the attempt
on the brake. The answer is `AuthReply`, as for `POST /api/auth/pair`, with `machine`, and on a
200 also `ticket`:

```json
{ "ok": true, "status": 200, "access": "…", "refresh": "…", "device": 7, "expires": "…",
  "machine": { "id": "<machine id>", "publicKey": "<base64url>" },
  "ticket": { "id": "<16 bytes>", "expires": 1795561073000, "signature": "<64 bytes>" } }
```

| Status | When |
| --- | --- |
| 200 | the tag verifies; the session is now a device session for the new row: the machine marks it `device` on the uplink, and the pairing timers stop |
| 400 | no `start` in this session, or a slot that isn't 0 to 3 |
| 401 | the tag doesn't verify ("That code isn't right. Check it, or make a new one."), or the slot's code was used or expired meanwhile (pairing's own sentences) |

After a 200, the device checks that `machine.id` and `machine.publicKey` are the session's, and
pins the key. After any other answer, the machine ends the session with `pairing-over`.

**The stored hash is the password.** Whoever holds an outstanding code's stored hash (from a copy
of the machine's database) can complete this exchange without knowing the code (ADR 0008,
section 5).

## Versioning

- This page is version 1: the preamble's `01`, and `v1` in every prefix above.
- **Within version 1**, these may be added without breaking a peer: record types `0x81` to
  `0xff`, hello and `accept` extensions, padding, close codes (read as `protocol` by older
  peers), and routes inside sessions.
- **Anything else is version 2**: a new preamble byte and new prefixes. The machine lists the
  versions it speaks in its route list (`sealed`, api.md).
- **The version a device offers** is the highest it speaks that the machine's list names. For a
  relay route learned without a list (a pairing link, the account's list), it is the highest the
  device speaks, and a refusal names the versions the machine speaks.
- **The floor.** A device remembers, per machine, the highest version the machine has proved it
  speaks: one listed in a route list the device read inside a sealed session, or one a session
  completed at. A refusal never moves it down, so a forged refusal can't. Only the machine can,
  in an authenticated route list: when a route list read inside a sealed session no longer
  names the floor's version (a later ADR has retired it), the floor becomes the highest version
  that list names.
- A machine speaks every version it has spoken unless a later ADR retires one.

## Implementations

- **Rust**: `emberhut-sealed`. Noise runs on `snow` 0.10 (default features off; `use-curve25519`,
  `use-chacha20poly1305`, `use-sha2` and `use-getrandom` only), through its `HandshakeState` and
  `TransportState`, with the resolver wrapper above. CPace is written on `curve25519-dalek` 5
  (ristretto255) and `sha2` 0.11, with `hmac` 0.13 for the tags. emberhut-client drives it as
  `Sealed<D: Dial>`, emberhut-server as `Sealed<L: Listener>` (ADR 0008, section 4).
- **TypeScript**: `packages/connect`, with Noise's state machine and CPace on `@noble/curves`
  (X25519, ristretto255), `@noble/ciphers` (ChaCha20-Poly1305) and `@noble/hashes` (SHA-256,
  SHA-512, HMAC). The static key's operations are asynchronous, so a browser can keep it in
  WebCrypto.

## Vectors

In `crates/emberhut-api/tests/fixtures/connect/sealed/`, beside ADR 0007's. Keys come from fixed
seeds, and the vector generator fixes ephemeral keys (snow's fixed-ephemeral builder, used only
there) and CPace scalars; nothing in them is a real key. snow generates the Noise vectors, so the
TypeScript state machine is checked against it; cacophony's set for this protocol name runs in
both too. Both implementations run every file in CI.

| File | What it pins |
| --- | --- |
| `keys.json` | the seeds, and the identity, static and ephemeral keys and machine id derived from them |
| `noise-cacophony.json` | the community vectors for `Noise_XX_25519_ChaChaPoly_SHA256` (cacophony's, as snow runs them), copied with their licence noted, or regenerated with snow if the licence doesn't allow copying |
| `noise-xx.json` | generated with snow: the protocol name alone with our keys, messages, `h` and transport messages, with and without a prologue |
| `x25519-zero.json` | low-order X25519 points as `e` and `s`, and the `handshake` failure each must give |
| `static-statement.json` | statement bytes and signatures for both roles; refused: role swapped, another signer, expired past the leeway, expiring more than 72 hours ahead |
| `prologue.json` | prologue bytes: the default port, another port, an IDNA relay domain, uppercase input |
| `preamble.json` | preambles, refusals with version lists, the device's choice with and without a route list, and a refused downgrade below the floor |
| `handshake.json` | generated with snow: the three messages byte for byte, both hellos, `h`, and the first record each way |
| `handshake-refused.json` | each failure and its error: a changed message 1, another machine id or relay domain in the prologue, an identity key that isn't pinned, one that doesn't give the id, a bad signature, an expired statement, a message 1 of the wrong length, an oversize frame, a hello with malformed extensions |
| `records.json` | each record type each way, as ciphertext: `accept` (device, pairing, with a ticket), `data`, `ping` and `pong`, `padding`, a record with zero padding, one with non-zero padding (refused), an unknown type that may be ignored, an unknown type that must be understood, `rekey` and the records after it, `close` |
| `early-data.json` | early data at 8,192 bytes (taken after `accept`), over it (`protocol`), and dropped unread when the machine answers `busy` |
| `inner.json` | an inner yamux stream carrying one `GET /api/state` with `Connection: close` and its answer, as `data` records |
| `cpace-draft.json` | the CPace draft's appendix B.3 vectors (ristretto255, SHA-512), including its invalid inputs for `scalar_mult_vfy` |
| `cpace.json` | `PRS`, `CI`, `gen_str`, `gen_str_hash`, `g`, shares, `K` and `ISK` per slot, with fixed scalars; a filler slot with a fixed random `PRS` |
| `pairing.json` | `start` and `finish` bodies and their answers: the right code; a wrong code; a pairing session refused while no code is outstanding (`not-paired`); no `start` within 10 s; five codes outstanding (the oldest can't pair); a code used meanwhile; a changed tag; the session after a 200, with its ticket |
| `ticket.json` | ticket statements and their `accept` extension bytes; the relay's checks: another machine, another relay domain, expired, a bad signature, retired |
| `sessions.json` | the machine's decisions of message 3's table; a sign-out during a session closes it with `signed-out`; another row's token gets 401; a device holding tokens that gets kind pairing sends none |
