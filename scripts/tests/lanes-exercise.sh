#!/usr/bin/env bash
# Exercise lanes against throwaway registries. Safe to rerun; touches no live state.
# Usage: exercise.sh   (exit 0 only if every expectation held)
set -u
# Tools live one level up in scripts/; scratch state goes to a temp dir so
# rerunning never writes throwaway registries into the repository.
W=$(cd "$(dirname "$0")/.." && pwd)
SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/lanes-exercise.XXXXXX")
trap 'rm -rf "$SCRATCH"' EXIT
L="$W/lanes --registry $SCRATCH/reg-race"
rm -rf "$SCRATCH/reg-race"
fail=0
expect() { # expect RC DESCRIPTION -- CMD...
  local want=$1 what=$2; shift 3
  "$@" >"$SCRATCH/.out" 2>&1; local got=$?
  if [ "$got" = "$want" ]; then echo "PASS ($got) $what"; else echo "FAIL (got $got, want $want) $what"; sed 's/^/    /' "$SCRATCH/.out"; fail=1; fi
}

echo "## 8 lanes race for the same surface at once: exactly one may win"
python3 - "$W" "$SCRATCH" <<'EOF'
import subprocess, sys
w = sys.argv[1]; scratch = sys.argv[2]
ps = [subprocess.Popen([f'{w}/lanes', '--registry', f'{scratch}/reg-race', 'claim', f'racer{i}', '--base', 'master',
                        '--surface', 'crate:romacs-vm'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
      for i in range(8)]
rcs = [p.wait() for p in ps]
print('rcs', rcs)
sys.exit(0 if rcs.count(0) == 1 and rcs.count(2) == 7 else 1)
EOF
[ $? = 0 ] && echo "PASS race: one winner, seven refusals" || { echo "FAIL race"; fail=1; }
n=$(ls "$SCRATCH/reg-race/claims" | wc -l); [ "$n" = 1 ] && echo "PASS one claim file on disk" || { echo "FAIL $n claim files"; fail=1; }

echo "## expiry frees a stale claim"
expect 0 "short-lived claim" -- $L claim short --base master --surface crate:romacs-font --ttl 0.0003
sleep 2
expect 0 "successor claims the expired surface" -- $L claim successor --base master --surface crate:romacs-font
grep -q '"event": "expire", "lane": "short"' "$SCRATCH/reg-race/history.jsonl" && echo "PASS expiry logged" || { echo "FAIL expiry not logged"; fail=1; }

echo "## renew / release / renew-after-release"
expect 0 "renew" -- $L renew successor
expect 0 "release parked" -- $L release successor --outcome parked --note "revive when the font crate lands"
expect 2 "renew after release refuses" -- $L renew successor
grep -q '"outcome": "parked"' "$SCRATCH/reg-race/farm.jsonl" && echo "PASS release wrote a farm row" || { echo "FAIL no farm row"; fail=1; }

echo "## refusals"
expect 2 "base not on master" -- $L claim bad --base 3f59a13b --surface crate:romacs-font
expect 2 "surface typo" -- $L claim typo --base master --surface crate:romacs-vmm2
expect 2 "file inside a claimed crate" -- $L claim inner --base master --surface crates/romacs-vm/src/gc.rs
expect 0 "disjoint dir of a crate someone else partly holds" -- $L claim layout-font --base master --surface 'crates/romacs-layout-engine/src/font/'
expect 0 "sibling dir in the same crate" -- $L claim layout-rows --base master --surface 'crates/romacs-layout-engine/src/display_row/'
expect 2 "whole crate over two partial holders" -- $L claim layout-all --base master --surface crate:romacs-layout-engine
expect 2 "new-file probe: dir claim over a new subdir someone holds" -- $L claim newdir --base master --surface 'crates/romacs-layout-engine/src/font/new_backend/' --allow-empty

echo "## surface syntax and multi-surface claims"
# Both of these shipped broken and were caught only by using the tool for real.
# Repeated --surface flags replaced each other under nargs, so a lane that asked
# for three surfaces silently held one; and an invented WORD: prefix was taken as
# a literal filename, matching nothing. A surface matching nothing also overlaps
# nothing, so two lanes could claim one directory and both be told it was free.
expect 2 "invented dir: prefix is refused, not taken literally" -- $L claim pfx --base master --surface dir:lisp/international --allow-empty
grep -q "unknown surface prefix 'dir'" "$SCRATCH/.out" && echo "PASS names the bad prefix" || { echo "FAIL prefix message"; fail=1; }
grep -q "Did you mean 'lisp/international/'" "$SCRATCH/.out" && echo "PASS suggests the real syntax" || { echo "FAIL prefix hint"; fail=1; }
expect 2 "invented file: prefix is refused too" -- $L claim pfx2 --base master --surface file:lisp/transient.el --allow-empty
expect 2 "an unknown prefix is refused even on --forbid" -- $L claim pfx3 --base master --surface lisp/transient.el --forbid dir:lisp/vendor

expect 0 "three surfaces via repeated flags are ALL retained" -- $L claim multi --base master --surface lisp/international/ --surface lisp/transient.el --surface lisp/jka-compr.el
n=$($L list --json | python3 -c 'import json,sys; print(len([x for x in json.load(sys.stdin) if x["lane"]=="multi"][0]["surface"]))')
[ "$n" = 3 ] && echo "PASS all three surfaces retained" || { echo "FAIL retained $n of 3 surfaces"; fail=1; }
# a surface must match real files, or overlap detection is vacuous
owned=$($L list --json | python3 -c 'import json,sys; print([x for x in json.load(sys.stdin) if x["lane"]=="multi"][0]["surface"])')
case "$owned" in *"lisp/international/**"*) echo "PASS surface normalised to a real glob";; *) echo "FAIL surface not normalised: $owned"; fail=1;; esac
expect 2 "overlap now actually fires on a claimed directory" -- $L claim multi2 --base master --surface lisp/international/
grep -q 'surface overlaps multi' "$SCRATCH/.out" && echo "PASS overlap names the holder" || { echo "FAIL overlap"; fail=1; }
$L release multi --outcome parked --note "syntax test" >/dev/null 2>&1

echo "## transitive certification must refuse anything that touches gate inputs"
# This relaxes a safety signal, so its refusals matter more than its successes.
python3 - <<'PYEOF' || fail=1
from importlib.machinery import SourceFileLoader
import importlib.util
from pathlib import Path
loader = SourceFileLoader('lanes', 'scripts/lanes')
spec = importlib.util.spec_from_loader('lanes', loader)
L = importlib.util.module_from_spec(spec); loader.exec_module(L)
repo = Path('.').resolve()
bad = 0

# scripts/gate IS the harness and must never be treated as irrelevant.
if 'scripts/gate'.startswith(L.NON_GATE_INPUTS):
    print('FAIL scripts/gate is allowlisted as a non-gate input'); bad = 1
else:
    print('PASS scripts/gate is not allowlisted')

# A delta containing Rust source must refuse.
mt = L.git(repo, 'rev-parse', 'master^{tree}').stdout.strip()
for ref in ('master~1', 'master~2', 'master~3'):
    t = L.git(repo, 'rev-parse', ref + '^{tree}', check=False).stdout.strip()
    if not t:
        continue
    paths, ok = L._non_gate_only(repo, mt, t)
    if any(x.endswith('.rs') for x in paths):
        if ok:
            print(f'FAIL {ref}: delta has .rs files but was called gate-irrelevant'); bad = 1
        else:
            print(f'PASS {ref}: .rs in delta refuses transitive certification')
        break
else:
    print('PASS (no .rs delta available to test against; skipped)')

# The allowlist must stay small; growth is how this becomes a hand-wave.
if len(L.NON_GATE_INPUTS) > 4:
    print(f'FAIL allowlist grew to {len(L.NON_GATE_INPUTS)} entries: {L.NON_GATE_INPUTS}'); bad = 1
else:
    print(f'PASS allowlist is still {len(L.NON_GATE_INPUTS)} entries')
raise SystemExit(bad)
PYEOF

echo "## canonical checkout parked on a feature branch fails the merge gate"
C="$W/replay"
if [ -d "$C" ]; then
  git -C "$C" update-ref refs/heads/master 61682d33
  git -C "$C" symbolic-ref HEAD refs/heads/parked-feature
  expect 1 "check with canonical on a branch" -- "$W/lanes" --registry "$SCRATCH/reg-replay" --repo "$C" check --tip e536ccda --surface crate:romacs-vm --canonical "$C"
  grep -q "^FAIL canonical checkout" "$SCRATCH/.out" && echo "PASS names the parked checkout" || { echo "FAIL message"; fail=1; }
  git -C "$C" symbolic-ref HEAD refs/heads/master
  expect 0 "same check with canonical on master" -- "$W/lanes" --registry "$SCRATCH/reg-replay" --repo "$C" check --tip e536ccda --surface crate:romacs-vm --canonical "$C"
fi
rm -f "$SCRATCH/.out"
[ $fail = 0 ] && echo "ALL EXPECTATIONS HELD" || echo "SOME EXPECTATIONS FAILED"
exit $fail
