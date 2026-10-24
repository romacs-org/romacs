#!/usr/bin/env bash
# Mutation harness for the symbol-plist lane. Each mutant edits one line that
# a test protects, runs the protecting tests, records whether they FAILED
# (killed), and restores the file byte-for-byte from a copy taken first. Run from the repository root with
# the lane environment sourced. Exit status is non-zero if any mutant survives.
set -u
cd "$(git rev-parse --show-toplevel)"
out=crates/romacs-vm/evidence/plist/mutants
mkdir -p "$out"
survivors=0
mutate() { # name file python-literal-old python-literal-new
  local name=$1 file=$2 old=$3 new=$4
  cp "$file" "$out/.orig"
  python3 - "$file" "$old" "$new" <<'EOF' || { echo "MUTANT $name: pattern not found"; exit 2; }
import sys
p, old, new = sys.argv[1:]
old, new = (x.encode().decode('unicode_escape') for x in (old, new))
s = open(p).read()
assert s.count(old) == 1, f"{old!r} occurs {s.count(old)} times"
open(p, 'w').write(s.replace(old, new))
EOF
  timeout 900 cargo test -p romacs-vm --lib -- plist_tests reclamation_tests > "$out/$name.lib.log" 2>&1
  local lib=$?
  timeout 900 cargo test -p romacs-vm --test symbol_plists > "$out/$name.int.log" 2>&1
  local int=$?
  cp "$out/.orig" "$file"; rm -f "$out/.orig"
  # A mutant that does not compile proves nothing: only a FAILED test kills.
  if grep -q "error: could not compile" "$out/$name.lib.log" "$out/$name.int.log"; then
    echo "MUTANT $name: INVALID (does not compile)"; survivors=$((survivors + 1))
  elif grep -qE "^test .* FAILED$" "$out/$name.lib.log" "$out/$name.int.log"; then
    echo "MUTANT $name: KILLED (lib exit $lib, integration exit $int)"
    grep -hE "^test .* FAILED$" "$out/$name.lib.log" "$out/$name.int.log" | sed 's/^/    /'
    grep -hE -A1 "panicked at" "$out/$name.lib.log" "$out/$name.int.log" | grep -vE "^--$|panicked at" | cut -c1-220 | sed 's/^/      /'
  else
    echo "MUTANT $name: SURVIVED"; survivors=$((survivors + 1))
  fi
}
G=crates/romacs-vm/src/gc.rs
A=crates/romacs-vm/src/annotations.rs
C=crates/romacs-vm/src/code_image.rs
V=crates/romacs-vm/src/variable_runtime.rs
mutate no-mark-plist     $G '            pending.push(s.plist);\n' ''
mutate no-forward-plist  $G '            rewrite(&mut s.plist);\n' ''
mutate no-mark-local     $G '            pending.push(s.local);\n' ''
mutate no-forward-local  $G '            rewrite(&mut s.local);\n' ''
mutate name-keyed        $A '        Ok(Some(slot))\n    }\n    fn plist_owner_mut' '        Ok(Some(self.names.get(&self.symbol_names[slot]).copied().unwrap_or(slot)))\n    }\n    fn plist_owner_mut'
mutate no-image-plists   $C 'plists.push(json!([i, relocate(slot.plist)?.0]));' 'plists.push(json!([i, Value::NIL.0]));'
mutate no-localize       $V '            self.variable_redirects[slot] = Some(VariableRedirect::Localized);\n        }\n    }' '        }\n    }'
mutate prepend           $A '                        Some(prev) => self.conses[prev.index(3).expect("private cons")].1 = cell,' '                        Some(_) => { let old = self.slots[owner].plist; let (_, v) = self.cons_pair(cell).unwrap(); self.conses[v.index(3).unwrap()].1 = old; self.slots[owner].plist = cell; }'
mutate no-append-cow     $A '                if !prev.is_some_and(Self::image_value) {' '                if true {'
echo "survivors: $survivors"
[ "$survivors" -eq 0 ]
