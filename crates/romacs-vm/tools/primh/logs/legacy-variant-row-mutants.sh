#!/usr/bin/env bash
# Red proof for tails::tests::every_legacy_variant_has_its_row.
# Each mutant must apply (file bytes change), compile, leave the old
# ordinal test passing and fail the new test AT its target assertion.
set -u
cd "${ROMACS_MUTATION_ROOT:?set ROMACS_MUTATION_ROOT to a disposable source checkout}"
output=${ROMACS_MUTATION_OUTPUT:?set ROMACS_MUTATION_OUTPUT to an output directory}
mkdir -p "$output"
F=crates/romacs-vm/src/lib.rs
cp "$F" "$output/lib.rs.orig"
restore() { cp "$output/lib.rs.orig" "$F"; }
trap restore EXIT
run() {
  local name=$1 old=$2 new=$3
  restore
  python3 - "$F" "$old" "$new" <<'PY'
import sys
p, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(p).read()
assert s.count(old) == 1, f"pattern count {s.count(old)}"
open(p, "w").write(s.replace(old, new))
PY
  if cmp -s "$F" "$output/lib.rs.orig"; then echo "$name: MUTANT DID NOT APPLY"; return; fi
  cargo test -p romacs-vm --lib tails::tests:: > "$output/align-$name.log" 2>&1
  local rc=$?
  echo "$name: exit $rc"
  grep -E "^test tails::tests::|panicked at|enum Primitive has|is not declared variant|error\[" "$output/align-$name.log" | head -8
}
run trailing-variant-without-row $'    AutoloadDoLoad,\n}\n' $'    AutoloadDoLoad,\n    MutantTrailing,\n}\n'
run last-row-removed $'    ("autoload-do-load", Primitive::AutoloadDoLoad),\n' ''
restore
cargo test -p romacs-vm --lib tails::tests:: > "$output/align-green.log" 2>&1
echo "green: exit $?"; grep -E "^test tails::tests::|test result" "$output/align-green.log"
