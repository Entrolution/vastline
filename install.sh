#!/usr/bin/env bash
#
# install.sh — download the latest vastline release binary and wire it into Claude Code.
#
#   curl -fsSL https://raw.githubusercontent.com/Entrolution/vastline/main/install.sh | bash
#
# Or run it from a clone. Overrides (env):
#   VASTLINE_BIN_DIR=<dir>   where to install the binary (default: ~/.local/bin)
#
# Windows: use install.ps1 instead.
#
set -euo pipefail

REPO="Entrolution/vastline"
INSTALL_DIR="${VASTLINE_BIN_DIR:-$HOME/.local/bin}"

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin)
    case "$arch" in
      arm64) target="aarch64-apple-darwin" ;;
      x86_64) target="x86_64-apple-darwin" ;;
      *) echo "error: unsupported macOS arch: $arch" >&2; exit 1 ;;
    esac ;;
  Linux)
    case "$arch" in
      x86_64) target="x86_64-unknown-linux-gnu" ;;
      aarch64 | arm64) target="aarch64-unknown-linux-gnu" ;;
      *) echo "error: unsupported Linux arch: $arch" >&2; exit 1 ;;
    esac ;;
  *)
    echo "error: unsupported OS: $os (use install.ps1 on Windows)" >&2; exit 1 ;;
esac

asset="vastline-${target}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${asset}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
echo "downloading ${url}"
curl -fsSL "$url" -o "$tmp/$asset"
tar -xzf "$tmp/$asset" -C "$tmp"

mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmp/vastline" "$INSTALL_DIR/vastline"
echo "installed → $INSTALL_DIR/vastline"
echo

# Wire it into ~/.claude/settings.json (backs up first, captures any existing status line).
"$INSTALL_DIR/vastline" install

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo; echo "note: $INSTALL_DIR is not on your PATH — add it to run 'vastline' commands directly." ;;
esac

echo
echo "next: add a read-only API key —"
echo "  vastai create api-key --name vastline --permissions '{\"api\": {\"instance_read\": {}, \"user_read\": {}}}'"
echo "  $INSTALL_DIR/vastline key set"
