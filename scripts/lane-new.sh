#!/usr/bin/env bash
# Create a lane: worktree + a build dir PRIMED from the warm reference.
#
#   scripts/lane-new.sh <lane> <branch> [base]
#   scripts/lane-new.sh mnb vm2/minibuf master
#
# Replaces the hand-rolled `git worktree add` in every brief, which left each
# lane to compile the workspace from nothing. See scripts/lane-warm.sh for why
# cloning works and why sccache cannot do this job.
set -euo pipefail
export HOME=${ROMACS_REAL_HOME:-$HOME}
_lane_cache=${ROMACS_REAL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}}

LANE=${1:?usage: lane-new.sh <lane> <branch> [base]}
BRANCH=${2:?usage: lane-new.sh <lane> <branch> [base]}
BASE=${3:-master}

REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
WARM=$_lane_cache/romacs-warm-target
DIR=$HOME/Worktrees/romacs/$LANE
TARGET=$_lane_cache/lanes/$LANE-target



[ -e "$DIR" ] && { echo "lane-new: $DIR already exists -- refusing to clobber a lane" >&2; exit 1; }
[ -e "$TARGET" ] && { echo "lane-new: $TARGET already exists -- run lane-gc.sh first" >&2; exit 1; }

# Branch may already exist (a finisher resuming one); only create it if not.
# Track whether WE created it, so a rollback removes only what we made: deleting
# a pre-existing branch on a failed clone would destroy a finisher's work.
CREATED_BRANCH=no
if git -C "$REPO" show-ref -q --verify "refs/heads/$BRANCH"; then
  git -C "$REPO" worktree add "$DIR" "$BRANCH"
else
  git -C "$REPO" worktree add -b "$BRANCH" "$DIR" "$BASE"
  CREATED_BRANCH=yes
fi

mkdir -p $_lane_cache/lanes
if [ -d "$WARM" ]; then
  # --reflink=auto, not =always: if the cache ever moves off btrfs this degrades
  # to a plain copy instead of failing and leaving a lane with no build dir.
  #
  # FAIL CLOSED. This was `if cp ...; then echo ok; fi` with no else branch, so a
  # failed clone fell straight through to ACTIVE registration and a cheerful
  # "ready" message, handing the lane a build dir that was absent or truncated.
  # A lane that cannot build must not be reported as ready.
  if ! cp -a --reflink=auto "$WARM" "$TARGET"; then
    echo "lane-new: FAILED to prime $TARGET from $WARM" >&2
    # A failed clone can contain evidence too. Keep both trees unless the
    # owner supplied a coordinated record and all receipts already survive.
    GUARD="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lane-receipts.py"
    if [ -z "${ROMACS_RECORD:-}" ] ||
       ! python3 "$GUARD" check "$DIR" --record "$ROMACS_RECORD" ||
       ! python3 "$GUARD" prune-build "$TARGET" --record "$ROMACS_RECORD" \
           --build-root $_lane_cache/lanes; then
      echo "lane-new: rollback protected; preserve evidence before manual retirement" >&2
      exit 1
    fi
    git -C "$REPO" worktree remove "$DIR" || exit 1
    # Removing the worktree does NOT remove the branch. Testing this rollback
    # path for the first time left `infra/failtest-probe` behind, so a failing
    # warm reference would litter a branch per attempt. Delete ONLY a branch we
    # created ourselves.
    if [ "$CREATED_BRANCH" = yes ]; then
      git -C "$REPO" branch -D "$BRANCH" >/dev/null 2>&1 || true
    fi
    git -C "$REPO" worktree prune 2>/dev/null || true
    echo "lane-new: lane rolled back; fix the warm reference and retry" >&2
    exit 1
  fi
  echo "lane-new: primed $TARGET from the warm reference ($(du -sh --apparent-size "$TARGET" | cut -f1) apparent)"
else
  echo "lane-new: WARNING no warm reference at $WARM -- this lane will build cold."
  echo "lane-new:         run scripts/lane-warm.sh once, then future lanes are primed."
fi

# Register as in-progress so lane-gc.sh will not retire it while unmerged.
grep -qxF "$LANE" $_lane_cache/lanes/ACTIVE 2>/dev/null || echo "$LANE" >> $_lane_cache/lanes/ACTIVE

cat <<EOF

lane-new: $LANE ready on $BRANCH at $DIR (registered in lanes/ACTIVE)

  Brief the lane with exactly this. Every command goes through lane-run:

    cd $DIR
    $REPO/scripts/lane-run $LANE -- cargo test -p romacs-vm
    # or from anywhere:
    $REPO/scripts/lane-run $LANE -C $DIR -- <cmd>

  DO NOT brief \`. scripts/lane-env.sh\`. Sourcing is the single most expensive
  mistake in this project's history: its exports outlive the statement, so a
  shared agent terminal carries one lane's HOME, GNUPGHOME and
  CARGO_TARGET_DIR into every sibling that reuses it. That has produced false
  \`%G?\`=E and "No public key" on correctly signed commits, a gate insisting
  the repository did not exist, a docs check calling a present file missing,
  and a build that silently used another lane's target dir -- eleven lanes and
  the orchestrator, in one day. lane-run runs the command in a fresh process,
  so nothing it exports can escape, it forwards signals, it exports pipefail so
  a failed \`cargo clippy | tail\` cannot report 0, and it logs each run with
  peak RSS to the farm registry.

  Do NOT set SCCACHE_SERVER_PORT or RUSTC_WRAPPER: sccache measured as a net
  loss here (it forces CARGO_INCREMENTAL=0 for a 0% cross-worktree hit rate)
  and is deliberately unwired.
EOF
