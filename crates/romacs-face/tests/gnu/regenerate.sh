#!/bin/bash
# Regenerate the GNU oracle fixtures for romacs-face.
#   ROMACS_GNU_ORACLE=/abs/path/to/emacs regenerate.sh
# One isolated `emacs -Q --batch' per theme, throwaway HOME, no network.
# The oracle is GNU built at the pin (parity-reference.toml), named by
# $ROMACS_GNU_ORACLE and refused by scripts/gnu_oracle.py unless the binary
# itself reports the pin. There is no default: /usr/bin/emacs is 320 GNU
# commits older and produced the separator-line mismatch (R228).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
[ $# -eq 0 ] || { echo "regenerate.sh takes no arguments; set ROMACS_GNU_ORACLE" >&2; exit 2; }
emacs=$(python3 "$repo/scripts/gnu_oracle.py" path)
out="$here/fixtures"; mkdir -p "$out"
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
themes=$(find "$repo/etc/themes" -name '*-theme.el' -printf '%f\n' | sed 's/-theme\.el$//' | sort)
[ -n "$themes" ] || { echo "no themes under $repo/etc/themes" >&2; exit 1; }
for t in nil $themes; do
  q=$t; [ "$t" = nil ] || q="'$t"
  h=$(mktemp -d "$work/home.XXXX")
  env -i PATH=/usr/bin:/bin HOME="$h" TMPDIR="$h" LANG=C.UTF-8 timeout -s KILL 120 \
    "$emacs" -Q --batch -l "$here/dump.el" \
    --eval "(s86-dump $q \"$repo/etc/themes\" \"$work/$t.eld\")" > "$work/$t.log" 2>&1 \
    || { echo "$t failed:" >&2; tail -5 "$work/$t.log" >&2; exit 1; }
  cp "$work/$t.eld" "$out/$t.eld"
done
h=$(mktemp -d "$work/home.XXXX")
env -i PATH=/usr/bin:/bin HOME="$h" TMPDIR="$h" LANG=C.UTF-8 timeout -s KILL 60 \
  "$emacs" -Q --batch -l "$here/remap.el" --eval "(s86-remap-dump \"$out/remap.eld\")" \
  > "$work/remap.log" 2>&1 || { echo "remap failed:" >&2; tail -5 "$work/remap.log" >&2; exit 1; }
# One defface table for every run; a face must have the same spec in all.
python3 - "$work" "$out/deffaces.eld" <<'PY'
import sys, glob
seen = {}
for f in sorted(glob.glob(sys.argv[1] + '/*.eld.deffaces')):
    for line in open(f, encoding='utf-8'):
        line = line.rstrip('\n')
        if not line:
            continue
        name = line[1:line.index(' ')]
        if seen.setdefault(name, line) != line:
            sys.exit(f'defface spec for {name} differs between runs ({f})')
with open(sys.argv[2], 'w', encoding='utf-8') as o:
    for name in sorted(seen):
        o.write(seen[name] + '\n')
print(len(seen), 'deffaces')
PY
{ "$emacs" --version | head -1
  "$emacs" -Q --batch --eval '(princ (format "emacs-repository-version %s\n" emacs-repository-version))'
} > "$out/ORACLE"
echo "regenerated $(ls "$out"/*.eld | wc -l) fixture files from $(cat "$out/ORACLE")"
