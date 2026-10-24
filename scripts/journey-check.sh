#!/usr/bin/env bash
# journey-check.sh -- re-measure the owner's first-session journey ladder.
#
#   scripts/journey-check.sh [OUT_DIR]
#
# Builds this checkout's GUI and census binaries in RELEASE into a private
# target (never the owner's ~/.cache/romacs/target), then:
#   1. GNU 32.0.50 (-Q, throwaway HOME): which file defines each journey
#      command, and GNU's C primitive list;
#   2. VM2 (census binary, WARM: loadup.el chain first): which of those
#      primitives are not fbound, and where each journey file stops loading;
#   3. GNU interactive (-nw in a private tmux pty): every VM2-missing
#      primitive GNU calls while doing each step (gnu-trace.el);
#   4. the REAL GUI on the owner's Hyprland (workspace 2, HDMI-A-1), driven
#      with wtype, for the default launcher (VM2's workspace, no GNU Lisp),
#      --gnu-lisp (loadup.el alone) and --loadup (GNU's loadup.el + romacs
#      stand-ins where GNU's command did not load) (ladder.py); plus the GUI's
#      own loadup.el table, headless (--loadup-report -> loadup.tsv);
#   5. judge.py: verdicts + ranked fix list -> OUT_DIR/LADDER-generated.md;
#      mutate_judge.py: the judge still fails on wrong records -> mutants.txt.
#
# Step 4 takes keyboard focus on workspace 2 for about six minutes and gives
# it back. Never run two of these at once. Exit status is 0 when the
# measurement completed, whatever the verdicts are; it is not a gate.
#
# Environment: JOURNEY_TARGET (build dir), JOURNEY_NO_BUILD=1 (reuse binaries).
set -euo pipefail
root=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/.." && pwd)
j=$root/crates/romacs-vm-gui/evidence/journey
head=$(git -C "$root" rev-parse --short=8 HEAD)
out=$(readlink -f "${1:-${XDG_CACHE_HOME:-$HOME/.cache}/romacs-journey/$head}")
target=${JOURNEY_TARGET:-$root/target}
mkdir -p "$out"
echo "journey-check: HOME=$HOME head=$head out=$out target=$target" >&2


if [ "${JOURNEY_NO_BUILD:-}" != 1 ]; then
    (cd "$root" && env -u GIT_DIR -u GIT_WORK_TREE CARGO_TARGET_DIR="$target" \
        cargo build --release --locked -p romacs-vm-gui -p romacs-vm) >"$out/build.log" 2>&1 \
        || { tail -n 20 "$out/build.log" >&2; exit 1; }
fi
gui=$target/release/romacs-vm-gui census=$target/release/romacs-vm-census
gnu() { local h; h=$(mktemp -d "$out/gnuhome.XXXX"); env -i PATH=/usr/bin:/bin HOME="$h" LANG=C.UTF-8 /usr/bin/emacs -Q "$@"; }

echo "journey-check: 1/5 GNU command files and primitive list" >&2
gnu --batch -l "$j/where.el" >"$out/gnu-where.txt" 2>/dev/null
gnu --batch -l "$j/gnu-primitives.el" "$out/gnu-primitives.txt"
(cd ~/Projects/rust/gnu-emacs-ref/src && grep -o 'DEFUN ("[^"]*"' ./*.c | sed 's|^\./||; s/:DEFUN ("/ /; s/"$//') >"$out/defun-files.txt"

echo "journey-check: 2/5 VM2 warm: missing primitives, load frontier" >&2
chain=(emacs-lisp/debug-early emacs-lisp/byte-run emacs-lisp/backquote subr keymap version widget custom
       emacs-lisp/map-ynp international/mule international/mule-conf env format bindings window files
       emacs-lisp/macroexp)
pre=(); for n in "${chain[@]}"; do pre+=(--preload "$root/lisp/$n.el"); done
mapfile -t lp < <(echo "$root/lisp"; find "$root/lisp" -mindepth 1 -type d | sort)
python3 - "$out/gnu-primitives.txt" "$out/probe-all.el" <<'EOF'
import sys
names = open(sys.argv[1]).read().split()
q = lambda n: '"' + n.replace('\\', '\\\\').replace('"', '\\"') + '"'
open(sys.argv[2], 'w').write(';;; -*- lexical-binding: t -*-\n(let ((missing nil))\n  (dolist (n (list '
    + ' '.join(map(q, names)) + '))\n    (unless (fboundp (intern n)) (setq missing (cons n missing))))\n'
    '  (signal (quote probe-missing) (nreverse missing)))\n')
EOF
"$census" "${pre[@]}" -- "$out/probe-all.el" "${lp[@]}" >"$out/probe-all.json" || true
python3 - "$out/probe-all.json" "$out/vm-missing.txt" <<'EOF'
import json, re, sys
d = json.loads(open(sys.argv[1]).read().splitlines()[-1])
s = d.get('diagnostic_condition') or ''
assert s.startswith('(probe-missing'), f'probe did not complete: {s[:200]!r} {d.get("error", "")[:300]!r}'
m = re.findall(r'"((?:[^"\\]|\\.)*)"', s)
open(sys.argv[2], 'w').write('\n'.join(m) + '\n')
print(f'journey-check: {len(m)} GNU primitives not fbound in warm VM2', file=sys.stderr)
EOF
python3 "$j/load_frontier.py" "$census" "$root" "$out/frontier.json" >"$out/frontier.log"

echo "journey-check: 3/5 GNU interactive trace (-nw in a private tmux)" >&2
rm -f "$out/trace.json" "$out/trace.done"
th=$(mktemp -d "$out/gnuhome.XXXX")
sess=journey-$$
tmux new-session -d -s "$sess" -x 120 -y 45 \
  "env -i PATH=/usr/bin:/bin HOME=$th TERM=xterm-256color LANG=C.UTF-8 /usr/bin/emacs -Q -nw -l $j/gnu-trace.el --eval '(journey-trace \"$j/fixture/notes.org\" \"$out/vm-missing.txt\" \"$out/trace.json\")' 2>$out/trace.err; touch $out/trace.done; sleep 30"
for _ in $(seq 1 150); do [ -f "$out/trace.done" ] && break; sleep 2; done
tmux kill-session -t "$sess" 2>/dev/null || true
[ -s "$out/trace.json" ] || { echo "journey-check: GNU trace produced nothing; see $out/trace.err" >&2; exit 1; }

echo "journey-check: 4/5 real GUI on the compositor (takes focus on workspace 2)" >&2
python3 "$j/ladder.py" "$out" --mode default --binary "$gui"
python3 "$j/ladder.py" "$out" --mode gnu-lisp --binary "$gui"
python3 "$j/ladder.py" "$out" --mode loadup --binary "$gui"
env -u RUST_MIN_STACK ROMACS_LISP_DIR="$root/lisp" "$gui" --loadup-report >"$out/loadup.tsv" 2>"$out/loadup.err"

echo "journey-check: 5/5 verdicts" >&2
python3 "$j/judge.py" "$out" "$out/trace.json" "$out/frontier.json" "$out/vm-missing.txt" \
    "$out/defun-files.txt" "$out/LADDER-generated.md" >/dev/null
python3 "$j/mutate_judge.py" "$out" "$out/trace.json" "$out/frontier.json" "$out/vm-missing.txt" \
    "$out/defun-files.txt" >"$out/mutants.txt" || true
sed -n '3,5p' "$out/LADDER-generated.md"
tail -n 1 "$out/mutants.txt"
echo "journey-check: wrote $out/LADDER-generated.md and $out/LADDER-generated.tsv" >&2
