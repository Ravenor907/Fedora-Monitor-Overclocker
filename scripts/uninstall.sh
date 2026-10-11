#!/usr/bin/env bash
# Removes Monitor OC: the app, its menu entries, settings and Game Mode profile,
# and its private compiler. The ~/MonitorOC source folder is left alone.
set -u

BIN="$HOME/.local/bin/monitor-oc"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_ID="io.github.ravenor907.MonitorOC"

# The app removes what it set up (external monitor overrides ask for your password).
[ -x "$BIN" ] && "$BIN" uninstall --purge

rm -rf "$DATA/monitor-oc" "$HOME/.config/monitor-oc"
rm -f "$BIN" \
    "$DATA/applications/$APP_ID.desktop" \
    "$DATA/applications/$APP_ID.Rescue.desktop" \
    "$HOME/Desktop/$APP_ID.desktop" \
    "$HOME/.config/gamescope/scripts/zz-monitor-oc.lua"
command -v update-desktop-database > /dev/null 2>&1 &&
    update-desktop-database "$DATA/applications" > /dev/null 2>&1

echo "Monitor OC is removed. You can delete the ~/MonitorOC folder too."
echo "If you added it to Steam, remove that shortcut in Steam."
