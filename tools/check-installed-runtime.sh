#!/usr/bin/env bash
# check-installed-runtime -- prove an installed share/romacs is the source's Lisp and fonts.
#
#   scripts/check-installed-runtime.sh OUT [SRC]
#
# OUT is an installed prefix (a Nix output or an unpacked release tarball:
# OUT/share/romacs/...); SRC is the source tree it was built from (default:
# this checkout). Run at the end of nix/package.nix's postInstall, so a
# package that drops or alters any runtime file does not build, and by
# scripts/ci/smoke-release on every release tarball. Written after the
# package installed 0 of the 229 lisp-bundled/ files for a week without anyone
# noticing.
#
# Checks CONTENT, not counts: every regular file under SRC/<dir> must exist
# under OUT/share/romacs/<dir> byte-for-byte, with no extra file there either
# (diff -r). The counts printed are the receipt; the diff is the verdict. An
# empty source directory fails too, so the check cannot pass vacuously.
set -euo pipefail

out=${1:?usage: check-installed-runtime.sh OUT [SRC]}
src=${2:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}
share=$out/share/romacs
status=0

for dir in gnu-lisp rlisp etc assets/fonts; do
    if [ ! -d "$src/$dir" ]; then
        echo "check-installed-runtime: source has no $dir/ under $src" >&2
        status=1; continue
    fi
    files=$(find "$src/$dir" -type f | wc -l)
    if [ "$files" -eq 0 ]; then
        echo "check-installed-runtime: source $dir/ is empty; nothing to verify" >&2
        status=1; continue
    fi
    if [ ! -d "$share/$dir" ]; then
        echo "check-installed-runtime: FAIL $dir/: 0 of $files files installed (no $share/$dir)" >&2
        status=1; continue
    fi
    if ! diff -r "$src/$dir" "$share/$dir" >&2; then
        echo "check-installed-runtime: FAIL $dir/ differs from source (diff above)" >&2
        status=1; continue
    fi
    echo "check-installed-runtime: $dir/: $files of $files files byte-identical"
done

# The wrapper is how an installed binary finds that tree (startup::lisp_dir).
# Evaluate its last ROMACS_RUNTIME_ROOT assignment with the variable unset, as
# a user's launch would, rather than trusting that the strings appear in it.
wrapper=$out/bin/romacs-vm-gui
if [ ! -x "$wrapper" ]; then
    echo "check-installed-runtime: FAIL no executable wrapper $wrapper" >&2
    status=1
elif [ "$(head -c 2 "$wrapper")" != '#!' ]; then
    # An unwrapped binary (a release tarball) finds share/romacs beside its
    # bin/ (host::runtime_root). Ask it, with no runtime variables set.
    resolved=$(env -u ROMACS_RUNTIME_ROOT -u ROMACS_LISP_DIR \
        "$wrapper" -Q --batch --eval '(princ data-directory)' || true)
    if [ "$resolved" = "$share/etc/" ]; then
        echo "check-installed-runtime: unwrapped binary resolves data-directory $resolved"
    else
        echo "check-installed-runtime: FAIL unwrapped $wrapper resolves data-directory '$resolved', not $share/etc/" >&2
        status=1
    fi
else
    assignment=$(grep -E '^export ROMACS_RUNTIME_ROOT=' "$wrapper" | tail -n 1 || true)
    resolved=$(unset ROMACS_RUNTIME_ROOT; eval "$assignment"; printf '%s' "${ROMACS_RUNTIME_ROOT-}")
    if [ -n "$assignment" ] && [ "$resolved" = "$share" ]; then
        echo "check-installed-runtime: wrapper sets ROMACS_RUNTIME_ROOT to $share"
    else
        echo "check-installed-runtime: FAIL $wrapper sets ROMACS_RUNTIME_ROOT to '${resolved}', not $share" >&2
        status=1
    fi
fi

exit "$status"
