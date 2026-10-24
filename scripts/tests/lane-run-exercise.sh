#!/usr/bin/env bash
# Exercise lane-run. Uses lane name s78 and a throwaway run log; exit 0 iff all held.
set -u
# Tools live one level up in scripts/; scratch state goes to a temp dir so
# rerunning never writes throwaway registries into the repository.
W=$(cd "$(dirname "$0")/.." && pwd)
SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/lanes-exercise.XXXXXX")
trap 'rm -rf "$SCRATCH"' EXIT
export LANES_REGISTRY="$SCRATCH/reg-runs"; rm -rf "$LANES_REGISTRY"
WT=${1:-$HOME/.hermes/cache/scratch/romacs/vm2/s76}            # a built worktree
BARE=${2:-$HOME/.hermes/cache/scratch/romacs/vm2/s77-master}    # a bare worktree
fail=0
ok() { if [ "$1" = "$2" ]; then echo "PASS $3"; else echo "FAIL $3 (got $1, want $2)"; fail=1; fi; }

out=$(env GIT_DIR=/nonexistent CARGO_TARGET_DIR=/wrong LANE_TARGET_OVERRIDE=/wrong2 \
      "$W/lane-run" s78 -C "$WT" -- bash -c 'echo "$HOME|$CARGO_TARGET_DIR|${GIT_DIR-unset}|$(git rev-parse --show-toplevel)|$(git config user.email)"; cargo --version >/dev/null')
ok $? 0 "lane env applied, cargo on PATH"
IFS='|' read -r h t g top email <<<"$out"
ok "$h" "$HOME/.cache/lanes/s78" "HOME is the lane home"
ok "$t" "$HOME/.cache/lanes/s78-target" "leaked CARGO_TARGET_DIR and LANE_TARGET_OVERRIDE ignored"
ok "$g" unset "leaked GIT_DIR removed"
ok "$top" "$WT" "git sees the worktree"
[ -n "$email" ] && echo "PASS git identity present in lane HOME ($email)" || { echo "FAIL no git identity"; fail=1; }
ok "${CARGO_TARGET_DIR-unset}" unset "caller environment untouched"

"$W/lane-run" s78 -C "$WT" -- bash -c 'false | tail -1'; ok $? 1 "pipeline ending in tail reports the failure"
"$W/lane-run" s78 -C "$WT" -- bash -c 'exit 7'; ok $? 7 "exit status passes through"
"$W/lane-run" s78 -C "$BARE" -- bash -c 'exit 0' test; ok $? 0 "bare worktree permits VM2 test commands"

python3 - "$W" "$WT" <<'EOF'
import subprocess, sys, time, signal, json, os
w, wt = sys.argv[1], sys.argv[2]
p = subprocess.Popen([f'{w}/lane-run', 's78', '-C', wt, '--', 'sleep', '30'])
time.sleep(1.5); p.send_signal(signal.SIGTERM); rc = p.wait(timeout=10)
rows = [json.loads(l) for l in open(os.path.join(os.environ['LANES_REGISTRY'], 'runs.jsonl'))]
last = rows[-1]
alive = subprocess.run(['pgrep', '-f', '^sleep 30$'], capture_output=True).stdout.strip()
print('rc', rc, 'logged rc', last['rc'], 'wall', last['wall_s'], 'orphan', alive or None)
sys.exit(0 if rc == 143 and last['rc'] == 143 and last['wall_s'] < 5 and not alive else 1)
EOF
ok $? 0 "SIGTERM forwarded, logged as rc 143, no orphan"
n=$(wc -l < "$LANES_REGISTRY/runs.jsonl"); ok "$n" 5 "one run-log row per supervised command"
[ $fail = 0 ] && echo "ALL EXPECTATIONS HELD" || echo "SOME EXPECTATIONS FAILED"
exit $fail
