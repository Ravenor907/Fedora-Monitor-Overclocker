#!/bin/sh
# Installs Monitor OC for the current user, no sudo needed: downloads the
# latest release, checks its SHA-256 and runs `monitor-oc install`, which puts
# the program in ~/.local/bin and adds "Monitor OC" to the app menu.
#
#   curl -fsSL https://raw.githubusercontent.com/Ravenor907/monitor-oc/main/install.sh | sh
#
# Uninstall:          ... | sh -s -- --uninstall [--keep-settings]
# A specific release: ... | MONITOR_OC_VERSION=v0.2.0 sh

set -eu

REPO="Ravenor907/monitor-oc"
ASSET="monitor-oc-x86_64"

say() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<EOF
Usage: install.sh [--uninstall [--keep-settings]]

Installs Monitor OC for your user account (no sudo needed).

  --uninstall       remove Monitor OC and its settings (the built-in screen
                    goes back to its stock refresh rates in Game Mode)
  --keep-settings   with --uninstall: keep the built-in screen settings

Set MONITOR_OC_VERSION (e.g. v0.2.0) to install that release instead of
the latest one.
EOF
}

# fetch URL FILE [quiet]
fetch() {
    if command -v curl >/dev/null 2>&1; then
        if [ "${3:-}" = quiet ]; then
            curl -fsSL --retry 3 --proto '=https' -o "$2" "$1"
        else
            curl -fL --retry 3 --proto '=https' --progress-bar -o "$2" "$1"
        fi
    else
        wget -q -O "$2" "$1"
    fi
}

verify() {
    if ! command -v sha256sum >/dev/null 2>&1; then
        warn "sha256sum is not installed, so the download cannot be checked"
        return
    fi
    want=$(cut -d ' ' -f 1 <"$tmp/$ASSET.sha256")
    have=$(sha256sum "$tmp/$ASSET" | cut -d ' ' -f 1)
    if [ "$want" != "$have" ]; then
        die "checksum mismatch (expected $want, got $have); the download is damaged, please try again"
    fi
    say "Checksum OK."
}

main() {
    action=install
    keep=
    for arg in "$@"; do
        case $arg in
        --uninstall) action=uninstall ;;
        --keep-settings) keep=1 ;;
        --purge) ;; # removing everything is the default
        -h | --help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            die "unknown option: $arg"
            ;;
        esac
    done
    if [ -n "$keep" ] && [ "$action" != uninstall ]; then
        die "--keep-settings only works together with --uninstall"
    fi
    # Every Monitor OC release understands --purge (older ones keep the
    # settings without it); newer ones also --keep-settings.
    flags=
    if [ "$action" = uninstall ]; then
        if [ -n "$keep" ]; then flags=--keep-settings; else flags=--purge; fi
    fi

    if [ "$(id -u)" -eq 0 ]; then
        die "run this as your normal user, not with sudo or as root: Monitor OC installs into your home directory"
    fi
    [ "$(uname -s)" = Linux ] || die "Monitor OC only runs on Linux"
    arch=$(uname -m)
    [ "$arch" = x86_64 ] || die "Monitor OC is only built for x86_64 PCs and handhelds (this is $arch)"
    [ -n "${HOME:-}" ] || die "HOME is not set"

    # Uninstalling needs no download when the installed copy is there.
    installed="$HOME/.local/bin/monitor-oc"
    if [ "$action" = uninstall ] && [ -x "$installed" ]; then
        "$installed" uninstall "$flags" </dev/null
        return
    fi

    if [ -n "${MONITOR_OC_VERSION:-}" ]; then
        case $MONITOR_OC_VERSION in
        v*) tag=$MONITOR_OC_VERSION ;;
        *) tag=v$MONITOR_OC_VERSION ;;
        esac
        base="https://github.com/$REPO/releases/download/$tag"
        label="Monitor OC $tag"
    else
        base="https://github.com/$REPO/releases/latest/download"
        label="the latest Monitor OC"
    fi

    if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
        die "this needs curl or wget to download Monitor OC"
    fi
    tmp=$(mktemp -d "${TMPDIR:-/tmp}/monitor-oc.XXXXXX") || die "could not create a temporary directory"
    trap 'rm -rf "$tmp"' EXIT
    trap 'exit 130' INT TERM

    say "Downloading $label..."
    fetch "$base/$ASSET" "$tmp/$ASSET" || die "download failed: $base/$ASSET"
    if fetch "$base/$ASSET.sha256" "$tmp/$ASSET.sha256" quiet 2>/dev/null; then
        verify
    else
        warn "this release has no checksum file, so the download cannot be checked"
    fi
    chmod +x "$tmp/$ASSET"

    if ! version=$("$tmp/$ASSET" --version 2>&1); then
        printf '%s\n' "$version" >&2
        die "the downloaded program does not run here (see above). If it mentions GLIBC, use
monitor-oc-x86_64-musl (command line only) from https://github.com/$REPO/releases.
If it says 'Permission denied', your temporary directory does not allow programs: run
this again with TMPDIR=\$HOME/.cache"
    fi
    say "Downloaded $version."
    "$tmp/$ASSET" "$action" ${flags:+"$flags"} </dev/null
}

main "$@"
