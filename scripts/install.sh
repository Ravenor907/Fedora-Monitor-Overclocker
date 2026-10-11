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

# Add it to the Game Mode library once, like Desktop Mode's "Add to Steam"
# (SteamOS's helper; needs Steam running). Skipped when it's already there.
IN_STEAM=0
for vdf in "$HOME"/.local/share/Steam/userdata/*/config/shortcuts.vdf; do
    [ -f "$vdf" ] && grep -aq "monitor-oc" "$vdf" && IN_STEAM=1
done
if [ "$IN_STEAM" = 0 ] && [ -f "$ENTRY" ] && command -v steamos-add-to-steam > /dev/null 2>&1 &&
    pgrep -x steam > /dev/null 2>&1; then
    echo "==> Adding Monitor OC to your Game Mode library"
    steamos-add-to-steam "$ENTRY" > /dev/null 2>&1 && IN_STEAM=1
fi

echo
echo "Done! Open \"Monitor OC\" from the app menu or your desktop."
if [ "$IN_STEAM" = 1 ]; then
    echo "In Game Mode it's in your library (Non-Steam)."
else
    echo "For Game Mode: in Steam, Games > Add a Non-Steam Game to My Library, tick Monitor OC."
fi
echo "It keeps itself up to date from here on."
