#!/bin/sh
# Installs the Emberhut runner for this user on macOS or Linux, pairs it with a server and starts it as a service:
#
#   curl -fsSL https://github.com/emberhuthq/emberhut/releases/latest/download/install-runner.sh | sh -s -- --server URL --code CODE
#
# Options (or the environment variable in brackets):
#   --server URL       the Emberhut server to pair with            [EMBERHUT_SERVER]
#   --code CODE        the pairing code from the app's Machines page [EMBERHUT_PAIR_CODE]
#   --name NAME        this runner's name (default: the machine's)  [EMBERHUT_RUNNER_NAME]
#   --advertise URL    the address the server calls this runner at  [EMBERHUT_RUNNER_ADVERTISE]
#   --version V        a release (default: this script's own, else the latest) [EMBERHUT_RUNNER_VERSION]
#   --prefix DIR       where the runner is installed                [EMBERHUT_RUNNER_PREFIX]
#
# It downloads the release's build for this platform, checks its SHA-256 against the release's SHA256SUMS, unpacks it
# into <prefix>/versions/<version>, points <prefix>/current at it, links ~/.local/bin/emberhut-runner, and with a server
# and a code pairs and installs the service (launchd on macOS, a systemd user unit on Linux), then waits for it to answer.
# Without them it only installs: pair later with `emberhut-runner pair --server URL --code CODE`. Nothing needs root.
set -eu

REPO="emberhuthq/emberhut"
# A release sets this to its own version; empty means the latest release.
SCRIPT_VERSION=""

server="${EMBERHUT_SERVER:-}"
code="${EMBERHUT_PAIR_CODE:-}"
name="${EMBERHUT_RUNNER_NAME:-}"
advertise="${EMBERHUT_RUNNER_ADVERTISE:-}"
version="${EMBERHUT_RUNNER_VERSION:-$SCRIPT_VERSION}"
prefix="${EMBERHUT_RUNNER_PREFIX:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --server) server="$2"; shift 2 ;;
    --code) code="$2"; shift 2 ;;
    --name) name="$2"; shift 2 ;;
    --advertise) advertise="$2"; shift 2 ;;
    --version) version="$2"; shift 2 ;;
    --prefix) prefix="$2"; shift 2 ;;
    -h|--help) sed -n '2,20p' "$0" 2>/dev/null || true; exit 0 ;;
    *) echo "install-runner: unknown option $1" >&2; exit 2 ;;
  esac
done

say() { printf '%s\n' "$*"; }
fail() { printf 'install-runner: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Darwin) os=darwin; prefix="${prefix:-$HOME/Library/Emberhut Runner}" ;;
  Linux) os=linux; prefix="${prefix:-$HOME/.local/lib/emberhut-runner}" ;;
  *) fail "this script is for macOS and Linux; on Windows use install-runner.ps1" ;;
esac
case "$(uname -m)" in
  x86_64|amd64) cpu=x64 ;;
  arm64|aarch64) cpu=arm64 ;;
  *) fail "no runner build for $(uname -m)" ;;
esac
target="$os-$cpu"

fetch() { # fetch URL FILE
  if command -v curl >/dev/null 2>&1; then curl -fsSL --retry 3 -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then wget -q -O "$2" "$1"
  else fail "needs curl or wget"; fi
}
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

if [ -z "$version" ]; then
  base="https://github.com/$REPO/releases/latest/download"
else
  version="${version#v}"
  base="https://github.com/$REPO/releases/download/v$version"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
say "Downloading the runner for $target…"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "no SHA256SUMS at $base (is that a release?)"
file=$(grep -E "  emberhut-runner-[^ ]+-$target\.tar\.gz\$" "$tmp/SHA256SUMS" | head -n1 | sed 's/^[0-9a-f]*  //')
[ -n "$file" ] || fail "the release has no runner build for $target"
fetch "$base/$file" "$tmp/$file"
want=$(grep -E "  $file\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
got=$(sha256 "$tmp/$file")
[ "$want" = "$got" ] || fail "$file's SHA-256 is $got, not the release's $want"

v=$(printf '%s' "$file" | sed -E "s/^emberhut-runner-(.+)-$target\.tar\.gz\$/\1/")
mkdir -p "$prefix/versions"
rm -rf "$prefix/versions/$v.new"
mkdir -p "$prefix/versions/$v.new"
tar -xzf "$tmp/$file" -C "$prefix/versions/$v.new" --strip-components 1
rm -rf "$prefix/versions/$v"
mv "$prefix/versions/$v.new" "$prefix/versions/$v"
rm -f "$prefix/current"
ln -s "$prefix/versions/$v" "$prefix/current"
mkdir -p "$HOME/.local/bin"
ln -sfn "$prefix/current/bin/emberhut-runner" "$HOME/.local/bin/emberhut-runner"
runner="$prefix/current/bin/emberhut-runner"
# The service runs the launcher through `current`, so an upgrade needs only a restart.
export EMBERHUT_RUNNER_LAUNCHER="$runner"
say "Installed emberhut-runner $v in $prefix (linked from ~/.local/bin/emberhut-runner)."

if [ -n "$server" ] && [ -n "$code" ]; then
  set -- pair --server "$server" --code "$code"
  [ -n "$name" ] && set -- "$@" --name "$name"
  [ -n "$advertise" ] && set -- "$@" --advertise "$advertise"
  "$runner" "$@"
elif "$runner" paired >/dev/null 2>&1; then
  # Paired before: an upgrade. The service (re)written for this install, then started on the new version.
  "$runner" service install
else
  say "Not paired yet. Get a code from the app's Machines page (Add a hut), then run:"
  say "  emberhut-runner pair --server <your server> --code <code>"
  exit 0
fi

i=0
while [ $i -lt 30 ]; do
  if "$runner" status >/dev/null 2>&1; then "$runner" status; exit 0; fi
  i=$((i + 1)); sleep 1
done
fail "the runner didn't answer within 30 s; see its log (emberhut-runner service status)"
