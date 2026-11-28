# Installing Emberhut

Every build is a release on GitHub: <https://github.com/emberhuthq/emberhut/releases>. The
[download page](https://emberhut.com/download/) picks the right file for your computer.

There are two kinds of release:

- **Release candidates**, tagged `vX.Y.Z-rc.N` and marked as pre-releases. The website calls
  them "Preview". They may have rough edges.
- **Stable releases**, tagged `vX.Y.Z`.

Each release holds these files, where `V` is its version (for example `0.1.0-rc.1`):

| File | What it is |
| --- | --- |
| `Emberhut-V-macos-universal.dmg` | The Mac app, with the hut inside it. Apple silicon and Intel, macOS 13 or later. |
| `Emberhut-V-windows-x64.msi` | The Windows app, with the hut inside it. Windows 10 and 11, 64-bit. |
| `emberhut-server-V-linux-x64.tar.gz` | The hut on its own, as a server, for Linux on x86-64. |
| `emberhut-server-V-linux-arm64.tar.gz` | The same for Linux on 64-bit ARM. |
| `emberhut-server-V-macos-universal.tar.gz` | The same for macOS, Apple silicon and Intel. |
| `emberhut-server-V-windows-x64.zip` | The same for Windows, 64-bit. |
| `SHA256SUMS`, `SHA256SUMS.minisig` | The SHA-256 of every other file, and its signature. |
| `emberhut-desktop-macos.json`, `emberhut-desktop-windows.json` | What the apps read to update themselves. You don't need them. |

The hut is Emberhut's server: the program that keeps your chats, tasks and accounts and runs your
agent. The desktop app starts one on its own computer, or connects to one you run elsewhere.

- [macOS](#macos)
- [Windows](#windows)
- [The server on its own](#the-server-on-its-own)
- [The phone](#the-phone)
- [Checking a download](#checking-a-download)
- [Updates](#updates)

## macOS

You need macOS 13 (Ventura) or later, on Apple silicon or Intel: the app is universal, so the same
download runs on both.

1. Download `Emberhut-V-macos-universal.dmg`.
2. Open it and drag **Emberhut** onto **Applications**.
3. Eject the disk image, then open Emberhut from Applications or Launchpad.

The app and the disk image are signed with a Developer ID and notarized by Apple, so on the first
launch macOS asks only whether to open an app downloaded from the internet. Choose **Open**.

On first launch the app offers to make this Mac the home of your agent (it starts the hut inside
the app) or to connect to a hut you run elsewhere.

To remove it, quit Emberhut and drag it from Applications to the Bin. Its data stays in
`~/Library/Application Support/Emberhut/` until you delete that folder too.

## Windows

You need Windows 10 or 11, 64-bit.

1. Download `Emberhut-V-windows-x64.msi`.
2. [Check it](#checking-a-download) against `SHA256SUMS`. This matters more on Windows for now,
   since the installer isn't signed (below).
3. Open the `.msi`. It installs for your user alone, with no administrator prompt, to
   `%LOCALAPPDATA%\Programs\Emberhut`, and adds Emberhut to the Start menu.
4. Open Emberhut from the Start menu.

### While the installer isn't code-signed

The Windows installer doesn't have an Authenticode signature yet, so Windows SmartScreen may stop
it with "Windows protected your PC". To install it anyway:

1. Choose **More info**.
2. Check that the file name is the one you downloaded, then choose **Run anyway**.

Your browser may also warn when downloading it; keep the file. The checksum and its minisign
signature are how you know it's ours until the installer is signed.

### Uninstalling

Open **Settings → Apps** (**Installed apps** on Windows 11, **Apps & features** on Windows 10), find
**Emberhut**, and choose **Uninstall**.

## The server on its own

Run the hut on its own on a computer that stays on, such as a Linux box, a home server or a
spare Mac, and pair the desktop and phone apps with it. There's one archive per system; each
holds the `emberhut-server` binary, `LICENSE-MIT`, `LICENSE-APACHE` and a `README.md`.

On Linux (x64 shown; use `linux-arm64` on 64-bit ARM) and macOS:

```sh
tar -xzf emberhut-server-V-linux-x64.tar.gz
./emberhut-server --help    # from the folder the binary unpacked to
```

On Windows, unpack `emberhut-server-V-windows-x64.zip` (right-click → **Extract All**), then in
PowerShell in that folder:

```powershell
.\emberhut-server.exe --help
```

The macOS binary is signed and notarized. The Windows one isn't signed yet, so SmartScreen may
warn the same way as the installer.

To start it with an agent and pair your first device:

```sh
EMBERHUT_AGENT_COMMAND=claude ./emberhut-server   # serve; Claude Code must be installed and signed in
./emberhut-server pair                            # in a second terminal: a code that works once, for ten minutes
```

By default it listens on `127.0.0.1:18990` only. Every setting is a flag with an `EMBERHUT_*`
environment variable; `emberhut-server --help` lists them. How your devices reach it (your
network, a VPN or your own HTTPS address) is in [connect.md](connect.md), the API in
[api.md](api.md), and how the parts fit together in [architecture.md](architecture.md).

## The phone

The iPhone and Android apps are in testing, through TestFlight and Google Play's testing track, and
not in the stores yet. They pair with your hut: start a pairing from the desktop app or with
`emberhut-server pair`, then type the code or scan the QR code in the phone app. To hear when they
are out, write to [hello@emberhut.com](mailto:hello@emberhut.com).

## Checking a download

`SHA256SUMS` lists the SHA-256 of every other file in the release, in the format `sha256sum`
writes. Download it with the file you want, and compare.

On macOS:

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing
```

On Linux:

```sh
sha256sum -c SHA256SUMS --ignore-missing
```

On Windows, in Command Prompt or PowerShell:

```powershell
certutil -hashfile Emberhut-V-windows-x64.msi SHA256
```

or in PowerShell:

```powershell
Get-FileHash .\Emberhut-V-windows-x64.msi -Algorithm SHA256
```

and compare the hash it prints with the file's line in `SHA256SUMS` (`certutil` prints lowercase,
`Get-FileHash` uppercase; the letters are the same).

Then check that `SHA256SUMS` itself is ours. It's signed with [minisign](https://jedisct1.github.io/minisign/)
by the release key, whose public key is:

```
RWRER1atPjEA/9GrQdW+h0cy57Fiy526c7fbp1tkAabdvdSNURP3nnyD
```

With `SHA256SUMS` and `SHA256SUMS.minisig` in the same folder:

```sh
minisign -Vm SHA256SUMS -P RWRER1atPjEA/9GrQdW+h0cy57Fiy526c7fbp1tkAabdvdSNURP3nnyD
```

It prints `Signature and comment signature verified` when the file is intact and signed by that
key. The desktop apps check every update against the same key before installing it.

## Updates

The desktop app updates itself. Open **Settings → Updates**:

- **Channel**: **Stable** gets stable releases only. **Release candidates** gets every release,
  release candidates and stable alike, so a stable release newer than the candidates wins too.
- The app checks GitHub's releases once a day (on by default; you can turn it off), and whenever you
  choose **Check for updates**.
- When an update is ready it's downloaded and its signature checked; choose **Install and restart**.

The check reads the public release list from GitHub, with no account and nothing about you or your
hut beyond what any web request carries. The server archives don't update themselves: download the
new archive and replace the binary.
