# Emberhut Hands

The helper on the owner's Mac that gives the assistant hands there: one app at a time in the
background, the whole screen with the owner's OK (a bar with Stop shows while it does), recording
calls, the screen's lock, and the browser extension's folder. The server calls it over HTTP with
a bearer token (ADR 0012); the decisions are in
[ADR 0024](../../docs/adr/0024-mac-helper.md).

It is its own app, `Emberhut Hands.app` (bundle id `com.emberhut.hands`), started at login by the launch
agent `com.emberhut.hands`, and the only program holding Accessibility and Screen Recording for the
assistant.

## Layout

| Path | What |
| --- | --- |
| `src/hands/` | The requests: one app in the background (`session.rs`: the one-app rule; `snapshot.rs`, `steps.rs`), the whole screen (`screen.rs`) |
| `src/rules.rs` | The apps never used, Terminal only looked at, Chrome refused, password fields |
| `src/record/` | The recorder: start, stop, status, the live chunks, sending tracks to the server |
| `src/extension/` | The extension's folder: the id check, the safe unpack, the swap into place |
| `src/http/` | The bearer token on every request, the routes |
| `src/desktop.rs` | The `Desktop` trait: every call to the Mac, faked in the tests |
| `src/mac/` | The real Mac over objc2 (macOS only): every unsafe call is here |
| `src/serve.rs` | Starting it on the Mac: options, token, server thread, app loop |

## Build and test

On any system the crate builds and its tests run (the Mac's code is behind
`cfg(target_os = "macos")`; elsewhere the binary only says it runs on macOS):

```sh
cargo test -p emberhut-hands
cargo clippy -p emberhut-hands --all-targets -- -D warnings
# The macOS code, checked from Linux (nothing is linked):
cargo check --target aarch64-apple-darwin -p emberhut-hands
```

## On the Mac

Build, sign and install (Xcode's command line tools; run from the repository):

```sh
# A Developer ID keeps the Mac's grants across rebuilds; without --sign it is signed ad hoc
# and the grants must be given again after every build.
cargo xtask bundle-hands --sign "Developer ID Application: <name> (<team>)" --install
# With the server on another machine, reached over a private network:
cargo xtask bundle-hands --sign "…" --install \
  --listen <the Mac's private-network address> \
  --server-url <the server's URL> \
  --server-token-file ~/.emberhut/hands-device-token
```

`--install` puts the app in `~/Applications/Emberhut Hands.app`, writes
`~/Library/LaunchAgents/com.emberhut.hands.plist` and (re)starts it; its log is
`~/Library/Logs/Emberhut Hands.log`. Without `--install` the signed app and its zip are left in
`target/bundle/`.

### The grants

On its first start Emberhut Hands asks the Mac to list it under System Settings → Privacy & Security:

1. **Accessibility**: turn on Emberhut Hands.
2. **Screen Recording**: turn on Emberhut Hands, then restart it
   (`launchctl kickstart -k gui/$(id -u)/com.emberhut.hands`); macOS applies this grant at start.
3. **Microphone**: asked the first time a call is recorded; that recording has the Mac's sound
   only, the next ones the owner's microphone too.

### The token and the server

The first start writes a new token to `~/Library/Application Support/Emberhut/Hands/token`
(owner-only; a token file others may read is refused). Copy it to the server's
`--mac-helper-token-file`, and set the server's
`--mac-helper-url` to `http://<the Mac's address>:8794` (with `--listen` set to that private
address; only callers in its range and on the Mac are answered). Recording also needs a device
token for the server, in the file `--server-token-file` names; the server's URL is https, or
http only to a loopback, private or `.local` address. Calls the server refuses, or that keep
failing, are set aside in `~/Library/Application Support/Emberhut/Hands/calls-unsent`.

### Checks

```sh
TOKEN=$(cat ~/Library/Application\ Support/Emberhut/Hands/token)
h() { curl -s -X POST "localhost:8794$1" -H "Authorization: Bearer $TOKEN" -d "$2"; echo; }
h /health '{}'                 # accessibility and screen true once granted
h /open '{"app":"Calculator"}'
h /snapshot '{"app":"Calculator","picture":true}' | head -c 600
h /press '{"app":"Calculator","ref":"7"}'
h /keys '{"app":"Calculator","text":"*6="}'
h /menu '{"app":"Calculator","path":"View"}'
h /snapshot '{"app":"Google Chrome"}'  # refused: Chrome goes through the extension
h /release '{}'
curl -s localhost:8794/extension -H "Authorization: Bearer $TOKEN"
curl -s -X POST localhost:8794/extension/install -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/zip" --data-binary @emberhut-extension.zip
```

The whole screen (`/screen/start` with `{"who":"check","reason":"a check"}`, then `/screen/shot`
and `/screen/act`) moves the real mouse: try it while watching, and check that Stop on the bar
and touching the mouse both end it, also in the middle of a long `type`, and that the next
`/screen/act` says why. Check too that a `type` stops at a password field reached by Tab, that
nothing is done while Keychain Access or Emberhut is in front, and that a click on a Chrome window
is refused. The owner's touch is told from the assistant's input by the Mac's input counters
(`CGEventSourceCounterForEventType`): check that a long `type` doesn't end the session by
itself, and that one key of yours during it does.

### Rollback

Nothing of any other helper is touched (its bundle id, launch agent, port and token are its
own), so rolling back removes only Emberhut Hands:

```sh
launchctl bootout gui/$(id -u)/com.emberhut.hands
rm -rf ~/Applications/"Emberhut Hands.app" ~/Library/LaunchAgents/com.emberhut.hands.plist \
  ~/Library/Application\ Support/Emberhut/Hands ~/Library/Logs/"Emberhut Hands.log"
```

(`~/Library/Application Support/Emberhut/Extension` belongs to the browser extension and stays.)

Then turn it off (or remove it) in Privacy & Security, and point the server's
`--mac-helper-url` and `--mac-helper-token-file` back at the helper it used before.
