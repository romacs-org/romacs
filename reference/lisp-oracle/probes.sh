#!/usr/bin/env bash
# usage: CENSUS_BIN=path/to/romacs-vm-census bash reference/lisp-oracle/probes.sh
B="${CENSUS_BIN:?set CENSUS_BIN to the built romacs-vm-census executable}"
R="$(cd "$(dirname "$0")/../.." && pwd)"
P="$R/reference/lisp-oracle/probes"
pre=()
for n in emacs-lisp/debug-early emacs-lisp/byte-run emacs-lisp/backquote subr keymap version widget custom emacs-lisp/map-ynp international/mule international/mule-conf env format bindings window files emacs-lisp/macroexp; do pre+=(--preload "$R/lisp/gnu/$n.el"); done
lp=("$R/lisp/gnu" "$R/lisp/gnu/emacs-lisp")
for f in "$P"/*.el; do
  out=$(cd "$R" && timeout 30 "$B" "${pre[@]}" -- "$f" "${lp[@]}" 2>/dev/null)
  python3 - "$f" "$out" <<'PY'
import json, sys, os
f, out = sys.argv[1], sys.argv[2]
rep = None
for line in out.splitlines():
    try:
        v = json.loads(line)
        if isinstance(v, dict) and v.get('measurement'):
            rep = rep or v
    except ValueError:
        pass
if not rep:
    print(f"{os.path.basename(f):34} NO REPORT"); sys.exit()
chain = rep.get('chain') or []
reach = ','.join(f"{os.path.basename(c['file'])}:{'ok' if c['loaded'] else 'STOP'}" for c in chain)
own = [s for s in rep.get('steps', []) if s.get('file') == f]
bad = [s for s in own if s.get('status') != 'ok']
res = 'LOADED' if rep.get('ok') and not bad else f"FAIL form {bad[0].get('ordinal')}: {bad[0].get('error')} | {(rep.get('condition_result') or rep.get('error') or '')[:160]}" if bad else f"FAIL {(rep.get('error') or '')[:200]}"
print(f"{os.path.basename(f):34} {res}")
print(f"{'':34} chain {reach}")
PY
done
