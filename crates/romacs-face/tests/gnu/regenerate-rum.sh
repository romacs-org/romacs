#!/bin/bash
# Regenerate fixtures/rum-configured.eld, the configured Rum look.
#   ROMACS_GNU_ORACLE=/abs/path/to/emacs NOTMUCH_LISP=/abs/dir regenerate-rum.sh
# One isolated `emacs -Q --batch' at the pin (tools/lib/gnu_oracle.py), with
# throwaway HOME, no network.  NOTMUCH_LISP is the directory of the notmuch
# package's Lisp (notmuch.el, notmuch-show.el, coolj.el, ...); the fixture
# records its version.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
[ $# -eq 0 ] || { echo "regenerate-rum.sh takes no arguments" >&2; exit 2; }
[ -f "${NOTMUCH_LISP:-}/notmuch.el" ] || { echo "set NOTMUCH_LISP to notmuch's Lisp directory" >&2; exit 2; }
emacs=$(python3 "$repo/tools/lib/gnu_oracle.py" path)
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
env -i PATH=/usr/bin:/bin HOME="$work" TMPDIR="$work" LANG=C.UTF-8 timeout -s KILL 120 \
  "$emacs" -Q --batch -l "$here/rum-configured.el" \
  --eval "(rum-configured-dump \"$here/inputs\" \"$NOTMUCH_LISP\" \"$work/rum-configured.eld\")" \
  > "$work/log" 2>&1 || { tail -5 "$work/log" >&2; exit 1; }
# Every face the committed fixture has must still be there: a package that
# failed to load would shrink the comparison silently.
if [ -f "$here/fixtures/rum-configured.eld" ]; then
  lost=$(comm -23 <(grep -o '^  ([^ ]*' "$here/fixtures/rum-configured.eld" | sort) \
                  <(grep -o '^  ([^ ]*' "$work/rum-configured.eld" | sort))
  [ -z "$lost" ] || { echo "faces missing from the new capture:" >&2; echo "$lost" >&2; exit 1; }
fi
cp "$work/rum-configured.eld" "$here/fixtures/rum-configured.eld"
echo "regenerated rum-configured.eld: $(grep -c '^  (' "$here/fixtures/rum-configured.eld") faces"
