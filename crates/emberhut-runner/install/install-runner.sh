#!/bin/sh
# Installs the Emberhut runner for this user on Linux or macOS, pairs it with a server and starts
# it as a service:
#
#   curl -fsSL https://github.com/emberhuthq/emberhut/releases/latest/download/install-runner.sh | sh -s -- --server URL --pair CODE
#
# Options (or the environment variable in brackets):
#   --server URL    the Emberhut server to pair with                      [EMBERHUT_SERVER]
#   --pair CODE     the pairing code from the app's Machines page         [EMBERHUT_PAIR_CODE]
#   --version V     the release to install (default: this script's release, else the latest)
#   --prefix DIR    where the runner is installed                         [EMBERHUT_RUNNER_PREFIX]
#                   (default: ~/.local/lib/emberhut-runner)
#
# EMBERHUT_RUNNER_DOWNLOADS replaces https://github.com/emberhuthq/emberhut/releases/download, for
# a mirror holding the same v<version>/ folders.
#
# It downloads the release's build for this machine, checks it against the release's SHA256SUMS,
# unpacks it into <prefix>/versions/<version>, points <prefix>/current at it and links
# ~/.local/bin/emberhut-runner to it. With a server and a code it then runs
# `emberhut-runner install --server URL --pair CODE`, which pairs the runner and starts its
# service (a systemd user unit on Linux, a launchd agent on macOS) through <prefix>/current, so a
# later upgrade only moves that link. Nothing needs root.
set -eu

REPO="emberhuthq/emberhut"
# The release workflow writes its version here; empty means the latest release.
SCRIPT_VERSION=""

server="${EMBERHUT_SERVER:-}"
code="${EMBERHUT_PAIR_CODE:-}"
version="$SCRIPT_VERSION"
prefix="${EMBERHUT_RUNNER_PREFIX:-$HOME/.local/lib/emberhut-runner}"
downloads="${EMBERHUT_RUNNER_DOWNLOADS:-https://github.com/$REPO/releases/download}"

say() { printf '%s\n' "$*"; }
fail() {
  printf 'install-runner: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat << 'USAGE'
Usage: install-runner.sh [--server URL --pair CODE] [--version V] [--prefix DIR]

  --server URL    the Emberhut server to pair with (or EMBERHUT_SERVER)
  --pair CODE     the pairing code from the app's Machines page (or EMBERHUT_PAIR_CODE)
  --version V     the release to install (default: this script's release, else the latest)
  --prefix DIR    where the runner is installed (or EMBERHUT_RUNNER_PREFIX;
                  default: ~/.local/lib/emberhut-runner)
USAGE
}

needs_value() {
  if [ $# -lt 2 ] || [ -z "$2" ]; then fail "$1 needs a value"; fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    --server) needs_value "$@"; server="$2"; shift 2 ;;
    --pair) needs_value "$@"; code="$2"; shift 2 ;;
    --version) needs_value "$@"; version="$2"; shift 2 ;;
    --prefix) needs_value "$@"; prefix="$2"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) fail "unknown option $1 (see --help)" ;;
  esac
done

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  *) fail "this script is for Linux and macOS; on Windows use install-runner.ps1" ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) fail "there is no runner build for $(uname -m)" ;;
esac
# A shell running under Rosetta reports x86_64 on an Apple silicon Mac; the native build is better.
if [ "$os" = macos ] && [ "$arch" = x64 ] && [ "$(sysctl -n sysctl.proc_translated 2> /dev/null || echo 0)" = 1 ]; then
  arch=arm64
fi
target="$os-$arch"

if command -v curl > /dev/null 2>&1; then
  fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }
elif command -v wget > /dev/null 2>&1; then
  fetch() { wget -q -O "$2" "$1"; }
else
  fail "needs curl or wget"
fi
if command -v sha256sum > /dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

if [ -z "$version" ]; then
  fetch "https://api.github.com/repos/$REPO/releases/latest" "$tmp/latest.json" ||
    fail "can't find the latest release; name one with --version"
  version=$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "$tmp/latest.json" | head -n 1)
  [ -n "$version" ] || fail "can't read the latest release's version; name one with --version"
fi
version="${version#v}"
case "$version" in
  *[!0-9A-Za-z.-]* | "") fail "not a version: $version" ;;
esac

base="$downloads/v$version"
file="emberhut-runner-$version-$target.tar.gz"
say "Downloading emberhut-runner $version for $target..."
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "release v$version has no SHA256SUMS (is it published?)"
want=$(awk -v f="$file" '$2 == f || $2 == "*" f { print $1; exit }' "$tmp/SHA256SUMS")
[ -n "$want" ] || fail "release v$version has no runner build for $target"
fetch "$base/$file" "$tmp/$file" || fail "can't download $base/$file"
got=$(sha256 "$tmp/$file")
[ "$want" = "$got" ] || fail "$file's SHA-256 is $got, not the $want the release lists"

dest="$prefix/versions/$version"
mkdir -p "$prefix/versions"
rm -rf "$dest.new"
mkdir -p "$dest.new"
tar -xzf "$tmp/$file" -C "$dest.new" --strip-components 1
[ -x "$dest.new/emberhut-runner" ] || fail "$file holds no emberhut-runner"
rm -rf "$dest"
mv "$dest.new" "$dest"
ln -sfn "$dest" "$prefix/current"
mkdir -p "$HOME/.local/bin"
ln -sfn "$prefix/current/emberhut-runner" "$HOME/.local/bin/emberhut-runner"
runner="$prefix/current/emberhut-runner"
# The service runs the runner through `current`, which stays the same across upgrades.
EMBERHUT_RUNNER_LAUNCHER="$runner"
export EMBERHUT_RUNNER_LAUNCHER
say "Installed emberhut-runner $version in $dest (linked from ~/.local/bin/emberhut-runner)."
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) say "$HOME/.local/bin is not on your PATH; add it to run emberhut-runner by name." ;;
esac

if [ -n "$server" ] && [ -n "$code" ]; then
  "$runner" install --server "$server" --pair "$code"
else
  say "To pair it with your server and start its service, get a code from the app's Machines page, then run:"
  say "  emberhut-runner install --server <your server> --pair <code>"
  say "If its service was installed before, restart it to run this version: emberhut-runner service restart"
fi
