# 0030. The hut inside the app: the desktop app runs the server on its own computer

- Status: Proposed
- Date: 2026-11-27
- Builds on: [ADR 0002](0002-rust-server.md) (the server, and `emberhut-server pair` as the
  shell-trust path), [ADR 0005](0005-releases-and-updates.md) (the bundle and its updates) and
  [ADR 0024](0024-mac-helper.md) (a launch agent for a program that runs without the app)

## Context

The assistant needs a server, the hut: `emberhut-server`. Until now a person ran it themselves:
`cargo run -p emberhut-server`, then `emberhut-server pair` in a terminal, then the code typed into
the app. The desktop app never started a server. A new person should be able to make their own
computer the home of their agent from the app alone, with no terminal, while someone who runs the
server elsewhere keeps doing so.

## Decision

### Where it lives

- **The binary ships with the app.** `cargo xtask bundle-macos` builds `emberhut-server` in the
  same cargo run as the app (the crates they share build once) and puts it at
  `Emberhut.app/Contents/MacOS/emberhut-server`, beside the app's own binary: Apple's place for a
  bundle's helper executables, signed (hardened runtime, same identity) before the bundle that
  seals it, so notarization and the updater's signature checks cover it. `--no-hut` leaves it out
  for a build that only talks to a server elsewhere. On Linux there is no bundle: the binary is
  beside the app's (`target/<profile>/` for a build from source, or wherever a package installs
  both), or `EMBERHUT_SERVER_BIN` names it.
- **The app finds it** in that order: the bundle, beside the app's executable, then
  `EMBERHUT_SERVER_BIN`; only an executable file counts. Without one, the hut isn't available and
  the onboarding offers only a server elsewhere.
- **Its files** are the hut's own, apart from the app's config folder:
  `~/Library/Application Support/Emberhut/Hut/` on macOS, `$XDG_DATA_HOME/emberhut/hut/`
  (`~/.local/share/emberhut/hut/`) elsewhere. It holds `emberhut.db`, `machine.key` (the
  server's default, beside the database), `secrets/` and the app's `hut.json` (its port, the
  server build it last started, its process when it is the app's child, and how it stays lit).
  The folders are made owner-only (0700), and the empty database file 0600 before the first
  start, since SQLite gives its journal files the database's mode; the server makes the key
  owner-only itself. The log is `~/Library/Logs/Emberhut/hut.log` on macOS and
  `$XDG_STATE_HOME/emberhut/hut.log` elsewhere, 0600.
- **How it is run**: `emberhut-server serve --listen 127.0.0.1:<port> --database-url
  sqlite://<data>/emberhut.db --secrets-dir <data>/secrets [--agent-command <claude>]`, in the
  data folder. The agent command is `claude` when found: on the app's `PATH`, else in the places
  its installers use (`~/.local/bin`, `~/.claude/local`, Homebrew's, `/usr/local/bin`), since an
  app opened from the Finder gets a short `PATH`; the hut's `PATH` puts that folder first. The
  model provider is chosen later from the apps (`PUT /api/agent/provider`, ADR 0028). Without
  `claude`, the hut runs with the silent assistant until one is set up.

### Trust: local pairing through `pair`, as today

No new trust is added. ADR 0002 makes shell access to the server's machine the root of trust:
`emberhut-server pair` on that machine is how the first device gets in. The app is a program
the person runs on that machine, as that user, so it may do the same: it runs the bundled
`emberhut-server pair --json` against the hut's own database and the code it prints is then
traded through the normal `POST /api/auth/pair`, as any device's typed code is (one use, ten
minutes, the brake on wrong codes). The server's rules for `pair` are unchanged: it opens only an
existing emberhut database with its schema up to date, so the app lights the hut (which migrates
it) first.

`pair --json` is new: one line, `{"code", "expiresAt", "machine", "database", "link"}` (`link`
`null` when there is none), on stdout; the logs go to stderr. The app reads the JSON line's
`code` (and, from an older server, the `Pairing code: ABCD-EFGH` line), and takes only a code of
the server's shape.

### Ports

- The hut listens on `127.0.0.1:18990` by default, the server's default and the app's default
  server address; the loopback only, so nothing else on the network reaches it until the person
  sets up a route.
- Before starting, the app asks what answers on the port. **Nothing** (the port can be bound):
  the hut starts there. **This computer's hut**: `GET /health` answers `{ ok: true }`, and a
  fresh `GET /api/connect/probe` is signed by the key whose id `emberhut-server id` reads from the
  hut's `machine.key` (ADR 0007's probe, checked as a device checks a route): it is used as it is.
  **Anything else**, another program or another emberhut server with its own key: the next port
  is tried, up to 20 after the first, and the one chosen is remembered in `hut.json`, so later
  starts go straight to it. A server answers as this hut only with the hut's own key, so a hut
  left running by an earlier run of the app (or a crash) is found and reused, and nobody else's
  server is ever taken for it.
- A start waits up to 60 s for the hut to answer as itself (the first start migrates the
  database); a child that exits meanwhile, or a hut that doesn't answer in time, is an error that
  names the log.

### Keep lit: per system

`keep_lit(KeepLit { when_closed, at_login })`. With neither, the hut is the app's child.
`at_login` implies the hut runs without the app, so either switch hands it to the system's
service manager, which runs the same command line:

- **macOS: a launch agent**, `~/Library/LaunchAgents/com.emberhut.hut.plist`, written as ADR 0024's
  helper agent is (`ProgramArguments`, `WorkingDirectory`, `EnvironmentVariables` with the
  `PATH`, both outputs to the log) with `Umask` 077. From login, `RunAtLoad` and `KeepAlive` are
  true: it starts at every login and comes back if it stops. Kept lit only while the app is
  closed, both are false: launchd loads it at login but doesn't run it, the app starts it, and it
  runs until it stops or the person logs out. The app unloads (`launchctl bootout
  gui/<uid>/com.emberhut.hut`) and loads it again (`bootstrap`, tried three times, as launchd may
  refuse right after an unload) whenever the plist changes, then `kickstart`s it. Stopping
  unloads it, since a kept-alive agent comes back when killed; turning both switches off also
  removes the plist.
- **Linux: a systemd user unit**, `~/.config/systemd/user/emberhut-hut.service` (`Restart=on-failure`,
  `UMask=0077`, output appended to the log, `WantedBy=default.target`), then `systemctl --user
  daemon-reload`, `enable` (from login) or `disable`, and `restart`. Stopping is `systemctl
  --user stop`; both switches off is `disable`, `stop`, removing the unit and `daemon-reload`.
- **Otherwise, the app's child.** Its output goes to the log. The app stops it (`SIGTERM`, which
  the server shuts down cleanly on; killed if it hasn't stopped in 15 s) when the hut is moved to
  a service, stopped, or the `LocalHut` is dropped, and the app calls `stop` as it quits (wired
  with the onboarding). A child left by
  a crash keeps running and is found and reused by the next start, and its remembered process is
  what `stop` then ends (only once it answers as this hut). Keeping it lit without the app on a
  system with neither launchd nor systemd is refused, in words.

### Upgrades

An app update brings a newer server in the bundle (ADR 0005's swap replaces the whole bundle).
The server's `/health` says no version, so the app remembers which build it started: the
bundled binary's `--version` with its size and modification time, in `hut.json`. At every start,
a hut that answers as this one but was started from another build is restarted once on the
bundled binary: the app's child is stopped and started again; a service is rewritten and
restarted (`restart`, or `bootout` and `bootstrap`). The next start finds the same build and
keeps it. A service's command line names the binary in the bundle, so it never runs a copy
outside it.

### The app's side

`apps/desktop/src/hut/` holds `LocalHut`, built from a process spawner, a loopback prober and
the hut's paths (constructor injection; the tests use fakes and a temporary home):
`available()`, and `start`, `pair_code`, `keep_lit`, `stop` and `status`, each a GPUI task on
the background executor that runs one at a time. Errors are `HutError`, whose messages are
written to be shown as they are. The onboarding builds on it; there is no other UI.

## Consequences

- The macOS bundle grows by the server's binary, and `bundle-macos` builds a second package
  (sharing the first's crates); `--no-hut` skips both for a quicker build.
- The hut is one more process on the person's computer, the same server anyone can run by hand,
  with the same trust. A person who runs their server elsewhere never starts it: nothing runs
  until the onboarding asks.
- A hut kept lit only while the app is closed doesn't come back by itself after a crash (launchd
  won't keep it alive without also starting it at login); the next start of the app lights it
  again.
- The version check restarts once per new server build, not per API change: a server that says
  its version in `/health` would let it ask, but that is a contract change of its own.

### What is Mac-only

The bundle, its signing, the launch agent and `launchctl` are macOS only: they are type-checked
from Linux and their plist and commands are tested against a fake spawner, but loading the agent,
its `RunAtLoad`/`KeepAlive` behaviour across logins and the signed nested binary's notarization
need a real Mac. Linux's systemd unit is likewise tested by its text and the commands issued; the
child process path runs for real in an ignored test (`hut::real_tests`).
