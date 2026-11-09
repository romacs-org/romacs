#!/usr/bin/env bash
# Census dual-mode mutants: each must make test_lisp_census_modes fail at an assertion.
. "${ROMACS_CENSUS_ENV:?set ROMACS_CENSUS_ENV to a setup script defining cbuild and EV}"
B=crates/romacs-vm/src/bin/romacs-vm-census.rs
P=tools/lisp-census
run() { # name file old new
  local name=$1 file=$2 old=$3 new=$4
  cp "$file" "$file.orig"
  python3 - "$file" "$old" "$new" <<'EOF'
import sys; p,o,n=sys.argv[1:]; s=open(p).read(); assert o in s, o; open(p,'w').write(s.replace(o,n,1))
EOF
  if [ "$file" = "$B" ]; then cbuild > $EV/mutant-$name-build.log 2>&1; fi
  python3 -m unittest discover -s scripts -p 'test_lisp_census_modes.py' -v > $EV/census-mutant-$name.log 2>&1
  local rc=$?
  mv "$file.orig" "$file"
  echo "$name rc=$rc $(grep -cE '^FAIL:' $EV/census-mutant-$name.log) FAIL $(grep -cE '^ERROR:' $EV/census-mutant-$name.log) ERROR"
}
run nocut "$B" 'steps.drain(..chain_steps.min(steps.len()));' 'let _ = chain_steps;'
run nostop "$B" $'if failed {\n                    break;\n                }' 'let _ = failed;'
run consensus "$P" "    if len(chains) != 1:" "    if False:"
run order "$P" "BOOTSTRAP_CHAIN = ['emacs-lisp/debug-early', 'emacs-lisp/byte-run'," "BOOTSTRAP_CHAIN = ['emacs-lisp/byte-run',"
cbuild > $EV/mutant-restore-build.log 2>&1
python3 -m unittest discover -s scripts -p 'test_lisp_census_modes.py' > $EV/census-modes-restored.log 2>&1; echo restored rc=$?
