#!/usr/bin/env bash
# Reclaim lane build dirs AND stale worktrees.
#
#   scripts/lane-gc.sh                              # report only
#   scripts/lane-gc.sh --apply                      # retire merged/detached worktrees + dead targets
#   scripts/lane-gc.sh --apply --include-unmerged   # also retire unmerged lanes (branch is KEPT)
#
# WHY: nothing ever reclaimed either one. Measured 2026-10-20: 45 target dirs at
# 224 GB (37 dead, 106 GB) and 40 worktrees serving 6 live lanes. The worktrees
# matter twice over -- they waste space, and a surviving worktree makes its
# target dir look live, so target-only GC reclaims almost nothing. Liveness is
# NOT "has a worktree"; that was the bug that made the first version useless.
#
# SAFETY, strictest first:
#   - A DIRTY worktree is never touched. `git worktree remove` without --force
#     refuses it anyway, and that refusal is a feature: one of these held 2558
#     modified files.
#   - An UNMERGED branch's worktree is retired only with --include-unmerged, and
#     removing a worktree NEVER deletes its branch, so the work survives and can
#     be checked out again.
#   - Target dirs are pure build output, always regenerable.
#   - Lane HOMEs are never touched: reports cite their receipts and evidence.
set -euo pipefail
export HOME=${ROMACS_REAL_HOME:-$HOME}
_lane_cache=${ROMACS_REAL_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}}


REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
KEEP="$REPO $_lane_cache/romacs-warm-tree"
# Historical active entries remain protected until their owner retires them.
ACTIVE_FILE=$_lane_cache/lanes/ACTIVE
active=$(grep -vE '^\s*(#|$)' "$ACTIVE_FILE" 2>/dev/null || true)

APPLY=no; UNMERGED=no
for a in "$@"; do
  case "$a" in
    --apply) APPLY=yes;;
    --include-unmerged) UNMERGED=yes;;
    *) echo "lane-gc: unknown argument $a" >&2; exit 2;;
  esac
done

# Apply requires a separately coordinated, durable record. Nothing auto-copies
# into the owner's record: run lane-receipts.py preserve explicitly first.
GUARD="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/lane-receipts.py"
if [ "$APPLY" = yes ]; then
  : "${ROMACS_RECORD:?set ROMACS_RECORD to the coordinated durable evidence record}"
fi
cd "$REPO"
echo "== worktrees"
removed=0; kept=0
while IFS=$'\t' read -r wt br; do
  [ -n "$wt" ] || continue
  skip=no
  for k in $KEEP; do [ "$wt" = "$k" ] && skip=yes; done
  [ "$skip" = yes ] && continue
  n=$(basename "$wt")

  if printf '%s\n' "$active" | grep -qx "$n"; then
    printf '  ACTIVE %-19s in progress -- never retired\n' "$n"; kept=$((kept+1)); continue
  fi

  dirty=$(git -C "$wt" status --porcelain 2>/dev/null | wc -l)
  if [ "$dirty" -gt 0 ]; then
    printf '  KEEP   %-19s dirty=%s -- never touched\n' "$n" "$dirty"; kept=$((kept+1)); continue
  fi

  bn=${br#refs/heads/}
  if [ "$bn" = "-" ] || [ -z "$bn" ]; then state=detached
  elif git merge-base --is-ancestor "$bn" master 2>/dev/null; then state=merged
  else state=unmerged
  fi

  if [ "$state" = unmerged ] && [ "$UNMERGED" != yes ]; then
    printf '  KEEP   %-19s %-30s unmerged (pass --include-unmerged)\n' "$n" "$bn"; kept=$((kept+1)); continue
  fi

  printf '  RETIRE %-19s %-30s %s\n' "$n" "${bn:--(detached)}" "$state"
  if [ "$APPLY" = yes ]; then
    python3 "$GUARD" check "$wt" --record "$ROMACS_RECORD" || {
      printf '         receipt/runs protection refused retirement\n'; kept=$((kept+1)); continue;
    }
    git worktree remove "$wt" 2>/dev/null || { printf '         refused by git, left alone\n'; kept=$((kept+1)); continue; }
    removed=$((removed+1))
  fi
done < <(git worktree list --porcelain | awk '
    /^worktree /{w=$2; b="-"}
    /^branch /{b=$2}
    /^$/{if(w!="")print w"\t"b; w=""}
    END{if(w!="")print w"\t"b}')

[ "$APPLY" = yes ] && git worktree prune

echo
echo "== build dirs (dead once no worktree claims the lane)"
live=$(git worktree list --porcelain | awk '/^worktree /{print $2}' | sed 's#.*/##' | sort -u)
total=0; n=0
for d in $_lane_cache/lanes/*-target; do
  [ -d "$d" ] || continue
  lane=$(basename "$d" -target)
  if printf '%s\n' "$live" | grep -qx "$lane"; then
    printf '  keep   %-19s %6s\n' "$lane" "$(du -sh "$d" 2>/dev/null | cut -f1)"
  else
    mb=$(du -sm "$d" 2>/dev/null | cut -f1 || echo 0)
    printf '  DEAD   %-19s %6s MB\n' "$lane" "$mb"
    if [ "$APPLY" = yes ]; then
      python3 "$GUARD" prune-build "$d" --record "$ROMACS_RECORD" \
        --build-root $_lane_cache/lanes || {
          printf '         receipt/runs protection refused build prune\n'; continue;
        }
    fi
    total=$((total+mb)); n=$((n+1))
  fi
done

echo
if [ "$APPLY" = yes ]; then
  echo "lane-gc: retired $removed worktree(s), kept $kept, removed $n target dir(s) ($((total/1024)) GB)"
else
  echo "lane-gc: kept $kept worktree(s); $n dead target dir(s) holding $((total/1024)) GB -- rerun with --apply"
fi
du -sh $_lane_cache/lanes 2>/dev/null | sed 's/^/lane-gc: lanes dir now /'
