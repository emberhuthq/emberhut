# Switching over from the reference server

How an installation that runs the reference server moves to `emberhut-server`, as far as paired
devices go. The data itself moves with `emberhut-server import`
([ADR 0004](adr/0004-import-from-reference-server.md)); this page covers what the import
deliberately does not carry: every app, browser profile and helper pairs with the new server
once.

## Why devices pair again

- **The apps.** The reference server's devices (`auth_devices`) hold refresh tokens issued to the
  reference apps. The emberhut apps are other apps, with their own sign-in storage, so they never held
  those tokens. Each emberhut app pairs once, by QR code or code.
- **Browser profiles.** The emberhut browser extension is a new extension with its own id
  ([ADR 0023](adr/0023-browser-extension.md)); the reference extension's pairings
  (`chrome_devices`) belong to the reference extension. Each profile installs the emberhut extension
  and pairs once.
- **The Mac helper.** Emberhut Hands ([ADR 0024](adr/0024-mac-helper.md)) is a new app with its own
  bundle id, token and macOS privacy grants. It is installed beside the reference helper, never in
  place of it, so either can be stopped on its own.

Nothing of the reference side is changed by any of this: it keeps running until it is stopped.

## Re-imports keep emberhut's own pairings

While both servers run side by side, the import can be run again (ADR 0004, "Side by side, then
cut-over, and the way back", has the steps): import into a new database while `emberhut-server`
runs, stop it, carry over what was paired with it in between with `emberhut-server import
--keep-devices <current database> --database <next database>`, then put the next database in
place and start it. `--keep-devices` needs a next database with no devices or browser profiles
yet. It carries the apps' devices and sessions, the paired browser profiles, and the machine's
identity: the id it was served as, and `machine.key`, copied beside the next database (unless
`EMBERHUT_MACHINE_KEY` keeps it elsewhere for both). The next database is then served as the same
machine, so apps keep the key they pinned and nothing pairs again.

A server that remembers being served as one machine refuses to start without that machine's key
(a mistyped `EMBERHUT_MACHINE_KEY`, or a backup restored without its key). `--new-machine-key` makes
a new machine on purpose, for one start; every device then pairs again.

## The steps

1. **The server.** Import, then start `emberhut-server` on its own port. `emberhut-server id` prints the
   machine id; keep it to compare with what the apps show. For browser profiles, give it the
   extension's build (`EMBERHUT_CHROME_EXTENSION_ZIP`, the zip that `apps/extension` makes).
2. **The apps.** On the computer, `emberhut-server pair` prints a pairing code, then the pairing link
   and the link as a QR code. On the phone, scan it with the app's own scanner, paste the link, or
   type the address and the code. On a desktop, paste the link or type the code. A signed-in
   device makes more codes from "Pair another device", which shows the QR code next to the code.
3. **The Mac helper.** Build and install Emberhut Hands
   (`cargo xtask bundle-hands --sign … --install`, [apps/hands/README.md](../apps/hands/README.md)),
   grant it Accessibility and Screen Recording in System Settings (and restart it after the
   Screen Recording grant), and point the server at it (`EMBERHUT_MAC_HELPER_URL`,
   `EMBERHUT_MAC_HELPER_TOKEN_FILE`). Removing it is unloading its launch agent and deleting the app,
   its launch agent's file, its support folder and its log, then turning its grants off in
   Privacy & Security.
4. **Browser profiles.** The extension is loaded unpacked from one folder,
   `~/Library/Application Support/Emberhut/Extension`. Once the helper and the extension's zip are
   set up (steps 1 and 3), the server fills that folder through the helper within a few minutes
   of the helper answering, and keeps it in step with the server's build from then on
   ([docs/mac-helper.md](mac-helper.md)). In each Chrome profile, load the extension from that
   folder. Then, in the app,
   Settings › Chrome profiles › "Add a Chrome profile", open the link in that profile and accept
   the extension's offer to pair (its notification or its popup). The profile updates itself from
   then on.
5. **The account (optional).** In builds with the account turned on, sign in in the desktop app
   on the server's own computer, paired with that server, and choose "Add this computer to your
   account"; or run `emberhut-server link` on a headless machine. The account only lists the machine;
   access still comes from pairing.
6. **Switching off the reference side.** Once every device works against `emberhut-server`, stop the
   reference server, its extension and its helper. Nothing on the emberhut side depends on them.
