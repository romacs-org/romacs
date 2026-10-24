#!/usr/bin/env bash
# Build ONE warm reference target dir on master, so new lanes clone it instead
# of compiling the workspace again.
#
# WHY THIS EXISTS. On 2026-10-20 the lane cache held 45 target dirs, 224 GB, and
# 107 separate builds of libromacs_vm. The obvious fix was sccache; it does not
# work here, because sccache puts the compilation CWD in its cache key, so two
# worktrees share nothing (measured: 100% hits across target dirs in ONE
# worktree, 0% across two, and --remap-path-prefix does not help). What does
# work is btrfs reflink: /home is btrfs, so cloning a warm target is a
# copy-on-write operation costing almost no time and almost no disk.
#
#   measured  reflink clone of a 2.0 GB target : 2.10 s, ~0 real disk
#   measured  romacs-vm in a fresh worktree   : 4.96 s primed vs 15.43 s cold
#
# Run this after every landing, so the reference tracks master.
set -euo pipefail
export HOME=${ROMACS_REAL_HOME:-$HOME}
_lane_cache=${ROMACS_REAL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}}

WARM=$_lane_cache/romacs-warm-target
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
REF=$_lane_cache/romacs-warm-tree


unset XDG_CACHE_HOME XDG_STATE_HOME XDG_DATA_HOME XDG_CONFIG_HOME TMPDIR 2>/dev/null || true

export CARGO_TARGET_DIR="$WARM"

# DO NOT set CARGO_INCREMENTAL here. This script used to force it to 0 while
# lanes left it at the profile default, and the mismatch is recorded in the
# fingerprint: a lane cloning this reference then rebuilt every unit it touched,
# because incremental-off artifacts do not satisfy an incremental-on request.
# Measured by review: switching incremental mode rebuilt all 3 units in a probe,
# while a consistent configuration rebuilt 0. The warmer must compile EXACTLY as
# a lane does, or the clone is worthless.
unset CARGO_INCREMENTAL RUSTC_WRAPPER SCCACHE_DIR

# The reference tree is a worktree of its own at a FIXED path. It must never be
# a lane: lanes change, and a reference that drifts primes every future lane
# with someone's half-finished work.
if [ ! -d "$REF/.git" ] && [ ! -f "$REF/.git" ]; then
  git -C "$REPO" worktree add --detach "$REF" master
fi
git -C "$REF" fetch -q "$REPO" master 2>/dev/null || true
git -C "$REF" checkout -q --detach FETCH_HEAD 2>/dev/null || git -C "$REF" checkout -q --detach master
echo "lane-warm: reference tree at $(git -C "$REF" rev-parse --short HEAD)"

cd "$REF"
# Build the TEST binaries, not just the libs: the gate's cost is dominated by
# compiling 1542 vm2 tests and 3321 display tests, and that is what a new lane
# would otherwise pay in full.
cargo test --workspace --no-run 2>&1 | tail -5

echo "lane-warm: warm target is $(du -sh "$WARM" | cut -f1) at $WARM"
echo "lane-warm: new lanes should be created with scripts/lane-new.sh to clone it"
