#!/usr/bin/env bash
# R209 mutation evidence for evaluator_stack_tests. Run from the worktree root
# with the prepared project environment. Restores main.rs on exit.
set -u
src=crates/romacs-vm-gui/src/main.rs
out=${1:?evidence dir}
mkdir -p "$out"
cp "$src" "$out/main.rs.orig"
trap 'cp "$out/main.rs.orig" "$src"; rm -f "$out/main.rs.orig"' EXIT
run() { # name profile
  cargo test -p romacs-vm-gui --profile "$2" -j 8 --bin romacs-vm-gui \
    evaluator_stack_tests -- --nocapture >"$out/$1-$2.log" 2>&1
  echo "$1 $2 exit=$? $(grep -E '^test result' "$out/$1-$2.log" | tail -1)"
}
for prof in dev gate; do run green "$prof"; done
# A: remove the reservation entirely (std default 2 MiB).
sed -i 's/^        \.stack_size(EVALUATOR_STACK_SIZE)$/        \/\/ MUTANT: reservation removed/' "$src"
grep -q 'MUTANT: reservation removed' "$src" || { echo "mutant A not applied"; exit 2; }
for prof in dev gate; do run mutant-no-reservation "$prof"; done
cp "$out/main.rs.orig" "$src"
# B: shrink the reservation to the 2 MiB default explicitly.
sed -i 's/^const EVALUATOR_STACK_SIZE: usize = 64 \* 1024 \* 1024;$/const EVALUATOR_STACK_SIZE: usize = 2 * 1024 * 1024;/' "$src"
grep -q '= 2 \* 1024 \* 1024;' "$src" || { echo "mutant B not applied"; exit 2; }
for prof in dev gate; do run mutant-2mib "$prof"; done
cp "$out/main.rs.orig" "$src"
# C (control, explains OPTIMIZED_WIDTH): reservation removed AND the optimized
# 10x workload dropped -> the gate profile no longer notices, which is why the
# previous 320-operand test was vacuous under the gate.
sed -i 's/^        \.stack_size(EVALUATOR_STACK_SIZE)$/        \/\/ MUTANT: reservation removed/' "$src"
sed -i 's/^        if !cfg!(debug_assertions) {$/        if false {/' "$src"
grep -q 'if false {' "$src" || { echo "control C not applied"; exit 2; }
for prof in dev gate; do run control-no-reservation-no-10x "$prof"; done
