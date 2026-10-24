#!/usr/bin/env bash
set -u
_lane="${1:?usage: . scripts/lane-env.sh LANE}"
if [ -n "${BASHPID:-}" ] && [ "$BASHPID" != "$$" ]; then
    printf 'lane-env: source on its own line, not in a subshell\n' >&2
    return 1 2>/dev/null || exit 1
fi
ROMACS_REAL_HOME=${ROMACS_REAL_HOME:-$HOME}
ROMACS_REAL_CACHE=${ROMACS_REAL_CACHE:-${XDG_CACHE_HOME:-$ROMACS_REAL_HOME/.cache}}
export ROMACS_REAL_HOME ROMACS_REAL_CACHE
export CARGO_HOME=${CARGO_HOME:-$ROMACS_REAL_HOME/.cargo}
export GNUPGHOME=${GNUPGHOME:-$ROMACS_REAL_HOME/.gnupg}
export HOME=${HOME_OVERRIDE:-$ROMACS_REAL_CACHE/romacs-lanes/$_lane}
mkdir -p "$HOME/.cache" "$HOME/.config" "$HOME/.local/share" "$HOME/.local/state" "$HOME/tmp"
for name in emax-gate romacs-test-fonts sccache; do
    if [ -d "$ROMACS_REAL_CACHE/$name" ] && [ ! -e "$HOME/.cache/$name" ] && [ ! -L "$HOME/.cache/$name" ]; then
        ln -s "$ROMACS_REAL_CACHE/$name" "$HOME/.cache/$name"
    fi
done
if [ ! -e "$HOME/.cargo" ] && [ ! -L "$HOME/.cargo" ]; then
    ln -s "$CARGO_HOME" "$HOME/.cargo"
fi
export XDG_CONFIG_HOME="$HOME/.config" XDG_CACHE_HOME="$HOME/.cache"
export XDG_DATA_HOME="$HOME/.local/share" XDG_STATE_HOME="$HOME/.local/state"
export TMPDIR="$HOME/tmp"
export CARGO_TARGET_DIR=${LANE_TARGET_OVERRIDE:-$ROMACS_REAL_CACHE/romacs-lanes/$_lane-target}
lane_commit() { env HOME="$ROMACS_REAL_HOME" git commit -S "$@"; }
lane_emacs() {
    local h status=0
    h=$(mktemp -d "$TMPDIR/emacs.XXXXXX") || return
    env -i PATH="$PATH" HOME="$h" XDG_CONFIG_HOME="$h/c" XDG_CACHE_HOME="$h/ca" \
        XDG_DATA_HOME="$h/d" XDG_STATE_HOME="$h/s" TMPDIR="$h" LANG=C.UTF-8 \
        "${ROMACS_GNU_ORACLE:-emacs}" -Q "$@" || status=$?
    rm -rf -- "$h"
    return "$status"
}
nix_tool() { nix --extra-experimental-features 'nix-command flakes' build "nixpkgs#$1" --no-link --print-out-paths; }
printf 'lane-env: %s HOME=%s target=%s\n' "$_lane" "$HOME" "$CARGO_TARGET_DIR"
