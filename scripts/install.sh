#!/usr/bin/env bash
# Monitor OC installer for SteamOS and other Linux (run in Desktop Mode, no sudo needed).
# Builds Monitor OC from this folder with a private Rust compiler, then installs it:
# ~/.local/bin/monitor-oc, the app menu entry and a desktop shortcut.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_ID="io.github.ravenor907.MonitorOC"

if [ "$(id -u)" -eq 0 ]; then
    echo "Run this as your normal user, not with sudo." >&2
    exit 1
fi
if [ "$(uname -m)" != x86_64 ]; then
    echo "Monitor OC needs an x86_64 PC or handheld." >&2
    exit 1
fi

echo "==> Installing Monitor OC $(tr -d '[:space:]' < "$ROOT/VERSION")"
BIN="$(bash "$ROOT/scripts/build.sh" "$ROOT" | tail -n 1)"
"$BIN" install

# Handy desktop shortcut, like Ally Hub's.
ENTRY="${XDG_DATA_HOME:-$HOME/.local/share}/applications/$APP_ID.desktop"
if [ -d "$HOME/Desktop" ] && [ -f "$ENTRY" ]; then
    cp "$ENTRY" "$HOME/Desktop/$APP_ID.desktop"
    chmod +x "$HOME/Desktop/$APP_ID.desktop"
fi

echo
echo "Done! Open \"Monitor OC\" from the app menu or your desktop."
echo "It keeps itself up to date from here on."
