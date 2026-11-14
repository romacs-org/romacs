#!/bin/bash
# Regenerate the GNU oracle fixtures for romacs-face.
#   ROMACS_GNU_ORACLE=/abs/path/to/emacs regenerate.sh
# One isolated `emacs -Q --batch' per theme, throwaway HOME, no network.
# The oracle is GNU built at the pin (ledger/parity-reference.toml), named by
# $ROMACS_GNU_ORACLE and refused by tools/lib/gnu_oracle.py unless the binary
# itself reports the pin. There is no default: /usr/bin/emacs is 320 GNU
# commits older and produced the separator-line mismatch (R228).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
[ $# -eq 0 ] || { echo "regenerate.sh takes no arguments; set ROMACS_GNU_ORACLE" >&2; exit 2; }
emacs=$(python3 "$repo/tools/lib/gnu_oracle.py" path)
out="$here/fixtures"; mkdir -p "$out"
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
# The face model's evidence is GNU's own themes, which Romacs does not ship:
# they come from the oracle's own data directory.
sandbox() {
  h=$(mktemp -d "$work/home.XXXX")
  env -i PATH=/usr/bin:/bin HOME="$h" TMPDIR="$h" LANG=C.UTF-8 "$@"
}
theme_dir=$(sandbox "$emacs" -Q --batch --eval '(princ (expand-file-name "themes" data-directory))')
themes=$(find "$theme_dir" -name '*-theme.el' -printf '%f\n' | sed 's/-theme\.el$//' | sort)
[ -n "$themes" ] || { echo "no themes under $theme_dir" >&2; exit 1; }
# dump THEME DIR [FORM]: FORM runs before the theme loads.
dump() {
  q=$1; [ "$1" = nil ] || q="'$1"
  sandbox timeout -s KILL 120 "$emacs" -Q --batch -l "$here/dump.el" \
    --eval "${3:-nil}" --eval "(s86-dump $q \"$2\" \"$work/$1.eld\")" > "$work/$1.log" 2>&1 \
    || { echo "$1 failed:" >&2; tail -5 "$work/$1.log" >&2; exit 1; }
  cp "$work/$1.eld" "$out/$1.eld"
}
for t in nil $themes; do dump "$t" "$theme_dir"; done
# Rum beside them: its theme file and the Modus it is written for, with the
# colours Omarchy gives it.  It is the one dump with palette overrides.
rum=$work/rum-themes; mkdir "$rum"
cp "$here/../../../../etc/modus/modus-themes.el" "$rum/"
cp "$here/inputs/rum-emacs.el" "$rum/rum-theme.el"
dump rum "$rum" '(setq omarchy-color-bg "#000000" omarchy-color-fg "#d4c5a0" omarchy-color-bright-black "#756b59" omarchy-color-accent "#d48a20")'
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
