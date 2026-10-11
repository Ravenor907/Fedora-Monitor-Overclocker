#!/usr/bin/env bash
# Builds Monitor OC from a source folder (default: this repository) with a private
# Rust compiler in ~/.local/share/monitor-oc/toolchain, set up on first use like
# Ally Hub's Python environment: no sudo, nothing outside your home folder.
# Progress goes to stderr; the last line on stdout is the built program.
set -euo pipefail

SRC="$(cd "${1:-"$(dirname "${BASH_SOURCE[0]}")/.."}" && pwd)"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}/monitor-oc"
TOOLS="$DATA/toolchain"
export RUSTUP_HOME="$TOOLS/rustup" CARGO_HOME="$TOOLS/cargo"
export CARGO_TARGET_DIR="$DATA/build"
export PATH="$CARGO_HOME/bin:$PATH"
VERSION="$(tr -d '[:space:]' < "$SRC/VERSION")"

say() { printf '==> %s\n' "$*" >&2; }

if [ ! -x "$CARGO_HOME/bin/cargo" ]; then
    say "Setting up the Rust compiler (one time, about 300 MB)"
    mkdir -p "$TOOLS"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
        sh -s -- -y -q --profile minimal --default-toolchain stable --no-modify-path >&2
else
    # Newer sources may need a newer compiler.
    rustup update stable --no-self-update > /dev/null 2>&1 || true
fi

# Rust needs a C linker, and SteamOS has none. Zig (from PyPI, in a private
# Python environment) stands in for it there, through cargo-zigbuild.
if [ "${MONITOR_OC_LINKER:-}" = zig ] || ! command -v cc > /dev/null 2>&1; then
    VENV="$TOOLS/venv"
    if [ ! -x "$VENV/bin/cargo-zigbuild" ]; then
        say "Setting up the linker (one time)"
        if ! python3 -m venv "$VENV" 2> /dev/null; then
            rm -rf "$VENV"
            python3 -m venv --without-pip "$VENV"
            curl -sSL https://bootstrap.pypa.io/get-pip.py -o "$TOOLS/get-pip.py"
            "$VENV/bin/python" "$TOOLS/get-pip.py" -q >&2
            rm -f "$TOOLS/get-pip.py"
        fi
        "$VENV/bin/python" -m pip install -q --upgrade pip cargo-zigbuild >&2
    fi
    export PATH="$VENV/bin:$PATH"
    say "Building Monitor OC $VERSION (a few minutes)"
    (cd "$SRC" && cargo zigbuild --release --locked --target x86_64-unknown-linux-gnu.2.35) >&2
    out="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/monitor-oc"
else
    say "Building Monitor OC $VERSION (a few minutes)"
    (cd "$SRC" && cargo build --release --locked) >&2
    out="$CARGO_TARGET_DIR/release/monitor-oc"
fi

dest="$DATA/build/monitor-oc-$VERSION"
cp "$out" "$dest"
chmod 755 "$dest"
echo "$dest"
