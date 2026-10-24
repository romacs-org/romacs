#!/usr/bin/env bash
# Run via ~/cockpit/bin/neomacs-slot props --group props --light -- bash THIS
set -euo pipefail
# Toolchain: cargo/rustfmt from PATH (the repo's pinned rust-toolchain).
command -v cargo >/dev/null || { echo "cargo not on PATH" >&2; exit 1; }
HERE=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO=$(git -C "$HERE" rev-parse --show-toplevel)
cd "$REPO"
rustfmt --check --edition 2024 crates/romacs-props/src/{lib,tree,sets,p1_tests}.rs
cargo test -p romacs-props
cargo clippy -p romacs-props --all-targets -- -D warnings
export P1_MUTATION_ROOT=${TMPDIR:-$HOME/.cache}/P1-mutations-final-$(date +%Y%m%d-%H%M%S)-$$
python3 "$HERE/P1-mutations.py"
